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
            rsi:          0.17,   // +0.02 — mean-reversion works
            bollinger:    0.14,   // +0.02 — regime-aware, marginal boost
            macd:         0.07,   // -0.05 — harmful at 15m, significantly reduced
            ema_cross:    0.07,   // -0.06 — reduced; gap filter added in decision.rs
            order_flow:   0.14,   // +0.03 — real order-book data outperforms candle proxy
            z_score:      0.17,   // +0.09 — highest T-stat, significantly boosted
            volume:       0.03,   // -0.03 — directional removed; amplifier role kept
            sentiment:    0.10,   // +0.01 — orthogonal signal, slight boost
            funding_rate: 0.08,   // +0.01 — contrarian edge, slight boost
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
            *w = w.max(0.04).min(0.50);
        }
        // Trend 10-bar: floor lowered to 0.01 — backtest showed consistent harm,
        // kept for backward-compat with weight files but near-eliminated.
        self.trend = self.trend.max(0.01).min(0.30);

        // Optional signals: can go to 0 (no data = no contribution) or ceiling 0.40
        self.z_score      = self.z_score.max(0.0).min(0.40);
        self.volume       = self.volume.max(0.0).min(0.40);
        self.sentiment    = self.sentiment.max(0.0).min(0.40);
        self.funding_rate = self.funding_rate.max(0.0).min(0.40);

        let total = self.rsi + self.bollinger + self.macd + self.ema_cross
                  + self.order_flow + self.trend + self.z_score
                  + self.volume + self.sentiment + self.funding_rate;

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
}

// ─────────────────────────── Shared types ────────────────────────────────────

pub type SharedWeights = Arc<RwLock<SignalWeights>>;
