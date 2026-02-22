//! 🎯 Wyckoff Analysis Strategy
//! Detects accumulation/distribution phases and breakout signals
//! Wyckoff: Professional accumulation → Spring → Breakout → Profit Taking
//!
//! Four phases:
//! 1. Accumulation: Smart money buying at lows
//! 2. Spring: Price breaks below to shake out weak hands
//! 3. Markup: Explosive advance after spring
//! 4. Distribution: Smart money selling at highs

use crate::utils::Error;
use super::{StrategySignal, SignalType, StrategyContext};

/// Wyckoff Analysis: Detects market structure patterns
/// Primary signal: Price structure + volume confirms phase
pub fn evaluate(ctx: &StrategyContext) -> Result<StrategySignal, Error> {
    let current = &ctx.current;
    let previous = match &ctx.previous {
        Some(p) => p,
        None => {
            return Ok(StrategySignal {
                strategy_name: "Wyckoff Analysis".to_string(),
                signal_type: SignalType::Neutral,
                confidence: 0.0,
                position_size_multiplier: 1.0,
                rationale: "Insufficient data for Wyckoff analysis".to_string(),
                target_price: None,
                stop_loss_pct: 2.0,
            });
        }
    };

    let price_change = ((current.close - previous.close) / previous.close) * 100.0;
    let volume_ratio = current.volume / (previous.volume.max(1.0));

    // Range of price movement
    let intra_range = current.high - current.low;
    let is_tight_range = intra_range < (current.close * 0.005);  // <0.5% range = consolidation

    // Distance to support/resistance
    let dist_to_support = ((current.close - current.support_level) / current.close) * 100.0;
    let dist_to_resistance = ((current.resistance_level - current.close) / current.close) * 100.0;

    // PHASE 1: ACCUMULATION
    // Characteristics: Tight range, volume surge on up days, support holding
    // Smart money quietly buying at lows
    let in_accumulation = is_tight_range
        && dist_to_support < 2.0
        && current.rsi_14 < 45.0
        && volume_ratio > 1.3;

    if in_accumulation {
        return Ok(StrategySignal {
            strategy_name: "Wyckoff Analysis".to_string(),
            signal_type: SignalType::Buy,
            confidence: 0.65,
            position_size_multiplier: 1.15,
            rationale: format!(
                "Wyckoff Phase 1 - Accumulation: Tight range (${:.2}-${:.2}) at support with volume surge. Smart money buying.",
                current.low, current.high
            ),
            target_price: Some(current.resistance_level),
            stop_loss_pct: dist_to_support.abs() * 1.1,
        });
    }

    // PHASE 2: SPRING (Test/Shake Out)
    // Characteristics: Price breaks below support, high volume, then reverses quickly
    // Smart money forces out retail stops
    let just_broke_support = current.close < current.support_level
        && previous.close > current.support_level;
    let volume_surge_on_break = volume_ratio > 1.8;

    if just_broke_support && volume_surge_on_break {
        let distance_below = ((current.support_level - current.close) / current.close) * 100.0;

        // Spring typically goes 1-3% below support
        let is_valid_spring = distance_below < 3.0 && distance_below > 0.2;

        if is_valid_spring {
            return Ok(StrategySignal {
                strategy_name: "Wyckoff Analysis".to_string(),
                signal_type: SignalType::Buy,
                confidence: 0.82,
                position_size_multiplier: 1.35,
                rationale: format!(
                    "Wyckoff Phase 2 - Spring: Price broke support by {:.2}% with extreme volume ({:.1}x). \
                     Panic shake-out complete, prepare for breakup.",
                    distance_below, volume_ratio
                ),
                target_price: Some(current.resistance_level * 1.02),
                stop_loss_pct: distance_below * 1.5,
            });
        }
    }

    // PHASE 3: MARKUP
    // Characteristics: Strong uptrend, price above resistance, volume declining on pullbacks
    // Smart money aggressively buying
    let markup_setup = current.close > current.resistance_level
        && price_change > 0.5
        && current.adx > 30.0;  // ADX > 30 = strong trend

    if markup_setup {
        let advance_strength = price_change;

        return Ok(StrategySignal {
            strategy_name: "Wyckoff Analysis".to_string(),
            signal_type: SignalType::Buy,
            confidence: 0.75,
            position_size_multiplier: 1.2,
            rationale: format!(
                "Wyckoff Phase 3 - Markup: Strong advance (+{:.2}%) above resistance. Trend strength ADX: {:.1}. \
                 Continue long.",
                advance_strength, current.adx
            ),
            target_price: Some(current.resistance_level * 1.03),
            stop_loss_pct: 2.0,
        });
    }

    // PHASE 4: DISTRIBUTION
    // Characteristics: Price at highs, volume increases on up days (sellers disguised), range widens
    // Smart money is exiting positions
    let at_highs = dist_to_resistance < 2.0;
    let wide_range = intra_range > (current.close * 0.015);  // >1.5% range = wider
    let volume_on_advances = volume_ratio > 1.4 && price_change > 0.0;

    let in_distribution = at_highs && wide_range && volume_on_advances && current.rsi_14 > 65.0;

    if in_distribution {
        return Ok(StrategySignal {
            strategy_name: "Wyckoff Analysis".to_string(),
            signal_type: SignalType::Sell,
            confidence: 0.70,
            position_size_multiplier: 1.2,
            rationale: format!(
                "Wyckoff Phase 4 - Distribution: Price at highs (${:.2}) with wide range and volume surge on advances. \
                 Smart money exiting.",
                current.high
            ),
            target_price: Some(current.support_level),
            stop_loss_pct: dist_to_resistance.abs() * 1.1,
        });
    }

    // Neutral
    Ok(StrategySignal {
        strategy_name: "Wyckoff Analysis".to_string(),
        signal_type: SignalType::Neutral,
        confidence: 0.0,
        position_size_multiplier: 1.0,
        rationale: "No clear Wyckoff phase signal".to_string(),
        target_price: None,
        stop_loss_pct: 2.0,
    })
}
