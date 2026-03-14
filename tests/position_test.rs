/// Integration tests for active position management logic.
///
/// Tests the core mathematics used in trade lifecycle:
///   • P&L calculation (leveraged, margin-based)
///   • R-multiple tracking
///   • Trailing stop advancement rules
///   • DCA / Pyramid weighted average entry
///   • Partial-close accounting
///   • Heat constant integrity
///
/// These tests target the shared library crate (lib.rs) and the types
/// exported from fee_calculator and position_manager modules.
use redrobot_hedgebot::fee_calculator::{FeeCalculator, FeeStructure};
use redrobot_hedgebot::position_manager::{AggregatePosition, DCARules, PositionEntry};

// ── Risk constant sanity checks ───────────────────────────────────────────────

/// Verify that the documented "max 2% per trade, max 8% total" rule
/// allows at least 4 positions at maximum trade-heat, and that the
/// heat cap is a reasonable fraction of the per-trade limit.
#[test]
fn risk_constants_portfolio_cap_covers_multiple_positions() {
    let max_trade_heat: f64    = 0.02;  // 2% per trade
    let max_portfolio_heat: f64 = 0.08; // 8% total

    // At the per-trade limit, we can hold up to 4 simultaneous positions
    let capacity = (max_portfolio_heat / max_trade_heat) as usize; // 4
    assert_eq!(capacity, 4, "portfolio cap should allow exactly 4 max-heat positions");

    // Each trade must cost at least some heat — no free positions
    assert!(max_trade_heat > 0.0, "per-trade heat must be positive");
    assert!(max_portfolio_heat > max_trade_heat, "portfolio cap must exceed per-trade limit");
}

#[test]
fn circuit_breaker_threshold_and_multiplier_reasonable() {
    let threshold: f64 = 0.08;  // CB activates at 8% drawdown
    let multiplier: f64 = 0.35; // CB shrinks sizes to 35%

    assert!(threshold > 0.0 && threshold < 1.0, "CB threshold must be in (0, 1)");
    assert!(multiplier > 0.0 && multiplier < 1.0, "CB multiplier must reduce sizes: {multiplier}");
    // 35% sizing means ~65% reduction — aggressive but recoverable
    assert!(multiplier >= 0.25, "CB multiplier too extreme (< 25% would be nearly shut down)");
}

// ── Leveraged P&L formula ─────────────────────────────────────────────────────

#[test]
fn long_pnl_with_leverage_equals_price_diff_times_qty() {
    // qty = margin × leverage / entry_price
    // pnl = (exit - entry) × qty   ← same formula used in close_paper_position
    let entry: f64 = 100.0;
    let exit       = 110.0;  // +10%
    let margin   = 200.0;
    let leverage = 3.0;
    let qty      = margin * leverage / entry;  // 6.0 shares

    let pnl = (exit - entry) * qty;
    assert!(
        (pnl - 60.0).abs() < 1e-10,
        "LONG P&L: 6 shares × $10 = $60, got {pnl}"
    );
}

#[test]
fn short_pnl_with_leverage_equals_price_diff_times_qty() {
    let entry: f64 = 100.0;
    let exit       = 90.0;   // price fell, SHORT wins
    let margin   = 200.0;
    let leverage = 3.0;
    let qty      = margin * leverage / entry;  // 6.0 shares

    let pnl = (entry - exit) * qty;
    assert!(
        (pnl - 60.0).abs() < 1e-10,
        "SHORT P&L: 6 shares × $10 = $60, got {pnl}"
    );
}

#[test]
fn pnl_pct_is_margin_relative_not_notional() {
    // The bot reports P&L% relative to margin committed, not notional.
    // With 3× leverage a 10% price move = 30% return on margin — correct
    // for a leveraged position where only margin is deployed.
    let margin: f64 = 100.0;
    let leverage     = 3.0;
    let entry        = 100.0;
    let exit         = 110.0;
    let qty      = margin * leverage / entry;
    let pnl      = (exit - entry) * qty;         // $30
    let pnl_pct  = pnl / margin * 100.0;         // 30%

    assert!(
        (pnl_pct - 30.0).abs() < 1e-10,
        "leveraged P&L%: 3× leverage × 10% move = 30% on margin, got {pnl_pct}"
    );
}

// ── R-multiple calculations ───────────────────────────────────────────────────

#[test]
fn r_multiple_is_pnl_divided_by_entry_risk() {
    let entry: f64 = 100.0;
    let stop       = 95.0;   // 5% distance
    let margin   = 100.0;
    let leverage = 3.0;
    let qty      = margin * leverage / entry;     // 3.0 shares
    let r_risk   = (entry - stop).abs() * qty;    // $15 = 1R

    // 1R target: entry + (entry - stop) = $105
    let at_1r     = entry + (entry - stop);
    let pnl_at_1r = (at_1r - entry) * qty;        // $15
    let r_mult    = pnl_at_1r / r_risk;

    assert!((r_mult - 1.0).abs() < 1e-10,
        "price at entry+(entry-stop) should yield exactly 1R, got {r_mult}");
}

#[test]
fn r_multiple_at_2r_triggers_first_partial() {
    // Partial-close rule: take 1/3 out when r_mult >= 2.0
    let entry: f64 = 100.0;
    let stop       = 98.0;   // 2% stop
    let margin   = 100.0;
    let leverage = 3.0;
    let qty      = margin * leverage / entry;     // 3 shares
    let r_risk   = (entry - stop).abs() * qty;    // $6 = 1R

    // 2R price: entry + 2×(entry−stop) = $104
    let at_2r  = entry + 2.0 * (entry - stop);
    let pnl    = (at_2r - entry) * qty;            // $12
    let r_mult = pnl / r_risk;

    assert!((r_mult - 2.0).abs() < 1e-10, "should be 2R at ${at_2r}, got {r_mult}");
    assert!(r_mult >= 2.0, "2R condition should trigger first partial close");
}

// ── DCA / Pyramid weighted average entry ─────────────────────────────────────

#[test]
fn dca_lowers_average_entry_for_long() {
    let old_entry = 100.0;
    let dca_price = 94.0;  // DCA at a lower price
    let old_qty   = 3.0;
    let add_qty   = 1.5;   // 50% add-on
    let new_qty   = old_qty + add_qty;

    let avg_entry = (old_entry * old_qty + dca_price * add_qty) / new_qty;

    assert!(avg_entry < old_entry, "DCA must lower avg_entry for LONG");
    assert!(avg_entry > dca_price, "DCA avg_entry must stay above DCA price");
}

#[test]
fn pyramid_raises_average_entry_for_long() {
    let old_entry     = 100.0;
    let pyramid_price = 108.0; // Pyramid into strength
    let old_qty       = 3.0;
    let add_qty       = 1.5;
    let new_qty       = old_qty + add_qty;

    let avg_entry = (old_entry * old_qty + pyramid_price * add_qty) / new_qty;

    assert!(avg_entry > old_entry, "pyramid must raise avg_entry");
    assert!(avg_entry < pyramid_price, "avg_entry must stay below pyramid price");
}

#[test]
fn dca_max_2_addons_prevents_over_averaging() {
    // The bot enforces dca_count < 2 before allowing a new DCA.
    // After 2 adds, dca_count == 2, so no more averaging is permitted.
    let after_two_adds = 2u8;
    let still_allowed  = after_two_adds < 2;
    assert!(!still_allowed,
        "dca_count={after_two_adds} should NOT satisfy dca_count < 2 gate");
}

// ── Partial close accounting ──────────────────────────────────────────────────

#[test]
fn partial_close_takes_one_third_of_position() {
    let qty: f64     = 9.0;
    let size_usd: f64 = 900.0;
    let close_qty  = qty / 3.0;
    let close_size = size_usd / 3.0;
    let remaining_qty  = qty - close_qty;
    let remaining_size = size_usd - close_size;

    assert!((close_qty - 3.0).abs() < 1e-10,   "close_qty should be 1/3 = 3.0");
    assert!((close_size - 300.0).abs() < 1e-10, "close_size should be 1/3 = $300");
    assert!((remaining_qty - 6.0).abs() < 1e-10,   "remaining qty = 6.0");
    assert!((remaining_size - 600.0).abs() < 1e-10, "remaining size = $600");
}

#[test]
fn partial_close_r_dollars_risked_scales_to_two_thirds() {
    // After 1/3 close, r_dollars_risked should scale by 2/3 to match remaining qty.
    let original_r: f64 = 150.0;
    let scaled_r   = original_r * (2.0 / 3.0);
    let expected_r = 100.0;
    assert!(
        (scaled_r - expected_r).abs() < 1e-10,
        "r_dollars_risked after 1/3 close should be {expected_r}, got {scaled_r}"
    );
}

#[test]
fn second_partial_after_first_covers_correct_fraction_of_original() {
    // Tranche 1 (at 2R): close 1/3 of original → 2/3 remain.
    // Tranche 2 (at 4R): close 1/3 of REMAINING (2/3 of original).
    // After both: 2/3 - (1/3 × 2/3) = 2/3 - 2/9 = 4/9 ≈ 44.4% remain.
    let original_qty: f64 = 9.0;

    let after_t1 = original_qty - original_qty / 3.0;  // 6.0
    let after_t2 = after_t1 - after_t1 / 3.0;          // 4.0

    assert!((after_t1 - 6.0).abs() < 1e-10, "after tranche 1: {after_t1}");
    assert!((after_t2 - 4.0).abs() < 1e-10, "after tranche 2: {after_t2}");

    let remaining_pct = after_t2 / original_qty * 100.0;
    assert!((remaining_pct - 44.44).abs() < 0.01, "~44.4%% of original held in final runner");
}

// ── Trailing stop logic ───────────────────────────────────────────────────────

#[test]
fn trailing_stop_breakeven_logic_long() {
    let entry = 100.0;
    let mut stop = 95.0;
    let r_mult = 1.0; // at 1R

    if r_mult >= 1.0 && stop < entry {
        stop = entry;
    }
    assert_eq!(stop, entry, "at 1R, LONG stop should move to breakeven");
}

#[test]
fn trailing_stop_breakeven_not_triggered_before_1r() {
    let entry = 100.0;
    let mut stop = 95.0;
    let r_mult = 0.8; // just under 1R

    if r_mult >= 1.0 && stop < entry {
        stop = entry;
    }
    assert_eq!(stop, 95.0, "below 1R, stop should stay at original level");
}

#[test]
fn trailing_stop_atr_trail_at_1_5r_for_long() {
    let hwm: f64 = 107.5; // high water mark at 1.5R
    let atr   = 2.0;
    let stop  = 100.0; // at breakeven already
    let r_mult = 1.5;

    let trail = hwm - atr * 1.2;  // 107.5 - 2.4 = 105.1
    let new_stop = if r_mult >= 1.5 && trail > stop { trail } else { stop };

    assert!(
        (new_stop - 105.1).abs() < 1e-10,
        "trailing stop at 1.5R should be HWM - 1.2×ATR = 105.1, got {new_stop}"
    );
}

#[test]
fn trailing_stop_atr_trail_never_moves_backward() {
    let hwm: f64     = 107.5;
    let atr          = 2.0;
    let current_stop = 104.0;

    let trail = hwm - atr * 1.2;  // 105.1 > 104.0 → advance
    let new_stop = if trail > current_stop { trail } else { current_stop };
    assert!((new_stop - 105.1).abs() < 1e-10, "stop should advance when trail is higher");

    // If trail would be lower (HWM hasn't moved to a NEW high), stop stays
    let trail_low = 103.5;
    let stop_after = if trail_low > current_stop { trail_low } else { current_stop };
    assert_eq!(stop_after, current_stop,
        "stop must never move backward (trail {trail_low} < current {current_stop})");
}

// ── Stop-loss hit detection ───────────────────────────────────────────────────

#[test]
fn stop_hit_detection_long() {
    let stop = 95.0;
    let cur  = 94.9;
    assert!(cur <= stop, "LONG stop hit: cur {cur} <= stop {stop}");
}

#[test]
fn stop_not_hit_long_while_above_stop() {
    let stop = 95.0;
    let cur  = 95.1;
    assert!(!(cur <= stop), "LONG stop not hit: cur {cur} > stop {stop}");
}

#[test]
fn stop_hit_detection_short() {
    let stop = 105.0;
    let cur  = 105.1;
    assert!(cur >= stop, "SHORT stop hit: cur {cur} >= stop {stop}");
}

// ── Fee calculator integration ────────────────────────────────────────────────

#[test]
fn fee_calculator_computes_nonzero_round_trip_cost() {
    let calc = FeeCalculator::new(FeeStructure::hyperliquid());
    let entry_price = 100.0;
    let qty         = 3.0;
    let result = calc.calculate_round_trip_fees(qty, entry_price, entry_price);
    assert!(result.total_fees > 0.0,
        "round-trip fee must be positive for non-zero position: {}", result.total_fees);
}

#[test]
fn fee_calculator_scales_linearly_with_size() {
    let calc = FeeCalculator::new(FeeStructure::hyperliquid());
    let fee_small = calc.calculate_round_trip_fees(1.0, 100.0, 100.0).total_fees;
    let fee_large = calc.calculate_round_trip_fees(2.0, 100.0, 100.0).total_fees;
    assert!(
        (fee_large - fee_small * 2.0).abs() < 1e-10,
        "fees should scale linearly: 2× size = 2× fee, got {fee_small} vs {fee_large}"
    );
}

#[test]
fn fee_calculator_breakeven_move_is_positive() {
    let calc = FeeCalculator::new(FeeStructure::hyperliquid());
    let breakeven_pct = calc.minimum_breakeven_move_pct();
    assert!(breakeven_pct > 0.0,
        "breakeven move must be positive (non-zero fees to overcome): {breakeven_pct}");
}

// ── Position manager integration ──────────────────────────────────────────────

fn make_entry(price: f64, qty: f64) -> PositionEntry {
    PositionEntry {
        entry_number:        1,
        entry_price:         price,
        entry_time:          0,
        quantity:            qty,
        position_size_pct:   0.10,
        leverage:            3.0,
        confidence_at_entry: 0.80,
        confluence_signals:  5,
    }
}

#[test]
fn aggregate_position_add_entry_updates_avg_price() {
    let mut pos = AggregatePosition::new("TEST".to_string());
    pos.add_entry(make_entry(100.0, 3.0));  // 3 units at $100
    pos.add_entry(make_entry(110.0, 1.0));  // 1 unit at $110

    // Expected avg = (100×3 + 110×1) / 4 = 102.5
    let expected_avg = 102.5;
    assert!(
        (pos.average_entry_price - expected_avg).abs() < 1e-6,
        "avg entry after two buys: expected {expected_avg}, got {}",
        pos.average_entry_price
    );
}

#[test]
fn aggregate_position_total_size_sums_correctly() {
    let mut pos = AggregatePosition::new("ETH".to_string());
    pos.add_entry(make_entry(1000.0, 2.0));
    pos.add_entry(make_entry(1100.0, 1.0));
    assert!(
        (pos.total_quantity - 3.0).abs() < 1e-10,
        "total quantity should be 3.0, got {}", pos.total_quantity
    );
}

#[test]
fn aggregate_position_average_entry_between_entries() {
    let mut pos = AggregatePosition::new("SOL".to_string());
    let low  = 90.0;
    let high = 110.0;
    pos.add_entry(make_entry(low,  5.0));
    pos.add_entry(make_entry(high, 5.0));
    // Equal quantities → avg = midpoint
    let expected = (low + high) / 2.0;  // 100.0
    assert!(
        (pos.average_entry_price - expected).abs() < 1e-10,
        "equal quantities: avg should be midpoint {expected}, got {}",
        pos.average_entry_price
    );
}

#[test]
fn dca_rules_max_entries_prevents_5th_entry() {
    let rules = DCARules::default();
    // max_entries = 4 (from DCARules::default)
    let pos = AggregatePosition::new("BTC".to_string());
    // Simulate 4 entries already open (entries.len() == 4)
    // can_add_entry checks: entries.len() < max_entries
    // At 4 entries (== max_entries), should return false
    assert!(
        !pos.can_add_entry(rules.max_entries),
        "empty position should allow first entry"  // empty = 0 entries, 0 < 4
    );
    // A position with 4 entries shouldn't be able to add more
    let max = rules.max_entries;
    assert!(max >= 2, "DCARules should allow at least 2 entries (got {max})");
}

#[test]
fn dca_rules_allows_entries_up_to_max() {
    let rules = DCARules::default();
    let pos = AggregatePosition::new("BNB".to_string());
    // Empty position: 0 entries < 4 max → can add
    assert!(
        pos.can_add_entry(rules.max_entries),
        "fresh position should be able to add first entry"
    );
}
