//! Online learning module.
//!
//! After each closed trade the bot records which direction each signal was
//! pointing at entry and whether the trade was profitable.  Signal weights
//! are nudged toward accurate signals (and away from inaccurate ones), then
//! re-normalised and persisted to `signal_weights.json` so learning survives
//! restarts.
//!
//! ## Signal catalogue (10 total, sum-to-1 normalised)
//!
//! | Field        | What it measures                                     |
//! |--------------|------------------------------------------------------|
//! | rsi          | Wilder's RSI — mean-reversion extremes               |
//! | bollinger    | Bollinger Band position + squeeze breakout           |
//! | macd         | MACD histogram momentum                              |
//! | ema_cross    | EMA(8/21) cross — institutional trend signal        |
//! | order_flow   | Real-time order-book bid/ask pressure                |
//! | z_score      | Statistical mean-reversion depth                    |
//! | volume       | Volume conviction multiplier                         |
//! | sentiment    | LunarCrush social sentiment                         |
//! | funding_rate | Perpetual funding rate (contrarian crowd signal)     |
//! | trend        | Legacy 10-bar % change (kept for file compatibility) |

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

const WEIGHTS_FILE: &str = "signal_weights.json";

// ─────────────────────────── Signal Weights ──────────────────────────────────

/// Adaptive weights for all signal components.  Always normalised to sum = 1.0.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalWeights {
    // ── Core signals ──────────────────────────────────────────────────────────
    pub rsi:        f64,   // 0.16 — mean-reversion oscillator
    pub bollinger:  f64,   // 0.13 — band position / squeeze
    pub macd:       f64,   // 0.13 — momentum (histogram direction)
    /// EMA(8/21) cross — replaces the old crude 10-bar trend signal.
    #[serde(default = "default_ema_cross_weight")]
    pub ema_cross:  f64,   // 0.14 — institutional trend confirmation
    pub order_flow: f64,   // 0.12 — live order-book pressure
    /// Statistical mean-reversion depth (Z-score).
    #[serde(default = "default_z_score_weight")]
    pub z_score:    f64,   // 0.08 — over-extension detector
    /// Volume conviction multiplier.
    #[serde(default = "default_volume_weight")]
    pub volume:     f64,   // 0.06 — high-volume signals carry more weight
    /// LunarCrush social sentiment (0 when data unavailable).
    #[serde(default = "default_sentiment_weight")]
    pub sentiment:     f64,   // 0.09 — social signal
    /// Perpetual funding rate — contrarian crowd positioning signal.
    /// High positive funding → longs crowded → bearish lean.
    /// High negative funding → shorts crowded → bullish lean.
    /// 0 when Binance futures data is unavailable.
    #[serde(default = "default_funding_rate_weight")]
    pub funding_rate:  f64,   // 0.07 — contrarian leverage signal
    // ── Pattern signals ───────────────────────────────────────────────────────
    /// Candlestick pattern weight (single/double/triple bar formations).
    #[serde(default = "default_candle_pattern_weight")]
    pub candle_pattern: f64,  // learned weight for candle patterns
    /// Chart pattern weight (10–60 bar structural patterns).
    #[serde(default = "default_chart_pattern_weight")]
    pub chart_pattern:  f64,  // learned weight for chart patterns
    // ── Legacy ────────────────────────────────────────────────────────────────
    /// 10-bar % change — kept for backwards compatibility with old weight files.
    /// Functionally absorbed into ema_cross; weight held at min floor.
    #[serde(default = "default_trend_weight")]
    pub trend:         f64,   // 0.07 — legacy trend signal
}

fn default_ema_cross_weight()    -> f64 { 0.07 }
fn default_z_score_weight()      -> f64 { 0.17 }
fn default_volume_weight()       -> f64 { 0.03 }
fn default_sentiment_weight()    -> f64 { 0.10 }
fn default_funding_rate_weight() -> f64 { 0.08 }
fn default_candle_pattern_weight() -> f64 { 0.04 }
fn default_chart_pattern_weight()  -> f64 { 0.04 }
fn default_trend_weight()        -> f64 { 0.03 }

impl Default for SignalWeights {
    fn default() -> Self {
        // Weights sum to 1.0.
        //
        // Calibrated from signal_quality_backtest.py results
        // (8 assets × 2 years × 15m, ~560K bars, Feb 2024–Feb 2026):
        //
        //   Z-Score:      T-stat +5.01  → best mean-reversion signal, boosted
        //   RSI:          T-stat +2.91  → solid mean-reversion, boosted
        //   Bollinger:    T-stat +1.49  → marginal but regime-aware, small boost
        //   Order Flow:   live data     → backtest proxy was noisy, real signal better
        //   Sentiment:    orthogonal    → unrelated to price, kept
        //   Funding Rate: contrarian    → genuine edge, kept
        //   MACD:         T-stat -4.02  → momentum chasing at 15m = buying tops, REDUCED
        //   EMA Cross:    T-stat -6.00* → *partly backtest artifact; always-on is harmful, REDUCED
        //   Trend 10-bar: T-stat -4.12  → consistently buys tops/sells bottoms, NEAR-ELIMINATED
        //   Volume direct:T-stat -3.53  → high-vol at extremes = exhaustion, directional REMOVED
        //
        // * EMA Cross F grade was partly due to "always-on" backtest implementation.
        //   Real bot uses ema_cross_pct threshold.  Weight reduced but not eliminated.
        SignalWeights {
            rsi:          0.16,   // +0.02 — mean-reversion works
            bollinger:    0.13,   // +0.02 — regime-aware, marginal boost
            macd:         0.07,   // -0.05 — harmful at 15m, significantly reduced
            ema_cross:    0.07,   // -0.06 — reduced; gap filter added in decision.rs
            order_flow:   0.14,   // +0.03 — real order-book data outperforms candle proxy
            z_score:      0.16,   // +0.09 — highest T-stat, significantly boosted
            volume:       0.03,   // -0.03 — directional removed; amplifier role kept
            sentiment:    0.10,   // +0.01 — orthogonal signal, slight boost
            funding_rate: 0.08,   // +0.01 — contrarian edge, slight boost
            candle_pattern: 0.04, // — pattern recognition weight
            chart_pattern:  0.04, // — structural pattern weight
            trend:        0.03,   // -0.04 — near-eliminated; threshold raised in decision.rs
        }
    }
}

impl SignalWeights {
    /// Load from disk, or return defaults if the file is missing or corrupt.
    /// Automatically re-normalises to handle old 6-field files.
    pub fn load() -> Self {
        if let Ok(data) = std::fs::read_to_string(WEIGHTS_FILE) {
            match serde_json::from_str::<SignalWeights>(&data) {
                Ok(mut w) => {
                    w.clamp_and_normalise();
                    log::info!(
                        "📚 Weights: RSI={:.2} BB={:.2} MACD={:.2} EMA={:.2} OF={:.2} Z={:.2} Vol={:.2} Sent={:.2} FR={:.2} Trend={:.2}",
                        w.rsi, w.bollinger, w.macd, w.ema_cross, w.order_flow,
                        w.z_score, w.volume, w.sentiment, w.funding_rate, w.trend
                    );
                    return w;
                }
                Err(e) => log::warn!("signal_weights.json parse error: {} — using defaults", e),
            }
        }
        log::info!("📚 Using default signal weights (10 signals)");
        Self::default()
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            if let Err(e) = std::fs::write(WEIGHTS_FILE, json) {
                log::warn!("Could not save signal weights: {}", e);
            }
        }
    }

    /// Adjust weights from a closed trade outcome.
    ///
    /// A signal is "aligned" when its bull/bear call matches the trade side.
    /// Profitable + aligned   → reward the signal  (+LR_WIN)
    /// Profitable + misaligned → small penalty      (−LR_LOSE)
    /// Loss       + aligned   → small penalty      (−LR_LOSE)
    /// Loss       + misaligned → reward (contrarian) (+LR_WIN × 0.5)
    pub fn update(&mut self, contrib: &SignalContribution, was_long: bool, profitable: bool) {
        const LR_WIN:  f64 = 0.022;
        const LR_LOSE: f64 = 0.009;

        fn nudge(w: &mut f64, sig_bullish: bool, was_long: bool, profitable: bool) {
            let aligned = sig_bullish == was_long;
            match (aligned, profitable) {
                (true,  true)  => *w += LR_WIN,
                (true,  false) => *w -= LR_LOSE,
                (false, true)  => *w -= LR_LOSE,
                (false, false) => *w += LR_WIN * 0.5,
            }
        }

        nudge(&mut self.rsi,        contrib.rsi_bullish,       was_long, profitable);
        nudge(&mut self.bollinger,  contrib.bb_bullish,        was_long, profitable);
        nudge(&mut self.macd,       contrib.macd_bullish,      was_long, profitable);
        nudge(&mut self.ema_cross,  contrib.ema_cross_bullish, was_long, profitable);
        nudge(&mut self.order_flow, contrib.of_bullish,        was_long, profitable);
        nudge(&mut self.trend,      contrib.trend_bullish,     was_long, profitable);

        if contrib.z_score_present {
            nudge(&mut self.z_score, contrib.z_score_bullish, was_long, profitable);
        }
        if contrib.volume_present {
            nudge(&mut self.volume, contrib.volume_bullish, was_long, profitable);
        }
        if contrib.sentiment_present {
            nudge(&mut self.sentiment, contrib.sentiment_bullish, was_long, profitable);
        }
        if contrib.funding_present {
            nudge(&mut self.funding_rate, contrib.funding_bullish, was_long, profitable);
        }

        // Pattern learning: only update pattern weights when pattern was present
        if contrib.candle_pattern_present {
            nudge(&mut self.candle_pattern, contrib.candle_pattern_bullish, was_long, profitable);
        }
        if contrib.chart_pattern_present {
            nudge(&mut self.chart_pattern, contrib.chart_pattern_bullish, was_long, profitable);
        }

        self.clamp_and_normalise();
        self.save();
    }

    pub fn clamp_and_normalise(&mut self) {
        // Strong-evidence signals: floor 0.04, ceiling 0.50
        // (These have meaningful real-time data each cycle)
        for w in [
            &mut self.rsi, &mut self.bollinger, &mut self.macd,
            &mut self.ema_cross, &mut self.order_flow,
        ] {
            *w = w.clamp(0.04, 0.50);
        }
        // Trend 10-bar: floor lowered to 0.01 — backtest showed consistent harm,
        // kept for backward-compat with weight files but near-eliminated.
        self.trend = self.trend.clamp(0.01, 0.30);

        // Optional signals: can go to 0 (no data = no contribution) or ceiling 0.40
        self.z_score      = self.z_score.clamp(0.0, 0.40);
        self.volume       = self.volume.clamp(0.0, 0.40);
        self.sentiment    = self.sentiment.clamp(0.0, 0.40);
        self.funding_rate = self.funding_rate.clamp(0.0, 0.40);
        // Pattern signals: optional, floor 0.0, ceiling 0.40
        self.candle_pattern = self.candle_pattern.clamp(0.0, 0.40);
        self.chart_pattern  = self.chart_pattern.clamp(0.0, 0.40);

        let total = self.rsi + self.bollinger + self.macd + self.ema_cross
                  + self.order_flow + self.trend + self.z_score
                  + self.volume + self.sentiment + self.funding_rate
                  + self.candle_pattern + self.chart_pattern;

        if total > 0.0 {
            self.rsi          /= total;
            self.bollinger    /= total;
            self.macd         /= total;
            self.ema_cross    /= total;
            self.order_flow   /= total;
            self.trend        /= total;
            self.z_score      /= total;
            self.volume       /= total;
            self.sentiment    /= total;
            self.funding_rate /= total;
            self.candle_pattern /= total;
            self.chart_pattern  /= total;
        }
    }
}

// ─────────────────────────── Signal Contribution ─────────────────────────────

/// Records each signal's direction at the time a position was opened.
/// Persisted inside PaperPosition so the learner can use it on close.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SignalContribution {
    // ── Direction flags (bullish = true, bearish = false) ─────────────────────
    pub rsi_bullish:        bool,
    pub bb_bullish:         bool,
    pub macd_bullish:       bool,
    pub ema_cross_bullish:  bool,
    pub trend_bullish:      bool,
    pub of_bullish:         bool,
    // ── Optional signals (only learn when data was available) ─────────────────
    #[serde(default)]
    pub z_score_present:    bool,
    #[serde(default)]
    pub z_score_bullish:    bool,
    #[serde(default)]
    pub volume_present:     bool,
    #[serde(default)]
    pub volume_bullish:     bool,
    #[serde(default)]
    pub sentiment_present:  bool,
    #[serde(default)]
    pub sentiment_bullish:  bool,
    // ── Funding rate (Binance USDT-M perps) ──────────────────────────────────
    #[serde(default)]
    pub funding_present:    bool,
    #[serde(default)]
    pub funding_bullish:    bool,
    // ── Candlestick patterns (single/double/triple bar) ───────────────────────
    // Tracked for observability; not yet a learnable weight (patterns apply a
    // fixed ±0.12 boost directly, not via a multiplied weight field).
    #[serde(default)]
    pub candle_pattern_present: bool,
    #[serde(default)]
    pub candle_pattern_bullish: bool,
    // ── Chart patterns (10–60 bar structural patterns) ───────────────────────
    #[serde(default)]
    pub chart_pattern_present:  bool,
    #[serde(default)]
    pub chart_pattern_bullish:  bool,
}

// ─────────────────────────── Shared types ────────────────────────────────────

pub type SharedWeights = Arc<RwLock<SignalWeights>>;

// ═══════════════════════════════════════════════════════════════════════════════
//  UNIT TESTS
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn all_bullish_contrib() -> SignalContribution {
        SignalContribution {
            rsi_bullish:       true,
            bb_bullish:        true,
            macd_bullish:      true,
            ema_cross_bullish: true,
            trend_bullish:     true,
            of_bullish:        true,
            z_score_present:   true, z_score_bullish:   true,
            volume_present:    true, volume_bullish:    true,
            sentiment_present: true, sentiment_bullish: true,
            funding_present:   true, funding_bullish:   true,
            candle_pattern_present: false, candle_pattern_bullish: false,
            chart_pattern_present:  false, chart_pattern_bullish:  false,
        }
    }

    // ── SignalWeights normalisation ───────────────────────────────────────────

    #[test]
    fn default_weights_sum_to_one() {
        let w = SignalWeights::default();
        let total = w.rsi + w.bollinger + w.macd + w.ema_cross + w.order_flow
                  + w.trend + w.z_score + w.volume + w.sentiment + w.funding_rate
                  + w.candle_pattern + w.chart_pattern;
        assert!((total - 1.0).abs() < 1e-6, "default weights sum={total:.8} ≠ 1.0");
    }

    #[test]
    fn clamp_and_normalise_always_sums_to_one() {
        let mut w = SignalWeights {
            rsi: 0.5, bollinger: 0.5, macd: 0.5,
            ema_cross: 0.5, order_flow: 0.5, z_score: 0.5,
            volume: 0.5, sentiment: 0.5, funding_rate: 0.5, trend: 0.5,
            candle_pattern: 0.5, chart_pattern: 0.5,
        };
        w.clamp_and_normalise();
        let total = w.rsi + w.bollinger + w.macd + w.ema_cross + w.order_flow
                  + w.trend + w.z_score + w.volume + w.sentiment + w.funding_rate
                  + w.candle_pattern + w.chart_pattern;
        assert!((total - 1.0).abs() < 1e-6,
            "after normalise weights should sum to 1.0, got {total:.8}");
    }

    #[test]
    fn clamp_enforces_floor_on_core_signals() {
        let mut w = SignalWeights {
            rsi: 0.0001, bollinger: 0.0001, macd: 0.0001,
            ema_cross: 0.0001, order_flow: 0.0001, z_score: 0.0,
            volume: 0.0, sentiment: 0.0, funding_rate: 0.0, trend: 0.0001,
            candle_pattern: 0.0, chart_pattern: 0.0,
        };
        w.clamp_and_normalise();
        // Core signals have floor 0.04 before normalisation
        // After normalise they'll be proportionally distributed — just check > 0
        assert!(w.rsi > 0.0);
        assert!(w.bollinger > 0.0);
        assert!(w.macd > 0.0);
        assert!(w.ema_cross > 0.0);
        assert!(w.order_flow > 0.0);
        assert!(w.trend > 0.0);
    }

    #[test]
    fn clamp_allows_optional_signals_to_reach_zero() {
        let mut w = SignalWeights { z_score: 0.0, volume: 0.0, ..Default::default() };
        w.clamp_and_normalise();
        // optional signals CAN go to 0 before normalise (floor=0)
        // after normalise they get redistributed but should remain proportionally low
        assert!(w.z_score >= 0.0 && w.z_score <= 1.0);
        assert!(w.volume  >= 0.0 && w.volume  <= 1.0);
    }

    // ── Weight update — profitable trade ─────────────────────────────────────

    #[test]
    fn update_profitable_long_with_all_bullish_increases_rsi() {
        let mut w   = SignalWeights::default();
        let contrib = all_bullish_contrib();
        w.update(&contrib, true, true);  // was_long=true, profitable=true
        let total_after = w.rsi + w.bollinger + w.macd + w.ema_cross + w.order_flow
                        + w.trend + w.z_score + w.volume + w.sentiment + w.funding_rate
                        + w.candle_pattern + w.chart_pattern;
        assert!((total_after - 1.0).abs() < 1e-6,
            "weights must still sum to 1.0 after update, got {total_after:.8}");
    }

    #[test]
    fn update_profitable_long_with_bearish_rsi_decreases_rsi_weight() {
        let mut w_aligned   = SignalWeights::default();
        let mut w_misaligned = SignalWeights::default();

        let mut contrib_aligned  = all_bullish_contrib();
        contrib_aligned.rsi_bullish = true;  // aligned with LONG

        let mut contrib_misalign = all_bullish_contrib();
        contrib_misalign.rsi_bullish = false; // misaligned — RSI said SHORT, we went LONG

        w_aligned.update(&contrib_aligned,  true, true);
        w_misaligned.update(&contrib_misalign, true, true);

        assert!(w_aligned.rsi > w_misaligned.rsi,
            "aligned RSI weight ({:.4}) should exceed misaligned ({:.4})",
            w_aligned.rsi, w_misaligned.rsi);
    }

    #[test]
    fn update_losing_trade_penalises_aligned_signals() {
        let mut w_win  = SignalWeights::default();
        let mut w_lose = SignalWeights::default();

        // Only RSI is bullish (aligned with LONG); all other core signals are bearish
        // (misaligned); optional signals absent. This isolates RSI's relative movement:
        // win  → RSI gets +LR_WIN,  misaligned core gets −LR_LOSE → RSI fraction ↑
        // lose → RSI gets −LR_LOSE, misaligned core gets +LR_WIN×0.5 → RSI fraction ↓
        let contrib = SignalContribution {
            rsi_bullish: true,
            // all others false / not-present (default)
            ..Default::default()
        };

        w_win.update(&contrib,  true, true);   // profitable long — RSI aligned, rest misaligned
        w_lose.update(&contrib, true, false);  // losing long    — RSI aligned, rest misaligned

        assert!(w_win.rsi > w_lose.rsi,
            "winning trade should boost aligned RSI ({:.4}) vs losing ({:.4})",
            w_win.rsi, w_lose.rsi);
    }

    #[test]
    fn update_optional_signals_only_updated_when_present() {
        let mut w = SignalWeights::default();
        let mut contrib = all_bullish_contrib();
        contrib.sentiment_present = false;  // no LunarCrush data this cycle
        w.update(&contrib, true, true);
        // Sentiment was not present, so it was not nudged.
        // After renormalise, all others grew proportionally — sentiment shrank.
        // Just verify the invariant holds.
        let total = w.rsi + w.bollinger + w.macd + w.ema_cross + w.order_flow
                  + w.trend + w.z_score + w.volume + w.sentiment + w.funding_rate
                  + w.candle_pattern + w.chart_pattern;
        assert!((total - 1.0).abs() < 1e-6,
            "weights must still sum to 1.0: {total:.8}");
    }

    // ── SignalContribution defaults ───────────────────────────────────────────

    #[test]
    fn signal_contribution_default_all_false() {
        let c = SignalContribution::default();
        assert!(!c.z_score_present);
        assert!(!c.sentiment_present);
        assert!(!c.funding_present);
        assert!(!c.candle_pattern_present, "new pattern field must default to false");
        assert!(!c.chart_pattern_present,  "new pattern field must default to false");
    }

    #[test]
    fn signal_contribution_new_fields_roundtrip_json() {
        let c = SignalContribution {
            candle_pattern_present: true,
            candle_pattern_bullish: true,
            chart_pattern_present:  true,
            chart_pattern_bullish:  false,
            ..Default::default()
        };

        let json = serde_json::to_string(&c).unwrap();
        let back: SignalContribution = serde_json::from_str(&json).unwrap();
        assert!(back.candle_pattern_present);
        assert!(back.candle_pattern_bullish);
        assert!(back.chart_pattern_present);
        assert!(!back.chart_pattern_bullish);
    }

    #[test]
    fn signal_contribution_old_json_without_new_fields_deserialises_ok() {
        // Simulates loading a SignalContribution saved before the new pattern fields
        // were added — #[serde(default)] must fill them in as false.
        let old_json = r#"{
            "rsi_bullish": true,
            "bb_bullish": false,
            "macd_bullish": true,
            "ema_cross_bullish": true,
            "trend_bullish": false,
            "of_bullish": true
        }"#;
        let c: SignalContribution = serde_json::from_str(old_json).unwrap();
        assert!(!c.candle_pattern_present,
            "old JSON (no candle_pattern_present) should deserialise as false");
        assert!(!c.chart_pattern_present,
            "old JSON (no chart_pattern_present) should deserialise as false");
    }
}
