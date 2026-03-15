//! Hyperliquid exchange client.
//!
//! ## Modes
//!
//! | `config.mode` | Behaviour                                         |
//! |---------------|---------------------------------------------------|
//! | Paper         | All writes are stubs; reads hit live HL API       |
//! | Testnet       | Reads + writes hit `api.hyperliquid-testnet.xyz`  |
//! | Mainnet       | Reads + writes hit `api.hyperliquid.xyz`          |
//!
//! ## Builder code
//!
//! Every order submitted in Testnet/Mainnet mode embeds `config.builder_code`
//! so the platform earns the Hyperliquid builder fee on every fill.
//! Users never see this — it is invisible infrastructure revenue.
//!
//! ## Order signing (TODO — required before testnet orders go live)
//!
//! Hyperliquid uses a "phantom agent" EIP-712 pattern for order auth:
//!
//! 1. Compute `actionHash = keccak256(abi_encode(action_struct))`
//! 2. Compute `agentHash  = keccak256(("HyperliquidTransaction:Action",
//!                                      connectionId=actionHash, nonce))`
//! 3. Sign `agentHash` with the wallet private key (secp256k1 / ECDSA).
//! 4. Attach `{r, s, v}` to the `/exchange` POST body.
//!
//! Required crates already added to Cargo.toml:
//!   k256 = { version = "0.13", features = ["ecdsa"] }
//!   sha3 = "0.10"   (keccak256)
//!
//! Reference: https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/exchange-endpoint

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use crate::config::Config;
use crate::decision::Decision;
use crate::risk::Account;

// ─────────────────────────── API base URLs ───────────────────────────────────

const HL_MAINNET: &str = "https://api.hyperliquid.xyz";
const HL_TESTNET: &str = "https://api.hyperliquid-testnet.xyz";

// ─────────────────────────── Data types ──────────────────────────────────────

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

// ─── Hyperliquid REST shapes ──────────────────────────────────────────────────

/// POST /info  →  clearinghouseState
#[derive(Serialize)]
struct InfoRequest<'a> {
    #[serde(rename = "type")]
    req_type: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    user:     Option<&'a str>,
}

/// `marginSummary` inside the clearinghouseState response.
#[derive(Deserialize, Debug)]
struct MarginSummary {
    #[serde(rename = "accountValue", default)]
    account_value: serde_json::Value,
    #[serde(rename = "totalMarginUsed", default)]
    total_margin_used: serde_json::Value,
}

/// Top-level clearinghouseState response (only fields we use).
#[derive(Deserialize, Debug)]
struct ClearinghouseState {
    #[serde(rename = "marginSummary")]
    margin_summary: MarginSummary,
    #[serde(rename = "crossMaintenanceMarginUsed", default)]
    maintenance_margin_used: serde_json::Value,
}

// ─────────────────────────── Client ──────────────────────────────────────────

/// HTTP client for the Hyperliquid exchange API.
#[derive(Debug)]
pub struct HyperliquidClient {
    client:        reqwest::Client,
    base_url:      String,
    testnet:       bool,
    /// Wallet address (0x…) — needed for account queries and order signing.
    wallet_addr:   Option<String>,
    /// Builder code address (0x…) — embedded in every order for fee revenue.
    builder_code:  Option<String>,
    /// Raw private key (hex) — used for EIP-712 order signing.
    /// Stored as bytes to avoid accidental logging.
    #[allow(dead_code)]
    private_key:   Option<Vec<u8>>,
}

impl HyperliquidClient {
    pub fn new(config: &Config) -> Result<Self> {
        let (base_url, testnet) = match config.mode {
            crate::config::Mode::Testnet => (HL_TESTNET.to_string(), true),
            crate::config::Mode::Mainnet => (HL_MAINNET.to_string(), false),
            crate::config::Mode::Paper   => (HL_MAINNET.to_string(), false),
        };

        // Decode private key bytes once at startup (never log them)
        let private_key = config.hyperliquid_secret
            .as_deref()
            .and_then(|s| {
                let s = s.trim_start_matches("0x");
                hex::decode(s).ok()
            });

        if !config.paper_trading {
            if config.hyperliquid_wallet_address.is_none() {
                log::warn!("⚠ HYPERLIQUID_WALLET_ADDRESS not set — live account queries will fail");
            }
            if config.hyperliquid_secret.is_none() {
                log::warn!("⚠ HYPERLIQUID_SECRET not set — order signing will fail");
            }
            if config.builder_code.is_none() {
                log::warn!("⚠ BUILDER_CODE not set — platform will not earn builder fees");
            } else {
                log::info!("✓ Builder code configured ({})",
                    config.builder_code.as_deref().unwrap_or(""));
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

    // ── Read: clearinghouseState (no signing required) ────────────────────────

    /// Fetch live account equity, margin usage, and health from Hyperliquid.
    ///
    /// Paper mode returns a hardcoded healthy account so the paper trading loop
    /// never needs credentials.  Testnet/Mainnet hit the real API.
    pub async fn get_account(&self) -> Result<Account> {
        // Paper mode: return safe defaults — no API key required
        if !self.testnet && self.wallet_addr.is_none() {
            return Ok(Account {
                equity:           1000.0,
                margin:           0.0,
                health_factor:    999.0,
                daily_pnl:        0.0,
                daily_loss_limit: 50.0,
            });
        }

        let addr = self.wallet_addr.as_deref()
            .ok_or_else(|| anyhow!("HYPERLIQUID_WALLET_ADDRESS required for account query"))?;

        let payload = InfoRequest {
            req_type: "clearinghouseState",
            user:     Some(addr),
        };

        let resp = self.client
            .post(format!("{}/info", self.base_url))
            .json(&payload)
            .send()
            .await
            .map_err(|e| anyhow!("clearinghouseState request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body   = resp.text().await.unwrap_or_default();
            return Err(anyhow!("clearinghouseState HTTP {}: {}", status, body));
        }

        let state: ClearinghouseState = resp.json().await
            .map_err(|e| anyhow!("clearinghouseState parse error: {}", e))?;

        let equity = parse_f64(&state.margin_summary.account_value);
        let margin = parse_f64(&state.margin_summary.total_margin_used);
        let maint  = parse_f64(&state.maintenance_margin_used);

        // Health factor = equity / maintenance margin (∞ when no open positions)
        let health_factor = if maint > 0.0 { equity / maint } else { 999.0 };

        Ok(Account {
            equity,
            margin,
            health_factor,
            daily_pnl:        0.0,  // not in clearinghouseState; use trade log
            daily_loss_limit: 50.0, // from config (TODO: pass config here)
        })
    }

    // ── Write: place order (signing required for testnet/mainnet) ────────────

    /// Submit a BUY or SELL order to Hyperliquid.
    ///
    /// **Paper mode**: logs intent and returns a mock order ID.
    ///
    /// **Testnet / Mainnet**: builds the full HL exchange payload and signs it
    /// with the wallet private key (EIP-712 phantom agent pattern).
    ///
    /// # Builder code
    /// When `config.builder_code` is set, it is embedded in the `builder` field
    /// of every order so the platform earns the Hyperliquid builder fee on every
    /// fill.  This is the primary platform revenue stream at scale.
    pub async fn place_order(&self, decision: &Decision) -> Result<String> {
        if decision.action == "SKIP" {
            return Err(anyhow!("Decision is SKIP — nothing to place"));
        }

        // ── Paper mode: stub ─────────────────────────────────────────────────
        if !self.testnet && self.wallet_addr.is_none() {
            let order_id = uuid::Uuid::new_v4().to_string();
            log::info!("📍 [PAPER] {} {} @ {:.4}  id={}",
                decision.action, decision.symbol, decision.entry_price, order_id);
            return Ok(order_id);
        }

        // ── Testnet / Mainnet: real signed order ─────────────────────────────
        let private_key = self.private_key.as_deref()
            .ok_or_else(|| anyhow!("HYPERLIQUID_SECRET required for live order placement"))?;

        let is_buy     = decision.action == "BUY";
        let order_type = OrderType::Limit {
            price: format!("{:.4}", decision.entry_price),
            tif:   "Gtc".to_string(),
        };

        let order_payload = HlOrderPayload {
            asset:      symbol_to_asset_index(&decision.symbol)?,
            is_buy,
            limit_px:   format!("{:.4}", decision.entry_price),
            sz:         format!("{:.4}", decision.quantity),
            order_type,
            reduce_only: false,
            // Revenue hook: builder code on every order
            builder:    self.builder_code.clone(),
        };

        let nonce = chrono::Utc::now().timestamp_millis() as u64;

        // Sign the action (EIP-712 phantom agent)
        let signature = sign_order_action(&order_payload, nonce, self.testnet, private_key)?;

        let body = serde_json::json!({
            "action": {
                "type": "order",
                "orders": [order_payload],
                "grouping": "na",
                "builder": self.builder_code.as_deref().unwrap_or("")
            },
            "nonce": nonce,
            "signature": signature,
            "vaultAddress": null
        });

        let resp = self.client
            .post(format!("{}/exchange", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("order POST failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text   = resp.text().await.unwrap_or_default();
            return Err(anyhow!("order rejected HTTP {}: {}", status, text));
        }

        let result: serde_json::Value = resp.json().await
            .map_err(|e| anyhow!("order response parse error: {}", e))?;

        // Extract order ID from HL response
        let order_id = result["response"]["data"]["statuses"][0]["resting"]["oid"]
            .as_u64()
            .map(|n| n.to_string())
            .or_else(|| result["response"]["data"]["statuses"][0]["filled"]["oid"]
                .as_u64()
                .map(|n| n.to_string()))
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        log::info!("✅ Order placed: {} {} @ {:.4}  oid={}",
            decision.action, decision.symbol, decision.entry_price, order_id);

        Ok(order_id)
    }

    /// STUB — Returns empty positions list.
    /// Production: parse `assetPositions` from clearinghouseState.
    pub async fn get_positions(&self) -> Result<Vec<Position>> {
        Ok(vec![])
    }

    /// STUB — Logs the close request and returns a mock UUID.
    /// Production: place a reduce-only market order opposite to `position.size`.
    pub async fn close_position(&self, position: &Position) -> Result<String> {
        let order_id = uuid::Uuid::new_v4().to_string();
        log::info!("🔒 [STUB] Close position: {} ({})", order_id, position.symbol);
        Ok(order_id)
    }
}

// ─────────────────────────── Order signing ───────────────────────────────────

/// Hyperliquid order payload (matches `/exchange` API shape).
#[derive(Debug, Serialize)]
struct HlOrderPayload {
    /// Asset index (0 = BTC, 1 = ETH, etc. — see HL asset list)
    #[serde(rename = "a")]
    asset:       u32,
    /// true = buy, false = sell
    #[serde(rename = "b")]
    is_buy:      bool,
    /// Limit price as string
    #[serde(rename = "p")]
    limit_px:    String,
    /// Size in base asset as string
    #[serde(rename = "s")]
    sz:          String,
    /// Order type details
    #[serde(rename = "t")]
    order_type:  OrderType,
    /// Reduce-only flag
    #[serde(rename = "r")]
    reduce_only: bool,
    /// Builder code for fee revenue (optional)
    #[serde(rename = "c", skip_serializing_if = "Option::is_none")]
    builder:     Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum OrderType {
    Limit { price: String, tif: String },
}

/// EIP-712 phantom-agent signature `{r, s, v}`.
#[derive(Debug, Serialize)]
struct HlSignature {
    r: String,
    s: String,
    v: u8,
}

/// Sign a Hyperliquid order action using the phantom-agent EIP-712 pattern.
///
/// # Signing steps (per Hyperliquid docs)
///
/// 1. Serialize the action to canonical JSON.
/// 2. Hash: `actionHash  = keccak256(action_bytes)`
/// 3. Build phantom-agent struct:
///    `("HyperliquidTransaction:Action", connectionId=actionHash, nonce=nonce)`
/// 4. Hash: `agentHash = keccak256(encode(phantom_agent))`
/// 5. Sign `agentHash` with wallet key (secp256k1/ECDSA, Ethereum v=27|28).
///
/// # TODO
///
/// This function currently returns a placeholder signature.  Before testnet
/// orders go live, replace the body with the full k256 + keccak256 signing
/// implementation.  Required crates (already in Cargo.toml):
///   k256  = { version = "0.13", features = ["ecdsa"] }
///   sha3  = "0.10"
///
/// Reference implementation (Python SDK):
///   https://github.com/hyperliquid-dex/hyperliquid-python-sdk/blob/master/hyperliquid/utils/signing.py
fn sign_order_action(
    _payload: &HlOrderPayload,
    _nonce:   u64,
    _testnet: bool,
    _key:     &[u8],
) -> Result<HlSignature> {
    // ── TODO: implement EIP-712 phantom agent signing ─────────────────────
    //
    // use sha3::{Keccak256, Digest};
    // use k256::ecdsa::{SigningKey, signature::Signer};
    //
    // let action_json = serde_json::to_vec(_payload)?;
    // let action_hash = Keccak256::digest(&action_json);
    //
    // // Phantom agent struct encoding (EIP-712 style)
    // let mut agent_msg = Vec::new();
    // agent_msg.extend_from_slice(b"HyperliquidTransaction:Action");
    // agent_msg.extend_from_slice(&action_hash);
    // agent_msg.extend_from_slice(&_nonce.to_be_bytes());
    // let agent_hash = Keccak256::digest(&agent_msg);
    //
    // let signing_key = SigningKey::from_bytes(_key.into())?;
    // let (sig, recovery_id) = signing_key.sign_prehash_recoverable(&agent_hash)?;
    // let v = recovery_id.to_byte() + if _testnet { 27 } else { 27 };
    // let bytes = sig.to_bytes();
    // return Ok(HlSignature {
    //     r: format!("0x{}", hex::encode(&bytes[..32])),
    //     s: format!("0x{}", hex::encode(&bytes[32..])),
    //     v,
    // });
    // ─────────────────────────────────────────────────────────────────────

    // Placeholder — remove once real signing is implemented above
    Err(anyhow!(
        "Order signing not yet implemented. \
         Set MODE=paper to use paper trading. \
         See exchange.rs sign_order_action() for the implementation roadmap."
    ))
}

// ─────────────────────────── Asset index map ─────────────────────────────────

/// Translate a Hyperliquid perp symbol to its asset index.
///
/// Hyperliquid identifies assets by integer index in their order API.
/// This map covers the core perps; extend as new markets are listed.
/// Full list: https://api.hyperliquid.xyz/info  (type: "meta")
fn symbol_to_asset_index(symbol: &str) -> Result<u32> {
    // Strip trailing "USDT" if present (HL uses bare symbols, e.g. "SOL" not "SOLUSDT")
    let sym = symbol.trim_end_matches("USDT");
    let idx = match sym {
        "BTC"   =>  0,
        "ETH"   =>  1,
        "ATOM"  =>  2,
        "MATIC" =>  3,
        "DYDX"  =>  4,
        "SOL"   =>  5,
        "BNB"   =>  6,
        "APT"   =>  7,
        "ARB"   =>  8,
        "DOT"   =>  9,
        "AVAX"  => 12,
        "OP"    => 14,
        "LTC"   => 17,
        "LINK"  => 18,
        "NEAR"  => 20,
        "XRP"   => 22,
        "ADA"   => 23,
        "SUI"   => 35,
        "INJ"   => 42,
        "TIA"   => 55,
        "WIF"   => 61,
        "PEPE"  => 57,
        "BONK"  => 63,
        _ => return Err(anyhow!(
            "Unknown HL asset: {}. Add it to symbol_to_asset_index() in exchange.rs", sym
        )),
    };
    Ok(idx)
}

// ─────────────────────────── Helpers ─────────────────────────────────────────

/// Parse a JSON value that may be a quoted number string or a raw number.
fn parse_f64(v: &serde_json::Value) -> f64 {
    match v {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        serde_json::Value::String(s) => s.parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    }
}
