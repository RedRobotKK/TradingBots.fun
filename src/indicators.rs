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

use anyhow::Result;
use serde::{Deserialize, Serialize};
use crate::data::PriceData;

// ─────────────────────────── Output struct ───────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TechnicalIndicators {
    // ── Core mean-reversion ───────────────────────────────────────────────────
    /// Wilder's RSI(14). <30 = oversold, >70 = overbought.
    pub rsi:              f64,
    /// Bollinger Band upper  (20-bar, 2σ)
    pub bollinger_upper:  f64,
    /// Bollinger Band middle (20-bar SMA — serves as VWAP proxy)
    pub bollinger_middle: f64,
    /// Bollinger Band lower  (20-bar, 2σ)
    pub bollinger_lower:  f64,
    /// Bollinger Band width as % of middle.  Low value = squeeze (breakout imminent).
    pub bb_width_pct:     f64,

    // ── Momentum ──────────────────────────────────────────────────────────────
    /// MACD line = EMA(12) − EMA(26)
    pub macd:             f64,
    /// Signal line = EMA(9) of the MACD series  (proper calculation, not approximation)
    pub macd_signal:      f64,
    /// Histogram = MACD − signal.  Rising = accelerating momentum.
    pub macd_histogram:   f64,

    // ── Volatility / stop placement ───────────────────────────────────────────
    /// ATR(14) — Wilder's Average True Range.  Used for all stop distances.
    pub atr:              f64,

    // ── Trend / regime ────────────────────────────────────────────────────────
    /// Legacy: % price change over last 10 bars.
    pub trend:            f64,
    /// EMA(8) — fast trend line.
    pub ema8:             f64,
    /// EMA(21) — slow trend line.
    pub ema21:            f64,
    /// (ema8 − ema21) / ema21 × 100.  Positive = bull trend confirmed.
    pub ema_cross_pct:    f64,
    /// ADX(14).  0–100 scale.  >27 = trending market, <19 = ranging.
    pub adx:              f64,

    // ── Mean-reversion depth ──────────────────────────────────────────────────
    /// Z-score of close vs 20-bar rolling mean.  |>2.0| = statistically extreme.
    pub z_score:          f64,

    // ── Volume / conviction ───────────────────────────────────────────────────
    /// Current bar volume ÷ 20-bar average.  >1.5 = high-conviction bar.
    pub volume_ratio:     f64,
    /// 24-bar VWAP (institutional benchmark price).
    pub vwap:             f64,
    /// (close − vwap) / vwap × 100.  Positive = price trading above VWAP (bull bias).
    pub vwap_pct:         f64,
}

// ─────────────────────────── Public entry point ──────────────────────────────

/// Calculate all indicators from a slice of OHLCV candles (newest last).
/// Requires ≥ 26 candles; accuracy improves significantly with 50+.
pub fn calculate_all(candles: &[PriceData]) -> Result<TechnicalIndicators> {
    if candles.len() < 26 {
        anyhow::bail!("Need at least 26 candles for indicators, got {}", candles.len());
    }

    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let highs:  Vec<f64> = candles.iter().map(|c| c.high).collect();
    let lows:   Vec<f64> = candles.iter().map(|c| c.low).collect();

    let rsi                        = calc_rsi_wilder(&closes, 14);
    let (upper, middle, lower)     = calc_bollinger(&closes, 20, 2.0);
    let bb_width_pct               = if middle > 0.0 { (upper - lower) / middle * 100.0 } else { 0.0 };
    let (macd_line, macd_sig, hist) = calc_macd_proper(&closes);
    let atr                        = calc_atr_wilder(&highs, &lows, &closes, 14);
    let trend                      = calc_trend(&closes, 10);
    let ema8                       = ema_last(&closes, 8);
    let ema21                      = ema_last(&closes, 21);
    let ema_cross_pct              = if ema21 > 0.0 { (ema8 - ema21) / ema21 * 100.0 } else { 0.0 };
    let adx                        = calc_adx(&highs, &lows, &closes, 14);
    let z_score                    = calc_z_score(&closes, 20);
    let volume_ratio               = calc_volume_ratio(candles, 20);
    let (vwap, vwap_pct)           = calc_vwap(candles, &closes);

    Ok(TechnicalIndicators {
        rsi,
        bollinger_upper:  upper,
        bollinger_middle: middle,
        bollinger_lower:  lower,
        bb_width_pct,
        macd:             macd_line,
        macd_signal:      macd_sig,
        macd_histogram:   hist,
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
        if d > 0.0 { avg_gain += d; } else { avg_loss += d.abs(); }
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

    if avg_loss < 1e-12 { return 100.0; }
    let rs = avg_gain / avg_loss;
    100.0 - (100.0 / (1.0 + rs))
}

/// Bollinger Bands: SMA(period) ± mult × σ.
fn calc_bollinger(closes: &[f64], period: usize, mult: f64) -> (f64, f64, f64) {
    if closes.len() < period {
        let p = *closes.last().unwrap_or(&0.0);
        return (p * 1.02, p, p * 0.98);
    }
    let n     = closes.len();
    let slice = &closes[n - period..];
    let mean  = slice.iter().sum::<f64>() / period as f64;
    let var   = slice.iter().map(|&x| (x - mean).powi(2)).sum::<f64>() / period as f64;
    let std   = var.sqrt();
    (mean + mult * std, mean, mean - mult * std)
}

/// Full MACD(12, 26, 9) with a proper EMA-9 signal line.
///
/// Iterates over all candles to build the MACD series then smooths the last
/// 9+ values into the signal line.  Much more accurate than the old 0.80 approx.
fn calc_macd_proper(closes: &[f64]) -> (f64, f64, f64) {
    let n = closes.len();
    if n < 26 { return (0.0, 0.0, 0.0); }

    let k12 = 2.0 / 13.0;
    let k26 = 2.0 / 27.0;
    let k9  = 2.0 / 10.0;

    let mut e12  = closes[0];
    let mut e26  = closes[0];
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
    if n < 2 { return 0.0; }

    let trs: Vec<f64> = (1..n)
        .map(|i| {
            f64::max(
                highs[i] - lows[i],
                f64::max((highs[i] - closes[i-1]).abs(), (lows[i] - closes[i-1]).abs()),
            )
        })
        .collect();

    if trs.len() < period { return trs.iter().sum::<f64>() / trs.len() as f64; }

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
    if n <= period { return 0.0; }
    let old = closes[n - period - 1];
    if old == 0.0 { return 0.0; }
    (closes[n - 1] - old) / old * 100.0
}

/// Exponential Moving Average — single final value.
/// Seeded from prices[0], smoothed left-to-right.
pub fn ema_last(prices: &[f64], period: usize) -> f64 {
    if prices.is_empty() { return 0.0; }
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
///   20–27 — weakening or building trend (use balanced approach)
///   < 19  — ranging / choppy (use mean reversion strategies)
fn calc_adx(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> f64 {
    let n = highs.len();
    // Need at least 2×period bars for reliable ADX
    if n < period * 2 + 2 { return 25.0; }

    // --- Step 1: True Range, +DM, -DM at each bar ---
    let mut trs:       Vec<f64> = Vec::with_capacity(n - 1);
    let mut plus_dms:  Vec<f64> = Vec::with_capacity(n - 1);
    let mut minus_dms: Vec<f64> = Vec::with_capacity(n - 1);

    for i in 1..n {
        let tr       = f64::max(highs[i] - lows[i],
                       f64::max((highs[i] - closes[i-1]).abs(),
                                (lows[i]  - closes[i-1]).abs()));
        let up_move  = highs[i] - highs[i-1];
        let dn_move  = lows[i-1] - lows[i];
        let plus_dm  = if up_move > dn_move && up_move > 0.0 { up_move } else { 0.0 };
        let minus_dm = if dn_move > up_move && dn_move > 0.0 { dn_move } else { 0.0 };
        trs.push(tr);
        plus_dms.push(plus_dm);
        minus_dms.push(minus_dm);
    }

    let m = trs.len();
    if m < period { return 25.0; }

    // --- Step 2: Seed initial Wilder's sums ---
    let mut atr_s = trs[..period].iter().sum::<f64>();
    let mut pdm_s = plus_dms[..period].iter().sum::<f64>();
    let mut mdm_s = minus_dms[..period].iter().sum::<f64>();

    let mut dx_vals: Vec<f64> = Vec::new();

    // First DX from the seeded sums
    {
        let pdi = if atr_s > 0.0 { 100.0 * pdm_s / atr_s } else { 0.0 };
        let mdi = if atr_s > 0.0 { 100.0 * mdm_s / atr_s } else { 0.0 };
        let dx  = if pdi + mdi > 0.0 { 100.0 * (pdi - mdi).abs() / (pdi + mdi) } else { 0.0 };
        dx_vals.push(dx);
    }

    // --- Step 3: Roll Wilder's smoothing across remaining bars ---
    for i in period..m {
        atr_s = atr_s - atr_s / period as f64 + trs[i];
        pdm_s = pdm_s - pdm_s / period as f64 + plus_dms[i];
        mdm_s = mdm_s - mdm_s / period as f64 + minus_dms[i];

        let pdi = if atr_s > 0.0 { 100.0 * pdm_s / atr_s } else { 0.0 };
        let mdi = if atr_s > 0.0 { 100.0 * mdm_s / atr_s } else { 0.0 };
        let dx  = if pdi + mdi > 0.0 { 100.0 * (pdi - mdi).abs() / (pdi + mdi) } else { 0.0 };
        dx_vals.push(dx);
    }

    // --- Step 4: ADX = Wilder's smooth of DX ---
    if dx_vals.len() < period { return 25.0; }

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
    if n < period { return 0.0; }
    let slice  = &closes[n - period..];
    let mean   = slice.iter().sum::<f64>() / period as f64;
    let var    = slice.iter().map(|&x| (x - mean).powi(2)).sum::<f64>() / period as f64;
    let stddev = var.sqrt();
    if stddev < 1e-12 { return 0.0; }
    (closes[n - 1] - mean) / stddev
}

/// Current bar's volume relative to its 20-bar simple moving average.
///
/// > 1.5 = high conviction;  < 0.6 = low conviction (weak signal).
fn calc_volume_ratio(candles: &[PriceData], period: usize) -> f64 {
    let n = candles.len();
    if n < period + 1 { return 1.0; }
    // Average of the `period` bars BEFORE the most recent one
    let avg_vol = candles[n - period - 1..n - 1]
        .iter()
        .map(|c| c.volume)
        .sum::<f64>()
        / period as f64;
    let cur_vol = candles[n - 1].volume;
    if avg_vol > 0.0 { (cur_vol / avg_vol).clamp(0.1, 5.0) } else { 1.0 }
}

/// 24-bar VWAP (typical price × volume, averaged) and the current price's
/// deviation from it as a percentage.
fn calc_vwap(candles: &[PriceData], closes: &[f64]) -> (f64, f64) {
    let n       = candles.len();
    let lookback = 24_usize.min(n);
    let start   = n - lookback;

    let (sum_pv, sum_v) = candles[start..].iter().fold((0.0f64, 0.0f64), |(spv, sv), c| {
        let typical = (c.high + c.low + c.close) / 3.0;
        (spv + typical * c.volume, sv + c.volume)
    });

    let vwap = if sum_v > 0.0 { sum_pv / sum_v } else { *closes.last().unwrap_or(&0.0) };
    let vwap_pct = if vwap > 0.0 {
        (closes.last().unwrap_or(&vwap) - vwap) / vwap * 100.0
    } else {
        0.0
    };

    (vwap, vwap_pct)
}
