use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::env;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Mode {
    Paper,
    Testnet,
    Mainnet,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Config {
    pub mode: Mode,
    pub trading_symbol: String,        // legacy single-symbol field
    pub trading_symbols: Vec<String>,  // "ALL" or comma-separated list
    pub initial_capital: f64,
    pub max_position_pct: f64,
    pub max_leverage: f64,
    pub daily_loss_limit: f64,
    pub min_health_factor: f64,

    // API Keys – optional for paper-trading
    pub binance_api_key: Option<String>,
    pub hyperliquid_key: Option<String>,
    pub hyperliquid_secret: Option<String>,
    pub lunarcrush_api_key: String,
    pub anthropic_api_key: Option<String>,

    // Hyperliquid wallet — required for testnet/mainnet
    /// Ethereum-style wallet address (0x…) — used for clearinghouseState queries
    /// and as the signer identity for all order submissions.
    pub hyperliquid_wallet_address: Option<String>,

    // Revenue — builder code embedded in every HL order
    /// Hyperliquid builder address (0x…).  When set, every order routed through
    /// this bot embeds the builder code so the platform earns the builder fee.
    /// Leave unset in paper mode; set on testnet to validate the code path.
    pub builder_code: Option<String>,

    // Affiliate — Hyperliquid referral code
    /// Referral slug registered at app.hyperliquid.xyz (e.g. "REDROBOT").
    /// Displayed in the consumer /app page so new users sign up via our link.
    /// Earns 10 % fee rebate on all trading volume from referred users.
    pub referral_code: Option<String>,

    // Database – optional
    pub database_url: String,

    // Risk
    pub max_concurrent_trades: usize,

    // Paper-trading flag
    pub paper_trading: bool,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();

        let mode_str = env::var("MODE").unwrap_or_else(|_| "paper".to_string());
        let mode = match mode_str.to_lowercase().as_str() {
            "mainnet" => Mode::Mainnet,
            "testnet" => Mode::Testnet,
            _         => Mode::Paper,
        };

        let paper_trading = matches!(mode, Mode::Paper)
            || env::var("PAPER_TRADING").unwrap_or_else(|_| "true".to_string()) == "true";

        let symbols_str = env::var("TRADING_SYMBOLS").unwrap_or_else(|_| "ALL".to_string());
        let trading_symbols = if symbols_str.to_uppercase() == "ALL" {
            vec!["ALL".to_string()]
        } else {
            symbols_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };

        Ok(Config {
            mode,
            trading_symbol: env::var("TRADING_SYMBOL").unwrap_or_else(|_| "SOL".to_string()),
            trading_symbols,
            initial_capital: env::var("INITIAL_CAPITAL")
                .unwrap_or_else(|_| "1000.0".to_string())
                .parse()
                .unwrap_or(1000.0),
            max_position_pct: 0.15,
            max_leverage: 10.0,
            daily_loss_limit: 50.0,
            min_health_factor: 2.0,
            binance_api_key:            env::var("BINANCE_API_KEY").ok(),
            hyperliquid_key:            env::var("HYPERLIQUID_KEY").ok(),
            hyperliquid_secret:         env::var("HYPERLIQUID_SECRET").ok(),
            hyperliquid_wallet_address: env::var("HYPERLIQUID_WALLET_ADDRESS").ok(),
            builder_code:               env::var("BUILDER_CODE").ok(),
            referral_code:              env::var("REFERRAL_CODE").ok(),
            lunarcrush_api_key:         env::var("LUNARCRUSH_API_KEY")
                .unwrap_or_else(|_| "77c4fcm050bnxe49qo1h2n252umls0rrtkevh5uni".to_string()),
            anthropic_api_key:          env::var("ANTHROPIC_API_KEY").ok(),
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "sqlite:./redrobot.db".to_string()),
            max_concurrent_trades: 3,
            paper_trading,
        })
    }
}
