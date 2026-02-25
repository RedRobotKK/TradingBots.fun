//! Candlestick pattern recognition — single, double, and triple bar patterns.
//!
//! All 21 patterns from the trading reference images:
//!
//! **Single-bar**: Doji (Classic, Dragonfly, Gravestone), Hammer, Hanging Man,
//!   Inverted Hammer, Bull/Bear Pin Bar, Bull/Bear Marubozu
//!
//! **Double-bar**: Bull/Bear Engulfing, Bull/Bear Harami, Tweezer Top/Bottom,
//!   Dark Cloud Cover
//!
//! **Triple-bar**: Morning Star, Evening Star, Three White Soldiers,
//!   Three Black Crows
//!
//! Returns a `PatternSignal` with bull/bear boosts capped at 0.12 so patterns
//! act as confirmation signals rather than overriding the regime engine.

use crate::data::PriceData;

/// Signal returned by `detect()`.
pub struct PatternSignal {
    /// Additive bull contribution (0.0–0.12).
    pub bull_boost: f64,
    /// Additive bear contribution (0.0–0.12).
    pub bear_boost: f64,
    /// Human-readable names joined with "+" for the rationale string.
    pub name: Option<String>,
}

/// Detect candlestick patterns from the most recent 1–3 bars.
pub fn detect(candles: &[PriceData]) -> PatternSignal {
    let n = candles.len();
    if n < 2 {
        return PatternSignal { bull_boost: 0.0, bear_boost: 0.0, name: None };
    }

    let mut bull: f64 = 0.0;
    let mut bear: f64 = 0.0;
    let mut names: Vec<&'static str> = Vec::new();

    // ── Current bar (c0) ────────────────────────────────────────────────────
    let c0     = &candles[n - 1];
    let range0 = c0.high - c0.low;
    if range0 < 1e-10 {
        return PatternSignal { bull_boost: 0.0, bear_boost: 0.0, name: None };
    }
    let body0  = (c0.close - c0.open).abs();
    let uw0    = c0.high - c0.close.max(c0.open);   // upper wick
    let lw0    = c0.close.min(c0.open) - c0.low;    // lower wick
    let bull0  = c0.close > c0.open;

    // ── Previous bar (c1) ───────────────────────────────────────────────────
    let c1     = &candles[n - 2];
    let range1 = (c1.high - c1.low).max(1e-10);
    let body1  = (c1.close - c1.open).abs();
    let bull1  = c1.close > c1.open;

    // ════════════════════════════════════════════════════════════════════════
    //  SINGLE-BAR PATTERNS
    // ════════════════════════════════════════════════════════════════════════

    // ── Doji (body < 10% of range) ──────────────────────────────────────────
    let is_doji = body0 / range0 < 0.10;
    if is_doji {
        if lw0 > range0 * 0.60 && uw0 < range0 * 0.15 {
            // Dragonfly Doji: long lower wick, open/close near top → bullish
            bull += 0.07;
            names.push("Dragonfly Doji");
        } else if uw0 > range0 * 0.60 && lw0 < range0 * 0.15 {
            // Gravestone Doji: long upper wick, open/close near bottom → bearish
            bear += 0.07;
            names.push("Gravestone Doji");
        } else {
            // Classic Doji: indecision — no directional boost, just label
            names.push("Doji");
        }
    }

    // ── Hammer / Hanging Man ────────────────────────────────────────────────
    // Shape: small body at top of range, lower wick ≥ 2× body
    if !is_doji && lw0 >= body0 * 2.0 && uw0 <= body0 * 0.5 {
        if !bull1 {
            // After a down bar → Hammer (bullish reversal)
            bull += 0.08;
            names.push("Hammer");
        } else {
            // After an up bar → Hanging Man (bearish)
            bear += 0.07;
            names.push("Hanging Man");
        }
    }

    // ── Inverted Hammer ─────────────────────────────────────────────────────
    // Long upper wick ≥ 2× body, after downtrend bar → bullish reversal
    if !is_doji && uw0 >= body0 * 2.0 && lw0 <= body0 * 0.5 && !bull1 {
        bull += 0.07;
        names.push("Inv Hammer");
    }

    // ── Bullish Pin Bar ─────────────────────────────────────────────────────
    // Lower wick dominates (≥ 55% of range, ≥ 2.5× body) — strong rejection low
    if !is_doji && lw0 >= range0 * 0.55 && lw0 >= body0 * 2.5 {
        bull += 0.09;
        names.push("Bull Pin Bar");
    }

    // ── Bearish Pin Bar ─────────────────────────────────────────────────────
    if !is_doji && uw0 >= range0 * 0.55 && uw0 >= body0 * 2.5 {
        bear += 0.09;
        names.push("Bear Pin Bar");
    }

    // ── Bullish Marubozu ────────────────────────────────────────────────────
    // Strong bull candle, almost no wicks (body ≥ 90% of range)
    if bull0 && body0 >= range0 * 0.90 {
        bull += 0.09;
        names.push("Bull Marubozu");
    }

    // ── Bearish Marubozu ────────────────────────────────────────────────────
    if !bull0 && body0 >= range0 * 0.90 {
        bear += 0.09;
        names.push("Bear Marubozu");
    }

    // ════════════════════════════════════════════════════════════════════════
    //  TWO-BAR PATTERNS
    // ════════════════════════════════════════════════════════════════════════

    // ── Bullish Engulfing ───────────────────────────────────────────────────
    // Current bullish body fully engulfs previous bearish body
    if bull0 && !bull1
        && c0.open <= c1.close
        && c0.close >= c1.open
    {
        bull += 0.11;
        names.push("Bull Engulfing");
    }

    // ── Bearish Engulfing ───────────────────────────────────────────────────
    if !bull0 && bull1
        && c0.open >= c1.close
        && c0.close <= c1.open
    {
        bear += 0.11;
        names.push("Bear Engulfing");
    }

    // ── Bullish Harami ──────────────────────────────────────────────────────
    // Small bullish bar sitting inside prior large bearish bar
    if bull0 && !bull1 && body1 > 0.0
        && c0.open > c1.close
        && c0.close < c1.open
        && body0 < body1 * 0.50
    {
        bull += 0.07;
        names.push("Bull Harami");
    }

    // ── Bearish Harami ──────────────────────────────────────────────────────
    if !bull0 && bull1 && body1 > 0.0
        && c0.open < c1.close
        && c0.close > c1.open
        && body0 < body1 * 0.50
    {
        bear += 0.07;
        names.push("Bear Harami");
    }

    // ── Tweezer Bottom ──────────────────────────────────────────────────────
    // Two bars with matching lows (within 0.2% of prior bar range) — support held
    if !bull1 && bull0 && (c0.low - c1.low).abs() < range1 * 0.02 {
        bull += 0.08;
        names.push("Tweezer Bottom");
    }

    // ── Tweezer Top ─────────────────────────────────────────────────────────
    if bull1 && !bull0 && (c0.high - c1.high).abs() < range1 * 0.02 {
        bear += 0.08;
        names.push("Tweezer Top");
    }

    // ── Dark Cloud Cover ────────────────────────────────────────────────────
    // Bullish bar → bearish bar opens above prev high, closes below prior midpoint
    if bull1 && !bull0 {
        let mid1 = (c1.open + c1.close) / 2.0;
        if c0.open > c1.high && c0.close < mid1 && c0.close > c1.open {
            bear += 0.10;
            names.push("Dark Cloud");
        }
    }

    // ════════════════════════════════════════════════════════════════════════
    //  THREE-BAR PATTERNS
    // ════════════════════════════════════════════════════════════════════════
    if n >= 3 {
        let c2     = &candles[n - 3];
        let range2 = (c2.high - c2.low).max(1e-10);
        let body2  = (c2.close - c2.open).abs();
        let bull2  = c2.close > c2.open;

        // ── Morning Star ──────────────────────────────────────────────────
        // Bearish bar → small body/doji → bullish bar closing into first bar's body
        if !bull2 && body1 < range1 * 0.35 && bull0 {
            let mid2 = (c2.open + c2.close) / 2.0;
            if c0.close > mid2 {
                bull += 0.12;
                names.push("Morning Star");
            }
        }

        // ── Evening Star ──────────────────────────────────────────────────
        if bull2 && body1 < range1 * 0.35 && !bull0 {
            let mid2 = (c2.open + c2.close) / 2.0;
            if c0.close < mid2 {
                bear += 0.12;
                names.push("Evening Star");
            }
        }

        // ── Three White Soldiers ──────────────────────────────────────────
        // 3 consecutive bullish bars with higher closes, each body ≥ 60% of range
        if bull0 && bull1 && bull2
            && c0.close > c1.close && c1.close > c2.close
            && range0 > 0.0 && range1 > 0.0 && range2 > 0.0
            && body0 >= range0 * 0.60
            && body1 >= range1 * 0.60
            && body2 >= range2 * 0.60
        {
            bull += 0.12;
            names.push("3 White Soldiers");
        }

        // ── Three Black Crows ─────────────────────────────────────────────
        if !bull0 && !bull1 && !bull2
            && c0.close < c1.close && c1.close < c2.close
            && range0 > 0.0 && range1 > 0.0 && range2 > 0.0
            && body0 >= range0 * 0.60
            && body1 >= range1 * 0.60
            && body2 >= range2 * 0.60
        {
            bear += 0.12;
            names.push("3 Black Crows");
        }

        // Suppress unused warning when soldiers/crows branches don't fire
        let _ = body2;
        let _ = range2;
    }

    // ── Cap and return ──────────────────────────────────────────────────────
    let pattern_name = if names.is_empty() {
        None
    } else {
        Some(names.join("+"))
    };

    PatternSignal {
        bull_boost: bull.min(0.12),
        bear_boost: bear.min(0.12),
        name: pattern_name,
    }
}
