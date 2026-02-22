/// Multi-protocol, multi-chain trading system for Drift, Hyperliquid, and more
///
/// Features:
/// - Multi-account management across protocols
/// - Dynamic capital allocation
/// - Cross-chain bridging
/// - Liquidation prevention
/// - JPY/USD hedging
/// - RWA support

pub mod models;
pub mod modules;
pub mod utils;
pub mod strategies;
pub mod backtest;
pub mod simulator;
pub mod fee_calculator;
pub mod position_manager;
pub mod dynamic_position_sizing;
pub mod frameworks;
pub mod ai_decision_engine;
pub mod dashboard;
pub mod strategy_attribution;
pub mod strategy_analytics;
pub mod scoring_system;
pub mod strategies::institutional;
pub mod dca_scoring_integration;

pub use models::{AccountPurpose, HealthMetrics, LiquidationRisk, Protocol, TradingAccount};
pub use modules::{AccountManager, AccountSummary};
pub use utils::{Error, Result};
pub use strategies::{StrategySignal, StrategyContext, MarketSnapshot};
pub use backtest::{Backtester, BacktestConfig};
pub use simulator::{Simulator, SimulationResults};
pub use fee_calculator::{FeeCalculator, FeeStructure};
pub use position_manager::{AggregatePosition, ExitCalculator, ExitStrategy, DCARules};
pub use dynamic_position_sizing::{DynamicSizer, SupportResistance, TechnicalSetup, DynamicPositionSize};
pub use frameworks::{
    VolatilityAnalysis, VolatilityRegime, MultiTimeframeAnalysis, KellyCriterion,
    DrawdownTracker, StrategyAttributor, OrderFlowAnalysis, VolatilityScaler, MonteCarloResult,
};
pub use ai_decision_engine::{AIDecisionEngine, AIDecision, AIDecisionContext, AIDecisionValidator};
pub use dashboard::{DashboardMetrics, DashboardBuilder, CompleteDashboard, SentimentAnalyzer, SentimentMetrics, AIThoughts, RecentTrade, SystemAlert, AlertLevel};
pub use strategy_attribution::{StrategyAttributor, AttributedTrade, StrategyMetrics, MarketRegime, RegimeSpecificMetrics, StrategyCorrelation};
pub use strategy_analytics::{StrategyAnalytics, StrategyViability, CryptoStrategyProfile, StrategyComparison, MarketSpecificPerformance};
pub use scoring_system::{StrategyScore, StrategyScorer, ScoringAction, PortfolioScore, PortfolioAction, calculate_portfolio_score};
pub use strategies::institutional::{InstitutionalConfig, FundingRateConfig, PairsTradingConfig, OrderFlowConfig, SentimentConfig, VolatilityConfig, evaluate_funding_rate, evaluate_pairs_trading, evaluate_order_flow, evaluate_sentiment, evaluate_volatility_surface};
pub use dca_scoring_integration::{DCAPyramidDecision, CapitalStaging, PyramidStrategy, evaluate_dca_entry, create_pyramid_entry, get_pyramid_strategy_for_regime};

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod integration_tests {
    use crate::*;

    #[test]
    fn test_full_account_setup_workflow() {
        let mut manager = AccountManager::new();

        // Create multiple accounts with different strategies
        let mut scalp_account = TradingAccount::new(
            "drift-scalp-1".to_string(),
            Protocol::Drift,
            "7x".to_string(),
            AccountPurpose::Scalp,
        );
        scalp_account.capital_allocation = 0.30;

        let mut swing_account = TradingAccount::new(
            "drift-swing-1".to_string(),
            Protocol::Drift,
            "7y".to_string(),
            AccountPurpose::Swing,
        );
        swing_account.capital_allocation = 0.25;

        let mut position_account = TradingAccount::new(
            "drift-position-1".to_string(),
            Protocol::Drift,
            "7z".to_string(),
            AccountPurpose::Position,
        );
        position_account.capital_allocation = 0.20;

        let mut hedge_account = TradingAccount::new(
            "drift-hedge-1".to_string(),
            Protocol::Drift,
            "7w".to_string(),
            AccountPurpose::Hedge,
        );
        hedge_account.capital_allocation = 0.15;

        let mut reserve_account = TradingAccount::new(
            "drift-reserve-1".to_string(),
            Protocol::Drift,
            "7v".to_string(),
            AccountPurpose::Reserve,
        );
        reserve_account.capital_allocation = 0.10;

        // Register all accounts
        assert!(manager.register_account(scalp_account).is_ok());
        assert!(manager.register_account(swing_account).is_ok());
        assert!(manager.register_account(position_account).is_ok());
        assert!(manager.register_account(hedge_account).is_ok());
        assert!(manager.register_account(reserve_account).is_ok());

        // Verify counts
        assert_eq!(manager.total_account_count(), 5);
        assert_eq!(manager.active_account_count(), 5);

        // Verify capital allocation sums to 1.0
        assert_eq!(manager.total_capital_allocated(), 1.0);

        // Verify each purpose has correct account
        assert_eq!(
            manager.get_accounts_by_purpose(AccountPurpose::Scalp).len(),
            1
        );
        assert_eq!(
            manager.get_accounts_by_purpose(AccountPurpose::Swing).len(),
            1
        );
        assert_eq!(
            manager.get_accounts_by_purpose(AccountPurpose::Position).len(),
            1
        );
        assert_eq!(
            manager.get_accounts_by_purpose(AccountPurpose::Hedge).len(),
            1
        );
        assert_eq!(
            manager.get_accounts_by_purpose(AccountPurpose::Reserve).len(),
            1
        );

        // Verify all Drift
        assert_eq!(
            manager.get_accounts_by_protocol(Protocol::Drift).len(),
            5
        );
    }

    #[test]
    fn test_capital_rebalancing() {
        let mut manager = AccountManager::new();

        let mut acc1 = TradingAccount::new(
            "acc1".to_string(),
            Protocol::Drift,
            "key1".to_string(),
            AccountPurpose::Scalp,
        );
        acc1.capital_allocation = 0.50;

        let mut acc2 = TradingAccount::new(
            "acc2".to_string(),
            Protocol::Drift,
            "key2".to_string(),
            AccountPurpose::Swing,
        );
        acc2.capital_allocation = 0.50;

        manager.register_account(acc1).unwrap();
        manager.register_account(acc2).unwrap();

        // Rebalance
        manager.set_capital_allocation("acc1", 0.30).unwrap();
        manager.set_capital_allocation("acc2", 0.70).unwrap();

        assert_eq!(
            manager.get_account("acc1").unwrap().capital_allocation,
            0.30
        );
        assert_eq!(
            manager.get_account("acc2").unwrap().capital_allocation,
            0.70
        );
        assert_eq!(manager.total_capital_allocated(), 1.0);
    }

    #[test]
    fn test_leverage_adjustment_by_purpose() {
        let mut manager = AccountManager::new();

        let scalp = TradingAccount::new(
            "scalp".to_string(),
            Protocol::Drift,
            "key".to_string(),
            AccountPurpose::Scalp,
        );

        let swing = TradingAccount::new(
            "swing".to_string(),
            Protocol::Drift,
            "key".to_string(),
            AccountPurpose::Swing,
        );

        manager.register_account(scalp).unwrap();
        manager.register_account(swing).unwrap();

        // Max leverage for Scalp is 100
        assert!(manager.set_leverage("scalp", 100.0).is_ok());
        assert!(manager.set_leverage("scalp", 101.0).is_err());

        // Max leverage for Swing is 20
        assert!(manager.set_leverage("swing", 20.0).is_ok());
        assert!(manager.set_leverage("swing", 21.0).is_err());
    }
}
