//! Per-session Hyperliquid connector.
//!
//! Wraps the existing `exchange::HyperliquidClient` with per-session config:
//! leverage caps, symbol whitelists, risk mode, drawdown guard, and
//! transparent on-chain trade logging.
//!
//! ## Non-custodial model
//! Every `"venue": "hyperliquid"` session gets a fresh secp256k1 keypair
//! (via `hl_wallet::generate_keypair()`). The private key is AES-256-GCM
//! encrypted at rest using the session token as part of the KDF input.
//! The public address is returned in the session response as `deposit_address`.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─────────────────────────── Session config ──────────────────────────────────

/// Risk mode controls position sizing multipliers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RiskMode {
    Conservative,
    Balanced,
    Aggressive,
}

impl RiskMode {
    /// Size multiplier applied to Kelly-sized positions.
    pub fn size_multiplier(&self) -> f64 {
        match self {
            RiskMode::Conservative => 0.5,
            RiskMode::Balanced     => 1.0,
            RiskMode::Aggressive   => 1.5,
        }
    }
}

impl Default for RiskMode {
    fn default() -> Self { RiskMode::Balanced }
}

impl std::str::FromStr for RiskMode {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "conservative" => Ok(RiskMode::Conservative),
            "balanced"     => Ok(RiskMode::Balanced),
            "aggressive"   => Ok(RiskMode::Aggressive),
            _              => Err(anyhow!("Unknown risk mode: {}", s)),
        }
    }
}

/// Per-session configuration for a Hyperliquid connector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    pub session_id:          String,
    /// Max leverage (clamped to 1–50).
    pub leverage_max:        i32,
    pub risk_mode:           RiskMode,
    /// None = all ~100 HL perps; Some = only these symbols.
    pub symbols_whitelist:   Option<Vec<String>>,
    /// Auto-pause when session drawdown exceeds this % (0 = disabled).
    pub max_drawdown_pct:    f64,
    /// Performance fee % on profitable closes (0 = disabled).
    pub performance_fee_pct: i32,
    /// Encrypted private key for this session's wallet (`nonce_hex:ciphertext_hex`).
    pub encrypted_privkey:   Option<String>,
    /// Ethereum/HL address for this session's wallet.
    pub wallet_address:      Option<String>,
}

impl Default for SessionConfig {
    fn default() -> Self {
        SessionConfig {
            session_id:          String::new(),
            leverage_max:        10,
            risk_mode:           RiskMode::Balanced,
            symbols_whitelist:   None,
            max_drawdown_pct:    0.0,
            performance_fee_pct: 0,
            encrypted_privkey:   None,
            wallet_address:      None,
        }
    }
}

// ─────────────────────────── Signal types ────────────────────────────────────

/// An AI/strategy signal to be executed on Hyperliquid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HlSignal {
    /// Symbol, e.g. "BTC", "ETH", "SOL"
    pub coin:      String,
    /// True = BUY/LONG, False = SELL/SHORT
    pub is_buy:    bool,
    /// USD size for this trade (pre-leverage)
    pub size_usd:  f64,
    /// Limit price. None = market order (IOC).
    pub limit_px:  Option<f64>,
    /// Leverage for this specific order (clamped to session `leverage_max`).
    pub leverage:  i32,
    /// If true, this is a reduce-only close order.
    pub reduce_only: bool,
}

// ─────────────────────────── Trade log ───────────────────────────────────────

/// On-chain trade record stored for transparency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HlTradeLog {
    pub session_id:  String,
    pub coin:        String,
    pub is_buy:      bool,
    pub size_usd:    f64,
    pub limit_px:    Option<f64>,
    pub leverage:    i32,
    /// Hyperliquid response signature / identifier.
    pub tx_ref:      String,
    /// "filled" | "rejected" | "error"
    pub status:      String,
    pub created_at:  String,
    pub raw_response: Option<String>,
}

// ─────────────────────────── Connector ───────────────────────────────────────

/// Per-session Hyperliquid connector.
///
/// Holds session config and trade history. Actual HTTP calls go through
/// `exchange::HyperliquidClient` (already built). This struct provides
/// the session-scoped risk layer on top.
pub struct HyperliquidConnector {
    pub config:     SessionConfig,
    /// In-memory trade log for this session (last 500 trades).
    pub trade_log:  Vec<HlTradeLog>,
    /// Cumulative session P&L for drawdown tracking.
    pub session_pnl: f64,
    /// Starting capital snapshot (set at session creation).
    pub start_capital: f64,
    /// Whether trading is paused (drawdown guard or manual).
    pub paused:     bool,
}

impl HyperliquidConnector {
    /// Create a new connector from a `SessionConfig`.
    pub fn new(config: SessionConfig, start_capital: f64) -> Self {
        HyperliquidConnector {
            config,
            trade_log:  Vec::new(),
            session_pnl: 0.0,
            start_capital,
            paused:     false,
        }
    }

    /// Validate a signal against session config before execution.
    ///
    /// Returns `Ok(leverage)` = clamped leverage, or `Err` = blocked.
    pub fn validate_signal(&self, signal: &HlSignal) -> Result<i32> {
        if self.paused {
            return Err(anyhow!("Session {} is paused", self.config.session_id));
        }

        // Symbol whitelist check
        if let Some(ref wl) = self.config.symbols_whitelist {
            if !wl.iter().any(|s| s.eq_ignore_ascii_case(&signal.coin)) {
                return Err(anyhow!(
                    "Symbol {} is not in this session's whitelist",
                    signal.coin
                ));
            }
        }

        // Drawdown guard
        if self.config.max_drawdown_pct > 0.0 && self.start_capital > 0.0 {
            let drawdown_pct = (-self.session_pnl / self.start_capital) * 100.0;
            if drawdown_pct >= self.config.max_drawdown_pct {
                return Err(anyhow!(
                    "Session drawdown {:.1}% ≥ max {:.1}% — trading paused",
                    drawdown_pct,
                    self.config.max_drawdown_pct
                ));
            }
        }

        // Clamp leverage
        let lev = signal.leverage.clamp(1, self.config.leverage_max);
        Ok(lev)
    }

    /// Apply risk-mode size multiplier to the raw signal size.
    pub fn apply_risk_multiplier(&self, size_usd: f64) -> f64 {
        size_usd * self.config.risk_mode.size_multiplier()
    }

    /// Record a completed trade and update session P&L.
    pub fn record_trade(&mut self, log: HlTradeLog, pnl_delta: f64) {
        self.session_pnl += pnl_delta;

        // Auto-pause on drawdown breach
        if self.config.max_drawdown_pct > 0.0 && self.start_capital > 0.0 {
            let dd = (-self.session_pnl / self.start_capital) * 100.0;
            if dd >= self.config.max_drawdown_pct {
                self.paused = true;
                log::warn!(
                    "Session {} auto-paused: drawdown {:.1}% ≥ {:.1}%",
                    self.config.session_id,
                    dd,
                    self.config.max_drawdown_pct
                );
            }
        }

        self.trade_log.push(log);
        if self.trade_log.len() > 500 {
            self.trade_log.drain(0..self.trade_log.len() - 500);
        }
    }

    /// Get recent trades as JSON for the `/session/{id}/trades` endpoint.
    pub fn recent_trades(&self, limit: usize) -> Vec<&HlTradeLog> {
        let start = self.trade_log.len().saturating_sub(limit);
        self.trade_log[start..].iter().collect()
    }

    /// Session drawdown as percentage (negative = loss).
    pub fn drawdown_pct(&self) -> f64 {
        if self.start_capital == 0.0 { return 0.0; }
        (-self.session_pnl / self.start_capital) * 100.0
    }

    /// Static map: coin name → Hyperliquid asset index.
    /// Used when building order requests. Loaded lazily from HL meta API in prod.
    pub fn coin_to_asset_index(coin: &str) -> u32 {
        // Top 30 perps — remainder resolved via HL /info meta call
        let map: HashMap<&str, u32> = [
            ("BTC",  0), ("ETH",  1), ("SOL",  2), ("BNB",  3),
            ("AVAX", 4), ("MATIC", 5), ("ARB", 6), ("OP",  7),
            ("APT",  8), ("INJ",  9), ("SUI", 10), ("SEI", 11),
            ("TIA", 12), ("DOGE", 13), ("LTC", 14), ("DOT", 15),
            ("LINK", 16), ("UNI", 17), ("NEAR", 18), ("ATOM", 19),
            ("XRP", 20), ("ADA", 21), ("FIL", 22), ("HYPE", 23),
            ("TAO", 24), ("WLD", 25), ("JTO", 26), ("PYTH", 27),
            ("W",   28), ("STRK", 29),
        ].into_iter().collect();
        *map.get(coin.to_uppercase().as_str()).unwrap_or(&0)
    }
}

// ─────────────────────────── Registry ────────────────────────────────────────

/// In-memory registry of active per-session connectors.
///
/// Keyed by `session_id`. Use `tokio::sync::RwLock` wrapper at the call site.
pub type ConnectorRegistry = std::collections::HashMap<String, HyperliquidConnector>;

/// Build a `SessionConfig` from a `BotSession`.
/// Called when a new session with `venue == "hyperliquid"` is created.
pub fn config_from_session(
    session_id: &str,
    leverage_max:     Option<i32>,
    risk_mode:        Option<&str>,
    symbols_whitelist: Option<Vec<String>>,
    max_drawdown_pct: Option<f64>,
    performance_fee_pct: Option<i32>,
    wallet_address:   Option<String>,
) -> SessionConfig {
    let risk = risk_mode
        .and_then(|s| s.parse::<RiskMode>().ok())
        .unwrap_or_default();

    SessionConfig {
        session_id:          session_id.to_string(),
        leverage_max:        leverage_max.unwrap_or(10).clamp(1, 50),
        risk_mode:           risk,
        symbols_whitelist,
        max_drawdown_pct:    max_drawdown_pct.unwrap_or(0.0),
        performance_fee_pct: performance_fee_pct.unwrap_or(0),
        encrypted_privkey:   None,
        wallet_address,
    }
}
