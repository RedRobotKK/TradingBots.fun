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
use serde::{Deserialize, Serialize};
use crate::data::PriceData;
use crate::indicators::TechnicalIndicators;
use crate::signals::OrderFlowSignal;
use crate::learner::{SignalWeights, SignalContribution};
use crate::sentiment::SentimentData;
use crate::candlestick_patterns;
use crate::chart_patterns;

// ─────────────────────────── Public types ────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub action:             String,  // "BUY" | "SELL" | "SKIP"
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
    /// Higher = fewer but higher-quality entries. Raised from ~0.42 to avoid
    /// choppy-market noise that caused 0W/14L in ranging conditions.
    fn threshold(self) -> f64 {
        match self {
            Regime::Trending => 0.58,  // only strong, confirmed trend signals
            Regime::Ranging  => 0.52,  // mean-reversion requires clear extremes
            Regime::Neutral  => 0.55,  // balanced but still demanding
        }
    }
    /// Dominance ratio — winning side must exceed losing side by this factor.
    /// Higher = only enter when direction is unambiguous.
    fn dominance(self) -> f64 {
        match self {
            Regime::Trending => 1.55,
            Regime::Ranging  => 1.45,
            Regime::Neutral  => 1.50,
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
/// Ranging markets: max 2× (false breakouts are common; stops get hit more)
/// Neutral markets: max 2× (conservative until win rate improves)
/// Trending markets: max 3× (momentum carries positions; higher conviction warranted)
///
/// Minimum entry confidence is 0.68 (gated in main.rs).
/// Confidence scaling within the regime cap:
///   0.68–0.75 → 1.5×  (minimum — just above gate)
///   0.75–0.82 → 2×
///   0.82–0.90 → 2.5×
///   0.90+     → regime max
pub fn calc_leverage(confidence: f64, regime: Regime) -> f64 {
    let regime_cap: f64 = match regime {
        Regime::Ranging  => 2.0,
        Regime::Neutral  => 2.0,   // reduced until win rate established
        Regime::Trending => 3.0,   // reduced from 5× — protect capital
    };
    let raw: f64 = if confidence < 0.75      { 1.5 }
                   else if confidence < 0.82 { 2.0 }
                   else if confidence < 0.90 { 2.5 }
                   else                      { 3.0 };
    raw.min(regime_cap)
}

// ─────────────────────────── Decision engine ─────────────────────────────────

/// Regime-aware decision engine.
///
/// Returns a `Decision` with action BUY / SELL / SKIP.
/// `sentiment` is `None` when LunarCrush data is not available.
pub fn make_decision(
    candles:   &[PriceData],
    ind:       &TechnicalIndicators,
    of:        &OrderFlowSignal,
    weights:   &SignalWeights,
    sentiment: Option<&SentimentData>,
) -> Result<Decision> {
    let last  = candles.last().ok_or_else(|| anyhow::anyhow!("Empty candle slice"))?;
    let close = last.close;

    let regime = detect_regime(ind);
    let mut bull    = 0.0f64;
    let mut bear    = 0.0f64;
    let mut contrib = SignalContribution::default();

    // ═════════════════════════════════════════════════════════════════════════
    //  1. RSI — behaviour is REGIME-DEPENDENT
    // ═════════════════════════════════════════════════════════════════════════
    match regime {
        // TRENDING: RSI used as momentum gauge (50-line cross)
        // Above 55 = bull momentum building; below 45 = bear momentum building.
        // Extreme readings (>70 / <30) have REDUCED weight — in a strong trend
        // overbought can stay overbought for many bars.
        Regime::Trending => {
            if ind.rsi > 65.0 {
                // Strong but not extreme bull momentum
                bull += weights.rsi * 0.80;
                contrib.rsi_bullish = true;
            } else if ind.rsi > 55.0 {
                bull += weights.rsi * 0.55;
                contrib.rsi_bullish = true;
            } else if ind.rsi < 35.0 {
                // Very oversold even in uptrend = shake-out, reversal likely
                bull += weights.rsi * 0.70;
                contrib.rsi_bullish = true;
            } else if ind.rsi < 45.0 {
                bear += weights.rsi * 0.55;
                contrib.rsi_bullish = false;
            } else {
                contrib.rsi_bullish = ind.rsi > 50.0;
            }
        }
        // RANGING: RSI used as classic mean-reversion oscillator
        // Extremes (<30 / >70) are the PRIMARY signal source.
        Regime::Ranging | Regime::Neutral => {
            if ind.rsi < 28.0 {
                bull += weights.rsi;          // deeply oversold — strong reversal signal
                contrib.rsi_bullish = true;
            } else if ind.rsi > 72.0 {
                bear += weights.rsi;
                contrib.rsi_bullish = false;
            } else if ind.rsi < 40.0 {
                bull += weights.rsi * 0.60;
                contrib.rsi_bullish = true;
            } else if ind.rsi > 60.0 {
                bear += weights.rsi * 0.60;
                contrib.rsi_bullish = false;
            } else if ind.rsi < 47.0 {
                bull += weights.rsi * 0.25;
                contrib.rsi_bullish = true;
            } else if ind.rsi > 53.0 {
                bear += weights.rsi * 0.25;
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

    let ema_pct = ind.ema_cross_pct;
    if ema_pct > 0.5 {
        // Strong uptrend: fast EMA well above slow EMA
        bull += ema_w.min(weights.ema_cross * 1.40);
        contrib.ema_cross_bullish = true;
    } else if ema_pct > 0.0 {
        // Mild uptrend
        bull += ema_w * 0.60;
        contrib.ema_cross_bullish = true;
    } else if ema_pct < -0.5 {
        bear += ema_w.min(weights.ema_cross * 1.40);
        contrib.ema_cross_bullish = false;
    } else if ema_pct < 0.0 {
        bear += ema_w * 0.60;
        contrib.ema_cross_bullish = false;
    } else {
        contrib.ema_cross_bullish = ema_pct >= 0.0;
    }

    // ═════════════════════════════════════════════════════════════════════════
    //  5. Z-score mean-reversion (ranging regime's answer to EMA cross)
    // ═════════════════════════════════════════════════════════════════════════
    // Z-score = (close − 20-bar mean) / std_dev
    // Used most aggressively in RANGING markets.
    // Extreme readings predict high-probability reversions.
    let z_w = match regime {
        Regime::Ranging  => weights.z_score * 1.40,  // PRIMARY in ranging
        Regime::Trending => weights.z_score * 0.50,  // suppressed in trends
        Regime::Neutral  => weights.z_score,
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
    // ═════════════════════════════════════════════════════════════════════════
    let trend_w = match regime {
        Regime::Trending => weights.trend * 0.80,  // partially replaced by EMA cross
        Regime::Ranging  => weights.trend * 0.30,  // nearly irrelevant in range
        Regime::Neutral  => weights.trend * 0.60,
    };

    if ind.trend > 1.2 {
        bull += trend_w;
        contrib.trend_bullish = true;
    } else if ind.trend < -1.2 {
        bear += trend_w;
        contrib.trend_bullish = false;
    } else if ind.trend > 0.4 {
        bull += trend_w * 0.50;
        contrib.trend_bullish = true;
    } else if ind.trend < -0.4 {
        bear += trend_w * 0.50;
        contrib.trend_bullish = false;
    } else {
        contrib.trend_bullish = ind.trend > 0.0;
    }

    // ═════════════════════════════════════════════════════════════════════════
    //  8. VWAP bias — institutional reference price
    // ═════════════════════════════════════════════════════════════════════════
    // Price above VWAP = institutions net bought; bias bull.
    // Price below VWAP = institutions net sold; bias bear.
    // Used as a DIRECTIONAL FILTER that modulates the volume weight.
    let vwap_bull = ind.vwap_pct > 0.3;   // >0.3% above VWAP = bull bias
    let vwap_bear = ind.vwap_pct < -0.3;  // <0.3% below VWAP = bear bias

    // ═════════════════════════════════════════════════════════════════════════
    //  9. Volume conviction multiplier
    // ═════════════════════════════════════════════════════════════════════════
    // High volume amplifies the prevailing signal (more conviction = higher weight).
    // Low volume dampens signals (noise may dominate).
    // VWAP bias is also incorporated here.
    let vol_ratio = ind.volume_ratio;

    // Volume score: how much does volume confirm the current direction?
    let bull_vol_conf = vol_ratio > 1.3 && (vwap_bull || bull > bear);
    let bear_vol_conf = vol_ratio > 1.3 && (vwap_bear || bear > bull);

    if bull_vol_conf {
        bull += weights.volume * vol_ratio.min(2.5) / 2.5;
        contrib.volume_present = true;
        contrib.volume_bullish = true;
    } else if bear_vol_conf {
        bear += weights.volume * vol_ratio.min(2.5) / 2.5;
        contrib.volume_present = true;
        contrib.volume_bullish = false;
    }
    // After individual scores, apply volume as a global multiplier
    // High volume = amplify existing signal; Low volume = dampen
    let vol_mult = if vol_ratio > 2.0      { 1.20 }
                   else if vol_ratio > 1.4 { 1.10 }
                   else if vol_ratio < 0.6 { 0.85 }   // thin volume = weak conviction
                   else if vol_ratio < 0.4 { 0.75 }
                   else                    { 1.00 };

    bull *= vol_mult;
    bear *= vol_mult;

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
    //  11. Candlestick Patterns  (single / double / triple bar)
    // ═════════════════════════════════════════════════════════════════════════
    // Run on the last 3 bars at most.  Contribution capped at ±0.12 so
    // patterns confirm rather than override the regime-based scoring.
    let csp = candlestick_patterns::detect(candles);
    bull += csp.bull_boost;
    bear += csp.bear_boost;

    // ═════════════════════════════════════════════════════════════════════════
    //  12. Chart Patterns  (structural patterns over 10–60 bars)
    // ═════════════════════════════════════════════════════════════════════════
    // Includes: double/triple tops & bottoms, H&S, wedges, triangles,
    // flags, pennants, rectangles, cup & handle, and institutional PA patterns
    // (compression→expansion, liquidity grab, V-flash reversal).
    let chp = chart_patterns::detect(candles);
    bull += chp.bull_boost;
    bear += chp.bear_boost;

    // ═════════════════════════════════════════════════════════════════════════
    //  Final decision — regime-dependent thresholds
    // ═════════════════════════════════════════════════════════════════════════
    let threshold  = regime.threshold();
    let dominance  = regime.dominance();

    let (action, confidence) = if bull >= threshold && bull > bear * dominance {
        ("BUY".to_string(),  (bull / (bull + bear + 1e-8)).min(1.0))
    } else if bear >= threshold && bear > bull * dominance {
        ("SELL".to_string(), (bear / (bull + bear + 1e-8)).min(1.0))
    } else {
        ("SKIP".to_string(), f64::max(bull, bear) / (bull + bear + 1e-8))
    };

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

    let rationale = format!(
        "[{}] RSI:{:.0} Z:{:.1} EMA:{:+.2}% MACD-H:{:.5} VOL:{:.1}× ADX:{:.0} VWAP:{:+.1}%{}{}{}",
        regime.label(),
        ind.rsi,
        ind.z_score,
        ind.ema_cross_pct,
        ind.macd_histogram,
        ind.volume_ratio,
        ind.adx,
        ind.vwap_pct,
        sent_tag,
        csp_tag,
        chp_tag,
    );

    Ok(Decision {
        action,
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
