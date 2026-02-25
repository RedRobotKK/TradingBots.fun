use anyhow::Result;
use serde::{Deserialize, Serialize};
use crate::config::Config;
use crate::decision::Decision;
use crate::risk::Account;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub symbol: String,
    pub size: f64,
    pub entry_price: f64,
    pub current_price: f64,
    pub pnl: f64,
    pub leverage: f64,
}

impl Position {
    pub fn should_close(&self) -> bool {
        // Check if position has moved significantly
        let pnl_pct = (self.current_price - self.entry_price) / self.entry_price;
        pnl_pct.abs() > 0.05 // 5% move = close
    }
}

#[derive(Debug)]
pub struct HyperliquidClient {
    client: reqwest::Client,
    base_url: String,
    testnet: bool,
}

impl HyperliquidClient {
    pub fn new(config: &Config) -> Result<Self> {
        let base_url = match config.mode {
            crate::config::Mode::Testnet => "https://api.hyperliquid-testnet.xyz".to_string(),
            crate::config::Mode::Mainnet => "https://api.hyperliquid.xyz".to_string(),
            crate::config::Mode::Paper   => "https://api.hyperliquid.xyz".to_string(),
        };

        Ok(HyperliquidClient {
            client: reqwest::Client::new(),
            base_url,
            testnet: matches!(config.mode, crate::config::Mode::Testnet),
        })
    }

    pub async fn get_account(&self) -> Result<Account> {
        // Mock account for demo
        Ok(Account {
            equity: 100.0,
            margin: 23.0,
            health_factor: 4.2,
            daily_pnl: 2.50,
            daily_loss_limit: 30.0,
        })
    }

    pub async fn place_order(&self, decision: &Decision) -> Result<String> {
        if decision.action == "SKIP" {
            return Err(anyhow::anyhow!("Decision is SKIP"));
        }

        // Mock order ID
        let order_id = uuid::Uuid::new_v4().to_string();
        log::info!("📍 Order placed: {}", order_id);
        Ok(order_id)
    }

    pub async fn get_positions(&self) -> Result<Vec<Position>> {
        // Mock positions
        Ok(vec![])
    }

    pub async fn close_position(&self, position: &Position) -> Result<String> {
        let order_id = uuid::Uuid::new_v4().to_string();
        log::info!("🔒 Position closed: {}", order_id);
        Ok(order_id)
    }
}
