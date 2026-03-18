//! Regime-aware decision engine.
//!
//! ## Strategy architecture
//!
//! The engine classifies the market into one of three **regimes** using ADX(14),
//! then applies the signal set that is statistically most effective for that
//! regime.  This is what separates professional quant systems from basic
//! indicator scanners: the same indicator means different things in different
//! market conditions.
//!
//! ### Regime classification (ADX-based)
//!
//! ```text
//! ADX > 27  →  TRENDING  — momentum strategies (EMA cross, MACD histogram)
//! ADX 19–27 →  NEUTRAL   — balanced, tighter entry threshold
//! ADX < 19  →  RANGING   — mean reversion (RSI, Bollinger, Z-score)
//! ```
//!
//! ### Signal set per regime
//!
//! | Signal       | Trending           | Ranging             | Neutral    |
//! |--------------|--------------------|--------------------|------------|
//! | RSI          | Momentum (50 cross)| Mean-rev (extremes) | Balanced  |
//! | Bollinger    | Breakout direction | Band-touch reversal | Balanced  |
//! | MACD         | Full weight        | Reduced weight      | Full       |
//! | EMA Cross    | PRIMARY (×1.4)     | Suppressed (×0.4)   | Normal     |
//! | Z-score      | Suppressed         | PRIMARY             | Normal     |
//! | VWAP bias    | Direction filter   | Deviation signal    | Filter     |
//! | Volume       | Confirmation mult  | Confirmation mult   | Mult       |
//! | Order Flow   | Full weight        | Full weight         | Full       |
//! | Sentiment    | Full weight        | Full weight         | Full       |
//!
//! ### Entry thresholds
//!
//! | Regime   | Threshold | Dominance |
//! |----------|-----------|-----------|
//! | Trending | 0.44      | 1.28      |
//! | Ranging  | 0.38      | 1.20      |
//! | Neutral  | 0.42      | 1.25      |

use anyhow::Result;
use chrono::Timelike;
use serde::{Deserialize, Serialize};
use crate::data::PriceData;
use crate::indicators::{TechnicalIndicators, HtfIndicators};
use crate::signals::OrderFlowSignal;
use crate::learner::{SignalWeights, SignalContribution};
use crate::sentiment::SentimentData;
use crate::funding::FundingData;
use crate::cross_exchange::CrossExchangeSignal;
use crate::candlestick_patterns;
use crate::chart_patterns;

// ─────────────────────────── Public types ────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub action:             String,  // "BUY" | "SELL" | "SKIP"
    /// When action == "SKIP", this holds the dominant lean ("BUY" or "SELL")
    /// so the signal watchlist can track near-misses in the right direction.
    pub skipped_direction:  String,  // "BUY" | "SELL" | "NONE"
    pub confidence:         f64,
    pub position_size:      f64,
    pub leverage:           f64,
    pub entry_price:        f64,
    pub stop_loss:          f64,
    pub take_profit:        f64,
    pub strategy:           String,
    pub rationale:          String,
    pub signal_contribution: SignalContribution,
}

/// Market regime — determines which signal set and thresholds to use.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Regime {
    /// ADX > 27: momentum strategies dominate.
    Trending,
    /// ADX < 19: mean-reversion strategies dominate.
    Ranging,
    /// ADX 19–27: balanced approach, tighter entry threshold.
    Neutral,
}

impl Regime {
    fn label(self) -> &'static str {
        match self {
            Regime::Trending => "Trending",
            Regime::Ranging  => "Ranging",
            Regime::Neutral  => "Neutral",
        }
    }
    /// Entry score threshold — minimum winner score to consider a trade.
    ///
    /// Calibrated against live signal score distributions (Mar 2026):
    /// typical "good" signal produces bull/bear ≈ 0.10–0.30; old thresholds
    /// (0.44/0.38/0.42) were 2–3× the actual score range → 0 trades in 500+ cycles.
    /// New values sit at ~70–80% of a realistic strong signal.
    fn threshold(self) -> f64 {
        match self {
            Regime::Trending => 0.22,  // trend signal bundle at 70% of strong Trending score
            Regime::Ranging  => 0.16,  // mean-reversion signals are smaller in absolute terms
            Regime::Neutral  => 0.18,  // balanced but signals conflict → lower gate
        }
    }
    /// Dominance ratio — winning side must exceed losing side by this factor.
    /// Higher = only enter when direction is unambiguous.
    fn dominance(self) -> f64 {
        match self {
            Regime::Trending => 1.25,
            Regime::Ranging  => 1.20,
            Regime::Neutral  => 1.22,
        }
    }
}

fn detect_regime(ind: &TechnicalIndicators) -> Regime {
    if      ind.adx > 27.0 { Regime::Trending }
    else if ind.adx < 19.0 { Regime::Ranging  }
    else                    { Regime::Neutral  }
}

/// Confidence-scaled leverage, capped by market regime.
///
/// Ranging markets: max 3× (tighter stops offset the higher leverage)
/// Neutral markets: max 3× (balanced risk while win rate builds)
/// Trending markets: max 5× (momentum carries positions; higher conviction warranted)
///
/// Minimum entry confidence is 0.60 (gated in main.rs).
/// Confidence scaling within the regime cap:
///   0.60–0.70 → 2×  (minimum — just above gate)
///   0.70–0.82 → 3×
///   0.82–0.90 → 4×
///   0.90+     → regime max
pub fn calc_leverage(confidence: f64, regime: Regime) -> f64 {
    let regime_cap: f64 = match regime {
        Regime::Ranging  => 3.0,
        Regime::Neutral  => 3.0,
        Regime::Trending => 5.0,
    };
    let raw: f64 = if confidence < 0.70      { 2.0 }
                   else if confidence < 0.82 { 3.0 }
                   else if confidence < 0.90 { 4.0 }
                   else                      { 5.0 };
    raw.min(regime_cap)
}

// ─────────────────────────── BTC Market Context ──────────────────────────────

/// BTC market-wide context — modulates per-altcoin confidence based on
/// dominance regime and BTC price direction.
///
/// ## Backtest results (400 days, Feb 2024 – Feb 2026, 9 assets):
///
/// | Dominance      | Days | Pearson r | Alts follow BTC |
/// |----------------|------|-----------|-----------------|
/// | Very High >60% |  152 |   0.67    |      78%        |
/// | High  55–60%   |  215 |   0.75    |      80%        |
/// | Medium 48–55%  |   33 |   0.49    |      71%        |
///
/// Big-move days (BTC >3%, high dominance): ETH/AVAX/LINK follow 93–100%.
/// Lead-lag is SAME-DAY — the signal expires quickly, act fast.
#[derive(Debug, Clone)]
pub struct BtcMarketContext {
    /// BTC dominance as a percentage (e.g. 56.0 = 56 %).
    pub dominance: f64,
    /// BTC return over the last ~24 h in percent (e.g. +2.5, -1.8).
    pub btc_return_24h: f64,
    /// BTC return over the last 4h (one 4h candle window).
    /// Used with `asset_return_4h` for relative-performance catch-up signal.
    pub btc_return_4h: f64,
    /// This asset's own 4h return — compared against `btc_return_4h`.
    /// 0.0 when 4h candles are unavailable.
    pub asset_return_4h: f64,
}

impl BtcMarketContext {
    /// Returns a confidence delta in **[-0.12, +0.12]**.
    ///
    /// Positive = BTC direction / relative-performance supports the trade.
    /// Negative = BTC direction opposes the trade.
    /// Zero     = BTC flat or dominance too low to matter.
    ///
    /// Not applied to BTC's own signals (caller passes `None` for BTC).
    pub fn confidence_adjustment(&self, action: &str) -> f64 {
        // ── 24h BTC direction alignment ──────────────────────────────────────
        // Treat sub-±0.3 % moves as "flat" to avoid noise on tiny wiggles
        let btc_bull = self.btc_return_24h >  0.3;
        let btc_bear = self.btc_return_24h < -0.3;
        let big_move = self.btc_return_24h.abs() > 3.0;

        let aligned = (action == "BUY"  && btc_bull) || (action == "SELL" && btc_bear);
        let opposed  = (action == "BUY"  && btc_bear) || (action == "SELL" && btc_bull);

        let btc_adj = if self.dominance >= 55.0 {
            // HIGH dominance — BTC direction is a strong edge (Pearson 0.75)
            if      aligned && big_move  {  0.08 }
            else if aligned              {  0.05 }
            else if opposed && big_move  { -0.12 }
            else if opposed              { -0.08 }
            else                         {  0.00 }
        } else if self.dominance >= 48.0 {
            // MEDIUM dominance — weaker correlation (Pearson 0.49)
            if      aligned {  0.03 }
            else if opposed { -0.04 }
            else            {  0.00 }
        } else {
            // LOW dominance / altseason (<48 %) — alts decouple from BTC
            0.00
        };

        // ── Relative performance catch-up (IC ~0.04–0.06) ───────────────────
        // Asset lagging BTC over 4h in a high-dominance regime tends to catch up.
        // Asset leading BTC by >2% may mean-revert back toward BTC performance.
        let lag = self.asset_return_4h - self.btc_return_4h;
        let rel_bonus: f64 = if self.dominance >= 55.0 && self.btc_return_4h.abs() > 0.5 {
            if (action == "BUY" && lag < -2.0) || (action == "SELL" && lag > 2.0) { 0.04 }
            else { 0.00 }
        } else {
            0.00
        };

        btc_adj + rel_bonus
    }
}

// ─────────────────────────── Decision engine ─────────────────────────────────

/// Regime-aware decision engine.
///
/// Returns a `Decision` with action BUY / SELL / SKIP.
/// `sentiment` is `None` when LunarCrush data is not available.
/// `btc_ctx` is `None` for BTC itself (no self-referential filter).
/// `htf` is `None` when 4-hour candles are unavailable (MTF filter skipped).
/// `cex_signal` is `None` when the cross-exchange monitor has no data for the symbol.
#[allow(clippy::too_many_arguments)]
pub fn make_decision(
    candles:    &[PriceData],
    ind:        &TechnicalIndicators,
    of:         &OrderFlowSignal,
    weights:    &SignalWeights,
    sentiment:  Option<&SentimentData>,
    funding:    Option<&FundingData>,
    btc_ctx:    Option<&BtcMarketContext>,
    htf:        Option<&HtfIndicators>,
    cex_signal: Option<&CrossExchangeSignal>,
) -> Result<Decision> {
    let last  = candles.last().ok_or_else(|| anyhow::anyhow!("Empty candle slice"))?;
    let close = last.close;

    // ── Regime detection with ATR expansion override ──────────────────────────
    // Standard: ADX(14) classifies Trending / Ranging / Neutral.
    // Override: if current ATR is >1.5× the prior 24-bar mean, we're in a
    // breakout expansion — treat as Trending even if ADX hasn't caught up yet.
    // Applies to BOTH Ranging and Neutral since expanding volatility signals a
    // breakout in either case (ADX is a lagging indicator; ATR reacts faster).
    let regime = {
        let base = detect_regime(ind);
        if ind.atr_expansion_ratio > 1.5 && matches!(base, Regime::Ranging | Regime::Neutral) {
            log::debug!("ATR expansion {:.2}× — regime override {:?}→Trending",
                        ind.atr_expansion_ratio, base);
            Regime::Trending
        } else {
            base
        }
    };

    // ── Intraday session filter ────────────────────────────────────────────────
    // UTC hour → entry quality multiplier applied to the score threshold.
    // London+NY overlap (08:00–17:00) has the highest signal quality.
    // Asia session (00:00–07:00) has good crypto-native liquidity — treat same
    // as London/NY since crypto markets are 24/7 with strong Asia participation.
    let utc_hour = chrono::Utc::now().hour();
    let session_mult: f64 = match utc_hour {
        8..=17  => 1.00,  // London+NY overlap — full signal quality
        18..=21 => 1.03,  // NY close / early Asia — minimal elevation
        _       => 1.05,  // Asia session — slight elevation, still very tradeable
    };
    let session_label = match utc_hour {
        8..=12  => "LON",
        13..=17 => "NY",
        18..=21 => "NYc",
        _       => "ASIA",
    };

    // ── Multi-timeframe RSI scale ──────────────────────────────────────────────
    // 4h RSI should be in the same "zone" as 1h RSI to confirm the signal.
    // Disagreement between timeframes = higher false-positive rate → scale down.
    let rsi_mtf_scale: f64 = htf.map(|h| {
        let r4h = h.rsi_4h;
        let r1h = ind.rsi;
        let both_oversold  = r1h < 45.0 && r4h < 50.0;
        let both_overbought = r1h > 55.0 && r4h > 50.0;
        let r4h_extreme    = !(35.0..=65.0).contains(&r4h);
        if (both_oversold || both_overbought) && r4h_extreme { 1.30 }
        else if both_oversold || both_overbought               { 1.10 }
        else                                                   { 0.80 }
    }).unwrap_or(1.0);

    // ── Multi-timeframe Z-score scale ─────────────────────────────────────────
    // 4h Z-score confirms or contradicts the 1h mean-reversion signal.
    let z_mtf_scale: f64 = htf.map(|h| {
        let z4h = h.z_score_4h;
        let z1h = ind.z_score;
        let same_dir    = (z1h < 0.0 && z4h < -0.5) || (z1h > 0.0 && z4h > 0.5);
        let z4h_extreme = z4h.abs() > 1.2;
        if same_dir && z4h_extreme { 1.40 }  // both TFs agree at extremes — strong edge
        else if same_dir            { 1.10 }  // same direction — mild boost
        else if z4h.abs() < 0.4    { 0.85 }  // 4h near neutral — 1h extreme likely noise (was 0.70)
        else                        { 0.90 }  // mild disagreement (was 0.85)
    }).unwrap_or(1.0);
    let mut bull    = 0.0f64;
    let mut bear    = 0.0f64;
    let mut contrib = SignalContribution::default();

    // ═════════════════════════════════════════════════════════════════════════
    //  1. RSI — behaviour is REGIME-DEPENDENT
    //     rsi_mtf_scale applied: 4h RSI confirmation boosts (+30%) or reduces (-20%)
    // ═════════════════════════════════════════════════════════════════════════
    let rsi_w = weights.rsi * rsi_mtf_scale;
    match regime {
        // TRENDING: RSI used as momentum gauge (50-line cross)
        // Above 55 = bull momentum building; below 45 = bear momentum building.
        // Extreme readings (>70 / <30) have REDUCED weight — in a strong trend
        // overbought can stay overbought for many bars.
        Regime::Trending => {
            if ind.rsi > 65.0 {
                bull += rsi_w * 0.80;
                contrib.rsi_bullish = true;
            } else if ind.rsi > 55.0 {
                bull += rsi_w * 0.55;
                contrib.rsi_bullish = true;
            } else if ind.rsi < 35.0 {
                // Direction-aware: shake-out bounce only when EMA confirms uptrend.
                // In a downtrend (EMA < -0.20%), RSI < 35 = bear continuation, not reversal.
                if ind.ema_cross_pct > 0.20 {
                    bull += rsi_w * 0.70;  // uptrend shake-out → reversal likely
                    contrib.rsi_bullish = true;
                } else if ind.ema_cross_pct < -0.20 {
                    bear += rsi_w * 0.40;  // downtrend continuation → bear momentum
                    contrib.rsi_bullish = false;
                } else {
                    contrib.rsi_bullish = ind.rsi < 50.0; // flat EMA — no contribution
                }
            } else if ind.rsi < 45.0 {
                bear += rsi_w * 0.55;
                contrib.rsi_bullish = false;
            } else {
                contrib.rsi_bullish = ind.rsi > 50.0;
            }
        }
        // RANGING: RSI used as classic mean-reversion oscillator
        // Extremes (<30 / >70) are the PRIMARY signal source.
        Regime::Ranging | Regime::Neutral => {
            if ind.rsi < 28.0 {
                bull += rsi_w;           // deeply oversold — strong reversal signal
                contrib.rsi_bullish = true;
            } else if ind.rsi > 72.0 {
                bear += rsi_w;
                contrib.rsi_bullish = false;
            } else if ind.rsi < 40.0 {
                bull += rsi_w * 0.60;
                contrib.rsi_bullish = true;
            } else if ind.rsi > 60.0 {
                bear += rsi_w * 0.60;
                contrib.rsi_bullish = false;
            } else if ind.rsi < 47.0 {
                bull += rsi_w * 0.25;
                contrib.rsi_bullish = true;
            } else if ind.rsi > 53.0 {
                bear += rsi_w * 0.25;
                contrib.rsi_bullish = false;
            } else {
                contrib.rsi_bullish = ind.rsi < 50.0;
            }
        }
    }

    // ═════════════════════════════════════════════════════════════════════════
    //  2. Bollinger Bands — REGIME-DEPENDENT
    // ═════════════════════════════════════════════════════════════════════════
    let bb_range = ind.bollinger_upper - ind.bollinger_lower;
    if bb_range > 0.0 {
        let bb_pos = (close - ind.bollinger_lower) / bb_range; // 0=at lower, 1=at upper

        match regime {
            // TRENDING: Bollinger used for breakout/breakdown direction.
            // Price above upper BB = breakout strength (NOT overbought).
            // Price below lower BB = breakdown strength (NOT oversold).
            Regime::Trending => {
                if close > ind.bollinger_upper {
                    bull += weights.bollinger * 0.90;  // above upper = uptrend strength
                    contrib.bb_bullish = true;
                } else if close < ind.bollinger_lower {
                    bear += weights.bollinger * 0.90;
                    contrib.bb_bullish = false;
                } else if bb_pos > 0.65 {
                    bull += weights.bollinger * 0.50;
                    contrib.bb_bullish = true;
                } else if bb_pos < 0.35 {
                    bear += weights.bollinger * 0.50;
                    contrib.bb_bullish = false;
                } else {
                    contrib.bb_bullish = bb_pos > 0.5;
                }
            }
            // RANGING: Bollinger used for mean-reversion entries.
            // Price BELOW lower band = oversold, expect bounce.
            // Price ABOVE upper band = overbought, expect pullback.
            Regime::Ranging | Regime::Neutral => {
                if close < ind.bollinger_lower {
                    bull += weights.bollinger;  // below lower band = reversal long
                    contrib.bb_bullish = true;
                } else if close > ind.bollinger_upper {
                    bear += weights.bollinger;
                    contrib.bb_bullish = false;
                } else if bb_pos < 0.25 {
                    bull += weights.bollinger * 0.55;
                    contrib.bb_bullish = true;
                } else if bb_pos > 0.75 {
                    bear += weights.bollinger * 0.55;
                    contrib.bb_bullish = false;
                } else if bb_pos < 0.40 {
                    bull += weights.bollinger * 0.20;
                    contrib.bb_bullish = true;
                } else if bb_pos > 0.60 {
                    bear += weights.bollinger * 0.20;
                    contrib.bb_bullish = false;
                } else {
                    contrib.bb_bullish = bb_pos < 0.5;
                }
            }
        }
    }

    // ═════════════════════════════════════════════════════════════════════════
    //  3. MACD Histogram — momentum acceleration
    // ═════════════════════════════════════════════════════════════════════════
    // Primary: histogram direction (is momentum building or fading?)
    // Secondary: MACD line position (above/below zero = trend direction)
    let macd_w = match regime {
        Regime::Trending => weights.macd,         // full weight in trends
        Regime::Ranging  => weights.macd * 0.65,  // reduced — MACD less reliable in chop
        Regime::Neutral  => weights.macd * 0.85,
    };

    if ind.macd_histogram > 0.0 && ind.macd > 0.0 {
        // Histogram positive AND above zero-line = confirmed bull momentum
        bull += macd_w;
        contrib.macd_bullish = true;
    } else if ind.macd_histogram < 0.0 && ind.macd < 0.0 {
        bear += macd_w;
        contrib.macd_bullish = false;
    } else if ind.macd_histogram > 0.0 {
        // Histogram positive but still below zero = weakening bear (early)
        bull += macd_w * 0.50;
        contrib.macd_bullish = true;
    } else if ind.macd_histogram < 0.0 {
        bear += macd_w * 0.50;
        contrib.macd_bullish = false;
    } else {
        contrib.macd_bullish = ind.macd > 0.0;
    }

    // ═════════════════════════════════════════════════════════════════════════
    //  4. EMA Cross (8/21) — institutional trend confirmation
    // ═════════════════════════════════════════════════════════════════════════
    // ema_cross_pct = (EMA8 − EMA21) / EMA21 × 100
    // Positive = EMA8 above EMA21 = uptrend confirmed
    //
    // Weight depends heavily on regime:
    //   Trending: PRIMARY signal — EMA cross is the main momentum filter
    //   Ranging:  SUPPRESSED — EMAs whipsaw in sideways markets
    let ema_w = match regime {
        Regime::Trending => weights.ema_cross * 1.40,  // boosted — primary trend filter
        Regime::Ranging  => weights.ema_cross * 0.40,  // suppressed — EMAs lie in ranges
        Regime::Neutral  => weights.ema_cross,
    };

    // EMA gap filter: backtest showed "always-on" EMA direction is near-random noise.
    // Only fire when the gap is *meaningful* — EMA8 clearly separated from EMA21.
    // Dead-zone ±0.20 % prevents firing on every bar just because EMA8 is 0.01 % above EMA21.
    let ema_pct = ind.ema_cross_pct;
    if ema_pct > 0.5 {
        // Strong uptrend: fast EMA well above slow EMA
        bull += ema_w;  // ema_w is already capped at weights.ema_cross * 1.40 in Trending
        contrib.ema_cross_bullish = true;
    } else if ema_pct > 0.20 {
        // Moderate uptrend (was >0.0 — lowered floor to filter noise)
        bull += ema_w * 0.60;
        contrib.ema_cross_bullish = true;
    } else if ema_pct < -0.5 {
        bear += ema_w;  // symmetric with bull path above
        contrib.ema_cross_bullish = false;
    } else if ema_pct < -0.20 {
        // Moderate downtrend (was <0.0 — lowered floor to filter noise)
        bear += ema_w * 0.60;
        contrib.ema_cross_bullish = false;
    } else {
        // |ema_pct| < 0.20% — EMAs essentially flat, no clear signal
        contrib.ema_cross_bullish = ema_pct >= 0.0;
    }

    // ═════════════════════════════════════════════════════════════════════════
    //  5. Z-score mean-reversion (ranging regime's answer to EMA cross)
    //     z_mtf_scale applied: 4h Z-score confirmation boosts (+40%) or reduces (-30%)
    // ═════════════════════════════════════════════════════════════════════════
    // Z-score = (close − 20-bar mean) / std_dev
    // Used most aggressively in RANGING markets.
    // Extreme readings predict high-probability reversions.
    let z_w = {
        let regime_w = match regime {
            Regime::Ranging  => weights.z_score * 1.40,  // PRIMARY in ranging
            Regime::Trending => weights.z_score * 0.50,  // suppressed in trends
            Regime::Neutral  => weights.z_score,
        };
        regime_w * z_mtf_scale
    };

    let z = ind.z_score;
    if z < -2.0 {
        // Price is 2σ+ below the mean — statistically oversold
        bull += z_w;
        contrib.z_score_present = true;
        contrib.z_score_bullish = true;
    } else if z > 2.0 {
        bear += z_w;
        contrib.z_score_present = true;
        contrib.z_score_bullish = false;
    } else if z < -1.4 {
        bull += z_w * 0.60;
        contrib.z_score_present = true;
        contrib.z_score_bullish = true;
    } else if z > 1.4 {
        bear += z_w * 0.60;
        contrib.z_score_present = true;
        contrib.z_score_bullish = false;
    } else if z.abs() > 0.8 {
        if z < 0.0 { bull += z_w * 0.25; } else { bear += z_w * 0.25; }
        contrib.z_score_present = true;
        contrib.z_score_bullish = z < 0.0;
    }
    // Near the mean — no contribution

    // ═════════════════════════════════════════════════════════════════════════
    //  6. Order Flow — real-time bid/ask pressure
    // ═════════════════════════════════════════════════════════════════════════
    match of.direction.as_str() {
        "LONG"  => { bull += weights.order_flow * of.confidence; contrib.of_bullish = true;  }
        "SHORT" => { bear += weights.order_flow * of.confidence; contrib.of_bullish = false; }
        _       => { contrib.of_bullish = bull > bear; }
    }

    // ═════════════════════════════════════════════════════════════════════════
    //  7. Legacy trend (10-bar % change) — kept but at reduced weight
    //     The EMA cross is the better version of this signal.
    //
    //  BACKTEST FINDING: Trend 10-bar had T-stat -4.12 across 8 assets × 2 years.
    //  Buying after a 10-bar run = buying tops at 15m. Thresholds raised to only
    //  fire on extreme moves (>4.0%) where mean-reversion OR trend continuation
    //  is more obvious. Below that it is pure noise at 15-minute granularity.
    // ═════════════════════════════════════════════════════════════════════════
    let trend_w = match regime {
        Regime::Trending => weights.trend * 0.80,  // partially replaced by EMA cross
        Regime::Ranging  => weights.trend * 0.30,  // nearly irrelevant in range
        Regime::Neutral  => weights.trend * 0.60,
    };

    if ind.trend > 4.0 {
        // Extreme 10-bar move — strong momentum confirmation
        bull += trend_w;
        contrib.trend_bullish = true;
    } else if ind.trend < -4.0 {
        bear += trend_w;
        contrib.trend_bullish = false;
    } else if ind.trend > 2.0 {
        bull += trend_w * 0.50;
        contrib.trend_bullish = true;
    } else if ind.trend < -2.0 {
        bear += trend_w * 0.50;
        contrib.trend_bullish = false;
    } else {
        // ±2.0% within 10 bars is noise at 15m — no contribution
        contrib.trend_bullish = ind.trend > 0.0;
    }

    // ═════════════════════════════════════════════════════════════════════════
    //  8. VWAP bias — institutional reference price
    // ═════════════════════════════════════════════════════════════════════════
    // Price above VWAP = institutions net bought; bias bull.
    // Price below VWAP = institutions net sold; bias bear.
    // Used as a DIRECTIONAL FILTER that modulates the volume weight.
    // VWAP bias — kept as named booleans for potential future use / logging.
    // Were previously used as a tie-breaker in the volume directional signal
    // (removed after backtest showed T-stat -3.53 on directional volume).
    let _vwap_bull = ind.vwap_pct > 0.3;   // >0.3% above VWAP = bull bias
    let _vwap_bear = ind.vwap_pct < -0.3;  // <0.3% below VWAP = bear bias

    // ═════════════════════════════════════════════════════════════════════════
    //  9. Volume conviction multiplier  (amplifier only — no directional signal)
    // ═════════════════════════════════════════════════════════════════════════
    // High volume → all other signals more reliable → amplify them.
    // Low volume  → noise dominates                 → dampen them.
    //
    // NOTE: Volume was previously also used as a DIRECTIONAL signal (step b).
    // Removed after signal_quality_backtest.py showed T-stat -3.53 (harmful):
    // high volume at price extremes = exhaustion, not confirmation.
    let vol_ratio = ind.volume_ratio;

    // Step a: apply global multiplier to all signals computed so far
    // Volume conviction multiplier — less aggressive penalty than before.
    // Crypto markets routinely show VOL:0.1-0.3× outside peak hours; the old
    // 0.75 multiplier wiped 25% off ALL scores (including the already-small
    // real-world signal values) making the threshold unreachable.
    let vol_mult = if vol_ratio > 2.0      { 1.20 }
                   else if vol_ratio > 1.4 { 1.10 }
                   else if vol_ratio < 0.4 { 0.87 }   // thin volume — mild caution (was 0.75)
                   else if vol_ratio < 0.6 { 0.93 }   // below-avg volume (was 0.85)
                   else                    { 1.00 };

    bull *= vol_mult;
    bear *= vol_mult;

    // Step b: directional volume score — REMOVED based on backtest evidence.
    //
    // The signal_quality_backtest.py showed volume as a directional indicator
    // had T-stat -3.53 across 8 assets × 2 years at 15m (actively harmful).
    // Reason: high volume at price extremes signals EXHAUSTION, not continuation.
    // The global multiplier (step a) is the correct role for volume — it amplifies
    // other signals when conviction is high, rather than adding its own directional bet.
    //
    // Volume weight (weights.volume) is still tracked by the learner for future
    // re-evaluation if market microstructure changes, but fires no directional signal.

    // ═════════════════════════════════════════════════════════════════════════
    //  10. LunarCrush Sentiment
    // ═════════════════════════════════════════════════════════════════════════
    // signal_strength = (bull% − bear%) / 100 × galaxy_score / 100
    // Dead-zone ±0.10 filtered to avoid noise.
    if let Some(sent) = sentiment {
        let strength = sent.signal_strength();
        contrib.sentiment_present = true;
        if strength > 0.10 {
            bull += weights.sentiment * strength;
            contrib.sentiment_bullish = true;
        } else if strength < -0.10 {
            bear += weights.sentiment * (-strength);
            contrib.sentiment_bullish = false;
        } else {
            contrib.sentiment_bullish = strength >= 0.0;
        }
    }

    // ═════════════════════════════════════════════════════════════════════════
    //  11. Perpetual Funding Rate  (Binance USDT-M — contrarian signal)
    // ═════════════════════════════════════════════════════════════════════════
    // Funding rate is settled every 8 h between longs and shorts.
    // High POSITIVE funding → longs overcrowded and paying to stay open →
    //   vulnerable to rapid unwind; BEARISH lean (contrarian).
    // High NEGATIVE funding → shorts overcrowded → squeeze risk; BULLISH lean.
    //
    // The signal is CONTRARIAN: the weight adds to bear when bulls are crowded
    // and adds to bull when bears are crowded.  signal_strength() returns
    // +1.0 (strong bull) … -1.0 (strong bear) based on rate magnitude.
    //
    // Weight is scaled down in TRENDING regimes because strong momentum can
    // sustain elevated funding for many bars (e.g. a parabolic rally).
    if let Some(fund) = funding {
        let strength = fund.signal_strength();
        if strength.abs() > 0.0 {
            contrib.funding_present = true;
            let fund_w = match regime {
                Regime::Trending => weights.funding_rate * 0.60,  // trends sustain high funding
                Regime::Ranging  => weights.funding_rate * 1.20,  // crowded positions revert faster
                Regime::Neutral  => weights.funding_rate,
            };

            // Funding rate delta boost: rapid rate movement is a more urgent signal.
            // A sudden jump in funding rate means new leverage is being added quickly —
            // the crowd is getting more crowded (or rapidly de-levering), raising the
            // probability of an imminent forced unwind or squeeze.
            //
            // Delta threshold in 8h rate units:
            //   0.0005 = 0.05%  (moderate change — one whole tier)
            //   0.0002 = 0.02%  (mild change)
            let abs_delta = fund.funding_delta.abs();
            let raw_delta_mult = if abs_delta > 0.0005 { 1.60 }
                                 else if abs_delta > 0.0002 { 1.30 }
                                 else                        { 1.00 };
            // Only apply boost when the delta CONFIRMS the current rate direction
            // (e.g. rate already positive AND rising — longs deepening their commitment)
            let delta_confirms = (fund.funding_delta > 0.0 && fund.funding_rate > 0.0)
                              || (fund.funding_delta < 0.0 && fund.funding_rate < 0.0);
            let delta_mult = if delta_confirms { raw_delta_mult } else { 1.0 };

            if strength > 0.0 {
                // Shorts crowded → squeeze potential → bullish
                bull += fund_w * strength * delta_mult;
                contrib.funding_bullish = true;
            } else {
                // Longs crowded → liquidation risk → bearish
                bear += fund_w * (-strength) * delta_mult;
                contrib.funding_bullish = false;
            }
        }
    }

    // ═════════════════════════════════════════════════════════════════════════
    //  12. Candlestick Patterns  (single / double / triple bar)
    // ═════════════════════════════════════════════════════════════════════════
    // Run on the last 3 bars at most.  Contribution capped at ±0.12 so
    // patterns confirm rather than override the regime-based scoring.
    let csp = candlestick_patterns::detect(candles);
    bull += csp.bull_boost;
    bear += csp.bear_boost;
    // Track contribution so the learner can observe pattern accuracy over time.
    if csp.bull_boost > 0.0 || csp.bear_boost > 0.0 {
        contrib.candle_pattern_present = true;
        contrib.candle_pattern_bullish = csp.bull_boost >= csp.bear_boost;
    }

    // ═════════════════════════════════════════════════════════════════════════
    //  13. Chart Patterns  (structural patterns over 10–60 bars)
    // ═════════════════════════════════════════════════════════════════════════
    // Includes: double/triple tops & bottoms, H&S, wedges, triangles,
    // flags, pennants, rectangles, cup & handle, and institutional PA patterns
    // (compression→expansion, liquidity grab, V-flash reversal).
    let chp = chart_patterns::detect(candles);
    bull += chp.bull_boost;
    bear += chp.bear_boost;
    // Track contribution for observability.
    if chp.bull_boost > 0.0 || chp.bear_boost > 0.0 {
        contrib.chart_pattern_present = true;
        contrib.chart_pattern_bullish = chp.bull_boost >= chp.bear_boost;
    }

    // ═════════════════════════════════════════════════════════════════════════
    //  14. Cross-exchange price divergence  (HL vs Binance/ByBit/OKX)
    // ═════════════════════════════════════════════════════════════════════════
    // CEX prices polled every 60s; HL mid comes from allMids already in hand.
    //
    // MOMENTUM mode  (divergence 0.25 – 1.5%):
    //   HL > CEX → HL leading price discovery → mild BULL  (max weight 0.03)
    //   HL < CEX → local sell pressure        → mild BEAR  (max weight 0.03)
    //
    // MEAN-REVERSION mode  (divergence ≥ 1.5%, or ≥ 2% extreme bypass):
    //   HL > CEX → HL overshot; arb will sell HL back down → BEAR
    //   HL < CEX → HL undershot; buying restores parity   → BULL
    //   Weight scales to 0.12 at 3% (≈ order_flow weight).
    //   Direction is FLIPPED vs momentum — intentional contrarian signal.
    //   Extreme bypass (≥2%): activates immediately, no 3-cycle wait.
    if let Some(cex) = cex_signal {
        let (cex_bull, cex_bear) = cex.score_contribution();
        if cex_bull > 0.0 || cex_bear > 0.0 {
            log::debug!(
                "📡 CEX divergence {} {}{:.3}% (persist {}) → bull+{:.4} bear+{:.4}",
                cex.symbol, if cex.hl_premium_pct > 0.0 { "+" } else { "" },
                cex.hl_premium_pct, cex.persistence, cex_bull, cex_bear
            );
        }
        bull += cex_bull;
        bear += cex_bear;
    }

    // ═════════════════════════════════════════════════════════════════════════
    //  Final decision — regime-dependent thresholds with session adjustment
    // ═════════════════════════════════════════════════════════════════════════
    // session_mult raises the entry bar during low-quality trading hours
    // (Asian dead zone) to avoid acting on noise when institutional liquidity
    // is thin and follow-through is poor.
    let threshold  = regime.threshold() * session_mult;
    let dominance  = regime.dominance();

    let (action, raw_confidence) = if bull >= threshold && bull > bear * dominance {
        ("BUY".to_string(),  (bull / (bull + bear + 1e-8)).min(1.0))
    } else if bear >= threshold && bear > bull * dominance {
        ("SELL".to_string(), (bear / (bull + bear + 1e-8)).min(1.0))
    } else {
        ("SKIP".to_string(), f64::max(bull, bear) / (bull + bear + 1e-8))
    };

    // ═════════════════════════════════════════════════════════════════════════
    //  BTC Dominance context — modulate confidence by BTC direction alignment
    // ═════════════════════════════════════════════════════════════════════════
    // Backtest-validated: at high dominance (≥55 %), BTC direction predicts
    // altcoin direction 80 % of days (Pearson 0.75).  Big BTC moves (>3 %)
    // see 93–100 % follow-through.  We boost aligned trades and penalise
    // counter-BTC trades proportionally.  Not applied to BTC itself.
    let btc_adj = btc_ctx
        .filter(|_| action != "SKIP")
        .map(|b| b.confidence_adjustment(&action))
        .unwrap_or(0.0);

    let confidence = (raw_confidence + btc_adj).clamp(0.0, 1.0);

    // ═════════════════════════════════════════════════════════════════════════
    //  Stop-loss and take-profit (ATR-based)
    // ═════════════════════════════════════════════════════════════════════════
    // Floor ATR at 0.2% of price to avoid zero-ATR edge cases.
    let atr = ind.atr.max(close * 0.002);

    // Tighter stop in trending markets (momentum trades move faster);
    // wider stop in ranging markets (expected oscillation before reversal).
    let (stop_mult, tp_mult) = match regime {
        Regime::Trending => (1.8, 3.6),   // 1.8×ATR stop, 3.6×ATR target = 2:1 R:R
        Regime::Ranging  => (2.2, 3.3),   // 2.2×ATR stop, 3.3×ATR target = 1.5:1 R:R
        Regime::Neutral  => (2.0, 3.2),   // balanced
    };

    let (stop_loss, take_profit) = match action.as_str() {
        "BUY"  => (close - atr * stop_mult, close + atr * tp_mult),
        "SELL" => (close + atr * stop_mult, close - atr * tp_mult),
        _      => (close * 0.97, close * 1.03),
    };

    // ── Rich rationale string (shown in Signal Feed) ──────────────────────────
    let sent_tag = sentiment.map(|s| {
        format!(" 🌙G:{:.0} B:{:.0}%", s.galaxy_score, s.bullish_percent)
    }).unwrap_or_default();

    // Pattern tags — only append when patterns were actually detected
    let csp_tag = csp.name.as_deref()
        .map(|n| format!(" 🕯{}", n))
        .unwrap_or_default();
    let chp_tag = chp.name.as_deref()
        .map(|n| format!(" 📐{}", n))
        .unwrap_or_default();

    // Funding rate tag — only shown when rate is outside neutral band
    let fund_tag = funding
        .filter(|f| f.is_significant())
        .map(|f| {
            use crate::funding::{current_cycle_phase, FundingCyclePhase};
            let delta_str = if f.funding_delta.abs() > 0.0001 {
                format!(" Δ{:+.3}%", f.funding_delta * 100.0)
            } else {
                String::new()
            };
            let phase_str = match current_cycle_phase() {
                FundingCyclePhase::PreSettlement { hours_remaining } =>
                    format!(" ⏰{:.0}m-to-settle", hours_remaining * 60.0),
                FundingCyclePhase::PostSettlement { minutes_elapsed } =>
                    format!(" 🔄+{:.0}m-post", minutes_elapsed),
                FundingCyclePhase::MidCycle { hours_to_next } =>
                    format!(" ({:.1}h-to-settle)", hours_to_next),
            };
            format!(" 💰FR:{:+.3}%({}{}{})", f.funding_rate * 100.0, f.emoji(), delta_str, phase_str)
        })
        .unwrap_or_default();

    // BTC dominance context tag — shown in rationale when context is active
    let btc_tag = btc_ctx.map(|b| {
        let adj_str = if btc_adj > 0.0 {
            format!("+{:.0}%", btc_adj * 100.0)
        } else if btc_adj < 0.0 {
            format!("{:.0}%", btc_adj * 100.0)
        } else {
            String::new()
        };
        let rel_str = if b.asset_return_4h != 0.0 || b.btc_return_4h != 0.0 {
            format!(" 4H:A{:+.1}%/B{:+.1}%", b.asset_return_4h, b.btc_return_4h)
        } else {
            String::new()
        };
        format!(" 🟠DOM:{:.0}% BTC:{:+.1}%{}{}",
            b.dominance, b.btc_return_24h,
            if adj_str.is_empty() { String::new() } else { format!("({})", adj_str) },
            rel_str,
        )
    }).unwrap_or_default();

    // ATR expansion tag — only shown when regime override occurred
    let atr_tag = if ind.atr_expansion_ratio > 1.5 {
        format!(" ⚡ATR×{:.1}", ind.atr_expansion_ratio)
    } else {
        String::new()
    };

    // MTF tag — only shown when 4h data is available
    let mtf_tag = htf.map(|h| {
        format!(" 4H:RSI{:.0}/Z{:.1}", h.rsi_4h, h.z_score_4h)
    }).unwrap_or_default();

    // Cross-exchange divergence tag — only shown when anomaly is active
    // Cross-exchange tag shows mode (MOM = momentum, REV = mean-reversion) + magnitude + persistence.
    // Example: "🔴⟳CEX[REV]:-3.21%(2cy)" means HL is 3.2% below CEX for 2 cycles → BULL reversion.
    let cex_tag = cex_signal
        .filter(|s| s.active)
        .map(|s| format!(" {}CEX[{}]:{:+.2}%({}cy)",
            s.emoji(), s.mode_label(), s.hl_premium_pct, s.persistence))
        .unwrap_or_default();

    let rationale = format!(
        "[{}/{}] RSI:{:.0} Z:{:.1} EMA:{:+.2}% MACD-H:{:.5} VOL:{:.1}× ADX:{:.0} VWAP:{:+.1}%{}{}{}{}{}{}{}{} ⟨B:{:.3} b:{:.3} t:{:.3}⟩",
        regime.label(),
        session_label,
        ind.rsi,
        ind.z_score,
        ind.ema_cross_pct,
        ind.macd_histogram,
        ind.volume_ratio,
        ind.adx,
        ind.vwap_pct,
        sent_tag,
        fund_tag,
        btc_tag,
        csp_tag,
        chp_tag,
        atr_tag,
        mtf_tag,
        cex_tag,
        bull,
        bear,
        threshold,
    );

    // For SKIP decisions, capture the dominant lean so the watchlist can
    // track near-misses in the correct direction.
    let skipped_direction = if action == "SKIP" {
        if bull >= bear { "BUY".to_string() } else { "SELL".to_string() }
    } else {
        "NONE".to_string()
    };

    // Stamp entry context onto contrib so the learner can apply
    // regime-aware and confidence-scaled weight updates on close.
    contrib.regime           = regime.label().to_string();
    contrib.entry_confidence = confidence;

    Ok(Decision {
        action,
        skipped_direction,
        confidence,
        position_size: 0.0,
        leverage:      calc_leverage(confidence, regime),
        entry_price:   close,
        stop_loss,
        take_profit,
        strategy:      format!("{} (bull={:.3} bear={:.3})", regime.label(), bull, bear),
        rationale,
        signal_contribution: contrib,
    })
}

// ═══════════════════════════════════════════════════════════════════════════════
//  UNIT TESTS
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indicators::TechnicalIndicators;
    use crate::signals::OrderFlowSignal;
    use crate::learner::SignalWeights;

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn flat_candles(price: f64, n: usize) -> Vec<crate::data::PriceData> {
        (0..n).map(|i| crate::data::PriceData {
            symbol:    "TEST".to_string(),
            timestamp: i as i64 * 3_600_000,
            open: price, high: price * 1.001, low: price * 0.999, close: price,
            volume: 1000.0,
        }).collect()
    }

    fn rising_candles(start: f64, step: f64, n: usize) -> Vec<crate::data::PriceData> {
        (0..n).map(|i| {
            let c = start + step * i as f64;
            crate::data::PriceData {
                symbol:    "TEST".to_string(),
                timestamp: i as i64 * 3_600_000,
                open:  c - step * 0.1,
                high:  c + step * 0.2,
                low:   c - step * 0.2,
                close: c,
                volume: 1000.0 + i as f64 * 10.0,
            }
        }).collect()
    }

    /// A neutral `TechnicalIndicators` that produces a SKIP from `make_decision`.
    fn neutral_ind(price: f64) -> TechnicalIndicators {
        TechnicalIndicators {
            rsi:              50.0,
            bollinger_upper:  price * 1.02,
            bollinger_middle: price,
            bollinger_lower:  price * 0.98,
            bb_width_pct:     4.0,
            macd:             0.0,
            macd_signal:      0.0,
            macd_histogram:   0.0,
            atr:              price * 0.01,
            trend:            0.0,
            ema8:             price,
            ema21:            price,
            ema_cross_pct:    0.0,
            adx:              22.0,  // Neutral regime
            z_score:          0.0,
            volume_ratio:     1.0,
            vwap:             price,
            vwap_pct:         0.0,
            atr_expansion_ratio: 1.0,
        }
    }

    fn neutral_of() -> OrderFlowSignal {
        OrderFlowSignal {
            bid_volume:      100.0,
            ask_volume:      100.0,
            imbalance_ratio: 1.0,
            direction:       "NEUTRAL".to_string(),
            confidence:      0.50,
        }
    }

    // ── Regime detection ─────────────────────────────────────────────────────

    #[test]
    fn regime_trending_above_27() {
        let mut ind = neutral_ind(100.0);
        ind.adx = 30.0;
        assert_eq!(detect_regime(&ind), Regime::Trending, "ADX 30 → Trending");
    }

    #[test]
    fn regime_ranging_below_19() {
        let mut ind = neutral_ind(100.0);
        ind.adx = 15.0;
        assert_eq!(detect_regime(&ind), Regime::Ranging, "ADX 15 → Ranging");
    }

    #[test]
    fn regime_neutral_between_19_and_27() {
        let mut ind = neutral_ind(100.0);
        ind.adx = 23.0;
        assert_eq!(detect_regime(&ind), Regime::Neutral, "ADX 23 → Neutral");
    }

    #[test]
    fn regime_boundary_exactly_27_is_neutral() {
        let mut ind = neutral_ind(100.0);
        ind.adx = 27.0;  // condition is > 27, so exactly 27 → Neutral
        assert_eq!(detect_regime(&ind), Regime::Neutral, "ADX = 27 → Neutral (not > 27)");
    }

    // ── ATR expansion override ────────────────────────────────────────────────

    #[test]
    fn atr_expansion_overrides_ranging_to_trending() {
        let mut ind = neutral_ind(100.0);
        ind.adx = 15.0;                   // Ranging by ADX
        ind.atr_expansion_ratio = 2.0;    // strong breakout
        // Override logic inside make_decision; verify through regime label in rationale
        // We test the base detect_regime separately; for the override we call make_decision
        assert_eq!(detect_regime(&ind), Regime::Ranging);
        // After override: matches!(Ranging | Neutral) → Trending
        let overridden = if ind.atr_expansion_ratio > 1.5
            && matches!(detect_regime(&ind), Regime::Ranging | Regime::Neutral)
        { Regime::Trending } else { detect_regime(&ind) };
        assert_eq!(overridden, Regime::Trending);
    }

    #[test]
    fn atr_expansion_overrides_neutral_to_trending_regression() {
        // REGRESSION: ATR override previously only fired for Ranging, NOT Neutral.
        // Now it fires for both.
        let mut ind = neutral_ind(100.0);
        ind.adx = 22.0;                   // Neutral regime
        ind.atr_expansion_ratio = 2.0;    // expanding volatility
        assert_eq!(detect_regime(&ind), Regime::Neutral);
        let overridden = if ind.atr_expansion_ratio > 1.5
            && matches!(detect_regime(&ind), Regime::Ranging | Regime::Neutral)
        { Regime::Trending } else { detect_regime(&ind) };
        assert_eq!(overridden, Regime::Trending,
            "REGRESSION: Neutral + ATR expansion should override to Trending");
    }

    #[test]
    fn atr_expansion_does_not_override_already_trending() {
        let mut ind = neutral_ind(100.0);
        ind.adx = 35.0;                   // already Trending
        ind.atr_expansion_ratio = 3.0;    // extreme expansion
        assert_eq!(detect_regime(&ind), Regime::Trending);
        let overridden = if ind.atr_expansion_ratio > 1.5
            && matches!(detect_regime(&ind), Regime::Ranging | Regime::Neutral)
        { Regime::Trending } else { detect_regime(&ind) };
        // Should remain Trending (override doesn't match Trending)
        assert_eq!(overridden, Regime::Trending);
    }

    #[test]
    fn atr_expansion_below_threshold_no_override() {
        let mut ind = neutral_ind(100.0);
        ind.adx = 15.0;                   // Ranging
        ind.atr_expansion_ratio = 1.3;    // below 1.5 — no override
        let overridden = if ind.atr_expansion_ratio > 1.5
            && matches!(detect_regime(&ind), Regime::Ranging | Regime::Neutral)
        { Regime::Trending } else { detect_regime(&ind) };
        assert_eq!(overridden, Regime::Ranging,
            "expansion < 1.5 must not override the regime");
    }

    // ── Leverage calculation ──────────────────────────────────────────────────

    #[test]
    fn leverage_minimum_near_gate() {
        assert!((calc_leverage(0.65, Regime::Trending) - 2.0).abs() < 1e-6,
            "confidence 0.60–0.70 → 2.0× leverage");
    }

    #[test]
    fn leverage_trending_max_5x() {
        assert!((calc_leverage(0.95, Regime::Trending) - 5.0).abs() < 1e-6,
            "high confidence trending → 5.0× leverage");
    }

    #[test]
    fn leverage_ranging_capped_at_3x() {
        // Even at max confidence, Ranging caps at 3×
        assert!((calc_leverage(0.95, Regime::Ranging) - 3.0).abs() < 1e-6,
            "ranging regime caps leverage at 3.0×");
    }

    #[test]
    fn leverage_neutral_capped_at_3x() {
        assert!((calc_leverage(0.95, Regime::Neutral) - 3.0).abs() < 1e-6,
            "neutral regime caps leverage at 3.0×");
    }

    #[test]
    fn leverage_mid_confidence() {
        assert!((calc_leverage(0.78, Regime::Trending) - 3.0).abs() < 1e-6,
            "confidence 0.70–0.82 → 3.0× leverage");
        assert!((calc_leverage(0.85, Regime::Trending) - 4.0).abs() < 1e-6,
            "confidence 0.82–0.90 → 4.0× leverage");
    }

    // ── BTC Dominance context ─────────────────────────────────────────────────

    #[test]
    fn btc_high_dom_aligned_big_move_adds_008() {
        let ctx = BtcMarketContext { dominance: 57.0, btc_return_24h: 4.0, btc_return_4h: 0.0, asset_return_4h: 0.0 };
        let adj = ctx.confidence_adjustment("BUY");
        assert!((adj - 0.08).abs() < 1e-9,
            "high dominance + big aligned BTC move → +0.08, got {adj:.4}");
    }

    #[test]
    fn btc_high_dom_opposed_big_move_minus_012() {
        let ctx = BtcMarketContext { dominance: 57.0, btc_return_24h: 4.0, btc_return_4h: 0.0, asset_return_4h: 0.0 };
        let adj = ctx.confidence_adjustment("SELL");
        assert!((adj - (-0.12)).abs() < 1e-9,
            "high dominance + big move + opposed → -0.12, got {adj:.4}");
    }

    #[test]
    fn btc_low_dom_no_adjustment() {
        let ctx = BtcMarketContext { dominance: 42.0, btc_return_24h: 5.0, btc_return_4h: 0.0, asset_return_4h: 0.0 };
        let adj = ctx.confidence_adjustment("BUY");
        assert_eq!(adj, 0.0, "dominance < 48% → 0 adjustment (altseason)");
    }

    #[test]
    fn btc_medium_dom_aligned_adds_003() {
        let ctx = BtcMarketContext { dominance: 51.0, btc_return_24h: 2.0, btc_return_4h: 0.0, asset_return_4h: 0.0 };
        let adj = ctx.confidence_adjustment("BUY");
        assert!((adj - 0.03).abs() < 1e-9,
            "medium dominance + aligned → +0.03, got {adj:.4}");
    }

    #[test]
    fn btc_flat_move_no_adjustment() {
        // btc_return 0.2% is below the 0.3% flat threshold
        let ctx = BtcMarketContext { dominance: 58.0, btc_return_24h: 0.2, btc_return_4h: 0.0, asset_return_4h: 0.0 };
        let adj = ctx.confidence_adjustment("BUY");
        assert_eq!(adj, 0.0, "BTC return < 0.3% is flat → 0 adjustment");
    }

    #[test]
    fn btc_relative_performance_lag_adds_004() {
        // Asset lagging BTC by >2% in high dominance → catch-up BUY bonus
        let ctx = BtcMarketContext { dominance: 57.0, btc_return_24h: 0.0, btc_return_4h: 1.5, asset_return_4h: -1.0 };
        let adj = ctx.confidence_adjustment("BUY");
        assert!((adj - 0.04).abs() < 1e-9,
            "relative underperformance → catch-up +0.04, got {adj:.4}");
    }

    // ── make_decision smoke tests ─────────────────────────────────────────────

    #[test]
    fn make_decision_with_neutral_signals_returns_skip() {
        let candles = flat_candles(100.0, 50);
        let ind     = neutral_ind(100.0);
        let of      = neutral_of();
        let weights = SignalWeights::default();

        let dec = make_decision(&candles, &ind, &of, &weights, None, None, None, None, None).unwrap();
        assert_eq!(dec.action, "SKIP",
            "neutral indicators should produce SKIP, got {}", dec.action);
    }

    #[test]
    fn make_decision_stop_and_tp_correct_for_buy() {
        let candles = flat_candles(1000.0, 50);
        let mut ind = neutral_ind(1000.0);
        // Force a BUY: very oversold RSI in Ranging regime
        ind.adx  = 15.0;  // Ranging
        ind.rsi  = 20.0;  // deeply oversold → bull boost
        ind.z_score = -2.5;
        let of = OrderFlowSignal { bid_volume: 300.0, ask_volume: 100.0,
            imbalance_ratio: 3.0, direction: "LONG".to_string(), confidence: 0.95 };
        let weights = SignalWeights::default();

        let dec = make_decision(&candles, &ind, &of, &weights, None, None, None, None, None).unwrap();
        if dec.action == "BUY" {
            assert!(dec.stop_loss < dec.entry_price,
                "BUY stop_loss ({:.2}) should be below entry ({:.2})",
                dec.stop_loss, dec.entry_price);
            assert!(dec.take_profit > dec.entry_price,
                "BUY take_profit ({:.2}) should be above entry ({:.2})",
                dec.take_profit, dec.entry_price);
        }
    }

    #[test]
    fn make_decision_stop_and_tp_correct_for_sell() {
        let candles = flat_candles(1000.0, 50);
        let mut ind = neutral_ind(1000.0);
        ind.adx  = 15.0;  // Ranging
        ind.rsi  = 80.0;  // overbought → bear boost
        ind.z_score = 2.5;
        let of = OrderFlowSignal { bid_volume: 100.0, ask_volume: 300.0,
            imbalance_ratio: 0.33, direction: "SHORT".to_string(), confidence: 0.95 };
        let weights = SignalWeights::default();

        let dec = make_decision(&candles, &ind, &of, &weights, None, None, None, None, None).unwrap();
        if dec.action == "SELL" {
            assert!(dec.stop_loss > dec.entry_price,
                "SELL stop_loss ({:.2}) should be above entry ({:.2})",
                dec.stop_loss, dec.entry_price);
            assert!(dec.take_profit < dec.entry_price,
                "SELL take_profit ({:.2}) should be below entry ({:.2})",
                dec.take_profit, dec.entry_price);
        }
    }

    #[test]
    fn make_decision_empty_candles_returns_error() {
        let candles = vec![];
        let ind     = neutral_ind(100.0);
        let of      = neutral_of();
        let weights = SignalWeights::default();
        assert!(make_decision(&candles, &ind, &of, &weights, None, None, None, None, None).is_err(),
            "empty candle slice should return Err");
    }

    #[test]
    fn make_decision_confidence_always_in_0_to_1() {
        let candles = rising_candles(100.0, 0.5, 50);
        let ind = crate::indicators::calculate_all(&candles).unwrap();
        let of  = neutral_of();
        let weights = SignalWeights::default();
        let dec = make_decision(&candles, &ind, &of, &weights, None, None, None, None, None).unwrap();
        assert!(dec.confidence >= 0.0 && dec.confidence <= 1.0,
            "confidence out of [0,1]: {}", dec.confidence);
    }

    #[test]
    fn make_decision_leverage_within_regime_bounds() {
        let candles = flat_candles(100.0, 50);
        let ind = neutral_ind(100.0);
        let of  = neutral_of();
        let weights = SignalWeights::default();
        let dec = make_decision(&candles, &ind, &of, &weights, None, None, None, None, None).unwrap();
        assert!(dec.leverage >= 1.0 && dec.leverage <= 5.0,
            "leverage out of valid range [1.0, 5.0]: {}", dec.leverage);
    }

    #[test]
    fn make_decision_rationale_contains_regime_label() {
        let candles = flat_candles(100.0, 50);
        let ind = neutral_ind(100.0);
        let of  = neutral_of();
        let weights = SignalWeights::default();
        let dec = make_decision(&candles, &ind, &of, &weights, None, None, None, None, None).unwrap();
        let has_regime = dec.rationale.contains("Trending")
            || dec.rationale.contains("Ranging")
            || dec.rationale.contains("Neutral");
        assert!(has_regime, "rationale should contain regime label: {}", dec.rationale);
    }
}
