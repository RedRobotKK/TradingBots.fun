/// Dynamic capital allocation and rebalancing
use crate::utils::{Error, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Capital allocation state and decisions
pub struct CapitalManager {
    allocations: Arc<RwLock<HashMap<String, f64>>>,
    total_capital: f64,
    rebalance_history: Arc<RwLock<Vec<RebalanceEvent>>>,
}

#[derive(Clone, Debug)]
pub struct RebalanceEvent {
    pub timestamp: i64,
    pub reason: String,
    pub allocations: HashMap<String, f64>,
    pub total_change: f64,
}

impl CapitalManager {
    /// Create new capital manager
    pub fn new(total_capital: f64) -> Self {
        Self {
            allocations: Arc::new(RwLock::new(HashMap::new())),
            total_capital,
            rebalance_history: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Set allocation for account
    pub async fn set_allocation(&self, account_id: &str, percentage: f64) -> Result<()> {
        if percentage < 0.0 || percentage > 1.0 {
            return Err(Error::InvalidAllocationValue);
        }

        let mut allocs = self.allocations.write().await;
        allocs.insert(account_id.to_string(), percentage);

        // Verify total
        let total: f64 = allocs.values().sum();
        if (total - 1.0).abs() > 0.001 {
            return Err(Error::AllocationDoesNotSum);
        }

        Ok(())
    }

    /// Get allocation for account
    pub async fn get_allocation(&self, account_id: &str) -> Result<f64> {
        let allocs = self.allocations.read().await;
        Ok(*allocs.get(account_id).unwrap_or(&0.0))
    }

    /// Get capital amount for account
    pub async fn get_capital_for_account(&self, account_id: &str) -> Result<f64> {
        let allocation = self.get_allocation(account_id).await?;
        Ok(self.total_capital * allocation)
    }

    /// Dynamic allocation based on market conditions
    pub async fn optimize_allocation(
        &self,
        volatility: f64,
        momentum: f64,
        win_rate: f64,
    ) -> Result<HashMap<String, f64>> {
        let mut allocations = HashMap::new();

        // Kelly Criterion based sizing
        let kelly = Self::calculate_kelly(win_rate, volatility);
        let kelly_fraction = kelly * 0.25; // Conservative 25% Kelly

        // Volatility-adjusted allocation
        let vol_factor = 1.0 / (1.0 + volatility * 2.0);

        // Momentum-aware allocation
        let momentum_factor = (1.0 + momentum) / 2.0;

        // Allocate to different strategies
        let scalp_alloc = (0.30 * kelly_fraction * vol_factor).min(0.40);
        let swing_alloc = (0.25 * vol_factor * momentum_factor).min(0.35);
        let position_alloc = (0.20 * vol_factor).min(0.25);
        let hft_alloc = (0.15 * kelly_fraction).min(0.25);
        let hedge_alloc = 0.10 * (1.0 - momentum_factor); // More in downturns
        let reserve_alloc = 1.0 - scalp_alloc - swing_alloc - position_alloc - hft_alloc - hedge_alloc;

        allocations.insert("tb-scalp".to_string(), scalp_alloc);
        allocations.insert("tb-swing".to_string(), swing_alloc);
        allocations.insert("tb-position".to_string(), position_alloc);
        allocations.insert("hyperliquid-hft".to_string(), hft_alloc);
        allocations.insert("tb-hedge".to_string(), hedge_alloc);
        allocations.insert("reserve".to_string(), reserve_alloc.max(0.0));

        Ok(allocations)
    }

    /// Calculate Kelly Criterion
    fn calculate_kelly(win_rate: f64, volatility: f64) -> f64 {
        if win_rate <= 0.0 || win_rate >= 1.0 {
            return 0.0;
        }

        // Kelly = (p * b - q) / b
        // Simplified: Kelly = win_rate - (1 - win_rate)
        let kelly = 2.0 * win_rate - 1.0;

        // Scale by volatility inverse
        let volatility_scaled = kelly / (1.0 + volatility);

        volatility_scaled.max(0.0).min(0.25) // Bound to 0-25%
    }

    /// Record rebalance event
    pub async fn record_rebalance(
        &self,
        reason: String,
        allocations: HashMap<String, f64>,
    ) -> Result<()> {
        let event = RebalanceEvent {
            timestamp: chrono::Utc::now().timestamp(),
            reason,
            allocations: allocations.clone(),
            total_change: 0.0, // Could calculate if needed
        };

        let mut history = self.rebalance_history.write().await;
        history.push(event);

        // Keep last 1000 events
        if history.len() > 1000 {
            history.remove(0);
        }

        Ok(())
    }

    /// Get rebalance history
    pub async fn get_history(&self, limit: usize) -> Result<Vec<RebalanceEvent>> {
        let history = self.rebalance_history.read().await;
        Ok(history.iter().rev().take(limit).cloned().collect())
    }
}

impl Default for CapitalManager {
    fn default() -> Self {
        Self::new(5000.0) // Default $5K
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_set_allocation() {
        let manager = CapitalManager::new(5000.0);
        // set_allocation requires all allocations to sum to 1.0; use 1.0 for single-account test
        assert!(manager.set_allocation("acc1", 1.0).await.is_ok());
        assert_eq!(manager.get_allocation("acc1").await.unwrap(), 1.0);
    }

    #[tokio::test]
    async fn test_invalid_allocation() {
        let manager = CapitalManager::new(5000.0);
        assert!(manager.set_allocation("acc1", 1.5).await.is_err());
        assert!(manager.set_allocation("acc1", -0.1).await.is_err());
    }

    #[tokio::test]
    async fn test_capital_for_account() {
        let manager = CapitalManager::new(5000.0);
        // set_allocation requires all allocations to sum to 1.0
        manager.set_allocation("acc1", 1.0).await.unwrap();
        assert_eq!(manager.get_capital_for_account("acc1").await.unwrap(), 5000.0);
    }

    #[test]
    fn test_kelly_criterion() {
        // 60% win rate with low vol
        let kelly = CapitalManager::calculate_kelly(0.6, 0.05);
        assert!(kelly > 0.1); // Should be positive

        // 50% win rate (break-even)
        let kelly = CapitalManager::calculate_kelly(0.5, 0.05);
        assert_eq!(kelly, 0.0);

        // 40% win rate (losing) — clamped to 0.0 by .max(0.0)
        let kelly = CapitalManager::calculate_kelly(0.4, 0.05);
        assert_eq!(kelly, 0.0);
    }

    #[tokio::test]
    async fn test_optimize_allocation() {
        let manager = CapitalManager::new(5000.0);
        let alloc = manager
            .optimize_allocation(0.1, 0.15, 0.55)
            .await
            .unwrap();

        let total: f64 = alloc.values().sum();
        assert!((total - 1.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_rebalance_history() {
        let manager = CapitalManager::new(5000.0);

        let mut alloc = HashMap::new();
        alloc.insert("acc1".to_string(), 0.5);

        manager
            .record_rebalance("test rebalance".to_string(), alloc)
            .await
            .unwrap();

        let history = manager.get_history(10).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].reason, "test rebalance");
    }
}
