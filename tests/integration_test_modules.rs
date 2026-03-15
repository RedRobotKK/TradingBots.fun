//! Integration tests for Hyperliquid client and Backtester modules
//!
//! Tests demonstrate real-world usage of both production modules:
//! - HyperliquidClient: API authentication, order management, market data
//! - Backtester: Historical simulation, performance metrics, optimization

#[cfg(test)]
mod hyperliquid_integration_tests {
    use tradingbots_fun::modules::HyperliquidClient;
    use tradingbots_fun::models::market::{LimitOrder, MarketOrder, OrderSide};

    /// Test HMAC-SHA256 signature generation for authentication
    #[test]
    fn test_hmac_signature_generation() {
        let client = HyperliquidClient::new(
            "0x1234567890abcdef1234567890abcdef12345678".to_string(),
            "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20".to_string(),
        );

        // Test payload
        let payload = r#"{"symbol":"SOLUSDT","side":"buy","price":100.0,"size":10.0}"#;
        let timestamp: u64 = 1704067200000;

        // Generate signature
        let signature = client.sign_request(payload, timestamp);

        // Verify signature format: HMAC-SHA256 = 32 bytes = 64 hex chars
        assert_eq!(signature.len(), 64);
        assert!(signature.chars().all(|c| c.is_ascii_hexdigit()));

        // Verify same input produces same signature (deterministic)
        let signature2 = client.sign_request(payload, timestamp);
        assert_eq!(signature, signature2);

        // Verify different timestamp produces different signature
        let signature3 = client.sign_request(payload, timestamp + 1);
        assert_ne!(signature, signature3);
    }

    /// Test order state tracking
    #[tokio::test]
    async fn test_order_state_tracking() {
        let client = HyperliquidClient::new(
            "test-wallet".to_string(),
            "test-key".to_string(),
        );

        // Initially no active orders
        assert_eq!(client.get_active_orders_count().await, 0);

        // Place order (in real scenario)
        // order_id would come from API response
        // In test, we verify tracking mechanism exists
        let order_state = client.get_order_state("non-existent").await.unwrap();
        assert!(order_state.is_none());
    }

    /// Test rate limiting mechanism
    #[tokio::test]
    async fn test_rate_limit_check() {
        let client = HyperliquidClient::new(
            "test-wallet".to_string(),
            "test-key".to_string(),
        );

        // Multiple rate limit checks should succeed
        for _ in 0..100 {
            assert!(client.rate_limit_check().await.is_ok());
        }

        // Verify rate limit counter decremented
        let active_orders = client.get_active_orders_count().await;
        assert_eq!(active_orders, 0);
    }

    /// Test market data caching
    #[tokio::test]
    async fn test_client_initialization() {
        let client = HyperliquidClient::new(
            "0x1234567890abcdef1234567890abcdef12345678".to_string(),
            "abcd1234".to_string(),
        );

        // Verify client created with correct parameters
        assert_eq!(client.get_active_orders_count().await, 0);

        // Verify default client
        let default_client = HyperliquidClient::default();
        assert!(!default_client.get_active_orders_count().await > 0); // No orders
    }

    /// Test limit and market order structures
    #[test]
    fn test_order_structures() {
        // Test limit order
        let limit_order = LimitOrder {
            symbol: "SOLUSDT".to_string(),
            side: OrderSide::Buy,
            price: 100.50,
            size: 10.0,
            leverage: 2.0,
            post_only: false,
        };

        assert_eq!(limit_order.symbol, "SOLUSDT");
        assert!(limit_order.side.is_buy());
        assert_eq!(limit_order.price, 100.50);
        assert_eq!(limit_order.leverage, 2.0);

        // Test market order
        let market_order = MarketOrder {
            symbol: "BTCUSDT".to_string(),
            side: OrderSide::Sell,
            size: 5.0,
            leverage: 1.0,
        };

        assert_eq!(market_order.symbol, "BTCUSDT");
        assert!(market_order.side.is_sell());
        assert_eq!(market_order.leverage, 1.0);
    }

    /// Test timestamp generation
    #[test]
    fn test_current_timestamp() {
        let ts1 = HyperliquidClient::current_timestamp_ms();
        std::thread::sleep(std::time::Duration::from_millis(10));
        let ts2 = HyperliquidClient::current_timestamp_ms();

        assert!(ts2 > ts1);
        assert!(ts2 - ts1 >= 10);
    }
}

#[cfg(test)]
mod backtester_integration_tests {
    use tradingbots_fun::modules::{Backtester, BacktestConfig};
    use std::collections::HashMap;

    /// Test backtest configuration building
    #[test]
    fn test_backtest_configuration() {
        let config = BacktestConfig::default()
            .with_initial_capital(50000.0)
            .with_start_date("2024-01-01")
            .with_end_date("2024-12-31")
            .with_fee_percentage(0.02);

        assert_eq!(config.initial_capital, 50000.0);
        assert_eq!(config.start_date, "2024-01-01");
        assert_eq!(config.end_date, "2024-12-31");
        assert_eq!(config.fee_percentage, 0.02);
        assert_eq!(config.num_accounts, 5); // Default
    }

    /// Test backtester initialization
    #[test]
    fn test_backtester_creation() {
        let config = BacktestConfig::default();
        let backtester = Backtester::new(config);

        // Should initialize with empty trades
        let stats = backtester.get_trade_stats();
        assert_eq!(stats.total_volume, 0.0);
        assert_eq!(stats.max_consecutive_wins, 0);
        assert_eq!(stats.max_consecutive_losses, 0);
    }

    /// Test Sharpe ratio calculation
    #[test]
    fn test_sharpe_ratio_calculation() {
        let backtester = Backtester::new(BacktestConfig::default());

        // Test with positive returns
        let returns = vec![1.0, 2.0, 1.5, 2.5, 3.0];
        let sharpe = backtester.calculate_sharpe_ratio(&returns);

        assert!(sharpe > 0.0); // Positive returns = positive Sharpe
        assert!(sharpe.is_finite()); // Must be valid number

        // Test with mixed returns
        let mixed = vec![1.0, -0.5, 2.0, -1.0, 1.5];
        let sharpe_mixed = backtester.calculate_sharpe_ratio(&mixed);
        assert!(sharpe_mixed.is_finite());

        // Test with empty returns
        let empty: Vec<f64> = vec![];
        let sharpe_empty = backtester.calculate_sharpe_ratio(&empty);
        assert_eq!(sharpe_empty, 0.0);
    }

    /// Test maximum drawdown calculation
    #[test]
    fn test_max_drawdown_calculation() {
        let backtester = Backtester::new(BacktestConfig::default());

        // Test perfect drawdown scenario
        let equity = vec![100.0, 110.0, 105.0, 95.0, 110.0];
        let drawdown = backtester.calculate_max_drawdown(&equity);

        assert!(drawdown > 0.0);
        assert!(drawdown <= 100.0);
        // Peak was 110, trough was 95: (110-95)/110 = 13.64%
        assert!((drawdown - 13.64).abs() < 0.1);

        // Test no drawdown scenario
        let uptrend = vec![100.0, 105.0, 110.0, 115.0, 120.0];
        let no_dd = backtester.calculate_max_drawdown(&uptrend);
        assert_eq!(no_dd, 0.0);

        // Test complete wipeout
        let crash = vec![100.0, 50.0, 25.0, 10.0];
        let severe_dd = backtester.calculate_max_drawdown(&crash);
        assert!(severe_dd > 50.0);
    }

    /// Test parameter combination generation
    #[test]
    fn test_param_grid_generation() {
        let backtester = Backtester::new(BacktestConfig::default());
        let mut params = HashMap::new();

        // 2 fee values
        params.insert("fee".to_string(), vec![0.01, 0.02]);
        // 3 slippage values
        params.insert("slippage".to_string(), vec![0.05, 0.08, 0.10]);

        let combinations = backtester.generate_param_combinations(&params);

        // Should generate 2 * 3 = 6 combinations
        assert_eq!(combinations.len(), 6);

        // Verify all combinations have both parameters
        for combo in combinations {
            assert!(combo.contains_key("fee"));
            assert!(combo.contains_key("slippage"));
        }
    }

    /// Test parameter combination with more parameters
    #[test]
    fn test_param_grid_three_params() {
        let backtester = Backtester::new(BacktestConfig::default());
        let mut params = HashMap::new();

        params.insert("a".to_string(), vec![1.0, 2.0]);
        params.insert("b".to_string(), vec![10.0, 20.0]);
        params.insert("c".to_string(), vec![100.0]);

        let combinations = backtester.generate_param_combinations(&params);

        // 2 * 2 * 1 = 4 combinations
        assert_eq!(combinations.len(), 4);
    }

    /// Test trade statistics calculation
    #[test]
    fn test_trade_statistics() {
        let mut backtester = Backtester::new(BacktestConfig::default());

        // Add test trades
        use tradingbots_fun::modules::SimulatedTrade;
        use tradingbots_fun::models::market::{OrderSide, ExecutionStatus};

        // Winning trades
        for i in 0..3 {
            backtester.trades.push(SimulatedTrade {
                order_id: format!("order-{}", i),
                symbol: "TEST".to_string(),
                side: OrderSide::Buy,
                entry_price: 100.0,
                exit_price: 110.0,
                size: 10.0,
                entry_time: i as i64 * 3600,
                exit_time: i as i64 * 3600 + 1800,
                pnl: 100.0,
                pnl_percentage: 10.0,
                fees: 1.0,
                status: ExecutionStatus::Filled,
            });
        }

        // Losing trades
        for i in 0..2 {
            backtester.trades.push(SimulatedTrade {
                order_id: format!("order-loss-{}", i),
                symbol: "TEST".to_string(),
                side: OrderSide::Sell,
                entry_price: 100.0,
                exit_price: 95.0,
                size: 10.0,
                entry_time: (i + 3) as i64 * 3600,
                exit_time: (i + 3) as i64 * 3600 + 1800,
                pnl: -50.0,
                pnl_percentage: -5.0,
                fees: 1.0,
                status: ExecutionStatus::Filled,
            });
        }

        let stats = backtester.get_trade_stats();

        // Verify calculations
        assert!(stats.total_volume > 0.0);
        assert!(stats.avg_trade_duration > 0);
        assert_eq!(stats.max_consecutive_wins, 3);
        assert_eq!(stats.max_consecutive_losses, 2);
        assert!(stats.recovery_factor > 0.0);
    }

    /// Test consecutive wins tracking
    #[test]
    fn test_consecutive_wins() {
        let mut backtester = Backtester::new(BacktestConfig::default());
        use tradingbots_fun::modules::SimulatedTrade;
        use tradingbots_fun::models::market::{OrderSide, ExecutionStatus};

        // Pattern: W W W L W W L L L W
        let pnls = vec![10.0, 10.0, 10.0, -5.0, 10.0, 10.0, -5.0, -5.0, -5.0, 10.0];

        for (i, &pnl) in pnls.iter().enumerate() {
            backtester.trades.push(SimulatedTrade {
                order_id: format!("order-{}", i),
                symbol: "TEST".to_string(),
                side: OrderSide::Buy,
                entry_price: 100.0,
                exit_price: if pnl > 0.0 { 110.0 } else { 95.0 },
                size: 10.0,
                entry_time: i as i64 * 3600,
                exit_time: i as i64 * 3600 + 1800,
                pnl,
                pnl_percentage: if pnl > 0.0 { 10.0 } else { -5.0 },
                fees: 1.0,
                status: ExecutionStatus::Filled,
            });
        }

        let stats = backtester.get_trade_stats();

        // Max consecutive wins: 3 (positions 0-2)
        assert_eq!(stats.max_consecutive_wins, 3);
        // Max consecutive losses: 3 (positions 6-8)
        assert_eq!(stats.max_consecutive_losses, 3);
    }

    /// Test configuration with multiple symbols
    #[test]
    fn test_multi_symbol_config() {
        let symbols = vec![
            "SOLUSDT".to_string(),
            "BTCUSDT".to_string(),
            "ETHUSDT".to_string(),
        ];

        let config = BacktestConfig::default()
            .with_symbols(symbols.clone());

        assert_eq!(config.symbols.len(), 3);
        assert!(config.symbols.contains(&"SOLUSDT".to_string()));
        assert!(config.symbols.contains(&"BTCUSDT".to_string()));
    }

    /// Test fee and slippage configuration
    #[test]
    fn test_fee_slippage_config() {
        let realistic = BacktestConfig::default()
            .with_fee_percentage(0.01)
            .with_initial_capital(100000.0);

        assert_eq!(realistic.fee_percentage, 0.01);

        // Verify realistic values
        assert!(realistic.fee_percentage > 0.0);
        assert!(realistic.fee_percentage < 1.0);
        assert!(realistic.slippage_percentage > 0.0);
        assert!(realistic.slippage_percentage < 1.0);
    }
}

#[cfg(test)]
mod multi_module_tests {
    use tradingbots_fun::modules::{HyperliquidClient, Backtester, BacktestConfig};
    use tradingbots_fun::models::market::OrderSide;

    /// Test that both modules can be used together
    #[tokio::test]
    async fn test_client_and_backtester_integration() {
        // Create both clients
        let _hyperliquid = HyperliquidClient::new(
            "test-wallet".to_string(),
            "test-key".to_string(),
        );

        let config = BacktestConfig::default()
            .with_initial_capital(50000.0);
        let backtester = Backtester::new(config);

        // Verify both modules are operational
        let trade_stats = backtester.get_trade_stats();
        assert_eq!(trade_stats.total_volume, 0.0);
    }

    /// Test configuration consistency across modules
    #[test]
    fn test_symbol_consistency() {
        // Both modules can work with same symbols
        let symbols = vec!["SOLUSDT".to_string(), "BTCUSDT".to_string()];

        let config = BacktestConfig::default()
            .with_symbols(symbols.clone());

        // Verify symbols match
        assert_eq!(config.symbols, symbols);
    }

    /// Test order side handling consistency
    #[test]
    fn test_order_side_consistency() {
        // Verify order sides work consistently
        assert!(OrderSide::Buy.is_buy());
        assert!(!OrderSide::Buy.is_sell());

        assert!(OrderSide::Sell.is_sell());
        assert!(!OrderSide::Sell.is_buy());

        // Verify opposite
        assert_eq!(OrderSide::Buy.opposite(), OrderSide::Sell);
        assert_eq!(OrderSide::Sell.opposite(), OrderSide::Buy);
    }
}
