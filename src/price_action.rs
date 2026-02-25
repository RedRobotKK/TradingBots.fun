//! 🔥 INSTITUTIONAL PRICE ACTION PATTERN RECOGNITION
//!
//! This module implements the professional trading patterns that institutions use
//! to move and trap the market. Based on liquidity, order flow, and trap zones.
//!
//! Key Patterns:
//! - Compression → Expansion (consolidation breakouts)
//! - Liquidity Grabs (stop hunts into real moves)
//! - QML Setups (Quick Market Liquidation)
//! - Supply/Demand Flips (institutional order zones)
//! - Fakeout Recognition (false breakouts)
//! - Flag Patterns (momentum continuations)
//! - Order Blocks (institutional order placement)
//! - Reversal Patterns (V-flash, Can-Can, etc.)

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Maximum number of candles to store for pattern analysis
const MAX_CANDLES: usize = 500;

/// Candle data for price action analysis
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Candle {
    pub timestamp: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// A detected institutional pattern
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceActionPattern {
    /// Pattern type (Compression, Liquidity Grab, QML, etc.)
    pub pattern_type: PatternType,

    /// Confidence score (0-100)
    pub confidence: f64,

    /// Entry price (where institutions likely entered)
    pub entry_price: f64,

    /// Stop loss zone
    pub stop_zone: f64,

    /// Target zone(s)
    pub targets: Vec<f64>,

    /// Candle index where pattern forms
    pub formation_candle: usize,

    /// Candle index where pattern triggers
    pub trigger_candle: Option<usize>,

    /// Whether pattern has triggered
    pub is_triggered: bool,

    /// Detailed explanation
    pub description: String,

    /// Risk/Reward ratio
    pub risk_reward_ratio: f64,

    /// Expected move distance
    pub expected_move: f64,
}

/// All supported institutional price action patterns
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PatternType {
    /// Consolidation into breakout - high probability moves
    CompressionExpansion,

    /// Liquidity grab + stop hunt into real move
    LiquidityGrab,

    /// Quick Market Liquidation - fast through support/resistance
    QMLSetup,

    /// Supply flips to demand zone - institutional re-entry
    SupplyDemandFlip,

    /// False breakout into opposite direction
    FakeoutFlagPattern,

    /// Order block - where large orders are placed
    OrderBlock,

    /// V-shaped reversal from capitulation
    VFlashReversal,

    /// Can-Can pattern - double fake then real move
    CanCanReversal,

    /// Flag consolidation before continuation
    FlagContinuation,

    /// Stop hunt that triggers liquidity
    StopHunt,

    /// Wedge pattern tightening into breakout
    WedgeBreakout,

    /// Expansion from previous compression zone
    ExpansionFromCompression,
}

impl PatternType {
    pub fn as_str(&self) -> &'static str {
        match self {
            PatternType::CompressionExpansion => "Compression→Expansion",
            PatternType::LiquidityGrab => "Liquidity Grab",
            PatternType::QMLSetup => "QML Setup",
            PatternType::SupplyDemandFlip => "Supply/Demand Flip",
            PatternType::FakeoutFlagPattern => "Fakeout+Flag",
            PatternType::OrderBlock => "Order Block",
            PatternType::VFlashReversal => "V-Flash Reversal",
            PatternType::CanCanReversal => "Can-Can Reversal",
            PatternType::FlagContinuation => "Flag Continuation",
            PatternType::StopHunt => "Stop Hunt",
            PatternType::WedgeBreakout => "Wedge Breakout",
            PatternType::ExpansionFromCompression => "Expansion from Compression",
        }
    }

    /// Get minimum confidence required for this pattern
    pub fn min_confidence(&self) -> f64 {
        match self {
            PatternType::CompressionExpansion => 70.0, // Very reliable
            PatternType::LiquidityGrab => 75.0,        // High accuracy
            PatternType::QMLSetup => 65.0,             // Moderate
            PatternType::SupplyDemandFlip => 70.0,     // Very reliable
            PatternType::FakeoutFlagPattern => 60.0,   // Lower, but high reward
            PatternType::OrderBlock => 65.0,           // Moderate to good
            PatternType::VFlashReversal => 68.0,       // Good
            PatternType::CanCanReversal => 72.0,       // Very good
            PatternType::FlagContinuation => 68.0,     // Good
            PatternType::StopHunt => 70.0,             // Very reliable
            PatternType::WedgeBreakout => 75.0,        // Very reliable
            PatternType::ExpansionFromCompression => 72.0, // Very reliable
        }
    }
}

/// Price Action Pattern Detector
pub struct PriceActionDetector {
    /// Candle history (FIFO)
    candles: VecDeque<Candle>,

    /// Last detected patterns
    patterns: Vec<PriceActionPattern>,

    /// Compression zones (support/resistance pairs)
    compression_zones: Vec<(f64, f64, usize)>, // (support, resistance, formation_candle)

    /// Previous swing highs and lows
    swing_highs: VecDeque<(f64, usize)>, // (price, candle_index)
    swing_lows: VecDeque<(f64, usize)>,  // (price, candle_index)
}

impl PriceActionDetector {
    pub fn new() -> Self {
        Self {
            candles: VecDeque::with_capacity(MAX_CANDLES),
            patterns: Vec::new(),
            compression_zones: Vec::new(),
            swing_highs: VecDeque::new(),
            swing_lows: VecDeque::new(),
        }
    }

    /// Add a new candle and detect patterns
    pub fn add_candle(&mut self, candle: Candle) -> Vec<PriceActionPattern> {
        self.candles.push_back(candle);
        if self.candles.len() > MAX_CANDLES {
            self.candles.pop_front();
        }

        self.patterns.clear();

        // Detect all pattern types
        self.detect_swing_points();
        self.detect_compression_expansion();
        self.detect_liquidity_grabs();
        self.detect_supply_demand_flips();
        self.detect_fakeout_patterns();
        self.detect_order_blocks();
        self.detect_reversals();
        self.detect_flag_patterns();
        self.detect_wedge_patterns();

        self.patterns.clone()
    }

    /// Detect swing highs and lows
    fn detect_swing_points(&mut self) {
        if self.candles.len() < 3 {
            return;
        }

        let n = self.candles.len();
        let current_idx = n - 1;
        let prev_idx = n - 2;
        let prev_prev_idx = n - 3;

        let candles = &self.candles;

        // Swing high: prev_prev < prev > current
        if candles[prev_prev_idx].high < candles[prev_idx].high
            && candles[prev_idx].high > candles[current_idx].high
        {
            self.swing_highs.push_back((candles[prev_idx].high, prev_idx));
            if self.swing_highs.len() > 20 {
                self.swing_highs.pop_front();
            }
        }

        // Swing low: prev_prev > prev < current
        if candles[prev_prev_idx].low > candles[prev_idx].low
            && candles[prev_idx].low < candles[current_idx].low
        {
            self.swing_lows.push_back((candles[prev_idx].low, prev_idx));
            if self.swing_lows.len() > 20 {
                self.swing_lows.pop_front();
            }
        }
    }

    /// Detect compression → expansion patterns
    fn detect_compression_expansion(&mut self) {
        if self.candles.len() < 10 {
            return;
        }

        let n = self.candles.len();
        let recent_candles = &self.candles
            .iter()
            .rev()
            .take(20)
            .collect::<Vec<_>>();

        // Look for narrowing range (compression)
        let range_20 = recent_candles[19].high - recent_candles[19].low;
        let range_10 = recent_candles[9].high - recent_candles[9].low;
        let range_5 = recent_candles[4].high - recent_candles[4].low;
        let range_1 = recent_candles[0].high - recent_candles[0].low;

        // Compression: each range smaller than previous
        let is_compressing = range_20 > range_10 && range_10 > range_5 && range_5 > range_1;

        if is_compressing {
            // Find support and resistance
            let support = recent_candles
                .iter()
                .map(|c| c.low)
                .fold(f64::INFINITY, f64::min);
            let resistance = recent_candles
                .iter()
                .map(|c| c.high)
                .fold(f64::NEG_INFINITY, f64::max);

            let compression_height = resistance - support;

            // Calculate expansion target (typically 1.5-2x the compression)
            let expansion_target_up = resistance + (compression_height * 1.5);
            let expansion_target_down = support - (compression_height * 1.5);

            // Check if we're breaking out
            let current = recent_candles[0];
            let is_breaking_up = current.close > resistance && current.volume > range_20;
            let is_breaking_down = current.close < support && current.volume > range_20;

            if is_breaking_up || is_breaking_down {
                let (entry_price, target, expected_move, direction) = if is_breaking_up {
                    (resistance, expansion_target_up, expansion_target_up - resistance, 1.0)
                } else {
                    (support, expansion_target_down, support - expansion_target_down, -1.0)
                };

                let confidence = 70.0
                    + (current.volume / (range_20 * 1000.0)).min(20.0) // Volume bonus
                    + (compression_height / (support.abs().max(1.0)) * 10.0).min(15.0); // Height bonus

                self.patterns.push(PriceActionPattern {
                    pattern_type: PatternType::CompressionExpansion,
                    confidence: confidence.min(95.0),
                    entry_price,
                    stop_zone: if direction > 0.0 { support } else { resistance },
                    targets: vec![target],
                    formation_candle: n - 20,
                    trigger_candle: Some(n - 1),
                    is_triggered: true,
                    description: format!(
                        "Compression over 20 candles breaking out. Entry: {:.2}, Target: {:.2}",
                        entry_price, target
                    ),
                    risk_reward_ratio: expected_move / compression_height,
                    expected_move,
                });

                // Store compression zone
                self.compression_zones
                    .push((support, resistance, n - 20));
            }
        }
    }

    /// Detect liquidity grab + stop hunt patterns
    fn detect_liquidity_grabs(&mut self) {
        if self.candles.len() < 5 {
            return;
        }

        let n = self.candles.len();
        let current = self.candles[n - 1];
        let prev = self.candles[n - 2];

        // Look for swing highs/lows that get taken out then reverse hard
        if let Some((swing_high, swing_candle)) = self.swing_highs.back() {
            let candles_since = n - 1 - swing_candle;
            if candles_since >= 3 && candles_since <= 10 {
                // Price above swing high (liquidity grabbed)
                if current.high > *swing_high && prev.close < *swing_high {
                    // Now reversing hard
                    let wick_above = current.high - swing_high;
                    let body_below = swing_high - current.close;

                    if body_below > wick_above * 0.5 {
                        let expected_move = body_below * 2.0;
                        let confidence = 75.0
                            + (wick_above / (*swing_high / 100.0)).min(15.0) // Wick size bonus
                            + if current.volume > prev.volume { 5.0 } else { 0.0 }; // Volume bonus

                        self.patterns.push(PriceActionPattern {
                            pattern_type: PatternType::LiquidityGrab,
                            confidence: confidence.min(95.0),
                            entry_price: current.close,
                            stop_zone: current.high,
                            targets: vec![swing_high - expected_move],
                            formation_candle: n - candles_since,
                            trigger_candle: Some(n - 1),
                            is_triggered: true,
                            description: format!(
                                "Liquidity grab at swing high {:.2}, reversing. Entry: {:.2}",
                                swing_high, current.close
                            ),
                            risk_reward_ratio: expected_move / wick_above,
                            expected_move,
                        });
                    }
                }
            }
        }

        // Same for swing lows
        if let Some((swing_low, swing_candle)) = self.swing_lows.back() {
            let candles_since = n - 1 - swing_candle;
            if candles_since >= 3 && candles_since <= 10 {
                if current.low < *swing_low && prev.close > *swing_low {
                    let wick_below = swing_low - current.low;
                    let body_above = current.close - swing_low;

                    if body_above > wick_below * 0.5 {
                        let expected_move = body_above * 2.0;
                        let confidence = 75.0
                            + (wick_below / (*swing_low / 100.0)).min(15.0)
                            + if current.volume > prev.volume { 5.0 } else { 0.0 };

                        self.patterns.push(PriceActionPattern {
                            pattern_type: PatternType::LiquidityGrab,
                            confidence: confidence.min(95.0),
                            entry_price: current.close,
                            stop_zone: current.low,
                            targets: vec![swing_low + expected_move],
                            formation_candle: n - candles_since,
                            trigger_candle: Some(n - 1),
                            is_triggered: true,
                            description: format!(
                                "Liquidity grab at swing low {:.2}, reversing. Entry: {:.2}",
                                swing_low, current.close
                            ),
                            risk_reward_ratio: expected_move / wick_below,
                            expected_move,
                        });
                    }
                }
            }
        }
    }

    /// Detect supply/demand flips
    fn detect_supply_demand_flips(&mut self) {
        if self.candles.len() < 5 {
            return;
        }

        let n = self.candles.len();
        let current = self.candles[n - 1];

        // Supply zone: previous resistance that becomes support
        if let Some((prev_resistance, _)) = self.swing_highs.back() {
            // Price bounces off previous resistance (now support)
            if current.low < *prev_resistance && current.close > *prev_resistance {
                let bounce_strength = current.close - current.low;
                let distance_below = prev_resistance - current.low;

                let confidence = 70.0 + (bounce_strength / distance_below * 15.0).min(20.0);

                let expected_move = bounce_strength * 2.0;

                self.patterns.push(PriceActionPattern {
                    pattern_type: PatternType::SupplyDemandFlip,
                    confidence: confidence.min(90.0),
                    entry_price: current.close,
                    stop_zone: current.low,
                    targets: vec![prev_resistance + expected_move],
                    formation_candle: n - 1,
                    trigger_candle: Some(n - 1),
                    is_triggered: true,
                    description: format!(
                        "Supply zone flip at {:.2}. Demand taking over. Entry: {:.2}",
                        prev_resistance, current.close
                    ),
                    risk_reward_ratio: expected_move / distance_below,
                    expected_move,
                });
            }
        }
    }

    /// Detect fakeout + flag patterns
    fn detect_fakeout_patterns(&mut self) {
        if self.candles.len() < 7 {
            return;
        }

        let n = self.candles.len();
        let recent = &self.candles.iter().rev().take(7).collect::<Vec<_>>();

        // Fakeout: breakout above resistance then fast reversal
        let resistance = recent
            .iter()
            .map(|c| c.high)
            .fold(f64::NEG_INFINITY, f64::max);
        let support = recent
            .iter()
            .map(|c| c.low)
            .fold(f64::INFINITY, f64::min);

        let current = recent[0];
        let prev = recent[1];

        // Fakeout up: goes above resistance then closes below
        if prev.close < resistance && current.close < resistance && current.high > resistance {
            let fakeout_wick = current.high - resistance;
            let reversal = resistance - current.close;

            if reversal > fakeout_wick * 0.6 {
                let expected_move = reversal * 1.5;
                let confidence = 60.0 + (reversal / (support / 100.0)).min(20.0);

                self.patterns.push(PriceActionPattern {
                    pattern_type: PatternType::FakeoutFlagPattern,
                    confidence: confidence.min(85.0),
                    entry_price: current.close,
                    stop_zone: current.high,
                    targets: vec![support - expected_move],
                    formation_candle: n - 7,
                    trigger_candle: Some(n - 1),
                    is_triggered: true,
                    description: format!(
                        "Fakeout above {:.2}. Real move down. Entry: {:.2}",
                        resistance, current.close
                    ),
                    risk_reward_ratio: expected_move / fakeout_wick,
                    expected_move,
                });
            }
        }

        // Fakeout down: goes below support then closes above
        if prev.close > support && current.close > support && current.low < support {
            let fakeout_wick = support - current.low;
            let reversal = current.close - support;

            if reversal > fakeout_wick * 0.6 {
                let expected_move = reversal * 1.5;
                let confidence = 60.0 + (reversal / (resistance / 100.0)).min(20.0);

                self.patterns.push(PriceActionPattern {
                    pattern_type: PatternType::FakeoutFlagPattern,
                    confidence: confidence.min(85.0),
                    entry_price: current.close,
                    stop_zone: current.low,
                    targets: vec![resistance + expected_move],
                    formation_candle: n - 7,
                    trigger_candle: Some(n - 1),
                    is_triggered: true,
                    description: format!(
                        "Fakeout below {:.2}. Real move up. Entry: {:.2}",
                        support, current.close
                    ),
                    risk_reward_ratio: expected_move / fakeout_wick,
                    expected_move,
                });
            }
        }
    }

    /// Detect order blocks
    fn detect_order_blocks(&mut self) {
        if self.candles.len() < 3 {
            return;
        }

        let n = self.candles.len();
        let current = self.candles[n - 1];
        let prev = self.candles[n - 2];

        // Order block: large wick rejection
        let top_wick = current.high - current.close.max(current.open);
        let bottom_wick = current.close.min(current.open) - current.low;
        let body = (current.close - current.open).abs();

        let is_top_rejection = top_wick > body * 2.0 && top_wick > prev.high - prev.low;
        let is_bottom_rejection = bottom_wick > body * 2.0 && bottom_wick > prev.high - prev.low;

        if is_top_rejection {
            let confidence = 65.0 + (top_wick / (current.high / 100.0)).min(20.0);

            self.patterns.push(PriceActionPattern {
                pattern_type: PatternType::OrderBlock,
                confidence: confidence.min(85.0),
                entry_price: current.close,
                stop_zone: current.high,
                targets: vec![current.low - (top_wick * 1.5)],
                formation_candle: n - 1,
                trigger_candle: Some(n - 1),
                is_triggered: true,
                description: format!("Order block rejection at {:.2}. Entry: {:.2}", current.high, current.close),
                risk_reward_ratio: (top_wick * 1.5) / top_wick,
                expected_move: top_wick * 1.5,
            });
        }

        if is_bottom_rejection {
            let confidence = 65.0 + (bottom_wick / (current.low / 100.0)).min(20.0);

            self.patterns.push(PriceActionPattern {
                pattern_type: PatternType::OrderBlock,
                confidence: confidence.min(85.0),
                entry_price: current.close,
                stop_zone: current.low,
                targets: vec![current.high + (bottom_wick * 1.5)],
                formation_candle: n - 1,
                trigger_candle: Some(n - 1),
                is_triggered: true,
                description: format!("Order block rejection at {:.2}. Entry: {:.2}", current.low, current.close),
                risk_reward_ratio: (bottom_wick * 1.5) / bottom_wick,
                expected_move: bottom_wick * 1.5,
            });
        }
    }

    /// Detect reversal patterns (V-flash, Can-Can)
    fn detect_reversals(&mut self) {
        if self.candles.len() < 5 {
            return;
        }

        let n = self.candles.len();
        let recent = &self.candles.iter().rev().take(5).collect::<Vec<_>>();

        let current = recent[0];
        let prev1 = recent[1];
        let prev2 = recent[2];

        // V-Flash: huge down candle followed by reversal
        if prev2.close > prev1.close && prev1.low < (prev2.low * 0.95) && current.close > prev1.close {
            let reversal_strength = current.close - prev1.low;
            let down_move = prev2.close - prev1.low;

            let confidence = 68.0 + (reversal_strength / down_move * 20.0).min(20.0);

            self.patterns.push(PriceActionPattern {
                pattern_type: PatternType::VFlashReversal,
                confidence: confidence.min(90.0),
                entry_price: current.close,
                stop_zone: prev1.low,
                targets: vec![prev2.close + (reversal_strength * 0.8)],
                formation_candle: n - 2,
                trigger_candle: Some(n - 1),
                is_triggered: true,
                description: format!(
                    "V-Flash reversal from {:.2}. Entry: {:.2}",
                    prev1.low, current.close
                ),
                risk_reward_ratio: reversal_strength / (reversal_strength * 0.2),
                expected_move: reversal_strength * 0.8,
            });
        }

        // Can-Can: down, up, down, then up (double fake then real)
        if n >= 5 {
            let c5 = &self.candles[n - 5];
            let c4 = &self.candles[n - 4];
            let c3 = &self.candles[n - 3];
            let c2 = &self.candles[n - 2];
            let c1 = &self.candles[n - 1];

            if c5.close > c4.close // Down
                && c4.close < c3.close // Up
                && c3.close > c2.close // Down
                && c2.close < c1.close
            {
                // Up
                let lowest = c5.low.min(c4.low).min(c3.low).min(c2.low);
                let recovery = c1.close - lowest;

                let confidence = 72.0 + (recovery / (c5.close / 100.0)).min(15.0);

                self.patterns.push(PriceActionPattern {
                    pattern_type: PatternType::CanCanReversal,
                    confidence: confidence.min(88.0),
                    entry_price: c1.close,
                    stop_zone: lowest,
                    targets: vec![c5.close + (recovery * 1.2)],
                    formation_candle: n - 5,
                    trigger_candle: Some(n - 1),
                    is_triggered: true,
                    description: format!(
                        "Can-Can reversal pattern confirmed. Entry: {:.2}",
                        c1.close
                    ),
                    risk_reward_ratio: (recovery * 1.2) / recovery,
                    expected_move: recovery * 1.2,
                });
            }
        }
    }

    /// Detect flag patterns
    fn detect_flag_patterns(&mut self) {
        if self.candles.len() < 10 {
            return;
        }

        let n = self.candles.len();
        let recent = &self.candles.iter().rev().take(10).collect::<Vec<_>>();

        // Flag: narrow consolidation after big move
        let initial_move_candle = recent[9];
        let recent_range = recent
            .iter()
            .map(|c| c.high - c.low)
            .fold(0.0, f64::max);
        let initial_range = initial_move_candle.high - initial_move_candle.low;

        if initial_range > recent_range * 3.0 {
            // Initial move is much bigger than consolidation
            let consolidation_high = recent.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
            let consolidation_low = recent.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
            let consolidation_height = consolidation_high - consolidation_low;

            let current = recent[0];

            // Check if consolidation is being broken
            if current.close > consolidation_high || current.close < consolidation_low {
                let is_up_break = current.close > consolidation_high;
                let target = if is_up_break {
                    consolidation_high + (initial_range * 0.7)
                } else {
                    consolidation_low - (initial_range * 0.7)
                };

                let expected_move = (target - current.close).abs();
                let confidence = 68.0 + (consolidation_height / initial_range * 15.0).min(18.0);

                self.patterns.push(PriceActionPattern {
                    pattern_type: PatternType::FlagContinuation,
                    confidence: confidence.min(86.0),
                    entry_price: current.close,
                    stop_zone: if is_up_break { consolidation_low } else { consolidation_high },
                    targets: vec![target],
                    formation_candle: n - 10,
                    trigger_candle: Some(n - 1),
                    is_triggered: true,
                    description: format!(
                        "Flag pattern breaking out. Entry: {:.2}, Target: {:.2}",
                        current.close, target
                    ),
                    risk_reward_ratio: expected_move / consolidation_height,
                    expected_move,
                });
            }
        }
    }

    /// Detect wedge patterns
    fn detect_wedge_patterns(&mut self) {
        if self.candles.len() < 15 {
            return;
        }

        let n = self.candles.len();
        let recent = &self.candles.iter().rev().take(15).collect::<Vec<_>>();

        // Wedge: converging highs and lows
        let highs: Vec<f64> = recent.iter().map(|c| c.high).collect();
        let lows: Vec<f64> = recent.iter().map(|c| c.low).collect();

        let first_high = highs[14];
        let first_low = lows[14];
        let last_high = highs[0];
        let last_low = lows[0];

        let high_convergence = (first_high - last_high) / first_high;
        let low_convergence = (last_low - first_low) / first_low.abs();

        // Wedge: both converging
        if high_convergence > 0.01 && low_convergence > 0.01 {
            let wedge_height = first_high - first_low;
            let current_height = last_high - last_low;

            if wedge_height > current_height * 2.0 {
                let current = recent[0];

                // Breakout coming
                let breakout_target_up = last_high + (wedge_height * 0.5);
                let breakout_target_down = last_low - (wedge_height * 0.5);

                let confidence = 75.0 + ((high_convergence + low_convergence) * 50.0).min(15.0);

                let is_approaching_high = current.close > (last_high + last_low) / 2.0;
                let target = if is_approaching_high {
                    breakout_target_up
                } else {
                    breakout_target_down
                };

                let expected_move = (target - current.close).abs();

                self.patterns.push(PriceActionPattern {
                    pattern_type: PatternType::WedgeBreakout,
                    confidence: confidence.min(90.0),
                    entry_price: current.close,
                    stop_zone: if is_approaching_high { last_low } else { last_high },
                    targets: vec![target],
                    formation_candle: n - 15,
                    trigger_candle: Some(n - 1),
                    is_triggered: true,
                    description: format!(
                        "Wedge pattern tightening. Breakout expected to {:.2}",
                        target
                    ),
                    risk_reward_ratio: expected_move / current_height,
                    expected_move,
                });
            }
        }
    }

    /// Get all detected patterns
    pub fn get_patterns(&self) -> Vec<PriceActionPattern> {
        self.patterns.clone()
    }

    /// Get highest confidence pattern
    pub fn get_best_pattern(&self) -> Option<PriceActionPattern> {
        self.patterns
            .iter()
            .max_by(|a, b| a.confidence.partial_cmp(&b.confidence).unwrap())
            .cloned()
    }

    /// Get patterns above minimum confidence
    pub fn get_qualified_patterns(&self, min_confidence: f64) -> Vec<PriceActionPattern> {
        self.patterns
            .iter()
            .filter(|p| p.confidence >= min_confidence)
            .cloned()
            .collect()
    }

    /// Score a pattern for integration with strategy scoring system (0-100)
    pub fn score_pattern(&self, pattern: &PriceActionPattern) -> f64 {
        let mut score = pattern.confidence;

        // Bonus for high risk/reward
        score += (pattern.risk_reward_ratio / 2.0).min(10.0);

        // Bonus for confluence (multiple patterns)
        if self.patterns.len() > 1 {
            score += 5.0;
        }

        // Penalty if stop is too wide
        let stop_distance = (pattern.stop_zone - pattern.entry_price).abs();
        let target_distance = pattern.expected_move;
        if stop_distance > target_distance * 0.5 {
            score -= 5.0;
        }

        score.min(100.0).max(0.0)
    }

    /// Check if pattern has formed confluence with other patterns
    pub fn has_confluence(&self) -> bool {
        self.patterns.len() > 1
    }

    /// Get confluence score (0-100) based on number of aligned patterns
    pub fn get_confluence_score(&self) -> f64 {
        match self.patterns.len() {
            0 => 0.0,
            1 => 50.0,
            2 => 70.0,
            3 => 85.0,
            4 => 92.0,
            5 => 95.0,
            _ => 98.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detector_creation() {
        let detector = PriceActionDetector::new();
        assert_eq!(detector.get_patterns().len(), 0);
    }

    #[test]
    fn test_candle_addition() {
        let mut detector = PriceActionDetector::new();

        let candle = Candle {
            timestamp: 1000,
            open: 100.0,
            high: 105.0,
            low: 95.0,
            close: 102.0,
            volume: 1000.0,
        };

        detector.add_candle(candle);
        // Just verify it doesn't panic
    }

    #[test]
    fn test_pattern_type_display() {
        assert_eq!(
            PatternType::CompressionExpansion.as_str(),
            "Compression→Expansion"
        );
        assert_eq!(PatternType::LiquidityGrab.as_str(), "Liquidity Grab");
    }

    #[test]
    fn test_min_confidence_levels() {
        assert_eq!(PatternType::CompressionExpansion.min_confidence(), 70.0);
        assert_eq!(PatternType::LiquidityGrab.min_confidence(), 75.0);
        assert_eq!(PatternType::FakeoutFlagPattern.min_confidence(), 60.0);
    }
}
