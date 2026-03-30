//! Technical indicators used by the decision engine.
//!
//! All functions operate on a candle slice (newest candle last).
//! Minimum 26 candles required; 50+ gives stable ADX and MACD signal.
//!
//! ## Indicator catalogue
//!
//! | Name            | Notes                                               |
//! |-----------------|-----------------------------------------------------|
//! | RSI(14)         | Wilder's SMMA — industry standard                  |
//! | Bollinger(20,2) | Mean ± 2σ + width% for squeeze detection           |
//! | MACD(12,26,9)   | Proper EMA-9 signal line, not an approximation     |
//! | ATR(14)         | Wilder's smoothed True Range for stop placement    |
//! | EMA(8/21)       | Institutional momentum cross                        |
//! | ADX(14)         | Regime classifier: >27 trending, <19 ranging       |
//! | Z-score(20)     | Statistical mean-reversion gauge                   |
//! | VWAP(24)        | Volume-weighted avg price — institutional anchor   |
//! | Volume Ratio    | Current / 20-bar avg — conviction filter           |

use crate::data::PriceData;
use anyhow::Result;
use serde::{Deserialize, Serialize};

// ─────────────────────────── Output struct ───────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TechnicalIndicators {
    // ── Core mean-reversion ───────────────────────────────────────────────────
    /// Wilder's RSI(14). <30 = oversold, >70 = overbought.
    pub rsi: f64,
    /// Bollinger Band upper  (20-bar, 2σ)
    pub bollinger_upper: f64,
    /// Bollinger Band middle (20-bar SMA — serves as VWAP proxy)
    pub bollinger_middle: f64,
    /// Bollinger Band lower  (20-bar, 2σ)
    pub bollinger_lower: f64,
    /// Bollinger Band width as % of middle.  Low value = squeeze (breakout imminent).
    pub bb_width_pct: f64,

    // ── Momentum ──────────────────────────────────────────────────────────────
    /// MACD line = EMA(12) − EMA(26)
    pub macd: f64,
    /// Signal line = EMA(9) of the MACD series  (proper calculation, not approximation)
    pub macd_signal: f64,
    /// Histogram = MACD − signal.  Rising = accelerating momentum.
    pub macd_histogram: f64,

    // ── Volatility / stop placement ───────────────────────────────────────────
    /// ATR(14) — Wilder's Average True Range.  Used for all stop distances.
    pub atr: f64,

    // ── Trend / regime ────────────────────────────────────────────────────────
    /// Legacy: % price change over last 10 bars.
    pub trend: f64,
    /// EMA(8) — fast trend line.
    pub ema8: f64,
    /// EMA(21) — slow trend line.
    pub ema21: f64,
    /// (ema8 − ema21) / ema21 × 100.  Positive = bull trend confirmed.
    pub ema_cross_pct: f64,
    /// ADX(14).  0–100 scale.  >27 = trending market, <19 = ranging.
    pub adx: f64,

    // ── Mean-reversion depth ──────────────────────────────────────────────────
    /// Z-score of close vs 20-bar rolling mean.  |>2.0| = statistically extreme.
    pub z_score: f64,

    // ── Volume / conviction ───────────────────────────────────────────────────
    /// Current bar volume ÷ 20-bar average.  >1.5 = high-conviction bar.
    pub volume_ratio: f64,
    /// 24-bar VWAP (institutional benchmark price).
    pub vwap: f64,
    /// (close − vwap) / vwap × 100.  Positive = price trading above VWAP (bull bias).
    pub vwap_pct: f64,

    // ── Volatility regime ─────────────────────────────────────────────────────
    /// ATR expansion ratio: current ATR(14) ÷ mean ATR over the prior 24 bars.
    /// > 1.5 = breakout expansion (override ranging regime → trending).
    /// > < 0.7 = volatility compression (likely squeeze / low-conviction).
    /// > = 1.0 = normal / unavailable.
    pub atr_expansion_ratio: f64,
}

// ─────────────────────────── Higher-timeframe indicators ──────────────────────

/// Indicators computed from the 4-hour candle series.
///
/// Used as a multi-timeframe confirmation filter: a 1h signal that is also
/// confirmed on the 4h chart has substantially higher IC than an unconfirmed one.
#[derive(Debug, Clone)]
pub struct HtfIndicators {
    /// RSI(14) on the 4h series.  <45 = oversold context; >55 = overbought context.
    pub rsi_4h: f64,
    /// Z-score(20) on the 4h series.  |>1.2| = statistically extreme on higher TF.
    pub z_score_4h: f64,
}

impl Default for HtfIndicators {
    fn default() -> Self {
        // Neutral defaults: RSI=50 (no direction bias), Z=0.0 (at mean).
        // Used when insufficient candles are available — avoids spurious scaling.
        HtfIndicators {
            rsi_4h: 50.0,
            z_score_4h: 0.0,
        }
    }
}

// ─────────────────────────── Macro daily MAs ─────────────────────────────────

/// Simple Moving Average of the last `period` closes from a candle slice.
/// Returns `None` when there are fewer candles than `period`.
pub fn sma(candles: &[PriceData], period: usize) -> Option<f64> {
    if candles.len() < period || period == 0 {
        return None;
    }
    let slice = &candles[candles.len() - period..];
    Some(slice.iter().map(|c| c.close).sum::<f64>() / period as f64)
}

/// Daily MA5 / MA10 / MA20 from a slice of **daily** candles (newest last).
///
/// Used for macro regime detection:
///   * price > MA20 AND MA5 > MA10  →  BULL
///   * price < MA20 AND MA5 < MA10  →  BEAR
///   * mixed                        →  TRANSITION
///
/// Any unavailable value (insufficient candles) is returned as 0.0.
pub fn daily_mas(candles: &[PriceData]) -> (f64, f64, f64) {
    let ma5  = sma(candles, 5).unwrap_or(0.0);
    let ma10 = sma(candles, 10).unwrap_or(0.0);
    let ma20 = sma(candles, 20).unwrap_or(0.0);
    (ma5, ma10, ma20)
}

// ─────────────────────────── Public entry point ──────────────────────────────

/// Calculate all indicators from a slice of OHLCV candles (newest last).
/// Requires ≥ 26 candles; accuracy improves significantly with 50+.
pub fn calculate_all(candles: &[PriceData]) -> Result<TechnicalIndicators> {
    if candles.len() < 26 {
        anyhow::bail!(
            "Need at least 26 candles for indicators, got {}",
            candles.len()
        );
    }

    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let highs: Vec<f64> = candles.iter().map(|c| c.high).collect();
    let lows: Vec<f64> = candles.iter().map(|c| c.low).collect();

    let rsi = calc_rsi_wilder(&closes, 14);
    let (upper, middle, lower) = calc_bollinger(&closes, 20, 2.0);
    let bb_width_pct = if middle > 0.0 {
        (upper - lower) / middle * 100.0
    } else {
        0.0
    };
    let (macd_line, macd_sig, hist) = calc_macd_proper(&closes);
    let atr = calc_atr_wilder(&highs, &lows, &closes, 14);
    let trend = calc_trend(&closes, 10);
    let ema8 = ema_last(&closes, 8);
    let ema21 = ema_last(&closes, 21);
    let ema_cross_pct = if ema21 > 0.0 {
        (ema8 - ema21) / ema21 * 100.0
    } else {
        0.0
    };
    let adx = calc_adx(&highs, &lows, &closes, 14);
    let z_score = calc_z_score(&closes, 20);
    let volume_ratio = calc_volume_ratio(candles, 20);
    let (vwap, vwap_pct) = calc_vwap(candles, &closes);
    // ATR expansion: compare current 14-bar mean TR vs prior 24-bar mean TR
    let atr_expansion_ratio = calc_atr_expansion_ratio(&highs, &lows, &closes, 14, 24);

    Ok(TechnicalIndicators {
        rsi,
        bollinger_upper: upper,
        bollinger_middle: middle,
        bollinger_lower: lower,
        bb_width_pct,
        macd: macd_line,
        macd_signal: macd_sig,
        macd_histogram: hist,
        atr,
        trend,
        ema8,
        ema21,
        ema_cross_pct,
        adx,
        z_score,
        volume_ratio,
        vwap,
        vwap_pct,
        atr_expansion_ratio,
    })
}

// ─────────────────────────── Individual indicators ───────────────────────────

/// Wilder's Smoothed RSI (the industry-standard calculation).
///
/// Uses Wilder's SMMA (α = 1/period) rather than simple arithmetic average.
/// Differences from plain-average RSI are most pronounced near extremes.
fn calc_rsi_wilder(closes: &[f64], period: usize) -> f64 {
    if closes.len() < period + 1 {
        return 50.0;
    }

    let n = closes.len();

    // Seed averages from first `period` changes
    let mut avg_gain = 0.0f64;
    let mut avg_loss = 0.0f64;
    for i in 1..=period {
        let d = closes[i] - closes[i - 1];
        if d > 0.0 {
            avg_gain += d;
        } else {
            avg_loss += d.abs();
        }
    }
    avg_gain /= period as f64;
    avg_loss /= period as f64;

    // Wilder's smoothing: avg = (prev_avg × (period-1) + current) / period
    for i in (period + 1)..n {
        let d = closes[i] - closes[i - 1];
        let g = if d > 0.0 { d } else { 0.0 };
        let l = if d < 0.0 { d.abs() } else { 0.0 };
        avg_gain = (avg_gain * (period as f64 - 1.0) + g) / period as f64;
        avg_loss = (avg_loss * (period as f64 - 1.0) + l) / period as f64;
    }

    if avg_loss < 1e-12 {
        // No losses: flat market (no gains either) → neutral 50; pure uptrend → 100
        return if avg_gain < 1e-12 { 50.0 } else { 100.0 };
    }
    let rs = avg_gain / avg_loss;
    100.0 - (100.0 / (1.0 + rs))
}

/// Bollinger Bands: SMA(period) ± mult × σ.
fn calc_bollinger(closes: &[f64], period: usize, mult: f64) -> (f64, f64, f64) {
    if closes.len() < period {
        let p = *closes.last().unwrap_or(&0.0);
        return (p * 1.02, p, p * 0.98);
    }
    let n = closes.len();
    let slice = &closes[n - period..];
    let mean = slice.iter().sum::<f64>() / period as f64;
    let var = slice.iter().map(|&x| (x - mean).powi(2)).sum::<f64>() / period as f64;
    let std = var.sqrt();
    (mean + mult * std, mean, mean - mult * std)
}

/// Full MACD(12, 26, 9) with a proper EMA-9 signal line.
///
/// Iterates over all candles to build the MACD series then smooths the last
/// 9+ values into the signal line.  Much more accurate than the old 0.80 approx.
fn calc_macd_proper(closes: &[f64]) -> (f64, f64, f64) {
    let n = closes.len();
    if n < 26 {
        return (0.0, 0.0, 0.0);
    }

    let k12 = 2.0 / 13.0;
    let k26 = 2.0 / 27.0;
    let k9 = 2.0 / 10.0;

    let mut e12 = closes[0];
    let mut e26 = closes[0];
    let mut macd_series: Vec<f64> = Vec::with_capacity(n);

    for &p in &closes[1..] {
        e12 = p * k12 + e12 * (1.0 - k12);
        e26 = p * k26 + e26 * (1.0 - k26);
        macd_series.push(e12 - e26);
    }

    let macd_line = *macd_series.last().unwrap_or(&0.0);

    // Signal = EMA(9) of MACD series
    let sig_series = &macd_series[macd_series.len().saturating_sub(60)..];
    let signal = if sig_series.len() >= 9 {
        let mut s = sig_series[0];
        for &m in &sig_series[1..] {
            s = m * k9 + s * (1.0 - k9);
        }
        s
    } else {
        macd_line * 0.80
    };

    (macd_line, signal, macd_line - signal)
}

/// Wilder's Average True Range (proper SMMA smoothing, not simple average).
fn calc_atr_wilder(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> f64 {
    let n = closes.len();
    if n < 2 {
        return 0.0;
    }

    let trs: Vec<f64> = (1..n)
        .map(|i| {
            f64::max(
                highs[i] - lows[i],
                f64::max(
                    (highs[i] - closes[i - 1]).abs(),
                    (lows[i] - closes[i - 1]).abs(),
                ),
            )
        })
        .collect();

    if trs.len() < period {
        return trs.iter().sum::<f64>() / trs.len() as f64;
    }

    // Seed with simple average of first `period` TRs
    let mut atr = trs[..period].iter().sum::<f64>() / period as f64;
    // Wilder's smoothing for remainder
    for &tr in &trs[period..] {
        atr = (atr * (period as f64 - 1.0) + tr) / period as f64;
    }
    atr
}

/// Legacy trend signal: % change over `period` bars.
fn calc_trend(closes: &[f64], period: usize) -> f64 {
    let n = closes.len();
    if n <= period {
        return 0.0;
    }
    let old = closes[n - period - 1];
    if old == 0.0 {
        return 0.0;
    }
    (closes[n - 1] - old) / old * 100.0
}

/// Exponential Moving Average — single final value.
/// Seeded from prices[0], smoothed left-to-right.
pub fn ema_last(prices: &[f64], period: usize) -> f64 {
    if prices.is_empty() {
        return 0.0;
    }
    let k = 2.0 / (period as f64 + 1.0);
    let mut val = prices[0];
    for &p in &prices[1..] {
        val = p * k + val * (1.0 - k);
    }
    val
}

/// ADX(14) — Average Directional Index.
///
/// Uses Wilder's running smoothing for TR, +DM14, -DM14, then derives DX
/// and smooths it into the ADX line.  Returns 0–100.
///
/// Interpretation:
///   > 27  — clearly trending (use momentum strategies)
///   > 20–27 — weakening or building trend (use balanced approach)
///   > < 19  — ranging / choppy (use mean reversion strategies)
fn calc_adx(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> f64 {
    let n = highs.len();
    // Need at least 2×period bars for reliable ADX
    if n < period * 2 + 2 {
        return 25.0;
    }

    // --- Step 1: True Range, +DM, -DM at each bar ---
    let mut trs: Vec<f64> = Vec::with_capacity(n - 1);
    let mut plus_dms: Vec<f64> = Vec::with_capacity(n - 1);
    let mut minus_dms: Vec<f64> = Vec::with_capacity(n - 1);

    for i in 1..n {
        let tr = f64::max(
            highs[i] - lows[i],
            f64::max(
                (highs[i] - closes[i - 1]).abs(),
                (lows[i] - closes[i - 1]).abs(),
            ),
        );
        let up_move = highs[i] - highs[i - 1];
        let dn_move = lows[i - 1] - lows[i];
        let plus_dm = if up_move > dn_move && up_move > 0.0 {
            up_move
        } else {
            0.0
        };
        let minus_dm = if dn_move > up_move && dn_move > 0.0 {
            dn_move
        } else {
            0.0
        };
        trs.push(tr);
        plus_dms.push(plus_dm);
        minus_dms.push(minus_dm);
    }

    let m = trs.len();
    if m < period {
        return 25.0;
    }

    // --- Step 2: Seed initial Wilder's sums ---
    let mut atr_s = trs[..period].iter().sum::<f64>();
    let mut pdm_s = plus_dms[..period].iter().sum::<f64>();
    let mut mdm_s = minus_dms[..period].iter().sum::<f64>();

    let mut dx_vals: Vec<f64> = Vec::new();

    // First DX from the seeded sums
    {
        let pdi = if atr_s > 0.0 {
            100.0 * pdm_s / atr_s
        } else {
            0.0
        };
        let mdi = if atr_s > 0.0 {
            100.0 * mdm_s / atr_s
        } else {
            0.0
        };
        let dx = if pdi + mdi > 0.0 {
            100.0 * (pdi - mdi).abs() / (pdi + mdi)
        } else {
            0.0
        };
        dx_vals.push(dx);
    }

    // --- Step 3: Roll Wilder's smoothing across remaining bars ---
    for i in period..m {
        atr_s = atr_s - atr_s / period as f64 + trs[i];
        pdm_s = pdm_s - pdm_s / period as f64 + plus_dms[i];
        mdm_s = mdm_s - mdm_s / period as f64 + minus_dms[i];

        let pdi = if atr_s > 0.0 {
            100.0 * pdm_s / atr_s
        } else {
            0.0
        };
        let mdi = if atr_s > 0.0 {
            100.0 * mdm_s / atr_s
        } else {
            0.0
        };
        let dx = if pdi + mdi > 0.0 {
            100.0 * (pdi - mdi).abs() / (pdi + mdi)
        } else {
            0.0
        };
        dx_vals.push(dx);
    }

    // --- Step 4: ADX = Wilder's smooth of DX ---
    if dx_vals.len() < period {
        return 25.0;
    }

    let mut adx = dx_vals[..period].iter().sum::<f64>() / period as f64;
    for &dx in &dx_vals[period..] {
        adx = (adx * (period as f64 - 1.0) + dx) / period as f64;
    }

    adx.clamp(0.0, 100.0)
}

/// Z-score of the most recent close vs the `period`-bar rolling mean.
///
/// Values outside ±2.0 are statistically unusual and typically revert.
fn calc_z_score(closes: &[f64], period: usize) -> f64 {
    let n = closes.len();
    if n < period {
        return 0.0;
    }
    let slice = &closes[n - period..];
    let mean = slice.iter().sum::<f64>() / period as f64;
    let var = slice.iter().map(|&x| (x - mean).powi(2)).sum::<f64>() / period as f64;
    let stddev = var.sqrt();
    if stddev < 1e-12 {
        return 0.0;
    }
    (closes[n - 1] - mean) / stddev
}

/// Current bar's volume relative to its 20-bar simple moving average.
///
/// > 1.5 = high conviction;  < 0.6 = low conviction (weak signal).
fn calc_volume_ratio(candles: &[PriceData], period: usize) -> f64 {
    let n = candles.len();
    if n < period + 1 {
        return 1.0;
    }
    // Average of the `period` bars BEFORE the most recent one
    let avg_vol = candles[n - period - 1..n - 1]
        .iter()
        .map(|c| c.volume)
        .sum::<f64>()
        / period as f64;
    let cur_vol = candles[n - 1].volume;
    if avg_vol > 0.0 {
        (cur_vol / avg_vol).clamp(0.1, 5.0)
    } else {
        1.0
    }
}

/// ATR expansion ratio: current `period`-bar mean TR ÷ prior `lookback`-bar mean TR.
///
/// Compares recent volatility against the recent historical baseline.
/// > 1.5 = breakout / expanding range (acts as regime override → Trending)
/// > < 0.7 = compression / squeeze (low-conviction environment)
/// > = 1.0 = default / insufficient data
fn calc_atr_expansion_ratio(
    highs: &[f64],
    lows: &[f64],
    closes: &[f64],
    period: usize,
    lookback: usize,
) -> f64 {
    let n = closes.len();
    if n < period + lookback + 1 {
        return 1.0;
    }

    // Build True Range series (length = n - 1)
    let trs: Vec<f64> = (1..n)
        .map(|i| {
            f64::max(
                highs[i] - lows[i],
                f64::max(
                    (highs[i] - closes[i - 1]).abs(),
                    (lows[i] - closes[i - 1]).abs(),
                ),
            )
        })
        .collect();

    let m = trs.len();
    if m < period + lookback {
        return 1.0;
    }

    // Current window: last `period` TRs
    let current_mean = trs[m - period..].iter().sum::<f64>() / period as f64;
    // Historical window: the `lookback` TRs immediately before the current window
    let hist_end = m - period;
    let hist_start = hist_end.saturating_sub(lookback);
    let hist_count = hist_end - hist_start;
    if hist_count == 0 {
        return 1.0;
    }
    let hist_mean = trs[hist_start..hist_end].iter().sum::<f64>() / hist_count as f64;

    if hist_mean < 1e-12 {
        return 1.0;
    }
    (current_mean / hist_mean).clamp(0.1, 5.0)
}

// ─────────────────────────── HTF entry point ─────────────────────────────────

/// Compute higher-timeframe (4h) indicators from the provided candle slice.
///
/// Returns `HtfIndicators::default()` (rsi=50, z=0) when insufficient data.
/// Requires ≥ 26 candles for meaningful results.
pub fn calculate_htf(candles: &[PriceData]) -> HtfIndicators {
    if candles.len() < 26 {
        return HtfIndicators::default();
    }
    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    HtfIndicators {
        rsi_4h: calc_rsi_wilder(&closes, 14),
        z_score_4h: calc_z_score(&closes, 20),
    }
}

/// 24-bar VWAP (typical price × volume, averaged) and the current price's
/// deviation from it as a percentage.
fn calc_vwap(candles: &[PriceData], closes: &[f64]) -> (f64, f64) {
    let n = candles.len();
    let lookback = 24_usize.min(n);
    let start = n - lookback;

    let (sum_pv, sum_v) = candles[start..]
        .iter()
        .fold((0.0f64, 0.0f64), |(spv, sv), c| {
            let typical = (c.high + c.low + c.close) / 3.0;
            (spv + typical * c.volume, sv + c.volume)
        });

    let vwap = if sum_v > 0.0 {
        sum_pv / sum_v
    } else {
        *closes.last().unwrap_or(&0.0)
    };
    let vwap_pct = if vwap > 0.0 {
        (closes.last().unwrap_or(&vwap) - vwap) / vwap * 100.0
    } else {
        0.0
    };

    (vwap, vwap_pct)
}

// ═══════════════════════════════════════════════════════════════════════════════
//  UNIT TESTS
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ──────────────────────────────────────────────────────────────

    /// Build a flat candle slice (all OHLCV = `price`) of length `n`.
    fn flat_candles(price: f64, n: usize) -> Vec<PriceData> {
        (0..n)
            .map(|i| PriceData {
                symbol: "TEST".to_string(),
                timestamp: i as i64 * 3_600_000,
                open: price,
                high: price,
                low: price,
                close: price,
                volume: 1000.0,
            })
            .collect()
    }

    /// Arithmetic rising series: close[i] = start + step × i.
    /// high = close + 0.2×step, low = close − 0.2×step.
    fn rising_candles(start: f64, step: f64, n: usize) -> Vec<PriceData> {
        (0..n)
            .map(|i| {
                let c = start + step * i as f64;
                PriceData {
                    symbol: "TEST".to_string(),
                    timestamp: i as i64 * 3_600_000,
                    open: c - step * 0.1,
                    high: c + step * 0.2,
                    low: c - step * 0.2,
                    close: c,
                    volume: 1000.0 + i as f64 * 10.0,
                }
            })
            .collect()
    }

    fn falling_candles(start: f64, step: f64, n: usize) -> Vec<PriceData> {
        rising_candles(start, -step, n)
    }

    // ── RSI ───────────────────────────────────────────────────────────────────

    #[test]
    fn rsi_flat_price_is_50() {
        let closes = vec![100.0f64; 30];
        let rsi = calc_rsi_wilder(&closes, 14);
        assert!(
            (rsi - 50.0).abs() < 1.0,
            "flat RSI should be ~50, got {rsi:.2}"
        );
    }

    #[test]
    fn rsi_perpetual_rise_approaches_100() {
        let closes: Vec<f64> = (0..50).map(|i| 100.0 + i as f64).collect();
        let rsi = calc_rsi_wilder(&closes, 14);
        assert!(rsi > 85.0, "rising RSI should be > 85, got {rsi:.2}");
    }

    #[test]
    fn rsi_perpetual_fall_approaches_0() {
        let closes: Vec<f64> = (0..50).map(|i| 200.0 - i as f64).collect();
        let rsi = calc_rsi_wilder(&closes, 14);
        assert!(rsi < 15.0, "falling RSI should be < 15, got {rsi:.2}");
    }

    #[test]
    fn rsi_always_in_bounds() {
        let closes: Vec<f64> = (0..60).map(|i| 1.0 + i as f64 * 10.0).collect();
        let rsi = calc_rsi_wilder(&closes, 14);
        assert!((0.0..=100.0).contains(&rsi), "RSI out of [0,100]: {rsi:.4}");
    }

    // ── Bollinger Bands ───────────────────────────────────────────────────────

    #[test]
    fn bollinger_flat_price_zero_width() {
        let closes = vec![50.0f64; 30];
        let (upper, middle, lower) = calc_bollinger(&closes, 20, 2.0);
        assert!(upper >= middle && middle >= lower);
        let width_pct = if middle > 0.0 {
            (upper - lower) / middle * 100.0
        } else {
            0.0
        };
        assert!(
            width_pct < 0.001,
            "flat BB width should be ~0, got {width_pct:.6}"
        );
    }

    #[test]
    fn bollinger_middle_is_sma20() {
        // close[i] = 100 + i for i in 0..50.  Last 20 closes: [130..149], mean = 139.5
        let closes: Vec<f64> = (0..50).map(|i| 100.0 + i as f64).collect();
        let (_, middle, _) = calc_bollinger(&closes, 20, 2.0);
        assert!(
            (middle - 139.5).abs() < 0.01,
            "Bollinger middle should be 139.5, got {middle:.4}"
        );
    }

    #[test]
    fn bollinger_upper_lower_symmetric() {
        let closes: Vec<f64> = (0..30).map(|i| 100.0 + i as f64).collect();
        let (upper, middle, lower) = calc_bollinger(&closes, 20, 2.0);
        let diff_up = upper - middle;
        let diff_down = middle - lower;
        assert!(
            (diff_up - diff_down).abs() < 1e-8,
            "Bollinger bands not symmetric: up={diff_up:.8} down={diff_down:.8}"
        );
    }

    // ── MACD ──────────────────────────────────────────────────────────────────

    #[test]
    fn macd_flat_price_is_zero() {
        let closes = vec![100.0f64; 50];
        let (macd, _, _) = calc_macd_proper(&closes);
        assert!(macd.abs() < 1e-6, "flat MACD should be ~0, got {macd:.8}");
    }

    #[test]
    fn macd_rising_is_positive() {
        let closes: Vec<f64> = (0..50).map(|i| 100.0 + i as f64).collect();
        let (macd, _, _) = calc_macd_proper(&closes);
        assert!(macd > 0.0, "rising MACD should be positive, got {macd:.6}");
    }

    #[test]
    fn macd_falling_is_negative() {
        let closes: Vec<f64> = (0..50).map(|i| 200.0 - i as f64).collect();
        let (macd, _, _) = calc_macd_proper(&closes);
        assert!(macd < 0.0, "falling MACD should be negative, got {macd:.6}");
    }

    #[test]
    fn macd_histogram_invariant() {
        // histogram must always equal macd_line - signal_line
        let closes: Vec<f64> = (0..50).map(|i| 100.0 + i as f64 * 0.5).collect();
        let (line, signal, hist) = calc_macd_proper(&closes);
        assert!(
            (hist - (line - signal)).abs() < 1e-10,
            "histogram {hist:.10} ≠ line - signal {:.10}",
            line - signal
        );
    }

    // ── ATR ───────────────────────────────────────────────────────────────────

    #[test]
    fn atr_flat_price_is_zero() {
        let n = 30;
        let highs = vec![100.0f64; n];
        let lows = vec![100.0f64; n];
        let closes = vec![100.0f64; n];
        let atr = calc_atr_wilder(&highs, &lows, &closes, 14);
        assert!(atr < 1e-6, "flat ATR should be ~0, got {atr:.8}");
    }

    #[test]
    fn atr_is_positive_for_volatile_series() {
        let candles = rising_candles(100.0, 2.0, 50);
        let h: Vec<f64> = candles.iter().map(|c| c.high).collect();
        let l: Vec<f64> = candles.iter().map(|c| c.low).collect();
        let c: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let atr = calc_atr_wilder(&h, &l, &c, 14);
        assert!(atr > 0.0, "volatile ATR should be positive, got {atr:.4}");
    }

    #[test]
    fn atr_approximate_for_known_series() {
        // rising_candles step=1 → high=close+0.2, low=close-0.2
        // TR = max(high-low=0.4, |high-prev_close|=1.2, |low-prev_close|=0.8) = 1.2 per bar
        let candles = rising_candles(100.0, 1.0, 50);
        let h: Vec<f64> = candles.iter().map(|c| c.high).collect();
        let l: Vec<f64> = candles.iter().map(|c| c.low).collect();
        let c: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let atr = calc_atr_wilder(&h, &l, &c, 14);
        assert!(
            (atr - 1.2).abs() < 0.3,
            "ATR for step=1 should ≈ 1.2, got {atr:.4}"
        );
    }

    // ── ADX ───────────────────────────────────────────────────────────────────

    #[test]
    fn adx_always_in_bounds() {
        let candles = rising_candles(100.0, 1.0, 60);
        let h: Vec<f64> = candles.iter().map(|c| c.high).collect();
        let l: Vec<f64> = candles.iter().map(|c| c.low).collect();
        let c: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let adx = calc_adx(&h, &l, &c, 14);
        assert!((0.0..=100.0).contains(&adx), "ADX out of [0,100]: {adx:.4}");
    }

    #[test]
    fn adx_strong_uptrend_exceeds_threshold() {
        let candles = rising_candles(100.0, 2.0, 60);
        let h: Vec<f64> = candles.iter().map(|c| c.high).collect();
        let l: Vec<f64> = candles.iter().map(|c| c.low).collect();
        let c: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let adx = calc_adx(&h, &l, &c, 14);
        assert!(
            adx > 27.0,
            "strong uptrend ADX should be > 27, got {adx:.2}"
        );
    }

    // ── Z-score ───────────────────────────────────────────────────────────────

    #[test]
    fn z_score_flat_is_zero() {
        let closes = vec![100.0f64; 30];
        let z = calc_z_score(&closes, 20);
        assert!(z.abs() < 1e-6, "flat Z-score should be 0, got {z:.8}");
    }

    #[test]
    fn z_score_rising_is_positive() {
        let closes: Vec<f64> = (0..50).map(|i| 100.0 + i as f64).collect();
        let z = calc_z_score(&closes, 20);
        assert!(z > 0.0, "rising Z-score should be positive, got {z:.4}");
    }

    #[test]
    fn z_score_falling_is_negative() {
        let closes: Vec<f64> = (0..50).map(|i| 200.0 - i as f64).collect();
        let z = calc_z_score(&closes, 20);
        assert!(z < 0.0, "falling Z-score should be negative, got {z:.4}");
    }

    // ── ATR expansion ratio ───────────────────────────────────────────────────

    #[test]
    fn atr_expansion_flat_returns_one() {
        // flat series → TR=0 everywhere → hist_mean < ε → returns 1.0 guard
        let candles = flat_candles(100.0, 60);
        let h: Vec<f64> = candles.iter().map(|c| c.high).collect();
        let l: Vec<f64> = candles.iter().map(|c| c.low).collect();
        let c: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let ratio = calc_atr_expansion_ratio(&h, &l, &c, 14, 24);
        assert!(
            (ratio - 1.0).abs() < 0.01,
            "flat expansion ratio should be 1.0, got {ratio:.4}"
        );
    }

    #[test]
    fn atr_expansion_spike_exceeds_1_5() {
        // 40 quiet bars then 14 volatile bars → ratio should exceed 1.5
        let mut highs = vec![100.1f64; 40];
        let mut lows = vec![99.9f64; 40];
        let mut closes = vec![100.0f64; 40];
        for _ in 0..14 {
            highs.push(105.0);
            lows.push(95.0);
            closes.push(100.0);
        }
        let ratio = calc_atr_expansion_ratio(&highs, &lows, &closes, 14, 24);
        assert!(
            ratio > 1.5,
            "post-spike expansion ratio should exceed 1.5, got {ratio:.4}"
        );
    }

    // ── calculate_all guards ──────────────────────────────────────────────────

    #[test]
    fn calculate_all_requires_26_candles() {
        let candles = flat_candles(100.0, 25);
        assert!(
            calculate_all(&candles).is_err(),
            "should fail with < 26 candles"
        );
    }

    #[test]
    fn calculate_all_succeeds_at_26_candles() {
        let candles = rising_candles(100.0, 1.0, 26);
        assert!(
            calculate_all(&candles).is_ok(),
            "should succeed with exactly 26 candles"
        );
    }

    #[test]
    fn calculate_all_all_fields_finite() {
        let candles = rising_candles(100.0, 0.5, 200);
        let ind = calculate_all(&candles).unwrap();
        assert!(ind.rsi.is_finite());
        assert!(ind.atr.is_finite());
        assert!(ind.adx.is_finite());
        assert!(ind.macd.is_finite());
        assert!(ind.z_score.is_finite());
        assert!(ind.vwap.is_finite());
        assert!(ind.atr_expansion_ratio.is_finite());
    }

    #[test]
    fn calculate_all_zero_volume_no_panic() {
        let mut candles = rising_candles(100.0, 1.0, 50);
        for c in candles.iter_mut() {
            c.volume = 0.0;
        }
        assert!(
            calculate_all(&candles).is_ok(),
            "zero-volume candles should not panic"
        );
    }

    // ── HtfIndicators ────────────────────────────────────────────────────────

    #[test]
    fn htf_defaults_on_short_series() {
        let candles = flat_candles(100.0, 10);
        let htf = calculate_htf(&candles);
        assert_eq!(
            htf.rsi_4h, 50.0,
            "htf short-series rsi should default to 50"
        );
        assert_eq!(
            htf.z_score_4h, 0.0,
            "htf short-series z should default to 0"
        );
    }

    #[test]
    fn htf_rising_gives_high_rsi_positive_z() {
        let candles = rising_candles(100.0, 2.0, 40);
        let htf = calculate_htf(&candles);
        assert!(
            htf.rsi_4h > 50.0,
            "rising htf RSI should be > 50, got {:.2}",
            htf.rsi_4h
        );
        assert!(
            htf.z_score_4h > 0.0,
            "rising htf Z-score should be positive, got {:.4}",
            htf.z_score_4h
        );
    }

    #[test]
    fn htf_falling_gives_low_rsi_negative_z() {
        let candles = falling_candles(200.0, 2.0, 40);
        let htf = calculate_htf(&candles);
        assert!(
            htf.rsi_4h < 50.0,
            "falling htf RSI should be < 50, got {:.2}",
            htf.rsi_4h
        );
        assert!(
            htf.z_score_4h < 0.0,
            "falling htf Z-score should be negative, got {:.4}",
            htf.z_score_4h
        );
    }

    // ── EMA cross ────────────────────────────────────────────────────────────

    #[test]
    fn ema_cross_positive_in_uptrend() {
        let candles = rising_candles(100.0, 1.0, 50);
        let ind = calculate_all(&candles).unwrap();
        assert!(
            ind.ema8 > ind.ema21,
            "uptrend: EMA8 ({:.4}) should > EMA21 ({:.4})",
            ind.ema8,
            ind.ema21
        );
        assert!(
            ind.ema_cross_pct > 0.0,
            "uptrend ema_cross_pct should be positive, got {:.4}",
            ind.ema_cross_pct
        );
    }

    #[test]
    fn ema_cross_negative_in_downtrend() {
        let candles = falling_candles(200.0, 1.0, 50);
        let ind = calculate_all(&candles).unwrap();
        assert!(
            ind.ema8 < ind.ema21,
            "downtrend: EMA8 ({:.4}) should < EMA21 ({:.4})",
            ind.ema8,
            ind.ema21
        );
        assert!(
            ind.ema_cross_pct < 0.0,
            "downtrend ema_cross_pct should be negative, got {:.4}",
            ind.ema_cross_pct
        );
    }
}
