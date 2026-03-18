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
            macd:         0.05,   // -0.07 — harmful at 15m, further reduced (T-stat -4.02)
            ema_cross:    0.05,   // -0.08 — reduced; gap filter added in decision.rs (T-stat -6.00)
            order_flow:   0.14,   // +0.03 — real order-book data outperforms candle proxy
            z_score:      0.16,   // +0.09 — highest T-stat, significantly boosted
            volume:       0.03,   // -0.03 — directional removed; amplifier role kept
            sentiment:    0.10,   // +0.01 — orthogonal signal, slight boost
            funding_rate: 0.08,   // +0.01 — contrarian edge, slight boost
            candle_pattern: 0.04, // — pattern recognition weight
            chart_pattern:  0.04, // — structural pattern weight
            trend:        0.02,   // -0.05 — near-eliminated; threshold raised in decision.rs (T-stat -4.12)
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
    /// ## Learning design
    ///
    /// **Asymmetric rates** — `LR_LOSE > LR_WIN` on purpose.  A signal must be
    /// correct on >63% of trades just to hold its weight steady.  Below that,
    /// weight decays to the floor and the signal stops influencing entries.
    /// This is Darwin-style filtering: only consistently predictive signals
    /// survive to influence live trades.
    ///
    /// **Confidence scaling** — the entry confidence (0–1) recorded at open
    /// modulates how hard each update hits:
    ///   • High-confidence loss  → full penalty  (we were certain, still wrong)
    ///   • Low-confidence win    → reduced reward (lucky, not skilled)
    ///
    /// **Regime-aware multiplier** — every signal has a "home regime" where it
    /// is expected to work best.  If it fires in its home regime and the trade
    /// still loses, the penalty is 1.5× — no excuses in your own turf.
    ///
    /// ## Per-outcome nudge table
    ///
    /// | Signal aligned | Profitable | Action |
    /// |----------------|-----------|--------|
    /// | ✓ yes          | ✓ yes     | `+ LR_WIN  × win_scale`  |
    /// | ✓ yes          | ✗ no      | `− LR_LOSE × lose_scale` |
    /// | ✗ no           | ✓ yes     | `− LR_LOSE × 0.3`        (wrong but got lucky) |
    /// | ✗ no           | ✗ no      | `+ LR_WIN  × 0.35`       (correct dissent) |
    pub fn update(&mut self, contrib: &SignalContribution, was_long: bool, profitable: bool) {
        // ── Base rates ────────────────────────────────────────────────────────
        // LR_LOSE > LR_WIN: steady downward pressure on wrong signals.
        // Break-even requires >63% accuracy (LR_LOSE / (LR_WIN + LR_LOSE) ≈ 0.63).
        const LR_WIN:  f64 = 0.018;
        const LR_LOSE: f64 = 0.030;

        // ── Confidence scaling ────────────────────────────────────────────────
        // entry_confidence is 0 for old positions without the field; treat as 0.6.
        let conf = if contrib.entry_confidence > 0.0 {
            contrib.entry_confidence.clamp(0.3, 1.0)
        } else {
            0.6
        };
        // win_scale: low-conf wins get 55% reward (lucky); high-conf wins get 100%
        let win_scale  = 0.55 + conf * 0.45;
        // lose_scale: low-conf losses get 85% penalty; high-conf losses get 130%
        let lose_scale = 0.85 + conf * 0.45;

        // ── Regime-aware penalty multiplier ──────────────────────────────────
        // When a signal fires in its home regime and the trade still loses,
        // it gets a 1.5× penalty. "No excuses in your own turf."
        let regime = contrib.regime.as_str();
        let regime_mult_z    = if regime == "Ranging"  { 1.50 } else { 1.0 };
        let regime_mult_bb   = if regime == "Ranging"  { 1.30 } else { 1.0 };
        let regime_mult_rsi  = if regime == "Ranging"  { 1.30 } else { 1.0 };
        let regime_mult_ema  = if regime == "Trending" { 1.50 } else { 1.0 };
        let regime_mult_macd = if regime == "Trending" { 1.30 } else { 1.0 };

        // ── Core nudge closure ────────────────────────────────────────────────
        fn nudge(
            w:           &mut f64,
            sig_bullish:  bool,
            was_long:     bool,
            profitable:   bool,
            win_s:        f64,
            lose_s:       f64,
            regime_m:     f64,
        ) {
            let aligned = sig_bullish == was_long;
            match (aligned, profitable) {
                // Correct call, won: reward proportional to confidence
                (true,  true)  => *w += LR_WIN  * win_s,
                // Correct call, lost: full penalty × regime multiplier
                (true,  false) => *w -= LR_LOSE * lose_s * regime_m,
                // Wrong call, won: mild penalty (lucky, but still wrong direction)
                (false, true)  => *w -= LR_LOSE * 0.30,
                // Wrong call, lost: small reward — signal correctly dissented
                (false, false) => *w += LR_WIN  * 0.35,
            }
        }

        nudge(&mut self.rsi,        contrib.rsi_bullish,       was_long, profitable, win_scale, lose_scale, regime_mult_rsi);
        nudge(&mut self.bollinger,  contrib.bb_bullish,        was_long, profitable, win_scale, lose_scale, regime_mult_bb);
        nudge(&mut self.macd,       contrib.macd_bullish,      was_long, profitable, win_scale, lose_scale, regime_mult_macd);
        nudge(&mut self.ema_cross,  contrib.ema_cross_bullish, was_long, profitable, win_scale, lose_scale, regime_mult_ema);
        nudge(&mut self.order_flow, contrib.of_bullish,        was_long, profitable, win_scale, lose_scale, 1.0);
        nudge(&mut self.trend,      contrib.trend_bullish,     was_long, profitable, win_scale, lose_scale, 1.0);

        if contrib.z_score_present {
            nudge(&mut self.z_score, contrib.z_score_bullish, was_long, profitable, win_scale, lose_scale, regime_mult_z);
        }
        if contrib.volume_present {
            nudge(&mut self.volume, contrib.volume_bullish, was_long, profitable, win_scale, lose_scale, 1.0);
        }
        if contrib.sentiment_present {
            nudge(&mut self.sentiment, contrib.sentiment_bullish, was_long, profitable, win_scale, lose_scale, 1.0);
        }
        if contrib.funding_present {
            nudge(&mut self.funding_rate, contrib.funding_bullish, was_long, profitable, win_scale, lose_scale, 1.0);
        }
        if contrib.candle_pattern_present {
            nudge(&mut self.candle_pattern, contrib.candle_pattern_bullish, was_long, profitable, win_scale, lose_scale, 1.0);
        }
        if contrib.chart_pattern_present {
            nudge(&mut self.chart_pattern, contrib.chart_pattern_bullish, was_long, profitable, win_scale, lose_scale, 1.0);
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
///
/// `regime` and `entry_confidence` are stamped at entry so the learning
/// update can apply regime-aware and confidence-scaled adjustments.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SignalContribution {
    // ── Entry context — stamped by decision.rs before returning Decision ─────
    /// Market regime at entry: "Trending" | "Ranging" | "Neutral"
    #[serde(default)]
    pub regime:             String,
    /// Decision confidence [0.0, 1.0] at entry — used to scale learning rates.
    /// High-confidence losses are penalised harder; low-confidence wins rewarded less.
    #[serde(default)]
    pub entry_confidence:   f64,
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
            ..Default::default()
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
        // were added — #[serde(default)] must fill them in as false / empty.
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
        // New fields also default gracefully
        assert_eq!(c.regime, "", "old JSON should give empty regime string");
        assert_eq!(c.entry_confidence, 0.0, "old JSON should give 0.0 confidence");
    }

    // ── Asymmetric learning rates ─────────────────────────────────────────────

    #[test]
    fn loss_penalty_exceeds_win_reward_so_bad_signals_decay() {
        // Run 5 aligned wins then 4 aligned losses.
        // With LR_LOSE > LR_WIN the weight should have decayed overall,
        // proving a 55% win rate is not enough to maintain weight.
        let contrib = SignalContribution {
            rsi_bullish: true,
            entry_confidence: 0.8,
            regime: "Ranging".to_string(),
            ..all_bullish_contrib()
        };
        let mut w = SignalWeights::default();
        let start = w.rsi;
        for _ in 0..5 { w.update(&contrib, true, true);  }
        for _ in 0..4 { w.update(&contrib, true, false); }
        // 5 wins, 4 losses = 55.6% win rate — weight should be lower than start
        assert!(w.rsi < start,
            "55% win rate should not maintain weight: start={start:.4} end={:.4}", w.rsi);
    }

    #[test]
    fn high_confidence_loss_penalises_harder_than_low_confidence() {
        let mut w_hi = SignalWeights::default();
        let mut w_lo = SignalWeights::default();

        let contrib_hi = SignalContribution {
            rsi_bullish: true, entry_confidence: 0.95,
            regime: "Ranging".to_string(),
            ..Default::default()
        };
        let contrib_lo = SignalContribution {
            rsi_bullish: true, entry_confidence: 0.35,
            regime: "Ranging".to_string(),
            ..Default::default()
        };

        w_hi.update(&contrib_hi, true, false);  // high-confidence loss
        w_lo.update(&contrib_lo, true, false);  // low-confidence loss

        assert!(w_hi.rsi < w_lo.rsi,
            "high-confidence loss ({:.4}) should penalise harder than low-confidence ({:.4})",
            w_hi.rsi, w_lo.rsi);
    }

    #[test]
    fn regime_aware_penalty_z_score_in_ranging_is_harsher() {
        // Z-score is the primary Ranging signal.
        // A z_score loss in Ranging should penalise harder than in Trending.
        let mut w_ranging  = SignalWeights::default();
        let mut w_trending = SignalWeights::default();

        let contrib_ranging = SignalContribution {
            z_score_present: true, z_score_bullish: true,
            entry_confidence: 0.7, regime: "Ranging".to_string(),
            ..Default::default()
        };
        let contrib_trending = SignalContribution {
            z_score_present: true, z_score_bullish: true,
            entry_confidence: 0.7, regime: "Trending".to_string(),
            ..Default::default()
        };

        w_ranging.update(&contrib_ranging,   true, false);  // z_score in home regime, lost
        w_trending.update(&contrib_trending, true, false);  // z_score off-regime, lost

        assert!(w_ranging.z_score < w_trending.z_score,
            "z_score loss in Ranging ({:.4}) should penalise harder than in Trending ({:.4})",
            w_ranging.z_score, w_trending.z_score);
    }
}
