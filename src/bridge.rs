//! Hyperliquid ↔ Arbitrum bridge — real on-chain and API implementation.
//!
//! # Two directions
//!
//! ## HL → Arbitrum (withdrawal)
//!
//! Uses Hyperliquid's `withdraw3` exchange action.  The action is signed with
//! the tenant's HL private key using a dedicated EIP-712 domain (`HyperliquidX`)
//! that is different from the order-signing domain (`Exchange`).
//!
//! Flow:
//! 1. Build `withdraw3` action JSON with amount, destination, timestamp.
//! 2. EIP-712 sign with tenant key (chainId = Arbitrum One = 42161 = `0xa4b1`).
//! 3. POST to `https://api.hyperliquid.xyz/exchange`.
//! 4. HL processes within ~30 s; USDC appears on Arbitrum.
//!
//! Reference: Hyperliquid Python SDK `sign_withdraw_from_bridge_action`.
//!
//! ## Arbitrum → HL (deposit)
//!
//! Uses Hyperliquid's bridge contract on Arbitrum One.  The platform wallet
//! (or the user's wallet, depending on custody model) sends USDC to the bridge.
//!
//! Flow:
//! 1. ERC-20 `approve(HL_BRIDGE_ADDRESS_ARB, amount)` on Arbitrum USDC.
//! 2. `IBridge.sendDeposit(hl_destination, amount)` on the HL bridge contract.
//! 3. Both transactions are signed with raw EIP-1559 format using the
//!    existing `k256` + `sha3` infrastructure (no `alloy`/`ethers` dependency).
//! 4. Broadcast via Arbitrum JSON-RPC (`ARBITRUM_RPC_URL` env var).
//! 5. Poll for HL credit (balance increase on HL account).
//!
//! # Contract addresses (Arbitrum One)
//!
//! | Name            | Address                                      | Source        |
//! |-----------------|----------------------------------------------|---------------|
//! | HL Bridge       | `HL_BRIDGE_ADDRESS_ARB` env var              | HL docs        |
//! | Native USDC     | `0xaf88d065e77c8cC2239327C5EDb3A432268e5831` | Circle/Arbitrum|
//! | USDC.e (bridged)| `0xFF970A61A04b1cA14834A43f5dE4533eBDDB5CC8` | Arbitrum bridge|
//!
//! Set `HL_BRIDGE_ADDRESS_ARB` from: https://hyperliquid.gitbook.io/hyperliquid-docs/onboarding/how-to-transfer-usdc-from-arbitrum-to-hyperliquid
//!
//! # Environment variables
//!
//! | Variable              | Required | Description                              |
//! |-----------------------|----------|------------------------------------------|
//! | `HL_BRIDGE_ADDRESS_ARB` | Deposit | HL bridge contract on Arbitrum One      |
//! | `ARBITRUM_RPC_URL`    | Deposit  | Arbitrum JSON-RPC endpoint (Alchemy etc.)|
//! | `ARBITRUM_USDC`       | Optional | USDC contract (default: native USDC)     |
//! | `ARB_PRIVATE_KEY`     | Deposit  | Platform wallet key for on-chain txs     |

use crate::exchange::HyperliquidClient;
use crate::fund_tracker::{self, EventType, FundEvent};
use crate::tenant::TenantId;
use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

// ─────────────────────────── Constants ───────────────────────────────────────

/// Arbitrum One chain ID (42161 = 0xa4b1).
const ARBITRUM_CHAIN_ID: u64 = 42161;

/// Native USDC on Arbitrum One (Circle's official contract).
#[allow(dead_code)]
const DEFAULT_ARB_USDC: &str = "0xaf88d065e77c8cC2239327C5EDb3A432268e5831";

/// How many seconds to wait between polling HL for balance after a deposit.
#[allow(dead_code)]
const DEPOSIT_POLL_INTERVAL_SECS: u64 = 15;

/// How many poll attempts before giving up on deposit confirmation.
#[allow(dead_code)]
const DEPOSIT_MAX_POLLS: u32 = 20; // 20 × 15s = 5 minutes

// ─────────────────────────── EIP-712 signatures ──────────────────────────────

/// EIP-712 signature `{r, s, v}`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HlSignature {
    pub r: String,
    pub s: String,
    pub v: u8,
}

/// Sign a Hyperliquid `withdraw3` action using the `HyperliquidX` EIP-712 domain.
///
/// This is a **different** domain from order signing (`Exchange` domain).
/// Mirrors `sign_withdraw_from_bridge_action` in the Hyperliquid Python SDK.
///
/// Domain:
/// ```text
/// {
///   name: "HyperliquidX",
///   version: "1",
///   chainId: signatureChainId,   // 42161 for Arbitrum One
///   verifyingContract: 0x0...0
/// }
/// ```
///
/// Type:
/// ```text
/// HyperliquidTransaction:Withdraw(
///   string hyperliquidChain,
///   string destination,
///   string amount,
///   uint64 time
/// )
/// ```
fn sign_withdraw3(
    destination: &str,
    amount_str: &str,
    time_ms: u64,
    signature_chain_id: u64,
    key: &[u8],
) -> Result<HlSignature> {
    use k256::ecdsa::{signature::hazmat::PrehashSigner, SigningKey};
    use sha3::{Digest, Keccak256};

    // ── Domain type hash ──────────────────────────────────────────────────────
    let domain_type_hash: [u8; 32] = Keccak256::digest(
        b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    )
    .into();

    let tx_type_hash: [u8; 32] = Keccak256::digest(
        b"HyperliquidTransaction:Withdraw(\
          string hyperliquidChain,\
          string destination,\
          string amount,\
          uint64 time)",
    )
    .into();

    // ── Domain separator ──────────────────────────────────────────────────────
    let name_hash: [u8; 32] = Keccak256::digest(b"HyperliquidX").into();
    let version_hash: [u8; 32] = Keccak256::digest(b"1").into();

    // chainId as 32-byte big-endian
    let mut chain_id_bytes = [0u8; 32];
    let be = signature_chain_id.to_be_bytes();
    chain_id_bytes[24..32].copy_from_slice(&be);

    let zero_address = [0u8; 32]; // verifyingContract = 0x0

    let mut domain_enc = [0u8; 160]; // 5 × 32
    domain_enc[0..32].copy_from_slice(&domain_type_hash);
    domain_enc[32..64].copy_from_slice(&name_hash);
    domain_enc[64..96].copy_from_slice(&version_hash);
    domain_enc[96..128].copy_from_slice(&chain_id_bytes);
    domain_enc[128..160].copy_from_slice(&zero_address);
    let domain_sep: [u8; 32] = Keccak256::digest(domain_enc).into();

    // ── Struct hash ───────────────────────────────────────────────────────────
    let hl_chain_hash: [u8; 32] = Keccak256::digest(b"Mainnet").into();
    let dest_hash: [u8; 32] = Keccak256::digest(destination.as_bytes()).into();
    let amount_hash: [u8; 32] = Keccak256::digest(amount_str.as_bytes()).into();

    // uint64 time — right-aligned in 32 bytes
    let mut time_bytes = [0u8; 32];
    time_bytes[24..32].copy_from_slice(&time_ms.to_be_bytes());

    let mut struct_enc = [0u8; 160]; // 5 × 32
    struct_enc[0..32].copy_from_slice(&tx_type_hash);
    struct_enc[32..64].copy_from_slice(&hl_chain_hash);
    struct_enc[64..96].copy_from_slice(&dest_hash);
    struct_enc[96..128].copy_from_slice(&amount_hash);
    struct_enc[128..160].copy_from_slice(&time_bytes);
    let struct_hash: [u8; 32] = Keccak256::digest(struct_enc).into();

    // ── Final EIP-712 hash ────────────────────────────────────────────────────
    let mut final_msg = Vec::with_capacity(66);
    final_msg.extend_from_slice(b"\x19\x01");
    final_msg.extend_from_slice(&domain_sep);
    final_msg.extend_from_slice(&struct_hash);
    let final_hash: [u8; 32] = Keccak256::digest(&final_msg).into();

    // ── secp256k1 sign ────────────────────────────────────────────────────────
    let signing_key =
        SigningKey::from_bytes(key.into()).map_err(|e| anyhow!("Invalid private key: {}", e))?;
    let (sig, recid): (k256::ecdsa::Signature, k256::ecdsa::RecoveryId) = signing_key
        .sign_prehash(&final_hash)
        .map_err(|e| anyhow!("Signing failed: {}", e))?;

    let sig_bytes = sig.to_bytes();
    Ok(HlSignature {
        r: format!("0x{}", hex::encode(&sig_bytes[..32])),
        s: format!("0x{}", hex::encode(&sig_bytes[32..])),
        v: recid.to_byte() + 27,
    })
}

// ─────────────────────────── Arbitrum JSON-RPC helpers ───────────────────────

/// Make a raw JSON-RPC call to the Arbitrum RPC endpoint.
#[allow(dead_code)]
async fn arb_rpc(rpc_url: &str, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id":      1,
        "method":  method,
        "params":  params,
    });
    let resp = reqwest::Client::new()
        .post(rpc_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow!("Arbitrum RPC request failed: {}", e))?;
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| anyhow!("Arbitrum RPC response parse: {}", e))?;
    if let Some(err) = json.get("error") {
        return Err(anyhow!("Arbitrum RPC error: {}", err));
    }
    Ok(json["result"].clone())
}

/// Encode a 4-byte function selector + 32-byte argument (simple ABI encode).
/// Sufficient for `approve(address spender, uint256 amount)` and
/// `sendDeposit(address destination, uint64 amount)`.
#[allow(dead_code)]
fn abi_encode_2(selector: [u8; 4], arg1: [u8; 32], arg2: [u8; 32]) -> Vec<u8> {
    let mut data = Vec::with_capacity(68);
    data.extend_from_slice(&selector);
    data.extend_from_slice(&arg1);
    data.extend_from_slice(&arg2);
    data
}

/// Address string `"0x..."` → 20-byte array, right-aligned in 32 bytes.
#[allow(dead_code)]
fn addr_to_word(addr: &str) -> Result<[u8; 32]> {
    let stripped = addr.trim_start_matches("0x");
    if stripped.len() != 40 {
        return Err(anyhow!("Invalid address length: {}", addr));
    }
    let bytes = hex::decode(stripped).map_err(|e| anyhow!("Address hex decode: {}", e))?;
    let mut word = [0u8; 32];
    word[12..32].copy_from_slice(&bytes);
    Ok(word)
}

/// USDC amount (f64 USD) → 32-byte word (uint256, 6 decimals).
#[allow(dead_code)]
fn usdc_to_word(amount_usd: f64) -> [u8; 32] {
    let units = (amount_usd * 1_000_000.0) as u64; // 6 decimal places
    let mut word = [0u8; 32];
    word[24..32].copy_from_slice(&units.to_be_bytes());
    word
}

/// Minimal RLP encoding for an EIP-1559 transaction.
/// Only supports the fields we use; sufficient for simple contract calls.
#[allow(dead_code)]
fn rlp_encode_u64(v: u64) -> Vec<u8> {
    if v == 0 {
        return vec![0x80]; // empty string (zero)
    }
    let bytes = v.to_be_bytes();
    let trimmed: &[u8] = {
        let start = bytes.iter().position(|&b| b != 0).unwrap_or(7);
        &bytes[start..]
    };
    rlp_encode_bytes(trimmed)
}

#[allow(dead_code)]
fn rlp_encode_bytes(data: &[u8]) -> Vec<u8> {
    if data.len() == 1 && data[0] < 0x80 {
        return vec![data[0]];
    }
    let mut result = Vec::new();
    if data.len() < 56 {
        result.push(0x80 + data.len() as u8);
    } else {
        let len_bytes = data.len().to_be_bytes();
        let trimmed_len: Vec<u8> = len_bytes
            .iter()
            .skip_while(|&&b| b == 0)
            .copied()
            .collect();
        result.push(0xb7 + trimmed_len.len() as u8);
        result.extend_from_slice(&trimmed_len);
    }
    result.extend_from_slice(data);
    result
}

#[allow(dead_code)]
fn rlp_encode_list(items: &[Vec<u8>]) -> Vec<u8> {
    let payload: Vec<u8> = items.iter().flat_map(|i| i.iter().copied()).collect();
    let mut result = Vec::new();
    if payload.len() < 56 {
        result.push(0xc0 + payload.len() as u8);
    } else {
        let len_bytes = payload.len().to_be_bytes();
        let trimmed_len: Vec<u8> = len_bytes
            .iter()
            .skip_while(|&&b| b == 0)
            .copied()
            .collect();
        result.push(0xf7 + trimmed_len.len() as u8);
        result.extend_from_slice(&trimmed_len);
    }
    result.extend_from_slice(&payload);
    result
}

/// Build, sign, and broadcast one EIP-1559 contract call on Arbitrum.
/// Returns the transaction hash.
#[allow(dead_code)]
async fn send_arb_tx(
    rpc_url: &str,
    private_key: &[u8],
    to: &str,
    data: Vec<u8>,
    value: u64,
) -> Result<String> {
    use k256::ecdsa::{signature::hazmat::PrehashSigner, SigningKey};
    use sha3::{Digest, Keccak256};

    // ── Derive from-address ───────────────────────────────────────────────────
    let signing_key =
        SigningKey::from_bytes(private_key.into()).map_err(|e| anyhow!("Bad key: {}", e))?;
    let verifying_key = signing_key.verifying_key();
    let pubkey_bytes = verifying_key.to_encoded_point(false);
    let pubkey_uncompressed = pubkey_bytes.as_bytes(); // 65 bytes: 0x04 || x || y
    let addr_hash: [u8; 32] = Keccak256::digest(&pubkey_uncompressed[1..]).into(); // skip 0x04
    let from_addr = format!("0x{}", hex::encode(&addr_hash[12..]));

    // ── Fetch nonce ───────────────────────────────────────────────────────────
    let nonce_result = arb_rpc(
        rpc_url,
        "eth_getTransactionCount",
        serde_json::json!([from_addr, "latest"]),
    )
    .await?;
    let nonce_hex = nonce_result.as_str().unwrap_or("0x0");
    let nonce = u64::from_str_radix(nonce_hex.trim_start_matches("0x"), 16).unwrap_or(0);

    // ── Fetch gas price ───────────────────────────────────────────────────────
    let fee_result = arb_rpc(rpc_url, "eth_maxPriorityFeePerGas", serde_json::json!([])).await?;
    let priority_fee_hex = fee_result.as_str().unwrap_or("0x3B9ACA00"); // 1 gwei default
    let priority_fee =
        u64::from_str_radix(priority_fee_hex.trim_start_matches("0x"), 16).unwrap_or(1_000_000_000);
    let max_fee = priority_fee * 2; // simple heuristic

    // ── Estimate gas ──────────────────────────────────────────────────────────
    let gas_est = arb_rpc(
        rpc_url,
        "eth_estimateGas",
        serde_json::json!([{
            "from": from_addr,
            "to": to,
            "data": format!("0x{}", hex::encode(&data)),
            "value": format!("0x{:x}", value),
        }]),
    )
    .await
    .ok()
    .and_then(|v| v.as_str().and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok()))
    .unwrap_or(200_000);
    let gas_limit = (gas_est as f64 * 1.2) as u64; // 20% buffer

    // ── RLP-encode unsigned EIP-1559 tx ──────────────────────────────────────
    let to_bytes = hex::decode(to.trim_start_matches("0x"))
        .map_err(|e| anyhow!("Bad 'to' address: {}", e))?;
    let access_list_rlp = rlp_encode_list(&[]); // empty access list

    let chain_id_rlp = rlp_encode_u64(ARBITRUM_CHAIN_ID);
    let nonce_rlp = rlp_encode_u64(nonce);
    let max_priority_rlp = rlp_encode_u64(priority_fee);
    let max_fee_rlp = rlp_encode_u64(max_fee);
    let gas_rlp = rlp_encode_u64(gas_limit);
    let to_rlp = rlp_encode_bytes(&to_bytes);
    let value_rlp = rlp_encode_u64(value);
    let data_rlp = rlp_encode_bytes(&data);

    let fields = vec![
        chain_id_rlp,
        nonce_rlp,
        max_priority_rlp.clone(),
        max_fee_rlp.clone(),
        gas_rlp.clone(),
        to_rlp.clone(),
        value_rlp.clone(),
        data_rlp.clone(),
        access_list_rlp,
    ];
    let unsigned_rlp = rlp_encode_list(&fields);

    // sigHash = keccak256(0x02 || RLP(fields))
    let mut sig_input = Vec::with_capacity(unsigned_rlp.len() + 1);
    sig_input.push(0x02); // EIP-1559 type
    sig_input.extend_from_slice(&unsigned_rlp);
    let sig_hash: [u8; 32] = Keccak256::digest(&sig_input).into();

    // ── Sign ──────────────────────────────────────────────────────────────────
    let (sig, recid): (k256::ecdsa::Signature, k256::ecdsa::RecoveryId) = signing_key
        .sign_prehash(&sig_hash)
        .map_err(|e| anyhow!("Signing failed: {}", e))?;
    let sig_bytes = sig.to_bytes();
    let v_bit = recid.to_byte(); // 0 or 1 for EIP-1559

    // ── RLP-encode signed tx ──────────────────────────────────────────────────
    let signed_fields = vec![
        rlp_encode_u64(ARBITRUM_CHAIN_ID),
        rlp_encode_u64(nonce),
        max_priority_rlp,
        max_fee_rlp,
        gas_rlp,
        to_rlp,
        value_rlp,
        data_rlp,
        rlp_encode_list(&[]),      // empty access list
        rlp_encode_u64(v_bit as u64), // yParity
        rlp_encode_bytes(&sig_bytes[..32]),  // r
        rlp_encode_bytes(&sig_bytes[32..]),  // s
    ];
    let signed_rlp = rlp_encode_list(&signed_fields);

    let mut raw_tx = Vec::with_capacity(signed_rlp.len() + 1);
    raw_tx.push(0x02); // EIP-1559 type
    raw_tx.extend_from_slice(&signed_rlp);

    let raw_hex = format!("0x{}", hex::encode(&raw_tx));

    // ── Broadcast ────────────────────────────────────────────────────────────
    let tx_hash_result = arb_rpc(
        rpc_url,
        "eth_sendRawTransaction",
        serde_json::json!([raw_hex]),
    )
    .await?;
    let tx_hash = tx_hash_result
        .as_str()
        .ok_or_else(|| anyhow!("No tx hash in response"))?
        .to_string();

    Ok(tx_hash)
}

// ─────────────────────────── Bridge manager ──────────────────────────────────

/// Automated bridge controller for Hyperliquid ↔ Arbitrum transfers.
#[derive(Clone)]
pub struct BridgeManager {
    hl: Arc<HyperliquidClient>,
    min_withdraw_usd: f64,
    trusted_destinations: Vec<String>,
    records: Arc<Mutex<HashMap<String, BridgeRequestRecord>>>,
}

impl BridgeManager {
    pub fn new(
        hl: Arc<HyperliquidClient>,
        min_withdraw_usd: f64,
        trusted_destinations: Vec<String>,
    ) -> Self {
        Self {
            hl,
            min_withdraw_usd,
            trusted_destinations,
            records: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Initiate a withdrawal from Hyperliquid to an Arbitrum address.
    ///
    /// This is the primary user-facing action: take profits from HL trading
    /// and bridge them to the user's Arbitrum wallet.
    pub async fn request_withdrawal(
        &self,
        tenant_id: &TenantId,
        amount_usd: f64,
        destination: &str,
    ) -> Result<BridgeRequestRecord> {
        if amount_usd < self.min_withdraw_usd {
            return Err(anyhow!(
                "Amount ${:.2} is below the minimum withdrawal ${:.2}",
                amount_usd,
                self.min_withdraw_usd
            ));
        }

        if !self.validate_destination(destination) {
            return Err(anyhow!("Destination {} is not trusted", destination));
        }

        // Bridge balance checks don't enforce trading limits — pass permissive sentinels.
        let account = self.hl.get_account(f64::MAX, 0.0).await?;
        if amount_usd > account.equity {
            return Err(anyhow!(
                "Requested ${:.2} exceeds available equity ${:.2}",
                amount_usd,
                account.equity
            ));
        }

        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let record = BridgeRequestRecord {
            id: id.clone(),
            tenant_id: tenant_id.clone(),
            amount_usd,
            destination: destination.to_string(),
            status: BridgeStatus::Initiated,
            status_reason: Some("withdrawal queued".to_string()),
            tx_hash: None,
            created_at: now,
            updated_at: now,
        };

        {
            let mut lock = self.records.lock().await;
            lock.insert(id.clone(), record.clone());
        }

        let bridge = self.clone();
        let tenant = tenant_id.clone();
        let dest = destination.to_string();
        tokio::spawn(async move {
            if let Err(e) = bridge
                .process_withdrawal(id.clone(), tenant, amount_usd, dest)
                .await
            {
                log::error!("Bridge withdrawal {} failed: {}", id, e);
            }
        });

        Ok(record)
    }

    /// Initiate a deposit from Arbitrum to Hyperliquid.
    ///
    /// Requires `ARB_PRIVATE_KEY`, `ARBITRUM_RPC_URL`, and
    /// `HL_BRIDGE_ADDRESS_ARB` environment variables to be set.
    #[allow(dead_code)]
    pub async fn request_deposit(
        &self,
        tenant_id: &TenantId,
        amount_usd: f64,
        hl_destination: &str,
    ) -> Result<BridgeRequestRecord> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let record = BridgeRequestRecord {
            id: id.clone(),
            tenant_id: tenant_id.clone(),
            amount_usd,
            destination: hl_destination.to_string(),
            status: BridgeStatus::Initiated,
            status_reason: Some("deposit queued — Arbitrum → HL".to_string()),
            tx_hash: None,
            created_at: now,
            updated_at: now,
        };

        {
            let mut lock = self.records.lock().await;
            lock.insert(id.clone(), record.clone());
        }

        let bridge = self.clone();
        let tenant = tenant_id.clone();
        let hl_dest = hl_destination.to_string();
        tokio::spawn(async move {
            if let Err(e) = bridge
                .process_deposit(id.clone(), tenant, amount_usd, hl_dest)
                .await
            {
                log::error!("Bridge deposit {} failed: {}", id, e);
            }
        });

        Ok(record)
    }

    pub async fn fetch_record(&self, id: &str) -> Option<BridgeRequestRecord> {
        let lock = self.records.lock().await;
        lock.get(id).cloned()
    }

    // ── HL → Arbitrum (real implementation) ──────────────────────────────────

    async fn process_withdrawal(
        &self,
        id: String,
        tenant_id: TenantId,
        amount_usd: f64,
        destination: String,
    ) -> Result<()> {
        self.set_status(
            &id,
            BridgeStatus::Pending,
            Some("signing withdraw3 action".to_string()),
            None,
        )
        .await;

        // Load the tenant's HL private key.
        // In production this comes from the encrypted key store.
        let private_key_hex =
            std::env::var("HYPERLIQUID_SECRET").unwrap_or_default();
        if private_key_hex.is_empty() {
            self.set_status(
                &id,
                BridgeStatus::Failed,
                Some("HYPERLIQUID_SECRET not configured".to_string()),
                None,
            )
            .await;
            return Err(anyhow!("HYPERLIQUID_SECRET not configured for withdrawal {}", id));
        }

        let key_bytes = hex::decode(private_key_hex.trim_start_matches("0x"))
            .map_err(|e| anyhow!("Invalid HYPERLIQUID_SECRET hex: {}", e))?;

        // Amount as string with up to 8 decimal places (HL API requirement)
        let amount_str = format!("{:.8}", amount_usd)
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string();
        // HL requires at least one decimal place
        let amount_str = if amount_str.contains('.') {
            amount_str
        } else {
            format!("{}.0", amount_str)
        };

        let time_ms = chrono::Utc::now().timestamp_millis() as u64;

        // Sign with HyperliquidX domain, Arbitrum chainId
        let sig = sign_withdraw3(
            &destination,
            &amount_str,
            time_ms,
            ARBITRUM_CHAIN_ID,
            &key_bytes,
        )?;

        // Build and POST the withdraw3 action
        let base_url = if std::env::var("HL_TESTNET").unwrap_or_default() == "true" {
            "https://api.hyperliquid-testnet.xyz"
        } else {
            "https://api.hyperliquid.xyz"
        };

        let body = serde_json::json!({
            "action": {
                "type": "withdraw3",
                "hyperliquidChain": "Mainnet",
                "signatureChainId": format!("0x{:x}", ARBITRUM_CHAIN_ID),
                "destination": destination,
                "amount": amount_str,
                "time": time_ms
            },
            "nonce": time_ms,
            "signature": sig
        });

        self.set_status(
            &id,
            BridgeStatus::Pending,
            Some("submitting to Hyperliquid API".to_string()),
            None,
        )
        .await;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/exchange", base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("HL withdraw3 POST failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            self.set_status(
                &id,
                BridgeStatus::Failed,
                Some(format!("HL API error HTTP {}: {}", status, text)),
                None,
            )
            .await;
            return Err(anyhow!("HL withdraw3 rejected: {} — {}", status, text));
        }

        let result: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| anyhow!("withdraw3 response parse: {}", e))?;

        // HL returns {"status":"ok"} or {"status":"err","response":"..."}
        if result.get("status").and_then(|s| s.as_str()) == Some("err") {
            let reason = result
                .get("response")
                .and_then(|r| r.as_str())
                .unwrap_or("unknown error")
                .to_string();
            self.set_status(
                &id,
                BridgeStatus::Failed,
                Some(format!("HL rejected withdrawal: {}", reason)),
                None,
            )
            .await;
            return Err(anyhow!("HL withdrawal rejected: {}", reason));
        }

        let balance = self.hl.get_account(f64::MAX, 0.0).await.map(|a| a.equity).unwrap_or(0.0);
        self.set_status(
            &id,
            BridgeStatus::Completed,
            Some(format!(
                "withdrawal submitted to Hyperliquid — USDC will appear on Arbitrum in ~30s. Remaining equity: ${:.2}",
                balance - amount_usd
            )),
            None,
        )
        .await;

        let event = FundEvent {
            event_type: EventType::Withdrawal,
            amount_usd,
            balance_after: (balance - amount_usd).max(0.0),
            timestamp: Utc::now().to_rfc3339(),
        };
        if let Err(e) = fund_tracker::append(&tenant_id, &event) {
            log::warn!("Bridge fund event append failed: {}", e);
        }

        log::info!(
            "Bridge {}: withdraw3 submitted — ${:.2} USDC → {} (Arbitrum)",
            id, amount_usd, destination
        );

        Ok(())
    }

    // ── Arbitrum → HL (real implementation) ──────────────────────────────────

    #[allow(dead_code)]
    async fn process_deposit(
        &self,
        id: String,
        tenant_id: TenantId,
        amount_usd: f64,
        hl_destination: String,
    ) -> Result<()> {
        let rpc_url = std::env::var("ARBITRUM_RPC_URL").unwrap_or_default();
        let bridge_addr = std::env::var("HL_BRIDGE_ADDRESS_ARB").unwrap_or_default();
        let private_key_hex = std::env::var("ARB_PRIVATE_KEY").unwrap_or_default();
        let usdc_addr = std::env::var("ARBITRUM_USDC")
            .unwrap_or_else(|_| DEFAULT_ARB_USDC.to_string());

        if rpc_url.is_empty() || bridge_addr.is_empty() || private_key_hex.is_empty() {
            self.set_status(
                &id,
                BridgeStatus::Failed,
                Some("Missing ARBITRUM_RPC_URL, HL_BRIDGE_ADDRESS_ARB, or ARB_PRIVATE_KEY".to_string()),
                None,
            )
            .await;
            return Err(anyhow!("Deposit env vars not configured for {}", id));
        }

        let key_bytes = hex::decode(private_key_hex.trim_start_matches("0x"))
            .map_err(|e| anyhow!("Invalid ARB_PRIVATE_KEY hex: {}", e))?;

        self.set_status(
            &id,
            BridgeStatus::Pending,
            Some("approving USDC on Arbitrum".to_string()),
            None,
        )
        .await;

        // ── Step 1: approve(bridge_address, amount) on USDC contract ─────────
        // ERC-20 approve selector: keccak256("approve(address,uint256)")[0..4]
        // = 0x095ea7b3
        let approve_selector = [0x09u8, 0x5e, 0xa7, 0xb3];
        let bridge_word = addr_to_word(&bridge_addr)?;
        let amount_word = usdc_to_word(amount_usd);
        let approve_data = abi_encode_2(approve_selector, bridge_word, amount_word);

        let approve_tx = send_arb_tx(
            &rpc_url,
            &key_bytes,
            &usdc_addr,
            approve_data,
            0, // no ETH value
        )
        .await
        .map_err(|e| anyhow!("USDC approve tx failed: {}", e))?;

        log::info!("Bridge {}: USDC approve tx {}", id, approve_tx);

        // Brief pause for approval to confirm
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

        // ── Step 2: sendDeposit(hl_destination, amount) on HL bridge ─────────
        // HL bridge selector: keccak256("sendDeposit(address,uint64)")[0..4]
        // Verify at: https://hyperliquid.gitbook.io/hyperliquid-docs/onboarding/how-to-transfer-usdc-from-arbitrum-to-hyperliquid
        use sha3::{Digest, Keccak256};
        let send_deposit_sig = Keccak256::digest(b"sendDeposit(address,uint64)");
        let deposit_selector: [u8; 4] = send_deposit_sig[..4].try_into().unwrap();
        let dest_word = addr_to_word(&hl_destination)?;
        let deposit_data = abi_encode_2(deposit_selector, dest_word, amount_word);

        self.set_status(
            &id,
            BridgeStatus::Pending,
            Some(format!("sending deposit tx (approve: {})", &approve_tx[..10])),
            Some(approve_tx.clone()),
        )
        .await;

        let deposit_tx = send_arb_tx(
            &rpc_url,
            &key_bytes,
            &bridge_addr,
            deposit_data,
            0,
        )
        .await
        .map_err(|e| anyhow!("HL bridge deposit tx failed: {}", e))?;

        log::info!("Bridge {}: deposit tx {} (${:.2} → {})", id, deposit_tx, amount_usd, hl_destination);

        self.set_status(
            &id,
            BridgeStatus::Pending,
            Some(format!(
                "deposit tx submitted ({}), polling HL for credit…",
                &deposit_tx[..12]
            )),
            Some(deposit_tx.clone()),
        )
        .await;

        // ── Step 3: poll HL for balance increase ──────────────────────────────
        let balance_before = self.hl.get_account(f64::MAX, 0.0).await.map(|a| a.equity).unwrap_or(0.0);
        for attempt in 0..DEPOSIT_MAX_POLLS {
            tokio::time::sleep(tokio::time::Duration::from_secs(DEPOSIT_POLL_INTERVAL_SECS)).await;
            if let Ok(acct) = self.hl.get_account(f64::MAX, 0.0).await {
                if acct.equity >= balance_before + amount_usd * 0.95 {
                    // Credit appeared (allow 5% tolerance for fees)
                    self.set_status(
                        &id,
                        BridgeStatus::Completed,
                        Some(format!(
                            "deposit confirmed — HL equity now ${:.2}",
                            acct.equity
                        )),
                        Some(deposit_tx),
                    )
                    .await;

                    let event = FundEvent {
                        event_type: EventType::Deposit,
                        amount_usd,
                        balance_after: acct.equity,
                        timestamp: Utc::now().to_rfc3339(),
                    };
                    let _ = fund_tracker::append(&tenant_id, &event);
                    return Ok(());
                }
            }
            log::debug!("Bridge {}: poll {}/{} waiting for HL credit", id, attempt + 1, DEPOSIT_MAX_POLLS);
        }

        // Timed out — tx sent but HL credit not confirmed in time
        self.set_status(
            &id,
            BridgeStatus::Pending,
            Some(format!(
                "deposit tx sent ({}) but HL credit not yet confirmed — check back in a few minutes",
                &deposit_tx[..12]
            )),
            Some(deposit_tx),
        )
        .await;
        Ok(())
    }

    async fn set_status(
        &self,
        id: &str,
        status: BridgeStatus,
        reason: Option<String>,
        tx_hash: Option<String>,
    ) {
        let mut lock = self.records.lock().await;
        if let Some(rec) = lock.get_mut(id) {
            rec.status = status;
            rec.status_reason = reason;
            if tx_hash.is_some() {
                rec.tx_hash = tx_hash;
            }
            rec.updated_at = Utc::now();
        }
    }

    fn validate_destination(&self, destination: &str) -> bool {
        if !destination.starts_with("0x") || destination.len() != 42 {
            return false;
        }
        if self.trusted_destinations.is_empty() {
            return true;
        }
        self.trusted_destinations.iter().any(|prefix| {
            destination
                .to_lowercase()
                .starts_with(prefix.to_lowercase().as_str())
        })
    }
}

// ─────────────────────────── Types ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BridgeStatus {
    Initiated,
    Pending,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct BridgeRequestRecord {
    pub id: String,
    #[serde(skip)]
    pub tenant_id: TenantId,
    pub amount_usd: f64,
    pub destination: String,
    pub status: BridgeStatus,
    pub status_reason: Option<String>,
    pub tx_hash: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct BridgeResponse {
    pub id: String,
    pub amount_usd: f64,
    pub destination: String,
    pub status: BridgeStatus,
    pub status_reason: Option<String>,
    pub tx_hash: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl BridgeRequestRecord {
    pub fn view(&self) -> BridgeResponse {
        BridgeResponse {
            id: self.id.clone(),
            amount_usd: self.amount_usd,
            destination: self.destination.clone(),
            status: self.status.clone(),
            tx_hash: self.tx_hash.clone(),
            status_reason: self.status_reason.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

// ─────────────────────────── Tests ───────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::exchange::HyperliquidClient;

    #[tokio::test]
    async fn rejects_small_withdrawals() {
        std::env::set_var("MODE", "paper");
        std::env::set_var("SESSION_SECRET", "test");
        let config = Config::from_env().unwrap();
        let hl = Arc::new(HyperliquidClient::new(&config).unwrap());
        let bridge = BridgeManager::new(hl, 50.0, vec!["0xtrusted".to_string()]);
        let tenant = TenantId::new();
        let err = bridge
            .request_withdrawal(&tenant, 10.0, "0xtrusted1234567890123456789012345678")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("below the minimum"));
    }

    #[tokio::test]
    async fn rejects_untrusted_destination() {
        std::env::set_var("MODE", "paper");
        std::env::set_var("SESSION_SECRET", "test");
        let config = Config::from_env().unwrap();
        let hl = Arc::new(HyperliquidClient::new(&config).unwrap());
        let bridge = BridgeManager::new(hl, 0.0, vec!["0xtrusted".to_string()]);
        let tenant = TenantId::new();
        let err = bridge
            .request_withdrawal(&tenant, 20.0, "0xunknown123")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not trusted"));
    }

    #[tokio::test]
    async fn records_and_fetches_status() {
        std::env::set_var("MODE", "paper");
        std::env::set_var("SESSION_SECRET", "test");
        let config = Config::from_env().unwrap();
        let hl = Arc::new(HyperliquidClient::new(&config).unwrap());
        let bridge = BridgeManager::new(hl, 0.0, vec![]);
        let tenant = TenantId::new();
        let rec = bridge
            .request_withdrawal(&tenant, 20.0, "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .await
            .unwrap();
        assert_eq!(rec.amount_usd, 20.0);
        let fetched = bridge.fetch_record(&rec.id).await.unwrap();
        assert_eq!(fetched.id, rec.id);
        assert!(matches!(fetched.status, BridgeStatus::Initiated));
    }

    #[test]
    fn test_sign_withdraw3_produces_valid_signature() {
        // Smoke-test: ensure the signing function doesn't panic and produces
        // properly formatted r/s/v fields.
        let key = [1u8; 32]; // test key
        let sig = sign_withdraw3(
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "100.0",
            1_700_000_000_000,
            ARBITRUM_CHAIN_ID,
            &key,
        )
        .unwrap();
        assert!(sig.r.starts_with("0x") && sig.r.len() == 66);
        assert!(sig.s.starts_with("0x") && sig.s.len() == 66);
        assert!(sig.v == 27 || sig.v == 28);
    }

    #[test]
    fn test_rlp_encode_u64_zero() {
        assert_eq!(rlp_encode_u64(0), vec![0x80]);
    }

    #[test]
    fn test_rlp_encode_small_value() {
        // 1 → [0x01] (single byte, < 0x80)
        assert_eq!(rlp_encode_u64(1), vec![0x01]);
    }

    #[test]
    fn test_usdc_to_word_100() {
        let word = usdc_to_word(100.0);
        // 100 USDC = 100_000_000 units = 0x5F5E100
        let expected: u64 = 100_000_000;
        let mut exp_word = [0u8; 32];
        exp_word[24..32].copy_from_slice(&expected.to_be_bytes());
        assert_eq!(word, exp_word);
    }
}
