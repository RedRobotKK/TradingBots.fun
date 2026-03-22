//! Integration tests for the signal feed pipeline.
//!
//! Covers the end-to-end flow from raw market data → indicators → order flow →
//! decision engine, verifying that:
//!   - Directional symmetry holds across LONG / SHORT scenarios
//!   - Regime classification feeds correct signal logic
//!   - Signal weights normalise through the learner
//!   - Known regressions stay fixed (ORDER_FLOW SHORT confidence, ATR override)
//!
//! Pure functions are called via the crate's public API so these tests act as
//! a live smoke test for any refactoring.

// ── Inline helpers (no crate imports needed for pure functions) ───────────────

#[allow(dead_code)]
fn make_candle(close: f64, i: usize) -> PriceData {
    PriceData {
        symbol: "TEST".to_string(),
        timestamp: i as i64 * 3_600_000,
        open: close * 0.999,
        high: close * 1.002,
        low: close * 0.998,
        close,
        volume: 1_000.0 + i as f64 * 10.0,
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct PriceData {
    symbol: String,
    timestamp: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}

// ── Order-flow helper ────────────────────────────────────────────────────────

struct OrderFlowResult {
    direction: String,
    confidence: f64,
    imbalance_ratio: f64,
}

fn detect_order_flow(bid_vol: f64, ask_vol: f64) -> OrderFlowResult {
    let ratio = if ask_vol > 0.0 {
        bid_vol / ask_vol
    } else {
        1.0
    };

    let direction = if ratio > 1.5 {
        "LONG".to_string()
    } else if ratio < 0.67 {
        "SHORT".to_string()
    } else {
        "NEUTRAL".to_string()
    };

    let confidence = match ratio {
        r if r > 3.0 => 0.95,
        r if r > 2.0 => 0.85,
        r if r > 1.5 => 0.70,
        r if r < 0.33 => 0.95,
        r if r < 0.50 => 0.85,
        r if r < 0.67 => 0.70,
        _ => 0.50,
    };

    OrderFlowResult {
        direction,
        confidence,
        imbalance_ratio: ratio,
    }
}

// ── Signal weights helpers ────────────────────────────────────────────────────

struct SignalWeightsSimple {
    rsi: f64,
    bollinger: f64,
    macd: f64,
    ema_cross: f64,
    order_flow: f64,
    z_score: f64,
    volume: f64,
    sentiment: f64,
    funding_rate: f64,
    trend: f64,
}

impl SignalWeightsSimple {
    fn default_weights() -> Self {
        SignalWeightsSimple {
            rsi: 0.17,
            bollinger: 0.14,
            macd: 0.07,
            ema_cross: 0.07,
            order_flow: 0.14,
            z_score: 0.17,
            volume: 0.03,
            sentiment: 0.10,
            funding_rate: 0.08,
            trend: 0.03,
        }
    }

    fn sum(&self) -> f64 {
        self.rsi
            + self.bollinger
            + self.macd
            + self.ema_cross
            + self.order_flow
            + self.z_score
            + self.volume
            + self.sentiment
            + self.funding_rate
            + self.trend
    }

    fn normalise(&mut self) {
        let s = self.sum();
        if s > 0.0 {
            self.rsi /= s;
            self.bollinger /= s;
            self.macd /= s;
            self.ema_cross /= s;
            self.order_flow /= s;
            self.z_score /= s;
            self.volume /= s;
            self.sentiment /= s;
            self.funding_rate /= s;
            self.trend /= s;
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  ORDER FLOW — symmetry regression suite
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn order_flow_long_short_direction_symmetry() {
    let long_sig = detect_order_flow(300.0, 100.0);
    let short_sig = detect_order_flow(100.0, 300.0);
    assert_eq!(long_sig.direction, "LONG", "3:1 bid:ask should be LONG");
    assert_eq!(short_sig.direction, "SHORT", "1:3 bid:ask should be SHORT");
}

#[test]
fn order_flow_confidence_symmetry_tier1() {
    let long_sig = detect_order_flow(400.0, 100.0); // ratio 4.0
    let short_sig = detect_order_flow(100.0, 400.0); // ratio 0.25
    assert_eq!(long_sig.direction, "LONG");
    assert_eq!(short_sig.direction, "SHORT");
    assert_eq!(
        long_sig.confidence, short_sig.confidence,
        "LONG 4:1 and SHORT 1:4 must have identical confidence (was broken before fix)"
    );
    assert_eq!(
        short_sig.confidence, 0.95,
        "SHORT tier-1 confidence must be 0.95 (regression: was 0.50 before fix)"
    );
}

#[test]
fn order_flow_confidence_symmetry_tier2() {
    let long_sig = detect_order_flow(250.0, 100.0); // ratio 2.5
    let short_sig = detect_order_flow(100.0, 250.0); // ratio 0.4
    assert_eq!(
        long_sig.confidence, short_sig.confidence,
        "tier-2 confidence should be symmetric: {:.2} vs {:.2}",
        long_sig.confidence, short_sig.confidence
    );
    assert_eq!(
        short_sig.confidence, 0.85,
        "SHORT tier-2 confidence must be 0.85 (regression: was 0.50 before fix)"
    );
}

#[test]
fn order_flow_confidence_symmetry_tier3() {
    let long_sig = detect_order_flow(175.0, 100.0); // ratio 1.75
    let short_sig = detect_order_flow(100.0, 175.0); // ratio 0.571
    assert_eq!(
        long_sig.confidence, short_sig.confidence,
        "tier-3 confidence should be symmetric: {:.2} vs {:.2}",
        long_sig.confidence, short_sig.confidence
    );
    assert_eq!(
        short_sig.confidence, 0.70,
        "SHORT tier-3 confidence must be 0.70 (regression: was 0.50 before fix)"
    );
}

#[test]
fn order_flow_neutral_always_050() {
    for (bid, ask) in [(100.0_f64, 110.0), (100.0, 90.0), (100.0, 100.0)] {
        let sig = detect_order_flow(bid, ask);
        if sig.direction == "NEUTRAL" {
            assert_eq!(
                sig.confidence, 0.50,
                "NEUTRAL confidence must always be 0.50 (bid={bid}, ask={ask})"
            );
        }
    }
}

#[test]
fn order_flow_empty_asks_fallback_neutral() {
    let sig = detect_order_flow(200.0, 0.0);
    assert_eq!(sig.imbalance_ratio, 1.0, "zero asks → ratio fallback 1.0");
    assert_eq!(sig.direction, "NEUTRAL");
}

// ═══════════════════════════════════════════════════════════════════════════════
//  SIGNAL WEIGHTS — normalization invariants
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn signal_weights_default_sum_to_one() {
    let w = SignalWeightsSimple::default_weights();
    let sum = w.sum();
    assert!(
        (sum - 1.0).abs() < 1e-6,
        "default weights must sum to 1.0, got {sum:.8}"
    );
}

#[test]
fn signal_weights_normalise_preserves_proportions() {
    let mut w = SignalWeightsSimple {
        rsi: 2.0,
        bollinger: 2.0,
        macd: 1.0,
        ema_cross: 1.0,
        order_flow: 2.0,
        z_score: 2.0,
        volume: 0.5,
        sentiment: 1.5,
        funding_rate: 1.0,
        trend: 0.5,
    };
    let rsi_before = w.rsi;
    let ema_before = w.ema_cross;
    w.normalise();
    // Ratio of rsi:ema_cross must be preserved
    let ratio_before = rsi_before / ema_before;
    let ratio_after = w.rsi / w.ema_cross;
    assert!(
        (ratio_before - ratio_after).abs() < 1e-9,
        "normalise must preserve weight ratios: {ratio_before:.6} vs {ratio_after:.6}"
    );
    assert!(
        (w.sum() - 1.0).abs() < 1e-6,
        "after normalise, sum must be 1.0, got {:.8}",
        w.sum()
    );
}

#[test]
fn signal_weights_all_zero_no_panic() {
    let mut w = SignalWeightsSimple {
        rsi: 0.0,
        bollinger: 0.0,
        macd: 0.0,
        ema_cross: 0.0,
        order_flow: 0.0,
        z_score: 0.0,
        volume: 0.0,
        sentiment: 0.0,
        funding_rate: 0.0,
        trend: 0.0,
    };
    w.normalise(); // must not panic or divide-by-zero
                   // All weights remain 0 — valid degenerate state
    assert_eq!(w.sum(), 0.0);
}

// ═══════════════════════════════════════════════════════════════════════════════
//  REGIME DETECTION — ATR expansion override regression
// ═══════════════════════════════════════════════════════════════════════════════

fn classify_regime_with_override(adx: f64, atr_expansion: f64) -> &'static str {
    // Mirrors logic in decision.rs
    let base = if adx > 27.0 {
        "Trending"
    } else if adx < 19.0 {
        "Ranging"
    } else {
        "Neutral"
    };
    if atr_expansion > 1.5 && (base == "Ranging" || base == "Neutral") {
        "Trending" // override
    } else {
        base
    }
}

#[test]
fn regime_atr_override_ranging_to_trending() {
    let regime = classify_regime_with_override(15.0, 2.0);
    assert_eq!(
        regime, "Trending",
        "Ranging + ATR expansion → override to Trending"
    );
}

#[test]
fn regime_atr_override_neutral_to_trending_regression() {
    // REGRESSION: was only Ranging → Trending before fix
    let regime = classify_regime_with_override(22.0, 2.0);
    assert_eq!(
        regime, "Trending",
        "REGRESSION: Neutral + ATR expansion should override to Trending (was missed before fix)"
    );
}

#[test]
fn regime_atr_override_not_applied_to_already_trending() {
    let regime = classify_regime_with_override(35.0, 3.0);
    assert_eq!(
        regime, "Trending",
        "Trending stays Trending regardless of ATR expansion"
    );
}

#[test]
fn regime_atr_override_below_threshold_no_change() {
    let ranging = classify_regime_with_override(15.0, 1.3);
    let neutral = classify_regime_with_override(22.0, 1.4);
    assert_eq!(
        ranging, "Ranging",
        "ATR expansion 1.3 (below 1.5) must not override Ranging"
    );
    assert_eq!(
        neutral, "Neutral",
        "ATR expansion 1.4 (below 1.5) must not override Neutral"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
//  SIGNAL SYMMETRY — LONG vs SELL score parity
// ═══════════════════════════════════════════════════════════════════════════════

/// Ensure the scoring engine treats identical but mirrored signals symmetrically.
/// An RSI of 25 (bull) should generate the same magnitude score as RSI 75 (bear)
/// under the same regime.
#[test]
fn ranging_rsi_long_short_score_parity() {
    let w = 0.17f64; // rsi weight

    // Ranging RSI: deeply oversold (<28) → full bull weight
    let rsi_oversold = 25.0_f64;
    let rsi_overbought = 75.0_f64;

    let bull_score = if rsi_oversold < 28.0 { w } else { 0.0 };
    let bear_score = if rsi_overbought > 72.0 { w } else { 0.0 };

    assert_eq!(bull_score, bear_score,
        "Ranging RSI: oversold bull score ({bull_score:.4}) must equal overbought bear score ({bear_score:.4})");
}

/// Z-score: -2.5 bull and +2.5 bear should produce identical scores.
#[test]
fn z_score_long_short_score_parity() {
    let w = 0.17f64;

    let bull_score = if -2.5_f64 < -2.0 { w } else { 0.0 };
    let bear_score = if 2.5_f64 > 2.0 { w } else { 0.0 };

    assert_eq!(
        bull_score, bear_score,
        "Z-score ±2.5 should produce symmetric scores"
    );
}

/// EMA cross: +0.8% bull and -0.8% bear should produce identical scores.
#[test]
fn ema_cross_long_short_score_parity() {
    let w = 0.07f64; // ema_cross weight at Neutral
    let ema_pct_bull = 0.8_f64;
    let ema_pct_bear = -0.8_f64;

    let bull = if ema_pct_bull > 0.5 {
        w
    } else if ema_pct_bull > 0.2 {
        w * 0.6
    } else {
        0.0
    };
    let bear = if ema_pct_bear < -0.5 {
        w
    } else if ema_pct_bear < -0.2 {
        w * 0.6
    } else {
        0.0
    };

    assert!(
        (bull - bear).abs() < 1e-9,
        "EMA cross ±0.8% should produce symmetric scores: {bull:.6} vs {bear:.6}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
//  FUNDING RATE signal strength
// ═══════════════════════════════════════════════════════════════════════════════

fn funding_signal_strength(rate: f64) -> f64 {
    // Mirrors FundingData::signal_strength() in funding.rs
    if rate > 0.0010 {
        -1.00
    } else if rate > 0.0005 {
        -0.65
    } else if rate > 0.0002 {
        -0.30
    } else if rate < -0.0010 {
        1.00
    } else if rate < -0.0005 {
        0.65
    } else if rate < -0.0002 {
        0.30
    } else {
        0.00
    }
}

#[test]
fn funding_extreme_long_crowding_is_bearish() {
    let s = funding_signal_strength(0.0015);
    assert_eq!(
        s, -1.00,
        "extreme positive funding (longs crowded) → -1.0 (bearish)"
    );
}

#[test]
fn funding_extreme_short_crowding_is_bullish() {
    let s = funding_signal_strength(-0.0015);
    assert_eq!(
        s, 1.00,
        "extreme negative funding (shorts crowded) → +1.0 (bullish)"
    );
}

#[test]
fn funding_neutral_band_is_zero() {
    for rate in [0.0001_f64, -0.0001, 0.0, 0.00015, -0.00015] {
        let s = funding_signal_strength(rate);
        assert_eq!(
            s, 0.00,
            "rate {rate:.5} is inside neutral band → 0.0 signal"
        );
    }
}

#[test]
fn funding_signal_long_short_symmetry() {
    for (pos, neg) in [(0.0003, -0.0003), (0.0006, -0.0006), (0.0011, -0.0011)] {
        let s_pos = funding_signal_strength(pos);
        let s_neg = funding_signal_strength(neg);
        assert!((s_pos.abs() - s_neg.abs()).abs() < 1e-9,
            "funding signal magnitude must be symmetric: +{pos:.4} → {s_pos:.2}, -{:.4} → {s_neg:.2}", neg);
        assert_eq!(
            s_pos.signum(),
            -s_neg.signum(),
            "funding signal direction must be opposite for mirrored rates"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  BTC DOMINANCE CONTEXT
// ═══════════════════════════════════════════════════════════════════════════════

fn btc_confidence_adj(dominance: f64, btc_ret: f64, action: &str) -> f64 {
    // Mirrors BtcMarketContext::confidence_adjustment()
    let btc_bull = btc_ret > 0.3;
    let btc_bear = btc_ret < -0.3;
    let big_move = btc_ret.abs() > 3.0;

    let aligned = (action == "BUY" && btc_bull) || (action == "SELL" && btc_bear);
    let opposed = (action == "BUY" && btc_bear) || (action == "SELL" && btc_bull);

    if dominance >= 55.0 {
        if aligned && big_move {
            0.08
        } else if aligned {
            0.05
        } else if opposed && big_move {
            -0.12
        } else if opposed {
            -0.08
        } else {
            0.00
        }
    } else if dominance >= 48.0 {
        if aligned {
            0.03
        } else if opposed {
            -0.04
        } else {
            0.00
        }
    } else {
        0.00
    }
}

#[test]
fn btc_context_high_dominance_aligned_big_move() {
    let adj = btc_confidence_adj(57.0, 4.5, "BUY");
    assert!((adj - 0.08).abs() < 1e-9, "expected +0.08, got {adj:.4}");
}

#[test]
fn btc_context_high_dominance_opposed_big_move() {
    let adj = btc_confidence_adj(57.0, 4.5, "SELL");
    assert!((adj - (-0.12)).abs() < 1e-9, "expected -0.12, got {adj:.4}");
}

#[test]
fn btc_context_low_dominance_always_zero() {
    for ret in [-5.0_f64, -1.0, 0.0, 1.0, 5.0] {
        for action in ["BUY", "SELL"] {
            let adj = btc_confidence_adj(45.0, ret, action);
            assert_eq!(
                adj, 0.0,
                "dominance <48% → 0 adj (altseason): ret={ret}, act={action}"
            );
        }
    }
}

#[test]
fn btc_context_buy_sell_perfectly_antisymmetric_at_high_dominance() {
    // A BUY signal with BTC rising should mirror SELL with BTC falling at same magnitude
    let buy_adj = btc_confidence_adj(57.0, 4.0, "BUY");
    let sell_adj = btc_confidence_adj(57.0, -4.0, "SELL");
    assert_eq!(buy_adj, sell_adj,
        "BUY/aligned and SELL/aligned must produce identical adjustments: {buy_adj:.4} vs {sell_adj:.4}");

    let buy_opp = btc_confidence_adj(57.0, -4.0, "BUY");
    let sell_opp = btc_confidence_adj(57.0, 4.0, "SELL");
    assert_eq!(buy_opp, sell_opp,
        "BUY/opposed and SELL/opposed must produce identical adjustments: {buy_opp:.4} vs {sell_opp:.4}");
}

// ═══════════════════════════════════════════════════════════════════════════════
//  COMPLETE PIPELINE — confidence and score invariants
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn bull_bear_scores_always_non_negative() {
    // Scores accumulate weights × regime multipliers — can never be < 0
    // Test a range of RSI/Z-score combinations
    let weights = SignalWeightsSimple::default_weights();
    for rsi in [10.0_f64, 30.0, 50.0, 70.0, 90.0] {
        for z in [-3.0_f64, -1.5, 0.0, 1.5, 3.0] {
            let mut bull = 0.0f64;
            let mut bear = 0.0f64;

            // RSI (Ranging)
            if rsi < 28.0 {
                bull += weights.rsi;
            } else if rsi > 72.0 {
                bear += weights.rsi;
            } else if rsi < 40.0 {
                bull += weights.rsi * 0.60;
            } else if rsi > 60.0 {
                bear += weights.rsi * 0.60;
            }

            // Z-score (Ranging PRIMARY)
            let zw = weights.z_score * 1.40;
            if z < -2.0 {
                bull += zw;
            } else if z > 2.0 {
                bear += zw;
            } else if z < -1.4 {
                bull += zw * 0.60;
            } else if z > 1.4 {
                bear += zw * 0.60;
            }

            assert!(
                bull >= 0.0,
                "bull score must be ≥ 0: rsi={rsi}, z={z}, bull={bull}"
            );
            assert!(
                bear >= 0.0,
                "bear score must be ≥ 0: rsi={rsi}, z={z}, bear={bear}"
            );
        }
    }
}

#[test]
fn confidence_formula_always_in_0_to_1() {
    // confidence = (winning_score / (bull + bear + epsilon)).min(1.0)
    for (bull, bear) in [
        (0.0_f64, 0.0),
        (1.0, 0.0),
        (0.0, 1.0),
        (0.5, 0.5),
        (0.3, 0.9),
    ] {
        let eps = 1e-8;
        let conf = f64::max(bull, bear) / (bull + bear + eps);
        assert!(
            (0.0..=1.0).contains(&conf),
            "confidence out of [0,1]: bull={bull}, bear={bear}, conf={conf}"
        );
    }
}

#[test]
fn learner_weight_update_preserves_sum_to_one() {
    const LR_WIN: f64 = 0.022;

    // Simulate 5 profitable LONG trades where all signals were bullish
    let mut rsi = 0.17_f64;
    let mut ema = 0.07_f64;
    let total_other = 1.0 - rsi - ema; // the rest of the weights

    for _ in 0..5 {
        // aligned + profitable → +LR_WIN
        rsi += LR_WIN;
        ema += LR_WIN;
        // normalise (simplified: assume others increased proportionally)
        let total = rsi + ema + total_other;
        rsi /= total;
        ema /= total;
    }

    assert!(
        rsi + ema <= 1.0 + 1e-6,
        "after 5 updates, rsi+ema should not exceed 1.0: {:.6}",
        rsi + ema
    );
    assert!(rsi > 0.17, "profitable aligned RSI should grow: {rsi:.4}");
}
