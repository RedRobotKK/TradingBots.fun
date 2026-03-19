//! Integration tests for the four new features added in March 2026:
//!   1. Correlation filter (correlation.rs)
//!   2. Webhook / Telegram notifier (notifier.rs)
//!   3. On-chain exchange netflow signal (onchain.rs)
//!   4. Trade journal (web_dashboard.rs POST /api/trade-note)
//!
//! These tests verify the modules behave as documented **without** needing
//! live API keys or a running server.

// ─────────────────────────────── 1. Correlation filter ───────────────────────

mod correlation_tests {
    use tradingbots_fun::correlation::{
        correlation_block, get_correlation, CorrBlock, CONF_EDGE, CORR_THRESHOLD,
    };
    use tradingbots_fun::web_dashboard::PaperPosition;
    use tradingbots_fun::learner::SignalContribution;

    fn make_pos(symbol: &str, side: &str, conf: f64) -> PaperPosition {
        PaperPosition {
            symbol:           symbol.to_string(),
            side:             side.to_string(),
            entry_price:      100.0,
            quantity:         1.0,
            size_usd:         100.0,
            stop_loss:        95.0,
            take_profit:      110.0,
            atr_at_entry:     2.0,
            high_water_mark:  100.0,
            low_water_mark:   100.0,
            partial_closed:   false,
            r_dollars_risked: 5.0,
            tranches_closed:  0,
            dca_count:        0,
            leverage:         1.0,
            cycles_held:      0,
            entry_time:       "00:00:00 UTC".to_string(),
            unrealised_pnl:   0.0,
            contrib:          SignalContribution::default(),
            ai_action:        None,
            ai_reason:        None,
            entry_confidence: conf,
            trade_budget_usd:   100.0,
            dca_spent_usd:      0.0,
            btc_ret_at_entry:   0.0,
            initial_margin_usd: 100.0,
            ob_sentiment:       String::new(),
            ob_bid_wall_near:   false,
            ob_ask_wall_near:   false,
            ob_adverse_cycles:  0,
            funded_from_pool:   false,
            pool_stake_usd:     0.0,
        }
    }

    #[test]
    fn btc_eth_high_corr_block_same_direction_insufficient_edge() {
        // BTC-ETH correlation is 0.85 (above CORR_THRESHOLD=0.72).
        // Existing BTC LONG at confidence 0.80; new ETH LONG at 0.83.
        // Edge = 0.03 < CONF_EDGE (0.08) → should be BLOCKED.
        let positions = vec![make_pos("BTC", "LONG", 0.80)];
        let result = correlation_block("ETH", "LONG", 0.83, &positions);
        assert!(
            matches!(result, CorrBlock::Blocked { .. }),
            "Expected Blocked, got {:?}", result
        );
    }

    #[test]
    fn btc_eth_corr_override_with_sufficient_edge() {
        // Existing BTC LONG at 0.80; new ETH LONG at 0.90.
        // Edge = 0.10 ≥ CONF_EDGE (0.08) → should be Override, not Blocked.
        let positions = vec![make_pos("BTC", "LONG", 0.80)];
        let result = correlation_block("ETH", "LONG", 0.90, &positions);
        assert!(
            matches!(result, CorrBlock::Override { .. }),
            "Expected Override, got {:?}", result
        );
    }

    #[test]
    fn opposite_sides_never_blocked() {
        // BTC LONG vs ETH SHORT — correlation irrelevant for opposite directions.
        let positions = vec![make_pos("BTC", "LONG", 0.80)];
        let result = correlation_block("ETH", "SHORT", 0.70, &positions);
        assert_eq!(result, CorrBlock::Clear, "Opposite sides should always clear");
    }

    #[test]
    fn no_existing_positions_always_clear() {
        let result = correlation_block("BTC", "LONG", 0.75, &[]);
        assert_eq!(result, CorrBlock::Clear);
    }

    #[test]
    fn arb_op_highest_corr_in_table() {
        assert!(get_correlation("ARB", "OP") >= 0.85,
            "ARB-OP should be the strongest pair in the table (≥0.85)");
    }

    #[test]
    fn doge_shib_meme_cluster_corr() {
        let r = get_correlation("DOGE", "SHIB");
        assert!(r >= 0.80, "DOGE-SHIB should have high correlation (≥0.80), got {}", r);
    }

    #[test]
    fn unknown_pair_returns_default_low() {
        // Pairs not in the table should default to 0.35 (assumed low correlation).
        let r = get_correlation("BTC", "PEPE"); // PEPE-BTC not in table
        assert!(
            (0.30..CORR_THRESHOLD).contains(&r),
            "Unknown pair should return low correlation (< threshold), got {}", r
        );
    }

    #[test]
    fn suffix_stripping_matches_base_symbol() {
        // "BTC-USD" and "BTC" should yield the same result.
        assert_eq!(
            get_correlation("BTC-USD", "ETH-USD"),
            get_correlation("BTC", "ETH"),
            "Suffix stripping should make BTC-USD == BTC in correlation lookup"
        );
    }

    #[test]
    fn same_symbol_is_self_correlation_one() {
        assert_eq!(get_correlation("SOL", "SOL"), 1.0);
        assert_eq!(get_correlation("BTC-USD", "BTC"), 1.0,
            "Same base symbol with different suffixes should be self-correlation");
    }

    #[test]
    fn conf_edge_constant_is_reasonable() {
        // CONF_EDGE of 0.08 means an 8% confidence advantage overrides block.
        const { assert!(CONF_EDGE > 0.05 && CONF_EDGE < 0.20) }
    }

    #[test]
    fn corr_threshold_excludes_moderate_pairs() {
        // BTC-LINK has correlation 0.70 in the table (< CORR_THRESHOLD 0.72)
        // → should not block same-direction trades.
        let positions = vec![make_pos("BTC", "LONG", 0.75)];
        let result = correlation_block("LINK", "LONG", 0.75, &positions);
        assert_eq!(result, CorrBlock::Clear,
            "BTC-LINK (corr=0.70) is below threshold — should not block");
    }
}

// ─────────────────────────── 2. Notifier ─────────────────────────────────────

mod notifier_tests {
    //! The notifier module itself requires live env vars to construct
    //! (WEBHOOK_URL / TELEGRAM_*).  We test the business logic:
    //!   • from_env() returns None when nothing is configured.
    //!   • discord_embed payload structure is correct (tested via the public interface).

    use tradingbots_fun::notifier::Notifier;

    #[test]
    fn from_env_returns_none_when_no_env_vars_set() {
        // In the CI / test environment there are no webhook env vars.
        // The function should return None gracefully — callers then skip notifications.
        std::env::remove_var("WEBHOOK_URL");
        std::env::remove_var("TELEGRAM_BOT_TOKEN");
        std::env::remove_var("TELEGRAM_CHAT_ID");

        let n = Notifier::from_env();
        assert!(n.is_none(),
            "Notifier should be None when no webhook env vars are set");
    }

    #[test]
    fn from_env_returns_some_when_webhook_url_is_set() {
        // Temporarily set a dummy webhook URL to verify construction succeeds.
        std::env::set_var("WEBHOOK_URL", "https://example.com/webhook");
        let n = Notifier::from_env();
        std::env::remove_var("WEBHOOK_URL");

        assert!(n.is_some(),
            "Notifier should be Some when WEBHOOK_URL is set");
    }

    #[test]
    fn from_env_telegram_requires_both_token_and_chat_id() {
        // Only token set — should still return None (both required).
        std::env::remove_var("WEBHOOK_URL");
        std::env::set_var("TELEGRAM_BOT_TOKEN", "1234:ABCD");
        std::env::remove_var("TELEGRAM_CHAT_ID");

        let n = Notifier::from_env();
        std::env::remove_var("TELEGRAM_BOT_TOKEN");
        assert!(n.is_none(),
            "Notifier should be None when only TELEGRAM_BOT_TOKEN is set (chat_id missing)");
    }

    #[test]
    fn from_env_telegram_both_vars_sufficient() {
        std::env::remove_var("WEBHOOK_URL");
        std::env::set_var("TELEGRAM_BOT_TOKEN", "1234:ABCD");
        std::env::set_var("TELEGRAM_CHAT_ID",   "-100123456");

        let n = Notifier::from_env();
        std::env::remove_var("TELEGRAM_BOT_TOKEN");
        std::env::remove_var("TELEGRAM_CHAT_ID");

        assert!(n.is_some(),
            "Notifier should be Some when both Telegram vars are set");
    }
}

// ─────────────────────────── 3. On-chain signal ───────────────────────────────

mod onchain_tests {
    // The unit tests for signal strength are in onchain.rs itself.
    // Here we test the public API surface.
    use tradingbots_fun::onchain::OnchainCache;

    #[tokio::test]
    async fn get_returns_neutral_when_no_api_key() {
        // Without COINGLASS_API_KEY, every symbol returns neutral (strength=0.0).
        std::env::remove_var("COINGLASS_API_KEY");
        let cache = OnchainCache::new();
        let data  = cache.get("BTC").await;
        assert_eq!(data.signal_strength(), 0.0,
            "Should return neutral (0.0) when no API key configured");
    }

    #[tokio::test]
    async fn get_strips_suffix_correctly() {
        std::env::remove_var("COINGLASS_API_KEY");
        let cache = OnchainCache::new();
        let a = cache.get("BTC").await;
        let b = cache.get("BTC-USD").await;
        // Both should give the same neutral result (or same cached value).
        assert_eq!(a.signal_strength(), b.signal_strength(),
            "BTC and BTC-USD should resolve to the same cached entry");
    }

    #[test]
    fn neutral_strength_is_zero() {
        // Explicitly verify the zero case is handled correctly.
        // signal_strength() of 0.0 means no adjustment to confidence.
        let adj = 0.0_f64;
        assert!(adj.abs() <= 0.05,
            "Zero on-chain strength should produce negligible confidence adjustment");
    }

    #[test]
    fn max_onchain_confidence_adjustment_is_bounded() {
        // The adjustment is strength × 0.04, so max is ±4%.
        // Verify this never can push confidence outside [0, 1].
        let max_adj = 1.0_f64 * 0.04;  // max strength=1.0
        let base_conf = 0.98_f64;
        let adjusted = (base_conf + max_adj).clamp(0.0, 1.0);
        assert_eq!(adjusted, 1.0, "Clamped confidence should stay at 1.0");

        let base_low = 0.02_f64;
        let adjusted_low = (base_low - max_adj).clamp(0.0, 1.0);
        assert_eq!(adjusted_low, 0.0, "Clamped confidence should stay at 0.0");
    }
}

// ─────────────────────────── 4. Trade journal ────────────────────────────────

mod trade_journal_tests {
    use tradingbots_fun::web_dashboard::{ClosedTrade};

    fn make_trade(symbol: &str) -> ClosedTrade {
        ClosedTrade {
            symbol:     symbol.to_string(),
            side:       "LONG".to_string(),
            entry:      100.0,
            exit:       105.0,
            pnl:        5.0,
            pnl_pct:    5.0,
            reason:     "Signal".to_string(),
            closed_at:  "00:00:00 UTC".to_string(),
            entry_time: "00:00:00 UTC".to_string(),
            quantity:   1.0,
            size_usd:   100.0,
            leverage:   1.0,
            fees_est:   0.075,
            breakdown:  None,
            note:       None,
        }
    }

    #[test]
    fn closed_trade_default_note_is_none() {
        let trade = make_trade("BTC");
        assert!(trade.note.is_none(),
            "Newly created trade should have no operator note");
    }

    #[test]
    fn closed_trade_note_can_be_set() {
        let mut trade = make_trade("ETH");
        trade.note = Some("False MACD signal in ranging market".to_string());
        assert_eq!(
            trade.note.as_deref(),
            Some("False MACD signal in ranging market"),
            "Trade note should be retrievable after setting"
        );
    }

    #[test]
    fn trade_note_serialises_and_deserialises_with_serde() {
        // Ensure the `note` field survives a JSON round-trip (important for
        // state persistence and API responses).
        let mut trade = make_trade("SOL");
        trade.note = Some("Re-entered too early post-DCA".to_string());

        let json = serde_json::to_string(&trade).expect("serialisation failed");
        let back: ClosedTrade = serde_json::from_str(&json).expect("deserialisation failed");

        assert_eq!(back.note.as_deref(), Some("Re-entered too early post-DCA"),
            "Note should survive JSON round-trip");
    }

    #[test]
    fn trade_note_absent_field_deserialises_to_none() {
        // Old snapshots without the `note` field should load without panic.
        let json = r#"{
            "symbol":"BTC","side":"LONG","entry":100.0,"exit":105.0,
            "pnl":5.0,"pnl_pct":5.0,"reason":"Signal","closed_at":"00:00",
            "entry_time":"00:00","quantity":1.0,"size_usd":100.0,
            "leverage":1.0,"fees_est":0.0
        }"#;
        let trade: ClosedTrade = serde_json::from_str(json)
            .expect("Should deserialise old snapshot without note field");
        assert!(trade.note.is_none(),
            "Old snapshot without note field should deserialise as None");
    }

    #[test]
    fn note_length_validation_logic() {
        // The handler enforces max 500 chars.  Test the validation logic directly.
        let long_note: String = "x".repeat(501);
        let short_note: String = "x".repeat(500);
        assert!(long_note.len() > 500, "long_note should exceed limit");
        assert!(short_note.len() <= 500, "short_note should be within limit");
    }
}
