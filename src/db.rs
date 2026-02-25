use anyhow::Result;
use crate::decision::Decision;

/// Lightweight no-op database wrapper for paper-trading mode.
/// When a real PostgreSQL URL is provided the struct can be extended,
/// but for now all writes silently succeed and reads return empty data.
#[derive(Clone)]
pub struct Database {
    #[allow(dead_code)]
    available: bool,
}

impl Database {
    pub async fn new(database_url: &str) -> Result<Self> {
        if database_url.starts_with("postgres://") || database_url.starts_with("postgresql://") {
            log::info!("Database: PostgreSQL URL detected ({}…) – using no-op stub", &database_url[..32.min(database_url.len())]);
        } else {
            log::info!("Database: no-op mode ({})", database_url);
        }
        Ok(Database { available: false })
    }

    pub async fn log_trade(&self, _decision: &Decision, _order_id: &str) -> Result<()> {
        Ok(())
    }

    pub async fn get_recent_trades(&self, _limit: i32) -> Result<Vec<serde_json::Value>> {
        Ok(vec![])
    }
}
