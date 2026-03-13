//! 🎯 Supply/Demand Zones Strategy
//! Identifies price areas where supply/demand imbalances exist
//! Buildup of unfilled orders creates zones for reversals/continuations

use crate::utils::Error;
use super::{StrategySignal, SignalType, StrategyContext};

/// Supply/Demand Zones: Based on support/resistance with volume analysis
/// Demand Zone (Buy Zone): Support level where buyers consistently emerge
/// Supply Zone (Sell Zone): Resistance level where sellers consistently appear
pub fn evaluate(ctx: &StrategyContext) -> Result<StrategySignal, Error> {
    let current = &ctx.current;
    let previous = match &ctx.previous {
        Some(p) => p,
        None => {
            return Ok(StrategySignal {
                strategy_name: "Supply/Demand Zones".to_string(),
                signal_type: SignalType::Neutral,
                confidence: 0.0,
                position_size_multiplier: 1.0,
                rationale: "Insufficient data".to_string(),
                target_price: None,
                stop_loss_pct: 2.0,
            });
        }
    };

    let price_change = ((current.close - previous.close) / previous.close) * 100.0;
    let volume_ratio = current.volume / (previous.volume.max(1.0));

    // Distance to support/resistance as percentage
    let dist_to_support = ((current.close - current.support_level) / current.close) * 100.0;
    let dist_to_resistance = ((current.resistance_level - current.close) / current.close) * 100.0;

    // DEMAND ZONE: Price falling toward support with volume
    // Signals institutional buyers stepping in at support
    if price_change < -0.5 && dist_to_support < 3.0 && volume_ratio > 1.3 {
        let zone_strength = (3.0 - dist_to_support) * volume_ratio;  // Closer to support + higher volume = stronger
        let _recovery_potential = dist_to_support.abs();

        let confidence = if zone_strength > 5.0 && current.rsi_14 < 40.0 {
            0.80
        } else if zone_strength > 3.0 {
            0.72
        } else {
            0.62
        };

        return Ok(StrategySignal {
            strategy_name: "Supply/Demand Zones".to_string(),
            signal_type: SignalType::Buy,
            confidence,
            position_size_multiplier: 1.25,
            rationale: format!(
                "Demand Zone Activation: Price at support (${:.2}), {:.2}% away. Volume surge ({:.1}x) confirms buyer demand.",
                current.support_level,
                dist_to_support,
                volume_ratio
            ),
            target_price: Some(current.close + (current.resistance_level - current.support_level) * 0.5),
            stop_loss_pct: dist_to_support.abs() * 1.2,
        });
    }

    // SUPPLY ZONE: Price rallying toward resistance with volume
    // Signals institutional sellers appearing at resistance
    if price_change > 0.5 && dist_to_resistance < 3.0 && volume_ratio > 1.3 {
        let zone_strength = (3.0 - dist_to_resistance) * volume_ratio;  // Closer to resistance + higher volume = stronger
        let _decline_potential = dist_to_resistance.abs();

        let confidence = if zone_strength > 5.0 && current.rsi_14 > 60.0 {
            0.80
        } else if zone_strength > 3.0 {
            0.72
        } else {
            0.62
        };

        return Ok(StrategySignal {
            strategy_name: "Supply/Demand Zones".to_string(),
            signal_type: SignalType::Sell,
            confidence,
            position_size_multiplier: 1.25,
            rationale: format!(
                "Supply Zone Activation: Price at resistance (${:.2}), {:.2}% away. Volume surge ({:.1}x) confirms seller pressure.",
                current.resistance_level,
                dist_to_resistance,
                volume_ratio
            ),
            target_price: Some(current.close - (current.resistance_level - current.support_level) * 0.5),
            stop_loss_pct: dist_to_resistance.abs() * 1.2,
        });
    }

    // ZONE BOUNCE: Price bouncing within zone without breaking
    let in_demand_zone = dist_to_support < 5.0 && current.close < current.resistance_level;
    let in_supply_zone = dist_to_resistance < 5.0 && current.close > current.support_level;

    // Weak bounce in demand zone (testing support)
    if in_demand_zone && price_change > 0.0 && volume_ratio < 1.2 {
        return Ok(StrategySignal {
            strategy_name: "Supply/Demand Zones".to_string(),
            signal_type: SignalType::Buy,
            confidence: 0.55,
            position_size_multiplier: 1.0,
            rationale: format!(
                "Demand Zone Test: Price testing support (${:.2}). Weak bounce forming.",
                current.support_level
            ),
            target_price: Some(current.resistance_level),
            stop_loss_pct: dist_to_support.abs() * 1.0,
        });
    }

    // Weak pullback in supply zone (testing resistance)
    if in_supply_zone && price_change < 0.0 && volume_ratio < 1.2 {
        return Ok(StrategySignal {
            strategy_name: "Supply/Demand Zones".to_string(),
            signal_type: SignalType::Sell,
            confidence: 0.55,
            position_size_multiplier: 1.0,
            rationale: format!(
                "Supply Zone Test: Price testing resistance (${:.2}). Weak pullback forming.",
                current.resistance_level
            ),
            target_price: Some(current.support_level),
            stop_loss_pct: dist_to_resistance.abs() * 1.0,
        });
    }

    // Neutral - price in middle of range
    Ok(StrategySignal {
        strategy_name: "Supply/Demand Zones".to_string(),
        signal_type: SignalType::Neutral,
        confidence: 0.0,
        position_size_multiplier: 1.0,
        rationale: format!(
            "Price in balanced zone. Support: ${:.2} ({:.2}% below), Resistance: ${:.2} ({:.2}% above)",
            current.support_level, dist_to_support, current.resistance_level, dist_to_resistance
        ),
        target_price: None,
        stop_loss_pct: 2.0,
    })
}
