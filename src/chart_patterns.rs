//! Chart pattern recognition across 20–60 candle windows.
//!
//! Covers all patterns from both reference images:
//!
//! **Reversal patterns** (structural):
//!   Double Top / Double Bottom, Triple Top / Triple Bottom,
//!   Head & Shoulders, Inverse H&S, Cup & Handle, Inv Cup & Handle
//!
//! **Continuation patterns** (structural):
//!   Ascending Triangle, Descending Triangle, Symmetrical Triangle,
//!   Rising Wedge (bearish), Falling Wedge (bullish),
//!   Bull Flag, Bear Flag, Bull Pennant, Bear Pennant,
//!   Bull Rectangle, Bear Rectangle
//!
//! **Institutional price-action patterns**:
//!   Compression → Expansion, Liquidity Grab (Stop Hunt), V-Flash Reversal
//!
//! Returns a `ChartSignal` with bull/bear boosts capped at 0.12.

use crate::data::PriceData;

/// Signal returned by `detect()`.
pub struct ChartSignal {
    /// Additive bull contribution (0.0–0.12).
    pub bull_boost: f64,
    /// Additive bear contribution (0.0–0.12).
    pub bear_boost: f64,
    /// Human-readable pattern names joined with "+" for the rationale string.
    pub name: Option<String>,
}

// ─── Swing-point type alias ──────────────────────────────────────────────────
type SwingPoint = (f64, usize); // (price, bar_index)

/// Detect chart-level patterns from the candle history.
pub fn detect(candles: &[PriceData]) -> ChartSignal {
    let n = candles.len();
    if n < 10 {
        return ChartSignal { bull_boost: 0.0, bear_boost: 0.0, name: None };
    }

    let mut bull: f64 = 0.0;
    let mut bear: f64 = 0.0;
    let mut names: Vec<String> = Vec::new();

    // Extract swing highs and lows (3-bar pivot confirmation)
    let (swing_highs, swing_lows) = find_swing_points(candles, 3);

    // ════════════════════════════════════════════════════════════════════════
    //  REVERSAL PATTERNS
    // ════════════════════════════════════════════════════════════════════════

    // Double Top
    if let Some(b) = detect_double_top(&swing_highs, candles) {
        bear += b;
        names.push("Double Top".to_string());
    }
    // Double Bottom
    if let Some(b) = detect_double_bottom(&swing_lows, candles) {
        bull += b;
        names.push("Double Bottom".to_string());
    }
    // Triple Top
    if let Some(b) = detect_triple_top(&swing_highs) {
        bear += b;
        names.push("Triple Top".to_string());
    }
    // Triple Bottom
    if let Some(b) = detect_triple_bottom(&swing_lows) {
        bull += b;
        names.push("Triple Bottom".to_string());
    }
    // Head & Shoulders (bearish)
    if let Some(b) = detect_head_and_shoulders(&swing_highs, &swing_lows) {
        bear += b;
        names.push("H&S".to_string());
    }
    // Inverse H&S (bullish)
    if let Some(b) = detect_inverse_head_and_shoulders(&swing_lows, &swing_highs) {
        bull += b;
        names.push("Inv H&S".to_string());
    }
    // Cup & Handle (bullish)
    if let Some(b) = detect_cup_and_handle(candles) {
        bull += b;
        names.push("Cup&Handle".to_string());
    }
    // Inverted Cup & Handle (bearish)
    if let Some(b) = detect_inverted_cup_and_handle(candles) {
        bear += b;
        names.push("Inv Cup&Handle".to_string());
    }

    // ════════════════════════════════════════════════════════════════════════
    //  CONTINUATION PATTERNS
    // ════════════════════════════════════════════════════════════════════════

    // Wedge patterns
    match detect_wedge(&swing_highs, &swing_lows) {
        Some(("rising",  b)) => { bear += b; names.push("Rising Wedge".to_string()); }
        Some(("falling", b)) => { bull += b; names.push("Falling Wedge".to_string()); }
        _ => {}
    }

    // Triangle patterns
    match detect_triangle(candles, &swing_highs, &swing_lows) {
        Some(("ascending",        b)) => { bull += b; names.push("Asc Triangle".to_string()); }
        Some(("descending",       b)) => { bear += b; names.push("Desc Triangle".to_string()); }
        Some(("symmetrical_bull", b)) => { bull += b; names.push("Sym Triangle↑".to_string()); }
        Some(("symmetrical_bear", b)) => { bear += b; names.push("Sym Triangle↓".to_string()); }
        _ => {}
    }

    // Flag / Pennant / Rectangle patterns
    match detect_flag_pennant_rectangle(candles) {
        Some(("bull_flag",      b)) => { bull += b; names.push("Bull Flag".to_string()); }
        Some(("bear_flag",      b)) => { bear += b; names.push("Bear Flag".to_string()); }
        Some(("bull_pennant",   b)) => { bull += b; names.push("Bull Pennant".to_string()); }
        Some(("bear_pennant",   b)) => { bear += b; names.push("Bear Pennant".to_string()); }
        Some(("bull_rectangle", b)) => { bull += b; names.push("Bull Rectangle".to_string()); }
        Some(("bear_rectangle", b)) => { bear += b; names.push("Bear Rectangle".to_string()); }
        _ => {}
    }

    // ════════════════════════════════════════════════════════════════════════
    //  INSTITUTIONAL PRICE-ACTION PATTERNS
    // ════════════════════════════════════════════════════════════════════════

    // Compression → Expansion (consolidation breakout)
    match detect_compression_expansion(candles) {
        Some(("bull", b)) => { bull += b; names.push("Compression↗".to_string()); }
        Some(("bear", b)) => { bear += b; names.push("Compression↘".to_string()); }
        _ => {}
    }

    // Liquidity Grab / Stop Hunt
    match detect_liquidity_grab(candles) {
        Some(("bull", b)) => { bull += b; names.push("Liq Grab↑".to_string()); }
        Some(("bear", b)) => { bear += b; names.push("Liq Grab↓".to_string()); }
        _ => {}
    }

    // V-Flash Reversal
    match detect_v_flash(candles) {
        Some(("bull", b)) => { bull += b; names.push("V-Flash↑".to_string()); }
        Some(("bear", b)) => { bear += b; names.push("V-Flash↓".to_string()); }
        _ => {}
    }

    let name = if names.is_empty() { None } else { Some(names.join("+")) };

    ChartSignal {
        bull_boost: bull.min(0.12),
        bear_boost: bear.min(0.12),
        name,
    }
}

// ════════════════════════════════════════════════════════════════════════════
//  SWING POINT EXTRACTION
// ════════════════════════════════════════════════════════════════════════════

fn find_swing_points(candles: &[PriceData], pivot: usize) -> (Vec<SwingPoint>, Vec<SwingPoint>) {
    let mut highs: Vec<SwingPoint> = Vec::new();
    let mut lows:  Vec<SwingPoint> = Vec::new();
    let n = candles.len();
    if n < 2 * pivot + 1 {
        return (highs, lows);
    }
    for i in pivot..(n - pivot) {
        let hi = candles[i].high;
        let lo = candles[i].low;
        let is_sh = (1..=pivot).all(|j| candles[i - j].high < hi)
                 && (1..=pivot).all(|j| candles[i + j].high < hi);
        let is_sl = (1..=pivot).all(|j| candles[i - j].low > lo)
                 && (1..=pivot).all(|j| candles[i + j].low > lo);
        if is_sh { highs.push((hi, i)); }
        if is_sl { lows.push((lo,  i)); }
    }
    (highs, lows)
}

// ════════════════════════════════════════════════════════════════════════════
//  REVERSAL PATTERN DETECTORS
// ════════════════════════════════════════════════════════════════════════════

fn detect_double_top(highs: &[SwingPoint], candles: &[PriceData]) -> Option<f64> {
    if highs.len() < 2 { return None; }
    let n = highs.len();
    let (h1, i1) = highs[n - 1];
    let (h0, i0) = highs[n - 2];
    let sep = i1.saturating_sub(i0);
    if sep < 5 { return None; }
    // Two highs within 1.5% of each other
    if (h1 - h0).abs() / h0.max(1e-10) * 100.0 > 1.5 { return None; }
    // Price has since broken below the trough between the peaks
    let trough = candles[i0..=i1].iter().map(|c| c.low).fold(f64::MAX, f64::min);
    let cur = candles.last()?.close;
    if cur < trough { Some(0.10) } else { None }
}

fn detect_double_bottom(lows: &[SwingPoint], candles: &[PriceData]) -> Option<f64> {
    if lows.len() < 2 { return None; }
    let n = lows.len();
    let (l1, i1) = lows[n - 1];
    let (l0, i0) = lows[n - 2];
    let sep = i1.saturating_sub(i0);
    if sep < 5 { return None; }
    if (l1 - l0).abs() / l0.max(1e-10) * 100.0 > 1.5 { return None; }
    let peak = candles[i0..=i1].iter().map(|c| c.high).fold(f64::MIN, f64::max);
    let cur  = candles.last()?.close;
    if cur > peak { Some(0.10) } else { None }
}

fn detect_triple_top(highs: &[SwingPoint]) -> Option<f64> {
    if highs.len() < 3 { return None; }
    let n = highs.len();
    let (h2, _) = highs[n - 1];
    let (h1, _) = highs[n - 2];
    let (h0, _) = highs[n - 3];
    let avg = (h0 + h1 + h2) / 3.0;
    if avg < 1e-10 { return None; }
    // All three highs within 1.5% of their average
    let max_dev = [(h0 - avg).abs(), (h1 - avg).abs(), (h2 - avg).abs()]
        .iter().cloned().fold(0.0f64, f64::max);
    if max_dev / avg * 100.0 < 1.5 { Some(0.11) } else { None }
}

fn detect_triple_bottom(lows: &[SwingPoint]) -> Option<f64> {
    if lows.len() < 3 { return None; }
    let n = lows.len();
    let (l2, _) = lows[n - 1];
    let (l1, _) = lows[n - 2];
    let (l0, _) = lows[n - 3];
    let avg = (l0 + l1 + l2) / 3.0;
    if avg < 1e-10 { return None; }
    let max_dev = [(l0 - avg).abs(), (l1 - avg).abs(), (l2 - avg).abs()]
        .iter().cloned().fold(0.0f64, f64::max);
    if max_dev / avg * 100.0 < 1.5 { Some(0.11) } else { None }
}

fn detect_head_and_shoulders(
    highs: &[SwingPoint],
    _lows: &[SwingPoint],
) -> Option<f64> {
    if highs.len() < 3 { return None; }
    let n = highs.len();
    let (left,  _) = highs[n - 3];
    let (head,  _) = highs[n - 2];
    let (right, _) = highs[n - 1];
    // Head must be the highest
    if head <= left || head <= right { return None; }
    // Shoulders within 5% of each other
    if left < 1e-10 { return None; }
    if (left - right).abs() / left * 100.0 > 5.0 { return None; }
    // Head must be meaningfully higher than shoulders (≥ 2%)
    if (head - left.max(right)) / left * 100.0 < 2.0 { return None; }
    Some(0.11)
}

fn detect_inverse_head_and_shoulders(
    lows:  &[SwingPoint],
    _highs: &[SwingPoint],
) -> Option<f64> {
    if lows.len() < 3 { return None; }
    let n = lows.len();
    let (left,  _) = lows[n - 3];
    let (head,  _) = lows[n - 2];
    let (right, _) = lows[n - 1];
    if head >= left || head >= right { return None; }
    if left < 1e-10 { return None; }
    if (left - right).abs() / left * 100.0 > 5.0 { return None; }
    if (left.min(right) - head) / left * 100.0 < 2.0 { return None; }
    Some(0.11)
}

fn detect_cup_and_handle(candles: &[PriceData]) -> Option<f64> {
    let n = candles.len();
    if n < 30 { return None; }
    let left_rim  = candles[n - 30].high;
    let right_rim = candles[n - 5..].iter().map(|c| c.high).fold(f64::MIN, f64::max);
    let cup_low   = candles[n - 30..n - 5].iter().map(|c| c.low).fold(f64::MAX, f64::min);
    if left_rim < 1e-10 { return None; }
    let depth_pct = (left_rim - cup_low) / left_rim * 100.0;
    let rim_diff  = (right_rim - left_rim).abs() / left_rim * 100.0;
    // Cup: ≥5% depth, rims within 3%, right rim approaching left rim
    if depth_pct >= 5.0 && rim_diff < 3.0 && right_rim >= left_rim * 0.97 {
        Some(0.10)
    } else {
        None
    }
}

fn detect_inverted_cup_and_handle(candles: &[PriceData]) -> Option<f64> {
    let n = candles.len();
    if n < 30 { return None; }
    let left_floor  = candles[n - 30].low;
    let right_floor = candles[n - 5..].iter().map(|c| c.low).fold(f64::MAX, f64::min);
    let cup_high    = candles[n - 30..n - 5].iter().map(|c| c.high).fold(f64::MIN, f64::max);
    if left_floor < 1e-10 { return None; }
    let height_pct = (cup_high - left_floor) / left_floor * 100.0;
    let floor_diff = (right_floor - left_floor).abs() / left_floor * 100.0;
    if height_pct >= 5.0 && floor_diff < 3.0 && right_floor <= left_floor * 1.03 {
        Some(0.10)
    } else {
        None
    }
}

// ════════════════════════════════════════════════════════════════════════════
//  CONTINUATION PATTERN DETECTORS
// ════════════════════════════════════════════════════════════════════════════

fn detect_wedge(
    highs: &[SwingPoint],
    lows:  &[SwingPoint],
) -> Option<(&'static str, f64)> {
    if highs.len() < 2 || lows.len() < 2 { return None; }
    let (h1, _) = highs[highs.len() - 1];
    let (h0, _) = highs[highs.len() - 2];
    let (l1, _) = lows[lows.len() - 1];
    let (l0, _) = lows[lows.len() - 2];
    let high_slope = h1 - h0;
    let low_slope  = l1 - l0;
    // Both slopes same sign = wedge (as opposed to triangle)
    if high_slope.signum() != low_slope.signum() { return None; }
    // Falling wedge: both declining, lows falling faster → converging → bullish
    if high_slope < 0.0 && low_slope < 0.0 && low_slope < high_slope {
        return Some(("falling", 0.09));
    }
    // Rising wedge: both rising, highs rising faster → converging → bearish
    if high_slope > 0.0 && low_slope > 0.0 && high_slope > low_slope {
        return Some(("rising", 0.09));
    }
    None
}

fn detect_triangle(
    candles:      &[PriceData],
    highs:        &[SwingPoint],
    lows:         &[SwingPoint],
) -> Option<(&'static str, f64)> {
    if highs.len() < 2 || lows.len() < 2 { return None; }
    let (h1, _) = highs[highs.len() - 1];
    let (h0, _) = highs[highs.len() - 2];
    let (l1, _) = lows[lows.len()  - 1];
    let (l0, _) = lows[lows.len()  - 2];
    let flat_tol = h0.max(1e-10) * 0.005; // 0.5% tolerance for "flat"
    let high_flat    = (h1 - h0).abs() < flat_tol;
    let low_rising   = l1 > l0 + flat_tol;
    let low_flat     = (l1 - l0).abs() < l0.max(1e-10) * 0.005;
    let high_falling = h1 < h0 - flat_tol;
    let cur = candles.last()?.close;
    // Ascending triangle: flat resistance + rising support → bullish breakout
    if high_flat && low_rising {
        return Some(("ascending", 0.08));
    }
    // Descending triangle: flat support + falling resistance → bearish breakdown
    if low_flat && high_falling {
        return Some(("descending", 0.08));
    }
    // Symmetrical: both converging — bias from current price vs mid of triangle
    if high_falling && low_rising {
        let mid = (h0 + l0) / 2.0;
        return if cur > mid {
            Some(("symmetrical_bull", 0.07))
        } else {
            Some(("symmetrical_bear", 0.07))
        };
    }
    None
}

/// Detect flag, pennant, and rectangle patterns.
///
/// Method: split last N bars into "pole" (first half) and "consolidation"
/// (second half).  The pole characterises the prior trend; the consolidation
/// shape distinguishes the sub-type.
fn detect_flag_pennant_rectangle(candles: &[PriceData]) -> Option<(&'static str, f64)> {
    let n = candles.len();
    if n < 20 { return None; }
    let pole = &candles[n - 20..n - 10];
    let cons = &candles[n - 10..];
    if pole.is_empty() || cons.is_empty() { return None; }
    let pole_open  = pole.first()?.open;
    let pole_close = pole.last()?.close;
    if pole_open < 1e-10 { return None; }
    let pole_move_pct = (pole_close - pole_open) / pole_open * 100.0;
    // Need a meaningful pole (≥ 2.5% move) to form any of these patterns
    if pole_move_pct.abs() < 2.5 { return None; }

    // Consolidation metrics
    let cons_high = cons.iter().map(|c| c.high).fold(f64::MIN, f64::max);
    let cons_low  = cons.iter().map(|c| c.low ).fold(f64::MAX, f64::min);
    let cons_range = (cons_high - cons_low).max(1e-10);
    let cons_open  = cons.first()?.open;
    let cons_close = cons.last()?.close;
    let cons_slope = (cons_close - cons_open) / cons_open.max(1e-10) * 100.0;

    // Pole range for relative comparisons
    let pole_range = pole.iter().map(|c| c.high - c.low).fold(0.0f64, f64::max);

    // Rectangle: consolidation range < 1.5% AND slope near flat (both bull/bear)
    if cons_range / cons_open.max(1e-10) * 100.0 < 1.5 && cons_slope.abs() < 0.5 {
        return if pole_move_pct > 0.0 {
            Some(("bull_rectangle", 0.07))
        } else {
            Some(("bear_rectangle", 0.07))
        };
    }

    // Pennant: consolidation narrows (range < 60% of pole range) AND has opposing slope
    if cons_range < pole_range * 0.60 {
        if pole_move_pct > 0.0 && cons_slope < 0.0 {
            return Some(("bull_pennant", 0.08));
        }
        if pole_move_pct < 0.0 && cons_slope > 0.0 {
            return Some(("bear_pennant", 0.08));
        }
    }

    // Flag: consolidation moves against the pole, retracement ≤ 50% of pole
    let retrace = -(cons_slope / pole_move_pct.abs());
    if pole_move_pct > 0.0 && cons_slope < 0.0 && retrace < 0.50 {
        return Some(("bull_flag", 0.08));
    }
    if pole_move_pct < 0.0 && cons_slope > 0.0 && retrace < 0.50 {
        return Some(("bear_flag", 0.08));
    }
    None
}

// ════════════════════════════════════════════════════════════════════════════
//  INSTITUTIONAL PRICE-ACTION PATTERN DETECTORS
// ════════════════════════════════════════════════════════════════════════════

/// Compression → Expansion: range narrows over 12 bars, then expands sharply.
fn detect_compression_expansion(candles: &[PriceData]) -> Option<(&'static str, f64)> {
    let n = candles.len();
    if n < 15 { return None; }
    // Max single-bar range in the compression zone (bars n-12 to n-4)
    let comp_range = candles[n - 12..n - 3]
        .iter()
        .map(|c| c.high - c.low)
        .fold(0.0f64, f64::max);
    // Max range in the expansion zone (last 3 bars)
    let exp_range = candles[n - 3..]
        .iter()
        .map(|c| c.high - c.low)
        .fold(0.0f64, f64::max);
    if comp_range < 1e-10 || exp_range <= comp_range * 1.50 {
        return None;
    }
    // Direction from the expansion bar
    let last = candles.last()?;
    if last.close > candles[n - 3].open {
        Some(("bull", 0.08))
    } else {
        Some(("bear", 0.08))
    }
}

/// Liquidity Grab (Stop Hunt): price spikes through a recent swing extreme
/// then immediately reverses — trapping breakout traders.
fn detect_liquidity_grab(candles: &[PriceData]) -> Option<(&'static str, f64)> {
    let n = candles.len();
    if n < 10 { return None; }
    let lookback = &candles[n - 10..n - 1];
    let swing_low  = lookback.iter().map(|c| c.low ).fold(f64::MAX, f64::min);
    let swing_high = lookback.iter().map(|c| c.high).fold(f64::MIN, f64::max);
    let last = candles.last()?;
    let prev = &candles[n - 2];
    // Bullish grab: last bar spiked below swing low then closed back above it
    if last.low < swing_low * 0.9985
        && last.close > swing_low
        && last.close > prev.close
    {
        return Some(("bull", 0.09));
    }
    // Bearish grab: last bar spiked above swing high then closed back below it
    if last.high > swing_high * 1.0015
        && last.close < swing_high
        && last.close < prev.close
    {
        return Some(("bear", 0.09));
    }
    None
}

/// V-Flash Reversal: sharp 3-bar move in one direction followed by an
/// equally sharp 2–3 bar reversal that recovers the full move.
fn detect_v_flash(candles: &[PriceData]) -> Option<(&'static str, f64)> {
    let n = candles.len();
    if n < 6 { return None; }
    let c = &candles[n - 6..];
    // Closing prices for the window
    let cl: Vec<f64> = c.iter().map(|b| b.close).collect();
    if cl[0] < 1e-10 || cl[2] < 1e-10 { return None; }
    // Bull V-flash: sharp drop (bars 0→2) then sharp recovery (bars 2→5)
    let drop  = (cl[2] - cl[0]) / cl[0] * 100.0;   // should be negative
    let rally = (cl[5] - cl[2]) / cl[2] * 100.0;   // should be positive
    if drop < -1.5 && rally > 2.0 && cl[5] > cl[0] {
        return Some(("bull", 0.09));
    }
    // Bear V-flash: sharp rise then sharp dump
    let rise = (cl[2] - cl[0]) / cl[0] * 100.0;    // positive
    let dump = (cl[5] - cl[2]) / cl[2] * 100.0;    // negative
    if rise > 1.5 && dump < -2.0 && cl[5] < cl[0] {
        return Some(("bear", 0.09));
    }
    None
}
