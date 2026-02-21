use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::env;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Mode {
    Testnet,
    Mainnet,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub mode: Mode,
    pub trading_symbol: String,
    pub initial_capital: f64,
    pub max_position_pct: f64,
    pub max_leverage: f64,
    pub daily_loss_limit: f64,
    pub min_health_factor: f64,
    
    // API Keys
    pub binance_api_key: String,
    pub hyperliquid_key: String,
    pub hyperliquid_secret: String,
    
    // Database
    pub database_url: String,
    
    // Risk
    pub max_concurrent_trades: usize,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenv::dotenv().ok();

        let mode = match env::var("MODE")?.as_str() {
            "mainnet" => Mode::Mainnet,
            _ => Mode::Testnet,
        };

        Ok(Config {
            mode,
            trading_symbol: env::var("TRADING_SYMBOL").unwrap_or_else(|_| "SOL".to_string()),
            initial_capital: env::var("INITIAL_CAPITAL")?.parse().unwrap_or(100.0),
            max_position_pct: 0.15,
            max_leverage: 15.0,
            daily_loss_limit: 30.0,
            min_health_factor: 2.0,
            binance_api_key: env::var("BINANCE_API_KEY")?,
            hyperliquid_key: env::var("HYPERLIQUID_KEY")?,
            hyperliquid_secret: env::var("HYPERLIQUID_SECRET")?,
            database_url: env::var("DATABASE_URL").unwrap_or_else(|_| 
                "postgres://postgres:postgres@localhost:5432/redrobot".to_string()
            ),
            max_concurrent_trades: 3,
        })
    }
}
