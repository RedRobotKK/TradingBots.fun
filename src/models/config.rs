/// Configuration structures for trading systems
use serde::{Deserialize, Serialize};

/// Hyperliquid API configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HyperliquidConfig {
    pub base_url: String,
    pub ws_url: String,
    pub wallet_address: String,
    pub private_key: String,
    pub testnet: bool,
    pub request_timeout_secs: u64,
}

impl HyperliquidConfig {
    pub fn mainnet(wallet: String, key: String) -> Self {
        Self {
            base_url: "https://api.hyperliquid.com".to_string(),
            ws_url: "wss://api.hyperliquid.com/ws".to_string(),
            wallet_address: wallet,
            private_key: key,
            testnet: false,
            request_timeout_secs: 30,
        }
    }

    pub fn testnet(wallet: String, key: String) -> Self {
        Self {
            base_url: "https://testnet.hyperliquid.com".to_string(),
            ws_url: "wss://testnet.hyperliquid.com/ws".to_string(),
            wallet_address: wallet,
            private_key: key,
            testnet: true,
            request_timeout_secs: 30,
        }
    }
}

/// Drift Protocol configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DriftConfig {
    pub rpc_endpoint: String,
    pub commitment: String,
    pub wallet_keypair_path: String,
}

impl DriftConfig {
    pub fn devnet() -> Self {
        Self {
            rpc_endpoint: "https://api.devnet.solana.com".to_string(),
            commitment: "confirmed".to_string(),
            wallet_keypair_path: "~/.config/solana/id.json".to_string(),
        }
    }

    pub fn mainnet() -> Self {
        Self {
            rpc_endpoint: "https://api.mainnet-beta.solana.com".to_string(),
            commitment: "confirmed".to_string(),
            wallet_keypair_path: "~/.config/solana/id.json".to_string(),
        }
    }
}

/// Trading bot configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TradingConfig {
    pub drift: Option<DriftConfig>,
    pub hyperliquid: Option<HyperliquidConfig>,
    pub slippage_tolerance: f64,
    pub max_order_size: f64,
    pub risk_per_trade: f64,
    pub max_daily_loss: f64,
    pub rebalance_interval_secs: u64,
    pub decision_interval_ms: u64,
    pub health_check_interval_ms: u64,
}

impl Default for TradingConfig {
    fn default() -> Self {
        Self {
            drift: None,
            hyperliquid: None,
            slippage_tolerance: 0.01,  // 1%
            max_order_size: 10000.0,   // USDC
            risk_per_trade: 0.02,      // 2%
            max_daily_loss: 0.05,      // 5%
            rebalance_interval_secs: 300,
            decision_interval_ms: 1000,
            health_check_interval_ms: 5000,
        }
    }
}

/// Capital allocation configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CapitalAllocationConfig {
    pub tb_scalp_pct: f64,
    pub tb_swing_pct: f64,
    pub tb_position_pct: f64,
    pub hyperliquid_hft_pct: f64,
    pub hyperliquid_swing_pct: f64,
    pub reserve_pct: f64,
}

impl Default for CapitalAllocationConfig {
    fn default() -> Self {
        Self {
            tb_scalp_pct: 0.30,
            tb_swing_pct: 0.25,
            tb_position_pct: 0.20,
            hyperliquid_hft_pct: 0.15,
            hyperliquid_swing_pct: 0.10,
            reserve_pct: 0.0,  // Calculated from rest
        }
    }
}

impl CapitalAllocationConfig {
    pub fn validate(&self) -> crate::utils::Result<()> {
        let total: f64 = self.tb_scalp_pct
            + self.tb_swing_pct
            + self.tb_position_pct
            + self.hyperliquid_hft_pct
            + self.hyperliquid_swing_pct
            + self.reserve_pct;

        if (total - 1.0).abs() > 0.001 {
            return Err(crate::utils::Error::ConfigError(
                format!("Allocations must sum to 100%, got {:.1}%", total * 100.0),
            ));
        }

        Ok(())
    }
}

/// Liquidation prevention configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LiquidationConfig {
    pub warning_health_factor: f64,
    pub critical_health_factor: f64,
    pub emergency_health_factor: f64,
    pub monitoring_interval_ms: u64,
    pub auto_reduce_at_warning: bool,
    pub auto_emergency_deleverage: bool,
}

impl Default for LiquidationConfig {
    fn default() -> Self {
        Self {
            warning_health_factor: 1.5,
            critical_health_factor: 1.2,
            emergency_health_factor: 1.0,
            monitoring_interval_ms: 5000,
            auto_reduce_at_warning: true,
            auto_emergency_deleverage: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trading_config_defaults() {
        let config = TradingConfig::default();
        assert_eq!(config.slippage_tolerance, 0.01);
        assert_eq!(config.max_daily_loss, 0.05);
    }

    #[test]
    fn test_capital_allocation_validation() {
        let mut config = CapitalAllocationConfig::default();
        assert!(config.validate().is_ok());

        // Invalid: sum > 1.0
        config.tb_scalp_pct = 0.6;
        assert!(config.validate().is_err());

        // Valid again
        config.tb_scalp_pct = 0.30;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_hyperliquid_config() {
        let config = HyperliquidConfig::mainnet("addr".to_string(), "key".to_string());
        assert!(!config.testnet);
        assert!(config.base_url.contains("api.hyperliquid.com"));

        let testnet = HyperliquidConfig::testnet("addr".to_string(), "key".to_string());
        assert!(testnet.testnet);
        assert!(testnet.base_url.contains("testnet"));
    }
}
