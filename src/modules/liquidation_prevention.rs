/// Liquidation prevention and risk management
use crate::utils::Result;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Liquidation monitoring and prevention
pub struct LiquidationPrevention {
    warning_threshold: f64,
    critical_threshold: f64,
    emergency_threshold: f64,
    monitoring_interval_ms: u64,
    alerts: Arc<RwLock<Vec<RiskAlert>>>,
}

#[derive(Clone, Debug)]
pub struct RiskAlert {
    pub timestamp: i64,
    pub account_id: String,
    pub risk_level: RiskLevel,
    pub health_factor: f64,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiskLevel {
    Safe,
    Warning,
    Critical,
    Emergency,
}

impl LiquidationPrevention {
    /// Create new liquidation prevention system
    pub fn new(warning: f64, critical: f64, emergency: f64) -> Self {
        Self {
            warning_threshold: warning,
            critical_threshold: critical,
            emergency_threshold: emergency,
            monitoring_interval_ms: 5000,
            alerts: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Create with defaults (Solana/Hyperliquid style thresholds)
    pub fn default_solana() -> Self {
        Self::new(1.5, 1.2, 1.0)
    }

    /// Assess account health
    pub fn assess_health(&self, health_factor: f64) -> RiskLevel {
        match health_factor {
            hf if hf > self.warning_threshold => RiskLevel::Safe,
            hf if hf > self.critical_threshold => RiskLevel::Warning,
            hf if hf > self.emergency_threshold => RiskLevel::Critical,
            _ => RiskLevel::Emergency,
        }
    }

    /// Check if account needs attention
    pub fn needs_action(&self, health_factor: f64) -> bool {
        let risk = self.assess_health(health_factor);
        risk != RiskLevel::Safe
    }

    /// Calculate safe leverage based on current health
    pub fn calculate_safe_leverage(
        &self,
        current_leverage: f64,
        health_factor: f64,
    ) -> f64 {
        let risk = self.assess_health(health_factor);

        match risk {
            RiskLevel::Safe => current_leverage,
            RiskLevel::Warning => current_leverage * 0.75, // Reduce to 75%
            RiskLevel::Critical => current_leverage * 0.5, // Reduce to 50%
            RiskLevel::Emergency => current_leverage * 0.25, // Reduce to 25%
        }
    }

    /// Calculate position reduction needed
    pub fn calculate_position_reduction(&self, health_factor: f64) -> f64 {
        let risk = self.assess_health(health_factor);

        match risk {
            RiskLevel::Safe => 0.0, // No reduction
            RiskLevel::Warning => 0.1, // Reduce 10%
            RiskLevel::Critical => 0.25, // Reduce 25%
            RiskLevel::Emergency => 0.5, // Reduce 50% immediately
        }
    }

    /// Log alert
    pub async fn log_alert(&self, alert: RiskAlert) -> Result<()> {
        let mut alerts = self.alerts.write().await;
        alerts.push(alert);

        // Keep last 10000 alerts
        if alerts.len() > 10000 {
            alerts.remove(0);
        }

        Ok(())
    }

    /// Get recent alerts
    pub async fn get_alerts(&self, limit: usize) -> Result<Vec<RiskAlert>> {
        let alerts = self.alerts.read().await;
        Ok(alerts
            .iter()
            .rev()
            .take(limit)
            .cloned()
            .collect())
    }

    /// Monitor account and generate alerts
    pub async fn monitor_account(
        &self,
        account_id: &str,
        health_factor: f64,
    ) -> Result<Option<RiskAlert>> {
        let risk_level = self.assess_health(health_factor);

        if risk_level == RiskLevel::Safe {
            return Ok(None);
        }

        let alert = RiskAlert {
            timestamp: chrono::Utc::now().timestamp(),
            account_id: account_id.to_string(),
            risk_level,
            health_factor,
            reason: self.generate_reason(risk_level, health_factor),
        };

        self.log_alert(alert.clone()).await?;

        Ok(Some(alert))
    }

    /// Generate alert message
    fn generate_reason(&self, risk_level: RiskLevel, health_factor: f64) -> String {
        match risk_level {
            RiskLevel::Safe => "Account healthy".to_string(),
            RiskLevel::Warning => {
                format!(
                    "Health factor {:.2} approaching warning threshold {:.2}",
                    health_factor, self.warning_threshold
                )
            }
            RiskLevel::Critical => {
                format!(
                    "Health factor {:.2} in critical range (threshold: {:.2})",
                    health_factor, self.critical_threshold
                )
            }
            RiskLevel::Emergency => {
                format!(
                    "EMERGENCY: Health factor {:.2} near liquidation",
                    health_factor
                )
            }
        }
    }

    /// Emergency deleveraging procedure
    pub fn emergency_deleverage_plan(&self, current_leverage: f64) -> f64 {
        // Reduce to minimum safe leverage
        (current_leverage / 4.0).max(1.0)
    }

    /// Estimate time to liquidation
    pub fn estimate_time_to_liquidation(
        &self,
        _current_price: f64,
        liquidation_price: f64,
        mark_price: f64,
    ) -> Option<u64> {
        if mark_price == liquidation_price {
            return None;
        }

        let distance = (mark_price - liquidation_price).abs();
        let current_distance_pct = (distance / mark_price) * 100.0;

        // Assume 1% movement per minute under normal conditions
        let minutes_to_liquidation = current_distance_pct / 1.0;

        if minutes_to_liquidation > 0.0 {
            Some(minutes_to_liquidation as u64)
        } else {
            None
        }
    }
}

impl Default for LiquidationPrevention {
    fn default() -> Self {
        Self::default_solana()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assess_health() {
        let prev = LiquidationPrevention::default_solana();

        assert_eq!(prev.assess_health(2.0), RiskLevel::Safe);
        assert_eq!(prev.assess_health(1.3), RiskLevel::Warning);
        assert_eq!(prev.assess_health(1.1), RiskLevel::Critical);
        assert_eq!(prev.assess_health(0.9), RiskLevel::Emergency);
    }

    #[test]
    fn test_safe_leverage() {
        let prev = LiquidationPrevention::default_solana();

        assert_eq!(prev.calculate_safe_leverage(100.0, 2.0), 100.0);
        assert_eq!(prev.calculate_safe_leverage(100.0, 1.3), 75.0);
        assert_eq!(prev.calculate_safe_leverage(100.0, 1.1), 50.0);
        assert_eq!(prev.calculate_safe_leverage(100.0, 0.9), 25.0);
    }

    #[test]
    fn test_position_reduction() {
        let prev = LiquidationPrevention::default_solana();

        assert_eq!(prev.calculate_position_reduction(2.0), 0.0);
        assert_eq!(prev.calculate_position_reduction(1.3), 0.1);
        assert_eq!(prev.calculate_position_reduction(1.1), 0.25);
        assert_eq!(prev.calculate_position_reduction(0.9), 0.5);
    }

    #[test]
    fn test_needs_action() {
        let prev = LiquidationPrevention::default_solana();

        assert!(!prev.needs_action(2.0));
        assert!(prev.needs_action(1.3));
        assert!(prev.needs_action(1.1));
        assert!(prev.needs_action(0.9));
    }

    #[tokio::test]
    async fn test_monitor_account() {
        let prev = LiquidationPrevention::default_solana();

        let alert = prev.monitor_account("acc1", 1.3).await.unwrap();
        assert!(alert.is_some());

        let alert_unwrapped = alert.unwrap();
        assert_eq!(alert_unwrapped.account_id, "acc1");
        assert_eq!(alert_unwrapped.risk_level, RiskLevel::Warning);
    }

    #[tokio::test]
    async fn test_get_alerts() {
        let prev = LiquidationPrevention::default_solana();

        prev.monitor_account("acc1", 1.3).await.unwrap();
        prev.monitor_account("acc2", 1.1).await.unwrap();

        let alerts = prev.get_alerts(10).await.unwrap();
        assert_eq!(alerts.len(), 2);
    }

    #[test]
    fn test_emergency_deleverage() {
        let prev = LiquidationPrevention::default_solana();
        assert_eq!(prev.emergency_deleverage_plan(100.0), 25.0);
        assert_eq!(prev.emergency_deleverage_plan(5.0), 1.25);
    }
}
