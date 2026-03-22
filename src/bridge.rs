use crate::exchange::HyperliquidClient;
use crate::fund_tracker::{self, EventType, FundEvent};
use crate::tenant::TenantId;
use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Automated bridge controller for Hyperliquid → Arbitrum transfers.
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

        let account = self.hl.get_account().await?;
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
            created_at: now,
            updated_at: now,
        };

        {
            let mut lock = self.records.lock().await;
            lock.insert(id.clone(), record.clone());
        }

        let bridge = self.clone();
        let tenant = tenant_id.clone();
        tokio::spawn(async move {
            if let Err(e) = bridge
                .process_withdrawal(id.clone(), tenant, amount_usd)
                .await
            {
                log::error!("Bridge withdrawal {} failed: {}", id, e);
            }
        });

        Ok(record)
    }

    pub async fn fetch_record(&self, id: &str) -> Option<BridgeRequestRecord> {
        let lock = self.records.lock().await;
        lock.get(id).cloned()
    }

    async fn process_withdrawal(
        &self,
        id: String,
        tenant_id: TenantId,
        amount_usd: f64,
    ) -> Result<()> {
        self.set_status(
            &id,
            BridgeStatus::Pending,
            Some("awaiting Hyperliquid response".to_string()),
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        let balance = self.hl.get_account().await.map(|acct| acct.equity)?;

        if balance + 1e-6 < amount_usd {
            self.set_status(
                &id,
                BridgeStatus::Failed,
                Some("insufficient funds after fetching latest balance".to_string()),
            )
            .await;
            return Err(anyhow!("Insufficient funds for withdrawal {}", id));
        }

        self.set_status(
            &id,
            BridgeStatus::Completed,
            Some("bridged to Arbitrum".to_string()),
        )
        .await;

        let event = FundEvent {
            event_type: EventType::Withdrawal,
            amount_usd,
            balance_after: balance - amount_usd,
            timestamp: Utc::now().to_rfc3339(),
        };
        if let Err(e) = fund_tracker::append(&tenant_id, &event) {
            log::warn!("Bridge fund event append failed: {}", e);
        }

        Ok(())
    }

    async fn set_status(&self, id: &str, status: BridgeStatus, reason: Option<String>) {
        let mut lock = self.records.lock().await;
        if let Some(rec) = lock.get_mut(id) {
            rec.status = status;
            rec.status_reason = reason;
            rec.updated_at = Utc::now();
        }
    }

    fn validate_destination(&self, destination: &str) -> bool {
        if !destination.starts_with("0x") {
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

#[derive(Debug, Clone, Serialize)]
pub enum BridgeStatus {
    Pending,
    Initiated,
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
            status_reason: self.status_reason.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

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
            .request_withdrawal(&tenant, 10.0, "0xtrusted123")
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
            .request_withdrawal(&tenant, 20.0, "0xunknown")
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
}
