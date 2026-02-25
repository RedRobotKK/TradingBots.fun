//! Online learning module.
//!
//! After each closed trade the bot records which direction each signal was
//! pointing at entry and whether the trade was profitable.  Signal weights
//! are nudged toward accurate signals (and away from inaccurate ones), then
//! re-normalised and persisted to `signal_weights.json` so learning survives
//! restarts.
//!
//! ## Signal catalogue (9 total, sum-to-1 normalised)
//!
//! | Field       | What it measures                                     |
//! |-------------|------------------------------------------------------|
//! | rsi         | Wilder's RSI — mean-reversion extremes               |
//! | bollinger   | Bollinger Band position + squeeze breakout           |
//! | macd        | MACD histogram momentum                              |
//! | ema_cross   | EMA(8/21) cross — institutional trend signal        |
//! | order_flow  | Real-time order-book bid/ask pressure                |
//! | z_score     | Statistical mean-reversion depth                    |
//! | volume      | Volume conviction multiplier                         |
//! | sentiment   | LunarCrush social sentiment                         |
//! | trend       | Legacy 10-bar % change (kept for file compatibility) |

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
    pub sentiment:  f64,   // 0.10 — social signal
    // ── Legacy ────────────────────────────────────────────────────────────────
    /// 10-bar % change — kept for backwards compatibility with old weight files.
    /// Functionally absorbed into ema_cross; weight held at min floor.
    #[serde(default = "default_trend_weight")]
    pub trend:      f64,   // 0.08 — legacy trend signal
}

fn default_ema_cross_weight() -> f64 { 0.14 }
fn default_z_score_weight()   -> f64 { 0.08 }
fn default_volume_weight()    -> f64 { 0.06 }
fn default_sentiment_weight() -> f64 { 0.10 }
fn default_trend_weight()     -> f64 { 0.08 }

impl Default for SignalWeights {
    fn default() -> Self {
        // Weights sum to 1.0.
        SignalWeights {
            rsi:        0.16,
            bollinger:  0.13,
            macd:       0.13,
            ema_cross:  0.14,
            order_flow: 0.12,
            z_score:    0.08,
            volume:     0.06,
            sentiment:  0.10,
            trend:      0.08,
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
                        "📚 Weights: RSI={:.2} BB={:.2} MACD={:.2} EMA={:.2} OF={:.2} Z={:.2} Vol={:.2} Sent={:.2} Trend={:.2}",
                        w.rsi, w.bollinger, w.macd, w.ema_cross, w.order_flow,
                        w.z_score, w.volume, w.sentiment, w.trend
                    );
                    return w;
                }
                Err(e) => log::warn!("signal_weights.json parse error: {} — using defaults", e),
            }
        }
        log::info!("📚 Using default signal weights (9 signals)");
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

        self.clamp_and_normalise();
        self.save();
    }

    pub fn clamp_and_normalise(&mut self) {
        // Core signals: floor 0.04, ceiling 0.50
        for w in [
            &mut self.rsi, &mut self.bollinger, &mut self.macd,
            &mut self.ema_cross, &mut self.order_flow, &mut self.trend,
        ] {
            *w = w.max(0.04).min(0.50);
        }
        // Optional signals: can go to 0 (no data = no contribution) or ceiling 0.40
        self.z_score   = self.z_score.max(0.0).min(0.40);
        self.volume    = self.volume.max(0.0).min(0.40);
        self.sentiment = self.sentiment.max(0.0).min(0.40);

        let total = self.rsi + self.bollinger + self.macd + self.ema_cross
                  + self.order_flow + self.trend + self.z_score
                  + self.volume + self.sentiment;

        if total > 0.0 {
            self.rsi        /= total;
            self.bollinger  /= total;
            self.macd       /= total;
            self.ema_cross  /= total;
            self.order_flow /= total;
            self.trend      /= total;
            self.z_score    /= total;
            self.volume     /= total;
            self.sentiment  /= total;
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
}

// ─────────────────────────── Shared types ────────────────────────────────────

pub type SharedWeights = Arc<RwLock<SignalWeights>>;
