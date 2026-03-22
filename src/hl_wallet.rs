//! Per-user Hyperliquid wallet management.
//!
//! Each consumer tenant gets a dedicated secp256k1 keypair at onboarding.
//! This wallet is separate from their Privy authentication identity and is
//! the address they deposit USDC into before the bot can trade.
//!
//! ## Security model
//!
//! - Private keys are **never** stored in plaintext.
//! - The encrypted form (`nonce_hex:ciphertext_hex`) uses AES-256-GCM keyed
//!   from `SHA-256(SESSION_SECRET || "hl-wallet:" || tenant_id)`.
//! - The decrypted key is only materialised in memory when the user explicitly
//!   requests an export from `/api/hl/wallet/export-key`.
//! - The key is shown once in plaintext on the `/app/setup` page immediately
//!   after generation, then only accessible via the authenticated export route.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{anyhow, Result};
use k256::ecdsa::SigningKey;
use rand::RngCore;
use sha2::{Digest, Sha256};
use sha3::Keccak256;

// ─────────────────────────────────────────────────────────────────────────────

/// Generate a fresh Ethereum / Hyperliquid keypair.
///
/// Returns `(checksum_address, private_key_0x_hex)`.
/// Uses `OsRng` for entropy — cryptographically secure.
pub fn generate_keypair() -> (String, String) {
    // 32 bytes of OS-level randomness → secp256k1 signing key
    let mut key_bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut key_bytes);
    let signing_key = SigningKey::from_slice(&key_bytes)
        .expect("32 random bytes always produce a valid secp256k1 key");

    // Uncompressed public key: 0x04 || x || y (65 bytes)
    let verifying_key = signing_key.verifying_key();
    let uncompressed = verifying_key.to_encoded_point(false);
    let pubkey_bytes = &uncompressed.as_bytes()[1..]; // drop the 0x04 prefix

    // Ethereum address = last 20 bytes of Keccak256(pubkey)
    let mut keccak = Keccak256::new();
    keccak.update(pubkey_bytes);
    let hash = keccak.finalize();
    let addr_bytes = &hash[12..];

    let address = eip55_checksum(addr_bytes);
    let private_key = format!("0x{}", hex::encode(key_bytes));

    (address, private_key)
}

/// Apply EIP-55 mixed-case checksum to a raw 20-byte address slice.
fn eip55_checksum(addr_bytes: &[u8]) -> String {
    let hex_lower = hex::encode(addr_bytes);

    let mut keccak = Keccak256::new();
    keccak.update(hex_lower.as_bytes());
    let hash = keccak.finalize();

    let checksum: String = hex_lower
        .chars()
        .enumerate()
        .map(|(i, c)| {
            if c.is_ascii_digit() {
                c
            } else {
                // Each hex char corresponds to a nibble in the hash
                let byte = hash[i / 2];
                let nibble = if i % 2 == 0 { byte >> 4 } else { byte & 0xf };
                if nibble >= 8 {
                    c.to_ascii_uppercase()
                } else {
                    c
                }
            }
        })
        .collect();

    format!("0x{}", checksum)
}

// ─────────────────────────────────────────────────────────────────────────────

/// Encrypt a private key string with AES-256-GCM.
///
/// The encryption key is derived from `SHA-256(session_secret || "hl-wallet:" || tenant_id)`.
/// A fresh 12-byte nonce is generated per call.
///
/// Returns `"<nonce_hex>:<ciphertext_hex>"` — safe to store in the DB.
pub fn encrypt_key(private_key: &str, session_secret: &str, tenant_id: &str) -> String {
    let key_bytes = derive_enc_key(session_secret, tenant_id);
    let cipher = Aes256Gcm::new_from_slice(&key_bytes).expect("32-byte key");

    let mut nonce_bytes = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, private_key.as_bytes())
        .expect("AES-256-GCM encryption is infallible for valid inputs");

    format!("{}:{}", hex::encode(nonce_bytes), hex::encode(ciphertext))
}

/// Decrypt a private key string encrypted by [`encrypt_key`].
pub fn decrypt_key(enc: &str, session_secret: &str, tenant_id: &str) -> Result<String> {
    let mut parts = enc.splitn(2, ':');
    let nonce_hex = parts
        .next()
        .ok_or_else(|| anyhow!("malformed enc: missing nonce"))?;
    let cipher_hex = parts
        .next()
        .ok_or_else(|| anyhow!("malformed enc: missing ciphertext"))?;

    let nonce_bytes = hex::decode(nonce_hex)?;
    let ciphertext = hex::decode(cipher_hex)?;

    let key_bytes = derive_enc_key(session_secret, tenant_id);
    let cipher = Aes256Gcm::new_from_slice(&key_bytes).expect("32-byte key");
    let nonce = Nonce::from_slice(&nonce_bytes);

    let plaintext = cipher
        .decrypt(nonce, ciphertext.as_ref())
        .map_err(|_| anyhow!("AES-256-GCM decryption failed — wrong key or corrupted data"))?;

    String::from_utf8(plaintext).map_err(Into::into)
}

/// Derive the 32-byte AES key from `SESSION_SECRET` and `tenant_id`.
fn derive_enc_key(session_secret: &str, tenant_id: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(session_secret.as_bytes());
    hasher.update(b"hl-wallet:");
    hasher.update(tenant_id.as_bytes());
    hasher.finalize().into()
}

// ─────────────────────────────────────────────────────────────────────────────

/// Check the cleared USD balance of a Hyperliquid address.
///
/// Calls the public HL info API — no authentication required.
/// Returns 0.0 if the account does not exist yet or on any error.
pub async fn check_balance(address: &str) -> f64 {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .unwrap_or_default();

    let body = serde_json::json!({
        "type": "clearinghouseState",
        "user": address
    });

    let response = match client
        .post("https://api.hyperliquid.xyz/info")
        .json(&body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => return 0.0,
    };

    let json: serde_json::Value = match response.json().await {
        Ok(j) => j,
        Err(_) => return 0.0,
    };

    // "accountValue" is a string like "1234.56" in the HL response
    json["marginSummary"]["accountValue"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0)
}

/// Register the platform referral code on a newly created Hyperliquid account.
///
/// This is fire-and-forget — a failure is logged but does not block onboarding.
/// The action must be signed by the *user's* trading wallet private key.
#[allow(dead_code)]
pub async fn register_referral(
    _wallet_address: &str,
    _private_key_hex: &str,
    _referral_code: &str,
) {
    // TODO: implement EIP-712 signed `setReferrer` action against
    //       POST https://api.hyperliquid.xyz/exchange
    // The signature uses the same HL action-hash scheme already implemented
    // in src/exchange.rs.  Wire it up once the exchange module exposes a
    // generic sign_and_post() helper.
    log::debug!("hl_wallet: referral registration is a no-op until sign_and_post is refactored");
}

// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keypair_roundtrip_address_format() {
        let (addr, priv_key) = generate_keypair();
        assert!(addr.starts_with("0x"), "address must start with 0x");
        assert_eq!(addr.len(), 42, "address must be 42 chars (0x + 40 hex)");
        assert!(priv_key.starts_with("0x"), "private key must start with 0x");
        assert_eq!(
            priv_key.len(),
            66,
            "private key must be 66 chars (0x + 64 hex)"
        );
    }

    #[test]
    fn keypair_unique() {
        let (a1, _) = generate_keypair();
        let (a2, _) = generate_keypair();
        assert_ne!(a1, a2, "two keypairs must never collide");
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let (_, private_key) = generate_keypair();
        let secret = "test-session-secret-32-chars-long";
        let tid = "00000000-0000-0000-0000-000000000001";

        let enc = encrypt_key(&private_key, secret, tid);
        assert!(
            enc.contains(':'),
            "enc must contain nonce:ciphertext separator"
        );

        let dec = decrypt_key(&enc, secret, tid).expect("decryption must succeed");
        assert_eq!(dec, private_key);
    }

    #[test]
    fn decrypt_fails_wrong_tenant() {
        let (_, private_key) = generate_keypair();
        let secret = "test-session-secret-32-chars-long";

        let enc = encrypt_key(&private_key, secret, "tenant-a");
        assert!(
            decrypt_key(&enc, secret, "tenant-b").is_err(),
            "decryption with wrong tenant_id must fail"
        );
    }

    #[test]
    fn eip55_known_address() {
        // Known EIP-55 checksum from the Ethereum spec
        let addr_bytes = hex::decode(
            "5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed"
                .to_lowercase()
                .trim_start_matches("0x"),
        )
        .unwrap();
        let result = eip55_checksum(&addr_bytes);
        assert_eq!(result, "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed");
    }
}
