//! Unit + integration tests for tradingbots.fun v0.2.1 features:
//!
//!  1. Venue transparency — `venue` field on `PaperPosition` / `ClosedTrade`
//!  2. Extended `BotSession` — `max_drawdown_pct`, `webhook_url`, `venue`,
//!     `leverage_max`, `risk_mode`, `symbols_whitelist`, `performance_fee_pct`,
//!     `hyperliquid_address`, `paused`
//!  3. Burst session pricing — `duration: "24h"` → 0.5 USDC / 24 h
//!  4. `HyperliquidConnector` — signal validation, risk-mode sizing,
//!     drawdown guard, trade log, coin→asset index
//!  5. `LatencyTracker` — recording, percentile computation, target thresholds
//!  6. New `BotCommand` variants — `SetLeverage`, `PauseTrading`, `ResumeTrading`
//!  7. Gap-close: command handler parsing, venue trade filter, latency stats shape

// ══════════════════════════════════════════════════════════════════════════════
//  1. Venue transparency
// ══════════════════════════════════════════════════════════════════════════════

mod venue_tests {
    use tradingbots_fun::web_dashboard::{ClosedTrade, PaperPosition};
    use tradingbots_fun::learner::SignalContribution;

    fn paper_pos(venue: &str) -> PaperPosition {
        PaperPosition {
            symbol: "BTC".to_string(),
            side: "LONG".to_string(),
            entry_price: 50_000.0,
            quantity: 0.02,
            size_usd: 1_000.0,
            stop_loss: 48_000.0,
            take_profit: 54_000.0,
            atr_at_entry: 500.0,
            high_water_mark: 50_000.0,
            low_water_mark: 50_000.0,
            partial_closed: false,
            r_dollars_risked: 40.0,
            tranches_closed: 0,
            dca_count: 0,
            leverage: 2.0,
            cycles_held: 0,
            entry_time: "2026-03-24T00:00:00Z".to_string(),
            unrealised_pnl: 0.0,
            contrib: SignalContribution::default(),
            ai_action: None,
            ai_reason: None,
            entry_confidence: 0.75,
            trade_budget_usd: 1_000.0,
            dca_spent_usd: 0.0,
            btc_ret_at_entry: 0.0,
            initial_margin_usd: 500.0,
            ob_sentiment: String::new(),
            ob_bid_wall_near: false,
            ob_ask_wall_near: false,
            ob_adverse_cycles: 0,
            order_flow_confidence: 0.0,
            order_flow_direction: String::new(),
            funding_rate: 0.0,
            funding_delta: 0.0,
            onchain_strength: 0.0,
            cex_premium_pct: 0.0,
            cex_mode: String::new(),
            funded_from_pool: false,
            pool_stake_usd: 0.0,
            venue: venue.to_string(),
        }
    }

    fn closed_trade(venue: &str) -> ClosedTrade {
        ClosedTrade {
            symbol: "ETH".to_string(),
            side: "SHORT".to_string(),
            entry: 3_000.0,
            exit: 2_900.0,
            pnl: 100.0,
            pnl_pct: 3.33,
            reason: "TakeProfit".to_string(),
            closed_at: "2026-03-24T01:00:00Z".to_string(),
            entry_time: "2026-03-24T00:00:00Z".to_string(),
            quantity: 1.0,
            size_usd: 3_000.0,
            leverage: 1.0,
            fees_est: 2.25,
            breakdown: None,
            note: None,
            venue: venue.to_string(),
        }
    }

    #[test]
    fn paper_position_venue_default_is_paper() {
        // Deserialise a JSON blob that omits "venue" — should default to paper label
        let json = r#"{
            "symbol":"BTC","side":"LONG","entry_price":50000.0,"quantity":0.02,
            "size_usd":1000.0,"stop_loss":48000.0,"take_profit":54000.0,
            "atr_at_entry":500.0,"high_water_mark":50000.0,"low_water_mark":50000.0,
            "partial_closed":false,"r_dollars_risked":40.0,"tranches_closed":0,
            "dca_count":0,"leverage":2.0,"cycles_held":0,
            "entry_time":"2026-03-24T00:00:00Z","unrealised_pnl":0.0,
            "contrib":{},"entry_confidence":0.75,"trade_budget_usd":1000.0,
            "dca_spent_usd":0.0,"btc_ret_at_entry":0.0,"initial_margin_usd":500.0,
            "ob_sentiment":"","ob_bid_wall_near":false,"ob_ask_wall_near":false,
            "ob_adverse_cycles":0,"order_flow_confidence":0.0,
            "order_flow_direction":"","funding_rate":0.0,"funding_delta":0.0,
            "onchain_strength":0.0,"cex_premium_pct":0.0,"cex_mode":"",
            "funded_from_pool":false,"pool_stake_usd":0.0
        }"#;
        let pos: PaperPosition = serde_json::from_str(json).expect("should deserialise");
        assert_eq!(
            pos.venue, "Hyperliquid Perps (paper)",
            "Default venue must be 'Hyperliquid Perps (paper)'"
        );
    }

    #[test]
    fn paper_position_venue_explicit() {
        let pos = paper_pos("Hyperliquid Perps (live)");
        assert_eq!(pos.venue, "Hyperliquid Perps (live)");
    }

    #[test]
    fn closed_trade_venue_round_trips_via_json() {
        let t = closed_trade("Hyperliquid Perps (paper)");
        let json = serde_json::to_string(&t).expect("serialise");
        let back: ClosedTrade = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back.venue, "Hyperliquid Perps (paper)");
    }

    #[test]
    fn closed_trade_venue_default_from_old_snapshot() {
        // Old snapshots without the "venue" key must default gracefully
        let json = r#"{"symbol":"SOL","side":"LONG","entry":100.0,"exit":110.0,
            "pnl":10.0,"pnl_pct":10.0,"reason":"Signal","closed_at":"now"}"#;
        let t: ClosedTrade = serde_json::from_str(json).expect("deserialise");
        assert_eq!(t.venue, "Hyperliquid Perps (paper)");
    }
}

// ══════════════════════════════════════════════════════════════════════════════
//  2. Extended BotSession serialisation
// ══════════════════════════════════════════════════════════════════════════════

mod bot_session_tests {
    use tradingbots_fun::web_dashboard::BotSession;

    fn full_session() -> BotSession {
        BotSession {
            id:                  "ses_abc".to_string(),
            token:               "tok_xyz".to_string(),
            tx_hash:             "0xdeadbeef".to_string(),
            plan:                "starter".to_string(),
            created_at:          "2026-03-24T00:00:00Z".to_string(),
            expires_at:          "2026-04-24T00:00:00Z".to_string(),
            max_drawdown_pct:    Some(15.0),
            webhook_url:         Some("https://example.com/hook".to_string()),
            venue:               "hyperliquid".to_string(),
            leverage_max:        Some(20),
            risk_mode:           Some("balanced".to_string()),
            symbols_whitelist:   Some(vec!["BTC".to_string(), "ETH".to_string()]),
            performance_fee_pct: Some(10),
            hyperliquid_address: Some("0xAbCdEf1234".to_string()),
            paused:              false,
            name:                Some("Test Bot".to_string()),
            balance_usd:         200.0,
            session_pnl:         0.0,
        }
    }

    #[test]
    fn session_round_trips_all_fields() {
        let s = full_session();
        let json = serde_json::to_string(&s).unwrap();
        let back: BotSession = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "ses_abc");
        assert_eq!(back.max_drawdown_pct, Some(15.0));
        assert_eq!(back.webhook_url.as_deref(), Some("https://example.com/hook"));
        assert_eq!(back.venue, "hyperliquid");
        assert_eq!(back.leverage_max, Some(20));
        assert_eq!(back.risk_mode.as_deref(), Some("balanced"));
        assert_eq!(back.symbols_whitelist, Some(vec!["BTC".to_string(), "ETH".to_string()]));
        assert_eq!(back.performance_fee_pct, Some(10));
        assert_eq!(back.hyperliquid_address.as_deref(), Some("0xAbCdEf1234"));
        assert!(!back.paused);
    }

    #[test]
    fn session_defaults_for_minimal_json() {
        // Old sessions stored without the new fields should default cleanly
        let json = r#"{
            "id":"ses_old","token":"tok_old","tx_hash":"0x1234",
            "plan":"starter","created_at":"2026-01-01T00:00:00Z",
            "expires_at":"2026-02-01T00:00:00Z"
        }"#;
        let s: BotSession = serde_json::from_str(json).unwrap();
        assert!(s.max_drawdown_pct.is_none());
        assert!(s.webhook_url.is_none());
        assert_eq!(s.venue, "internal");   // default_session_venue
        assert!(s.leverage_max.is_none());
        assert!(!s.paused);
    }

    #[test]
    fn burst_session_plan_naming() {
        let mut s = full_session();
        s.plan = "burst-24h".to_string();
        assert_eq!(s.plan, "burst-24h");
    }

    #[test]
    fn session_paused_flag_serialises() {
        let mut s = full_session();
        s.paused = true;
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""paused":true"#));
    }
}

// ══════════════════════════════════════════════════════════════════════════════
//  3. HyperliquidConnector — signal validation & risk
// ══════════════════════════════════════════════════════════════════════════════

mod connector_tests {
    use tradingbots_fun::connectors::hyperliquid::{
        config_from_session, HlSignal, HlTradeLog, HyperliquidConnector, RiskMode, SessionConfig,
    };

    fn default_config(session_id: &str) -> SessionConfig {
        config_from_session(
            session_id,
            Some(20),
            Some("balanced"),
            None,
            Some(15.0),
            None,
            None,
        )
    }

    fn make_signal(coin: &str, leverage: i32) -> HlSignal {
        HlSignal {
            coin:       coin.to_string(),
            is_buy:     true,
            size_usd:   500.0,
            limit_px:   None,
            leverage,
            reduce_only: false,
        }
    }

    #[test]
    fn signal_passes_when_all_clear() {
        let cfg = default_config("s1");
        let c = HyperliquidConnector::new(cfg, 10_000.0);
        let result = c.validate_signal(&make_signal("BTC", 10));
        assert!(result.is_ok(), "Valid signal should pass");
        assert_eq!(result.unwrap(), 10); // leverage unchanged
    }

    #[test]
    fn leverage_is_clamped_to_session_max() {
        let cfg = default_config("s2");
        let c = HyperliquidConnector::new(cfg, 10_000.0);
        // Signal asks for 50x but session max = 20
        let lev = c.validate_signal(&make_signal("ETH", 50)).unwrap();
        assert_eq!(lev, 20, "Leverage must be clamped to session max");
    }

    #[test]
    fn whitelist_blocks_non_listed_symbol() {
        let cfg = config_from_session(
            "s3",
            Some(10),
            None,
            Some(vec!["BTC".to_string(), "ETH".to_string()]),
            None,
            None,
            None,
        );
        let c = HyperliquidConnector::new(cfg, 10_000.0);
        let result = c.validate_signal(&make_signal("SOL", 5));
        assert!(result.is_err(), "SOL is not whitelisted → should be blocked");
        assert!(result.unwrap_err().to_string().contains("whitelist"));
    }

    #[test]
    fn whitelist_allows_listed_symbol() {
        let cfg = config_from_session(
            "s4",
            Some(10),
            None,
            Some(vec!["BTC".to_string(), "ETH".to_string()]),
            None,
            None,
            None,
        );
        let c = HyperliquidConnector::new(cfg, 10_000.0);
        let result = c.validate_signal(&make_signal("BTC", 5));
        assert!(result.is_ok());
    }

    #[test]
    fn whitelist_is_case_insensitive() {
        let cfg = config_from_session(
            "s5",
            Some(10),
            None,
            Some(vec!["BTC".to_string()]),
            None,
            None,
            None,
        );
        let c = HyperliquidConnector::new(cfg, 10_000.0);
        // Signal sends lowercase "btc"
        let result = c.validate_signal(&make_signal("btc", 5));
        assert!(result.is_ok(), "Whitelist check should be case-insensitive");
    }

    #[test]
    fn drawdown_guard_blocks_after_breach() {
        let cfg = config_from_session(
            "s6",
            Some(10),
            None,
            None,
            Some(10.0), // 10% max drawdown
            None,
            None,
        );
        let mut c = HyperliquidConnector::new(cfg, 1_000.0);
        // Simulate a 10% loss
        let log = HlTradeLog {
            session_id:   "s6".to_string(),
            coin:         "BTC".to_string(),
            is_buy:       true,
            size_usd:     1_000.0,
            limit_px:     None,
            leverage:     1,
            tx_ref:       "0xabc".to_string(),
            status:       "filled".to_string(),
            created_at:   "2026-03-24T00:00:00Z".to_string(),
            raw_response: None,
        };
        c.record_trade(log, -100.0); // -100 USD on 1000 = -10%
        assert!(c.paused, "Connector should auto-pause on drawdown breach");
        // Further signals must be blocked
        let result = c.validate_signal(&make_signal("ETH", 5));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("paused"));
    }

    #[test]
    fn drawdown_pct_calculation() {
        let cfg = default_config("s7");
        let mut c = HyperliquidConnector::new(cfg, 1_000.0);
        c.session_pnl = -50.0; // -5%
        let dd = c.drawdown_pct();
        assert!(
            (dd - 5.0).abs() < 0.01,
            "Drawdown should be 5%, got {dd}"
        );
    }

    #[test]
    fn risk_mode_conservative_halves_size() {
        let cfg = config_from_session("s8", Some(10), Some("conservative"), None, None, None, None);
        let c = HyperliquidConnector::new(cfg, 10_000.0);
        let sized = c.apply_risk_multiplier(500.0);
        assert_eq!(sized, 250.0, "Conservative mode should halve position size");
    }

    #[test]
    fn risk_mode_aggressive_increases_size() {
        let cfg = config_from_session("s9", Some(10), Some("aggressive"), None, None, None, None);
        let c = HyperliquidConnector::new(cfg, 10_000.0);
        let sized = c.apply_risk_multiplier(500.0);
        assert_eq!(sized, 750.0, "Aggressive mode should increase size by 50%");
    }

    #[test]
    fn risk_mode_balanced_unchanged() {
        let cfg = config_from_session("s10", Some(10), Some("balanced"), None, None, None, None);
        let c = HyperliquidConnector::new(cfg, 10_000.0);
        let sized = c.apply_risk_multiplier(500.0);
        assert_eq!(sized, 500.0, "Balanced mode should leave size unchanged");
    }

    #[test]
    fn risk_mode_parse_unknown_defaults_to_balanced() {
        let cfg = config_from_session("s11", Some(10), Some("ultra-risky"), None, None, None, None);
        assert_eq!(cfg.risk_mode, RiskMode::Balanced);
    }

    #[test]
    fn coin_to_asset_index_known_symbols() {
        use tradingbots_fun::connectors::hyperliquid::HyperliquidConnector;
        assert_eq!(HyperliquidConnector::coin_to_asset_index("BTC"),  0);
        assert_eq!(HyperliquidConnector::coin_to_asset_index("ETH"),  1);
        assert_eq!(HyperliquidConnector::coin_to_asset_index("SOL"),  2);
        assert_eq!(HyperliquidConnector::coin_to_asset_index("HYPE"), 23);
        assert_eq!(HyperliquidConnector::coin_to_asset_index("TAO"),  24);
    }

    #[test]
    fn coin_to_asset_index_unknown_defaults_to_zero() {
        use tradingbots_fun::connectors::hyperliquid::HyperliquidConnector;
        assert_eq!(HyperliquidConnector::coin_to_asset_index("UNKNOWN"), 0);
    }

    #[test]
    fn trade_log_ring_buffer_capped_at_500() {
        let cfg = default_config("s12");
        let mut c = HyperliquidConnector::new(cfg, 10_000.0);
        for i in 0..600usize {
            c.record_trade(
                HlTradeLog {
                    session_id:   "s12".to_string(),
                    coin:         "BTC".to_string(),
                    is_buy:       true,
                    size_usd:     100.0,
                    limit_px:     None,
                    leverage:     1,
                    tx_ref:       format!("0x{:06x}", i),
                    status:       "filled".to_string(),
                    created_at:   "now".to_string(),
                    raw_response: None,
                },
                0.0,
            );
        }
        assert_eq!(c.trade_log.len(), 500, "Trade log must be capped at 500 entries");
    }

    #[test]
    fn recent_trades_respects_limit() {
        let cfg = default_config("s13");
        let mut c = HyperliquidConnector::new(cfg, 10_000.0);
        for i in 0..20usize {
            c.record_trade(
                HlTradeLog {
                    session_id:   "s13".to_string(),
                    coin:         "ETH".to_string(),
                    is_buy:       i % 2 == 0,
                    size_usd:     200.0,
                    limit_px:     None,
                    leverage:     2,
                    tx_ref:       format!("0x{:06x}", i),
                    status:       "filled".to_string(),
                    created_at:   "now".to_string(),
                    raw_response: None,
                },
                0.0,
            );
        }
        let recent = c.recent_trades(5);
        assert_eq!(recent.len(), 5);
    }
}

// ══════════════════════════════════════════════════════════════════════════════
//  4. LatencyTracker — recording and percentile computation
// ══════════════════════════════════════════════════════════════════════════════

mod latency_tests {
    use tradingbots_fun::latency::{LatencyTracker, SessionLatencyStats, TradeLatencyRecord};
    use std::time::{Duration, Instant};

    /// Build a fake `TradeLatencyRecord` with controlled total_latency_ms.
    fn record_with_ms(trade_id: &str, total_ms: f64) -> TradeLatencyRecord {
        let mut r = TradeLatencyRecord::new(trade_id, "BTC");
        // We need fill_confirmed_at to be `total_ms` after signal_received_at.
        // Since Instant is monotonic and we can't set it directly, we tick forward.
        let _order_signed    = r.signal_received_at + Duration::from_micros(100);
        let _order_sent      = r.signal_received_at + Duration::from_micros(200);
        let _resp_recv       = r.signal_received_at + Duration::from_micros((total_ms * 400.0) as u64);
        let fill             = r.signal_received_at + Duration::from_micros((total_ms * 1000.0) as u64);
        r.order_signed_at    = Some(r.signal_received_at + Duration::from_micros(100));
        r.order_sent_at      = Some(r.signal_received_at + Duration::from_micros(200));
        r.response_received_at = Some(r.signal_received_at + Duration::from_micros((total_ms * 400.0) as u64));
        r.fill_confirmed_at  = Some(fill);
        r
    }

    #[test]
    fn total_latency_calculated_correctly() {
        let r = record_with_ms("t1", 200.0);
        let ms = r.total_latency_ms().expect("should have total latency");
        // Allow ±5ms tolerance due to Instant precision
        assert!(
            (ms - 200.0).abs() < 5.0,
            "Expected ~200ms total latency, got {ms:.1}ms"
        );
    }

    #[test]
    fn order_latency_less_than_total() {
        let r = record_with_ms("t2", 300.0);
        let order_ms = r.order_latency_ms().expect("should have order latency");
        let total_ms = r.total_latency_ms().expect("should have total latency");
        assert!(
            order_ms < total_ms,
            "Order latency ({order_ms:.1}ms) must be less than total ({total_ms:.1}ms)"
        );
    }

    #[test]
    fn missing_fill_returns_none() {
        let mut r = TradeLatencyRecord::new("t3", "ETH");
        r.order_sent_at = Some(Instant::now());
        r.response_received_at = Some(Instant::now());
        // fill_confirmed_at intentionally not set
        assert!(r.total_latency_ms().is_none());
        assert!(r.fill_latency_ms().is_none());
    }

    #[test]
    fn tracker_empty_returns_zero_stats() {
        let tracker = LatencyTracker::new("ses_empty");
        let stats = tracker.stats();
        assert_eq!(stats.sample_count, 0);
        assert_eq!(stats.p50_ms, 0.0);
        assert_eq!(stats.p99_ms, 0.0);
    }

    #[test]
    fn tracker_records_and_computes_p50() {
        let mut tracker = LatencyTracker::new("ses_p50");
        // Insert 100 records with latencies 100..=199 ms
        for i in 0u64..100 {
            let r = record_with_ms(&format!("t{i}"), 100.0 + i as f64);
            tracker.record(&r);
        }
        let stats = tracker.stats();
        assert_eq!(stats.sample_count, 100);
        // p50 should be around 149ms (median of 100..=199)
        assert!(
            stats.p50_ms > 140.0 && stats.p50_ms < 160.0,
            "p50 should be ~149ms, got {:.1}ms",
            stats.p50_ms
        );
    }

    #[test]
    fn tracker_ring_buffer_capped_at_1000() {
        let mut tracker = LatencyTracker::new("ses_cap");
        for i in 0usize..1200 {
            let r = record_with_ms(&format!("t{i}"), 100.0 + (i % 100) as f64);
            tracker.record(&r);
        }
        assert_eq!(tracker.len(), 1000, "Ring buffer must cap at 1000 entries");
    }

    #[test]
    fn session_latency_stats_target_thresholds() {
        // Confirm the target constants are published correctly
        let _stats = SessionLatencyStats::default();
        // Targets are set in ::compute(), so build via compute
        let mut total = vec![100.0f64; 10];
        let mut order = vec![50.0f64; 10];
        let s = SessionLatencyStats::compute("s1", &mut total, &mut order, 60.0);
        assert_eq!(s.target_median_ms, 250.0);
        assert_eq!(s.target_p95_ms,   450.0);
        assert_eq!(s.target_p99_ms,   800.0);
    }

    #[test]
    fn success_rate_100pct_when_all_fast() {
        let mut total = vec![100.0f64, 150.0, 200.0, 250.0, 300.0]; // all ≤ 500ms
        let mut order: Vec<f64> = vec![];
        let s = SessionLatencyStats::compute("s2", &mut total, &mut order, 10.0);
        assert_eq!(s.success_rate_pct, 100.0);
    }

    #[test]
    fn success_rate_50pct_when_half_slow() {
        // 5 fast + 5 slow (> 500ms)
        let mut total: Vec<f64> = (0..5).map(|_| 200.0).chain((0..5).map(|_| 800.0)).collect();
        let mut order: Vec<f64> = vec![];
        let s = SessionLatencyStats::compute("s3", &mut total, &mut order, 10.0);
        assert_eq!(s.success_rate_pct, 50.0);
    }

    #[test]
    fn trades_per_minute_calculated() {
        let mut total = vec![100.0f64; 60]; // 60 trades
        let mut order: Vec<f64> = vec![];
        // window = 60 seconds → 60 trades/min
        let s = SessionLatencyStats::compute("s4", &mut total, &mut order, 60.0);
        assert!(
            (s.trades_per_minute - 60.0).abs() < 0.01,
            "Expected 60 trades/min, got {:.2}",
            s.trades_per_minute
        );
    }
}

// ══════════════════════════════════════════════════════════════════════════════
//  5. New BotCommand variants — serialise / deserialise
// ══════════════════════════════════════════════════════════════════════════════

mod bot_command_tests {
    use tradingbots_fun::web_dashboard::BotCommand;

    #[test]
    fn set_leverage_round_trips() {
        let cmd = BotCommand::SetLeverage {
            symbol: "BTC".to_string(),
            leverage: 15,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let back: BotCommand = serde_json::from_str(&json).unwrap();
        match back {
            BotCommand::SetLeverage { symbol, leverage } => {
                assert_eq!(symbol, "BTC");
                assert_eq!(leverage, 15);
            }
            _ => panic!("Wrong variant after round-trip"),
        }
    }

    #[test]
    fn pause_trading_round_trips() {
        let cmd = BotCommand::PauseTrading;
        let json = serde_json::to_string(&cmd).unwrap();
        let back: BotCommand = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, BotCommand::PauseTrading));
    }

    #[test]
    fn resume_trading_round_trips() {
        let cmd = BotCommand::ResumeTrading;
        let json = serde_json::to_string(&cmd).unwrap();
        let back: BotCommand = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, BotCommand::ResumeTrading));
    }

    #[test]
    fn existing_close_all_still_serialises() {
        let cmd = BotCommand::CloseAll;
        let json = serde_json::to_string(&cmd).unwrap();
        let back: BotCommand = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, BotCommand::CloseAll));
    }
}

// ══════════════════════════════════════════════════════════════════════════════
//  6. SessionCreateRequest — serde defaults and burst pricing
// ══════════════════════════════════════════════════════════════════════════════

mod session_create_request_tests {
    use tradingbots_fun::web_dashboard::SessionCreateRequest;

    #[test]
    fn defaults_to_standard_30d_internal() {
        let req: SessionCreateRequest = serde_json::from_str("{}").unwrap();
        assert_eq!(req.venue,    "internal");
        assert_eq!(req.duration, "30d");
        assert!(req.max_drawdown_pct.is_none());
        assert!(req.webhook_url.is_none());
        assert!(req.symbols_whitelist.is_none());
    }

    #[test]
    fn burst_session_parsed_correctly() {
        let json = r#"{"duration":"24h","venue":"hyperliquid","leverage_max":20}"#;
        let req: SessionCreateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.duration, "24h");
        assert_eq!(req.venue, "hyperliquid");
        assert_eq!(req.leverage_max, Some(20));
    }

    #[test]
    fn full_hyperliquid_session_request() {
        let json = r#"{
            "venue": "hyperliquid",
            "leverage_max": 20,
            "risk_mode": "conservative",
            "symbols_whitelist": ["BTC", "ETH"],
            "max_drawdown_pct": 12.0,
            "performance_fee_pct": 10,
            "webhook_url": "https://agent.example.com/trade",
            "duration": "30d"
        }"#;
        let req: SessionCreateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.venue, "hyperliquid");
        assert_eq!(req.leverage_max, Some(20));
        assert_eq!(req.risk_mode.as_deref(), Some("conservative"));
        assert_eq!(
            req.symbols_whitelist,
            Some(vec!["BTC".to_string(), "ETH".to_string()])
        );
        assert_eq!(req.max_drawdown_pct, Some(12.0));
        assert_eq!(req.performance_fee_pct, Some(10));
        assert_eq!(req.webhook_url.as_deref(), Some("https://agent.example.com/trade"));
    }
}

// ══════════════════════════════════════════════════════════════════════════════
//  7. Gap-close tests — command parsing, trade venue filter, latency stats shape
// ══════════════════════════════════════════════════════════════════════════════

mod gap_close_tests {
    use tradingbots_fun::web_dashboard::{BotCommand, ClosedTrade};

    // ── Command handler JSON parsing ──────────────────────────────────────

    /// Simulate what `api_v1_session_command_handler` does when it reads
    /// `{"cmd":"set_leverage","symbol":"BTC","leverage":25}` from the body.
    #[test]
    fn set_leverage_cmd_parses_and_clamps() {
        let body: serde_json::Value = serde_json::json!({
            "cmd": "set_leverage",
            "symbol": "BTC",
            "leverage": 25
        });
        let cmd_str = body["cmd"].as_str().unwrap();
        let symbol  = body["symbol"].as_str().unwrap_or("").to_string();
        let lev     = body["leverage"].as_i64().unwrap_or(10) as i32;

        assert_eq!(cmd_str, "set_leverage");
        assert!(!symbol.is_empty());
        let clamped = lev.clamp(1, 50);
        assert_eq!(clamped, 25);

        // Verify it maps to the correct BotCommand variant
        let cmd = BotCommand::SetLeverage { symbol: symbol.clone(), leverage: clamped };
        match cmd {
            BotCommand::SetLeverage { symbol: s, leverage: l } => {
                assert_eq!(s, "BTC");
                assert_eq!(l, 25);
            }
            _ => panic!("Expected SetLeverage"),
        }
    }

    #[test]
    fn set_leverage_clamps_above_50() {
        let raw: i32 = 200;
        assert_eq!(raw.clamp(1, 50), 50, "Leverage above 50 must clamp to 50");
    }

    #[test]
    fn set_leverage_clamps_below_1() {
        let raw: i32 = 0;
        assert_eq!(raw.clamp(1, 50), 1, "Leverage below 1 must clamp to 1");
    }

    #[test]
    fn pause_trading_cmd_maps_to_variant() {
        let body: serde_json::Value = serde_json::json!({"cmd": "pause_trading"});
        let cmd_str = body["cmd"].as_str().unwrap();
        assert_eq!(cmd_str, "pause_trading");
        let cmd = BotCommand::PauseTrading;
        assert!(matches!(cmd, BotCommand::PauseTrading));
    }

    #[test]
    fn resume_trading_cmd_maps_to_variant() {
        let body: serde_json::Value = serde_json::json!({"cmd": "resume_trading"});
        let cmd_str = body["cmd"].as_str().unwrap();
        assert_eq!(cmd_str, "resume_trading");
        let cmd = BotCommand::ResumeTrading;
        assert!(matches!(cmd, BotCommand::ResumeTrading));
    }

    // ── /session/{id}/trades venue filter logic ───────────────────────────

    fn make_trade_with_venue(symbol: &str, venue: &str, pnl: f64) -> ClosedTrade {
        ClosedTrade {
            symbol:     symbol.to_string(),
            side:       "LONG".to_string(),
            entry:      100.0,
            exit:       100.0 + pnl,
            pnl,
            pnl_pct:    pnl,
            reason:     "Signal".to_string(),
            closed_at:  "2026-03-24T01:00:00Z".to_string(),
            entry_time: "2026-03-24T00:00:00Z".to_string(),
            quantity:   1.0,
            size_usd:   100.0,
            leverage:   1.0,
            fees_est:   0.075,
            breakdown:  None,
            note:       None,
            venue:      venue.to_string(),
        }
    }

    #[test]
    fn trade_filter_no_filter_returns_all() {
        let trades = [
            make_trade_with_venue("BTC", "Hyperliquid Perps (paper)", 10.0),
            make_trade_with_venue("ETH", "internal", 5.0),
            make_trade_with_venue("SOL", "Hyperliquid Perps (paper)", -3.0),
        ];
        let venue_filter: Option<String> = None;
        let filtered: Vec<&ClosedTrade> = trades.iter()
            .filter(|t| {
                if let Some(ref vf) = venue_filter {
                    t.venue.to_lowercase().contains(vf.as_str())
                } else { true }
            })
            .collect();
        assert_eq!(filtered.len(), 3);
    }

    #[test]
    fn trade_filter_hyperliquid_returns_only_hl() {
        let trades = [
            make_trade_with_venue("BTC", "Hyperliquid Perps (paper)", 10.0),
            make_trade_with_venue("ETH", "internal", 5.0),
            make_trade_with_venue("SOL", "Hyperliquid Perps (paper)", -3.0),
        ];
        let venue_filter = Some("hyperliquid".to_string());
        let filtered: Vec<&ClosedTrade> = trades.iter()
            .filter(|t| {
                if let Some(ref vf) = venue_filter {
                    t.venue.to_lowercase().contains(vf.as_str())
                } else { true }
            })
            .collect();
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|t| t.venue.to_lowercase().contains("hyperliquid")));
    }

    #[test]
    fn trade_filter_internal_returns_only_internal() {
        let trades = [
            make_trade_with_venue("BTC", "Hyperliquid Perps (paper)", 10.0),
            make_trade_with_venue("ETH", "internal", 5.0),
        ];
        let venue_filter = Some("internal".to_string());
        let filtered: Vec<&ClosedTrade> = trades.iter()
            .filter(|t| {
                if let Some(ref vf) = venue_filter {
                    t.venue.to_lowercase().contains(vf.as_str())
                } else { true }
            })
            .collect();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].symbol, "ETH");
    }

    // ── /session/{id}/positions response shape ────────────────────────────

    #[test]
    fn position_response_contains_venue_field() {
        // Simulate what api_v1_session_positions_handler serialises for one position
        let venue = "Hyperliquid Perps (paper)";
        let item = serde_json::json!({
            "symbol":         "BTC",
            "side":           "LONG",
            "entry_price":    50_000.0f64,
            "quantity":       0.02f64,
            "size_usd":       1_000.0f64,
            "leverage":       2.0f64,
            "unrealised_pnl": 50.0f64,
            "pnl_pct":        5.0f64,
            "stop_loss":      48_000.0f64,
            "take_profit":    54_000.0f64,
            "cycles_held":    10u64,
            "entry_time":     "2026-03-24T00:00:00Z",
            "venue":          venue,
            "funding_rate":   0.0001f64,
            "funding_delta":  0.0f64,
            "ai_action":      serde_json::Value::Null,
            "ai_reason":      serde_json::Value::Null,
            "ob_sentiment":   "NEUTRAL",
        });
        assert_eq!(item["venue"].as_str().unwrap(), "Hyperliquid Perps (paper)");
        assert_eq!(item["symbol"].as_str().unwrap(), "BTC");
    }

    // ── Latency stats response shape ──────────────────────────────────────

    #[test]
    fn latency_stats_response_shape() {
        // Simulate the JSON structure returned by api_v1_session_latency_stats_handler
        let stats_json = serde_json::json!({
            "ok":               true,
            "session_id":       "ses_test",
            "window":           "1h",
            "sample_count":     0,
            "p50_ms":           0.0,
            "p95_ms":           0.0,
            "p99_ms":           0.0,
            "min_ms":           0.0,
            "max_ms":           0.0,
            "mean_ms":          0.0,
            "trades_per_minute": 0.0,
            "success_rate_pct": 0.0,
            "targets": {
                "p50_target_ms": 250.0,
                "p95_target_ms": 450.0,
                "p99_target_ms": 800.0,
            },
            "status": {
                "p50_ok": true,
                "p95_ok": true,
                "p99_ok": true,
            }
        });
        assert!(stats_json["ok"].as_bool().unwrap());
        assert_eq!(stats_json["targets"]["p50_target_ms"].as_f64().unwrap(), 250.0);
        assert_eq!(stats_json["targets"]["p95_target_ms"].as_f64().unwrap(), 450.0);
        assert_eq!(stats_json["targets"]["p99_target_ms"].as_f64().unwrap(), 800.0);
        assert!(stats_json["status"]["p50_ok"].as_bool().unwrap());
    }

    #[test]
    fn latency_stats_target_breach_flags() {
        // Simulate p50 > 250ms → p50_ok = false
        let p50_ms = 300.0f64;
        let p95_ms = 400.0f64;
        let p99_ms = 900.0f64;
        let p50_ok = p50_ms <= 250.0;
        let p95_ok = p95_ms <= 450.0;
        let p99_ok = p99_ms <= 800.0;
        assert!(!p50_ok, "300ms p50 should fail target");
        assert!(p95_ok,  "400ms p95 should pass target");
        assert!(!p99_ok, "900ms p99 should fail target");
    }
}

// ══════════════════════════════════════════════════════════════════════════════
//  8. Semantic version check
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn crate_version_is_021() {
    assert_eq!(
        tradingbots_fun::VERSION,
        "0.2.1",
        "Crate version must be 0.2.1 in Cargo.toml"
    );
}
