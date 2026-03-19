//! Hyperliquid exchange client.
//!
//! ## Modes
//!
//! | `config.mode` | Behaviour                                         |
//! |---------------|---------------------------------------------------|
//! | Paper         | All writes are no-ops; reads use live HL API      |
//! | Testnet       | Reads + writes hit `api.hyperliquid-testnet.xyz`  |
//! | Mainnet       | Reads + writes hit `api.hyperliquid.xyz`          |
//!
//! ## Builder code
//!
//! Every live order embeds `config.builder_code` — the platform wallet address
//! that earns Hyperliquid builder fees on every fill.  Users never see this.
//!
//! ## Order signing  (EIP-712 phantom-agent pattern)
//!
//! 1. Serialise the order action struct with MessagePack (`rmp-serde`).
//! 2. `actionHash = keccak256(msgpack_bytes + nonce_u64_be + 0x00)`
//! 3. Build EIP-712 Agent struct:
//!    `{ source: "a"|"b", connectionId: actionHash }`
//!    (source="a" mainnet, source="b" testnet)
//! 4. Compute EIP-712 typed-data hash using:
//!    - domainSeparator  = keccak256(typeHash(domain) ‖ name ‖ version ‖ chainId ‖ addr)
//!    - structHash       = keccak256(typeHash(Agent)  ‖ source_hash ‖ connectionId)
//!    - finalHash        = keccak256("\x19\x01" ‖ domainSep ‖ structHash)
//! 5. Sign finalHash with secp256k1 ECDSA (k256); v = recovery_id + 27.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use crate::config::Config;
use crate::decision::Decision;
use crate::risk::Account;

// ─────────────────────────── API base URLs ───────────────────────────────────

const HL_MAINNET: &str = "https://api.hyperliquid.xyz";
const HL_TESTNET: &str = "https://api.hyperliquid-testnet.xyz";

// ─────────────────────────── Domain structs ──────────────────────────────────

/// An open position on Hyperliquid.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct Position {
    pub symbol:        String,
    pub size:          f64,
    pub entry_price:   f64,
    pub current_price: f64,
    pub pnl:           f64,
    pub leverage:      f64,
}

#[allow(dead_code)]
impl Position {
    pub fn should_close(&self) -> bool {
        ((self.current_price - self.entry_price) / self.entry_price).abs() > 0.05
    }
}

// ─── Hyperliquid REST request/response shapes ─────────────────────────────────

/// POST /info — generic info request body.
#[derive(Serialize)]
struct InfoRequest<'a> {
    #[serde(rename = "type")]
    req_type: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<&'a str>,
}

/// `marginSummary` in the clearinghouseState response.
#[derive(Deserialize, Debug, Default)]
struct MarginSummary {
    #[serde(rename = "accountValue",       default)]
    account_value: serde_json::Value,
    #[serde(rename = "totalMarginUsed",    default)]
    total_margin_used: serde_json::Value,
}

/// Top-level clearinghouseState response (only fields we need).
#[derive(Deserialize, Debug)]
struct ClearinghouseState {
    #[serde(rename = "marginSummary", default)]
    margin_summary: MarginSummary,
    #[serde(rename = "crossMaintenanceMarginUsed", default)]
    maintenance_margin_used: serde_json::Value,
}

// ─────────────────────────── Client ──────────────────────────────────────────

/// HTTP client for the Hyperliquid exchange API.
#[derive(Debug)]
pub struct HyperliquidClient {
    client:       reqwest::Client,
    base_url:     String,
    testnet:      bool,
    wallet_addr:  Option<String>,
    builder_code: Option<String>,
    /// Private key bytes — stored decoded to avoid accidental logging.
    private_key:  Option<Vec<u8>>,
}

impl HyperliquidClient {
    pub fn new(config: &Config) -> Result<Self> {
        let (base_url, testnet) = match config.mode {
            crate::config::Mode::Testnet => (HL_TESTNET.to_string(), true),
            crate::config::Mode::Mainnet => (HL_MAINNET.to_string(), false),
            crate::config::Mode::Paper   => (HL_MAINNET.to_string(), false),
        };

        let private_key = config.hyperliquid_secret
            .as_deref()
            .and_then(|s| hex::decode(s.trim_start_matches("0x")).ok());

        if !config.paper_trading {
            if config.hyperliquid_wallet_address.is_none() {
                log::warn!("⚠ HYPERLIQUID_WALLET_ADDRESS not set — live account queries will fail");
            }
            if config.hyperliquid_secret.is_none() {
                log::warn!("⚠ HYPERLIQUID_SECRET not set — order signing will fail");
            }
            match &config.builder_code {
                None    => log::warn!("⚠ BUILDER_CODE not set — platform will not earn builder fees"),
                Some(c) => log::info!("✓ Builder code: {}", c),
            }
        }

        Ok(HyperliquidClient {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()?,
            base_url,
            testnet,
            wallet_addr:  config.hyperliquid_wallet_address.clone(),
            builder_code: config.builder_code.clone(),
            private_key,
        })
    }

    // ── Read: clearinghouseState ──────────────────────────────────────────────

    /// Fetch live account equity and margin from Hyperliquid (no auth needed).
    /// Paper mode returns safe defaults so the trading loop never needs keys.
    pub async fn get_account(&self) -> Result<Account> {
        // Paper mode: stable stub
        if self.wallet_addr.is_none() {
            return Ok(Account {
                equity:           1000.0,
                margin:           0.0,
                health_factor:    999.0,
                daily_pnl:        0.0,
                daily_loss_limit: 50.0,
            });
        }

        let addr = self.wallet_addr.as_deref().unwrap();
        let resp = self.client
            .post(format!("{}/info", self.base_url))
            .json(&InfoRequest { req_type: "clearinghouseState", user: Some(addr) })
            .send().await
            .map_err(|e| anyhow!("clearinghouseState request failed: {}", e))?;

        if !resp.status().is_success() {
            let s = resp.status();
            let b = resp.text().await.unwrap_or_default();
            return Err(anyhow!("clearinghouseState HTTP {}: {}", s, b));
        }

        let state: ClearinghouseState = resp.json().await
            .map_err(|e| anyhow!("clearinghouseState parse: {}", e))?;

        let equity = parse_hl_f64(&state.margin_summary.account_value);
        let margin = parse_hl_f64(&state.margin_summary.total_margin_used);
        let maint  = parse_hl_f64(&state.maintenance_margin_used);
        let health = if maint > 0.0 { equity / maint } else { 999.0 };

        Ok(Account { equity, margin, health_factor: health, daily_pnl: 0.0, daily_loss_limit: 50.0 })
    }

    // ── Write: place order ────────────────────────────────────────────────────

    /// Submit a BUY/SELL order to Hyperliquid.
    ///
    /// `symbol`   — bare perp symbol ("SOL", "BTC", …)
    /// `capital`  — current free capital in USD (used to compute order quantity)
    /// `fee_bps`  — builder fee in basis points for this tenant (use
    ///              `tenant_config.builder_fee_bps()`).  1 bps for Pro/Internal,
    ///              3 bps for Free.  Embedded in the signed HL payload — the
    ///              exchange deducts it from fill proceeds and credits the builder
    ///              wallet.  Users never see a separate line item.
    ///
    /// Paper mode: logs and returns a mock UUID, no API call.
    /// Testnet/Mainnet: builds signed HL exchange payload.
    pub async fn place_order(&self, symbol: &str, decision: &Decision, capital: f64, fee_bps: u32) -> Result<String> {
        if decision.action == "SKIP" {
            return Err(anyhow!("Decision is SKIP — nothing to place"));
        }

        // ── Paper mode ────────────────────────────────────────────────────────
        if self.wallet_addr.is_none() {
            let id = uuid::Uuid::new_v4().to_string();
            log::info!("📍 [PAPER] {} {} @ {:.4}  id={}", decision.action, symbol, decision.entry_price, id);
            return Ok(id);
        }

        // ── Live mode ─────────────────────────────────────────────────────────
        let key = self.private_key.as_deref()
            .ok_or_else(|| anyhow!("HYPERLIQUID_SECRET required for live orders"))?;

        let is_buy  = decision.action == "BUY";
        let asset   = symbol_to_asset_index(symbol)?;
        // Quantity = notional / price  where  notional = margin * leverage
        let size_usd = capital * decision.position_size;
        let notional = size_usd * decision.leverage;
        let qty      = notional / decision.entry_price.max(1e-8);

        let order = HlOrder {
            asset,
            is_buy,
            limit_px: format!("{:.6}", decision.entry_price),
            sz:       format!("{:.6}", qty),
            reduce_only: false,
            order_type: HlOrderType { limit: HlLimitType { tif: "Gtc".to_string() } },
            cloid: None,
        };

        // Clamp fee_bps to HL's documented maximum of 3 to avoid rejected orders.
        let clamped_bps = fee_bps.min(3);
        let action = HlAction {
            action_type: "order".to_string(),
            orders:      vec![order],
            grouping:    "na".to_string(),
            builder:     self.builder_code.clone().map(|b| HlBuilder { b, f: clamped_bps }),
        };

        let nonce = chrono::Utc::now().timestamp_millis() as u64;
        let sig   = sign_l1_action(&action, nonce, self.testnet, key)?;

        let body = serde_json::json!({
            "action":       action,
            "nonce":        nonce,
            "signature":    sig,
            "vaultAddress": null
        });

        let resp = self.client
            .post(format!("{}/exchange", self.base_url))
            .json(&body)
            .send().await
            .map_err(|e| anyhow!("order POST failed: {}", e))?;

        if !resp.status().is_success() {
            let s = resp.status();
            let t = resp.text().await.unwrap_or_default();
            return Err(anyhow!("order rejected HTTP {}: {}", s, t));
        }

        let result: serde_json::Value = resp.json().await
            .map_err(|e| anyhow!("order response parse: {}", e))?;

        // Extract resting or filled oid from HL response
        let oid = result["response"]["data"]["statuses"][0]["resting"]["oid"]
            .as_u64()
            .or_else(|| result["response"]["data"]["statuses"][0]["filled"]["oid"].as_u64())
            .map(|n| n.to_string())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        log::info!("✅ {} {} @ {:.4}  oid={}", decision.action, symbol, decision.entry_price, oid);
        Ok(oid)
    }

    /// STUB — Returns empty positions list.
    #[allow(dead_code)]
    pub async fn get_positions(&self) -> Result<Vec<Position>> {
        Ok(vec![])
    }

    /// Place a reduce-only close order for an open position, collecting the
    /// builder fee on the exit leg just as on the entry leg.
    ///
    /// Parameters
    /// ----------
    /// `symbol`    — e.g. "BTC", "SOL"
    /// `is_long`   — true if the position being closed is LONG (so we SELL to close)
    /// `qty`       — exact quantity to close (coins, not USD)
    /// `price`     — current mid-price used as the limit price (GTC, reduce-only)
    /// `fee_bps`   — builder fee in basis points (same as for entry orders)
    ///
    /// Paper mode: no-op (returns a mock id).
    /// Live mode:  signed reduce-only order → HL exchange API.
    pub async fn close_position_qty(
        &self,
        symbol:  &str,
        is_long: bool,
        qty:     f64,
        price:   f64,
        fee_bps: u32,
    ) -> Result<String> {
        // Paper mode — nothing to send to the exchange
        if self.wallet_addr.is_none() {
            let id = uuid::Uuid::new_v4().to_string();
            log::info!("📍 [PAPER] CLOSE {} qty={:.6} @ {:.4}  id={}", symbol, qty, price, id);
            return Ok(id);
        }

        let key = self.private_key.as_deref()
            .ok_or_else(|| anyhow!("HYPERLIQUID_SECRET required for live close orders"))?;

        let asset  = symbol_to_asset_index(symbol)?;
        // To close a LONG we sell (is_buy=false); to close a SHORT we buy (is_buy=true)
        let is_buy = !is_long;

        let order = HlOrder {
            asset,
            is_buy,
            limit_px:    format!("{:.6}", price),
            sz:          format!("{:.6}", qty),
            reduce_only: true,  // ← key difference from entry orders
            order_type:  HlOrderType { limit: HlLimitType { tif: "Gtc".to_string() } },
            cloid:       None,
        };

        let clamped_bps = fee_bps.min(3);
        let action = HlAction {
            action_type: "order".to_string(),
            orders:      vec![order],
            grouping:    "na".to_string(),
            builder:     self.builder_code.clone().map(|b| HlBuilder { b, f: clamped_bps }),
        };

        let nonce = chrono::Utc::now().timestamp_millis() as u64;
        let sig   = sign_l1_action(&action, nonce, self.testnet, key)?;

        let body = serde_json::json!({
            "action":       action,
            "nonce":        nonce,
            "signature":    sig,
            "vaultAddress": null
        });

        let resp = self.client
            .post(format!("{}/exchange", self.base_url))
            .json(&body)
            .send().await
            .map_err(|e| anyhow!("close order POST failed: {}", e))?;

        if !resp.status().is_success() {
            let s = resp.status();
            let t = resp.text().await.unwrap_or_default();
            return Err(anyhow!("close order rejected HTTP {}: {}", s, t));
        }

        let result: serde_json::Value = resp.json().await
            .map_err(|e| anyhow!("close order response parse: {}", e))?;

        let oid = result["response"]["data"]["statuses"][0]["resting"]["oid"]
            .as_u64()
            .or_else(|| result["response"]["data"]["statuses"][0]["filled"]["oid"].as_u64())
            .map(|n| n.to_string())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        log::info!("✅ CLOSE {} qty={:.6} @ {:.4}  oid={}", symbol, qty, price, oid);
        Ok(oid)
    }
}

// ─────────────────────────── HL action structs ───────────────────────────────

#[derive(Debug, Clone, Serialize)]
struct HlOrder {
    #[serde(rename = "a")]  asset:       u32,
    #[serde(rename = "b")]  is_buy:      bool,
    #[serde(rename = "p")]  limit_px:    String,
    #[serde(rename = "s")]  sz:          String,
    #[serde(rename = "r")]  reduce_only: bool,
    #[serde(rename = "t")]  order_type:  HlOrderType,
    #[serde(rename = "c", skip_serializing_if = "Option::is_none")]
    cloid: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct HlOrderType {
    limit: HlLimitType,
}

#[derive(Debug, Clone, Serialize)]
struct HlLimitType {
    tif: String,
}

#[derive(Debug, Clone, Serialize)]
struct HlBuilder {
    b: String,   // builder address
    f: u32,      // fee (basis points, 1 = 0.001%)
}

#[derive(Debug, Clone, Serialize)]
struct HlAction {
    #[serde(rename = "type")]
    action_type: String,
    orders:      Vec<HlOrder>,
    grouping:    String,
    #[serde(skip_serializing_if = "Option::is_none")]
    builder:     Option<HlBuilder>,
}

/// EIP-712 signature `{r, s, v}`.
#[derive(Debug, Serialize)]
struct HlSignature {
    r: String,
    s: String,
    v: u8,
}

// ─────────────────────────── EIP-712 signing ─────────────────────────────────

/// Sign a Hyperliquid L1 action using the phantom-agent EIP-712 pattern.
///
/// Process (mirrors hyperliquid-python-sdk `sign_l1_action`):
///   1. MessagePack-encode the action, append 8-byte big-endian nonce + 0x00 sentinel.
///   2. `connectionId = keccak256(packed_bytes)`
///   3. EIP-712 typed-data hash over Agent{source, connectionId}.
///   4. secp256k1 ECDSA sign; v = recovery_id + 27.
fn sign_l1_action(
    action:   &HlAction,
    nonce:    u64,
    testnet:  bool,
    key:      &[u8],
) -> Result<HlSignature> {
    use sha3::{Digest, Keccak256};
    use k256::ecdsa::{SigningKey, signature::hazmat::PrehashSigner};

    // ── Step 1: msgpack(action) + nonce_be8 + 0x00 ───────────────────────────
    let mut packed = rmp_serde::to_vec_named(action)
        .map_err(|e| anyhow!("msgpack encode failed: {}", e))?;
    packed.extend_from_slice(&nonce.to_be_bytes());
    packed.push(0x00); // no vault address

    // ── Step 2: connectionId = keccak256(packed) ─────────────────────────────
    let connection_id: [u8; 32] = Keccak256::digest(&packed).into();

    // ── Step 3: EIP-712 typed-data hash ──────────────────────────────────────
    // Domain: {name:"Exchange", version:"1", chainId:1337, verifyingContract:0x0}
    let domain_type_hash: [u8; 32] = Keccak256::digest(
        b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"
    ).into();
    let agent_type_hash: [u8; 32] = Keccak256::digest(
        b"Agent(string source,bytes32 connectionId)"
    ).into();

    let name_hash:    [u8; 32] = Keccak256::digest(b"Exchange").into();
    let version_hash: [u8; 32] = Keccak256::digest(b"1").into();

    // chainId = 1337 (0x0539), zero-padded to 32 bytes
    let mut chain_id = [0u8; 32];
    chain_id[30] = 0x05;
    chain_id[31] = 0x39;

    // verifyingContract = 0x0000...0000
    let contract = [0u8; 32];

    // domainSeparator = keccak256(abi.encode(typeHash, name, version, chainId, contract))
    let mut domain_enc = [0u8; 32 * 5];
    domain_enc[0..32].copy_from_slice(&domain_type_hash);
    domain_enc[32..64].copy_from_slice(&name_hash);
    domain_enc[64..96].copy_from_slice(&version_hash);
    domain_enc[96..128].copy_from_slice(&chain_id);
    domain_enc[128..160].copy_from_slice(&contract);
    let domain_sep: [u8; 32] = Keccak256::digest(domain_enc).into();

    // source = "a" (mainnet) or "b" (testnet) — per HL SDK convention
    let source = if !testnet { "a" } else { "b" };
    let source_hash: [u8; 32] = Keccak256::digest(source.as_bytes()).into();

    // structHash = keccak256(abi.encode(agentTypeHash, keccak256(source), connectionId))
    let mut struct_enc = [0u8; 32 * 3];
    struct_enc[0..32].copy_from_slice(&agent_type_hash);
    struct_enc[32..64].copy_from_slice(&source_hash);
    struct_enc[64..96].copy_from_slice(&connection_id);
    let struct_hash: [u8; 32] = Keccak256::digest(struct_enc).into();

    // finalHash = keccak256("\x19\x01" + domainSep + structHash)
    let mut final_msg = Vec::with_capacity(66);
    final_msg.extend_from_slice(b"\x19\x01");
    final_msg.extend_from_slice(&domain_sep);
    final_msg.extend_from_slice(&struct_hash);
    let final_hash: [u8; 32] = Keccak256::digest(&final_msg).into();

    // ── Step 4: secp256k1 sign ────────────────────────────────────────────────
    let signing_key = SigningKey::from_bytes(key.into())
        .map_err(|e| anyhow!("Invalid private key: {}", e))?;
    let (sig, recid): (k256::ecdsa::Signature, k256::ecdsa::RecoveryId) =
        signing_key.sign_prehash(&final_hash)
            .map_err(|e| anyhow!("Signing failed: {}", e))?;

    let sig_bytes = sig.to_bytes();
    Ok(HlSignature {
        r: format!("0x{}", hex::encode(&sig_bytes[..32])),
        s: format!("0x{}", hex::encode(&sig_bytes[32..])),
        v: recid.to_byte() + 27,
    })
}

// ─────────────────────────── Asset index map ─────────────────────────────────

/// Translate a HL perp symbol to its exchange asset index.
/// Full list: POST /info {"type":"meta"} → universe[n].name
fn symbol_to_asset_index(symbol: &str) -> Result<u32> {
    let sym = symbol.trim_end_matches("USDT");
    let idx = match sym {
        "BTC"   =>  0,  "ETH"   =>  1,  "ATOM"  =>  2,
        "MATIC" =>  3,  "DYDX"  =>  4,  "SOL"   =>  5,
        "BNB"   =>  6,  "APT"   =>  7,  "ARB"   =>  8,
        "DOT"   =>  9,  "AVAX"  => 12,  "OP"    => 14,
        "LTC"   => 17,  "LINK"  => 18,  "NEAR"  => 20,
        "XRP"   => 22,  "ADA"   => 23,  "SUI"   => 35,
        "INJ"   => 42,  "TIA"   => 55,  "PEPE"  => 57,
        "WIF"   => 61,  "BONK"  => 63,
        _ => return Err(anyhow!(
            "Unknown HL asset index for '{}'. Add it to symbol_to_asset_index() in exchange.rs", sym
        )),
    };
    Ok(idx)
}

// ─────────────────────────── Helpers ─────────────────────────────────────────

/// Parse a HL JSON value that may be a quoted decimal string or a raw number.
fn parse_hl_f64(v: &serde_json::Value) -> f64 {
    match v {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        serde_json::Value::String(s) => s.parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

// ─────────────────────────── Tests ───────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── HlBuilder serialisation ───────────────────────────────────────────────

    #[test]
    fn builder_serialises_with_correct_field_names() {
        // HL requires exactly {"b": "<addr>", "f": <bps>} — wrong field names
        // cause silent fee loss or order rejection.
        let b = HlBuilder {
            b: "0xa765cd52ad56efc294fc6a7155a53920294ca3e3".to_string(),
            f: 3,
        };
        let j = serde_json::to_value(&b).unwrap();
        assert_eq!(j["b"], "0xa765cd52ad56efc294fc6a7155a53920294ca3e3",
            "builder address field must be 'b'");
        assert_eq!(j["f"], 3,
            "fee field must be 'f'");
        assert!(j.get("address").is_none(), "no stray 'address' field");
        assert!(j.get("fee").is_none(),     "no stray 'fee' field");
    }

    // ── HlAction: builder field present / absent ──────────────────────────────

    #[test]
    fn action_includes_builder_when_code_is_set() {
        let action = HlAction {
            action_type: "order".to_string(),
            orders:      vec![],
            grouping:    "na".to_string(),
            builder: Some(HlBuilder {
                b: "0xa765cd52ad56efc294fc6a7155a53920294ca3e3".to_string(),
                f: 3,
            }),
        };
        let j = serde_json::to_value(&action).unwrap();
        assert!(j.get("builder").is_some(),
            "builder field must be present when BUILDER_CODE is set — fees depend on this");
        assert_eq!(j["builder"]["f"], 3);
    }

    #[test]
    fn action_omits_builder_when_code_is_none() {
        // skip_serializing_if = "Option::is_none" must suppress the field entirely
        // (not serialize it as null — HL may reject null builder fields).
        let action = HlAction {
            action_type: "order".to_string(),
            orders:      vec![],
            grouping:    "na".to_string(),
            builder:     None,
        };
        let j = serde_json::to_value(&action).unwrap();
        assert!(j.get("builder").is_none(),
            "builder field must be absent (not null) when BUILDER_CODE is unset");
    }

    // ── fee_bps clamping ──────────────────────────────────────────────────────

    #[test]
    fn fee_bps_clamped_to_hl_maximum_of_3() {
        // HL rejects orders with builder fee > 3 bps.  Any value above 3 must
        // be silently clamped — never passed through unchecked.
        for input in [4u32, 5, 10, 100] {
            let clamped = input.min(3);
            assert_eq!(clamped, 3,
                "fee_bps {} must clamp to 3, got {}", input, clamped);
        }
    }

    #[test]
    fn fee_bps_below_max_passes_through_unchanged() {
        for input in [0u32, 1, 2, 3] {
            let clamped = input.min(3);
            assert_eq!(clamped, input,
                "fee_bps {} should not be altered, got {}", input, clamped);
        }
    }

    // ── per-tier fee rates ────────────────────────────────────────────────────

    #[test]
    fn free_tier_builder_fee_is_3_bps() {
        use crate::tenant::TenantConfig;
        let cfg = TenantConfig::paper("test", 1000.0);
        assert_eq!(cfg.builder_fee_bps(), 3,
            "free tier must carry maximum 3 bps to maximise revenue on non-paying users");
    }

    #[test]
    fn pro_tier_builder_fee_is_1_bps() {
        use crate::tenant::{TenantConfig, TenantTier};
        let mut cfg = TenantConfig::paper("test", 1000.0);
        cfg.tier = TenantTier::Pro;
        assert_eq!(cfg.builder_fee_bps(), 1,
            "Pro tier reward: lighter 1 bps take on paying subscribers");
    }

    #[test]
    fn builder_fee_never_exceeds_hl_maximum() {
        use crate::tenant::{TenantConfig, TenantTier};
        for tier in [TenantTier::Free, TenantTier::Pro, TenantTier::Internal] {
            let mut cfg = TenantConfig::paper("test", 1000.0);
            cfg.tier = tier.clone();
            assert!(cfg.builder_fee_bps() <= 3,
                "{:?} tier fee {} exceeds HL maximum of 3 bps", tier, cfg.builder_fee_bps());
        }
    }

    // ── symbol_to_asset_index ─────────────────────────────────────────────────

    #[test]
    fn btc_and_eth_have_correct_asset_indices() {
        // BTC=0, ETH=1 are load-bearing constants — wrong indices place orders
        // on the wrong market, causing financial losses.
        assert_eq!(symbol_to_asset_index("BTC").unwrap(),  0);
        assert_eq!(symbol_to_asset_index("ETH").unwrap(),  1);
        assert_eq!(symbol_to_asset_index("BTCUSDT").unwrap(), 0,
            "USDT suffix should be stripped");
    }

    #[test]
    fn unknown_symbol_returns_error_not_panic() {
        let result = symbol_to_asset_index("FAKECOIN");
        assert!(result.is_err(),
            "unknown symbol must return Err, not silently use index 0");
    }
}
