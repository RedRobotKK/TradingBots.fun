//! 🎯 Volume Surge Detection Strategy
//! Identifies unusual volume spikes that confirm price moves
//! Volume spike = institutional buying/selling pressure

use crate::utils::Error;
use super::{StrategySignal, SignalType, StrategyContext};

/// Volume Surge: Detects abnormal volume relative to recent average
/// Used to confirm breakouts, reversals, and accumulation/distribution
pub fn evaluate(ctx: &StrategyContext) -> Result<StrategySignal, Error> {
    let current = &ctx.current;
    let previous = match &ctx.previous {
        Some(p) => p,
        None => {
            return Ok(StrategySignal {
                strategy_name: "Volume Surge".to_string(),
                signal_type: SignalType::Neutral,
                confidence: 0.0,
                position_size_multiplier: 1.0,
                rationale: "Insufficient data".to_string(),
                target_price: None,
                stop_loss_pct: 2.0,
            });
        }
    };

    // Volume surge = current volume significantly higher than previous
    let volume_ratio = current.volume / (previous.volume.max(1.0));
    let price_change = ((current.close - previous.close) / previous.close) * 100.0;

    // Significant surge = 1.5x+ volume increase
    let surge_threshold = 1.5;
    let extreme_surge_threshold = 2.5;

    // BULLISH Volume Surge: Price up with volume spike
    if price_change > 0.5 && volume_ratio > surge_threshold {
        let surge_strength = volume_ratio;
        let is_extreme = volume_ratio > extreme_surge_threshold;

        // More bullish if price is above key levels
        let above_resistance = current.close > current.resistance_level;

        let confidence = if is_extreme && above_resistance {
            0.80
        } else if is_extreme {
            0.75
        } else if above_resistance {
            0.68
        } else {
            0.62
        };

        return Ok(StrategySignal {
            strategy_name: "Volume Surge".to_string(),
            signal_type: SignalType::Buy,
            confidence,
            position_size_multiplier: 1.25,
            rationale: format!(
                "Bullish Volume Surge: Price up {:.2}% with {:.1}x volume surge. Strong institutional buying.{}",
                price_change,
                surge_strength,
                if is_extreme { " EXTREME VOLUME." } else { "" }
            ),
            target_price: Some(current.close * 1.03),
            stop_loss_pct: 2.0,
        });
    }

    // BEARISH Volume Surge: Price down with volume spike
    if price_change < -0.5 && volume_ratio > surge_threshold {
        let surge_strength = volume_ratio;
        let is_extreme = volume_ratio > extreme_surge_threshold;

        // More bearish if price is below key levels
        let below_support = current.close < current.support_level;

        let confidence = if is_extreme && below_support {
            0.80
        } else if is_extreme {
            0.75
        } else if below_support {
            0.68
        } else {
            0.62
        };

        return Ok(StrategySignal {
            strategy_name: "Volume Surge".to_string(),
            signal_type: SignalType::Sell,
            confidence,
            position_size_multiplier: 1.25,
            rationale: format!(
                "Bearish Volume Surge: Price down {:.2}% with {:.1}x volume surge. Strong institutional selling.{}",
                price_change.abs(),
                surge_strength,
                if is_extreme { " EXTREME VOLUME." } else { "" }
            ),
            target_price: Some(current.close * 0.97),
            stop_loss_pct: 2.0,
        });
    }

    // Volume Spike without directional bias = accumulation/distribution signal
    if volume_ratio > 2.0 && price_change.abs() < 0.5 {
        // Near support = accumulation (institutions buying)
        let near_support = (current.close - current.support_level) / current.close < 0.03;
        if near_support {
            return Ok(StrategySignal {
                strategy_name: "Volume Surge".to_string(),
                signal_type: SignalType::Buy,
                confidence: 0.65,
                position_size_multiplier: 1.15,
                rationale: format!(
                    "Volume Accumulation: {:.1}x surge at support (${:.2}). Institutional buying detected.",
                    volume_ratio,
                    current.support_level
                ),
                target_price: Some(current.close * 1.02),
                stop_loss_pct: 1.5,
            });
        }

        // Near resistance = distribution (institutions selling)
        let near_resistance = (current.resistance_level - current.close) / current.close < 0.03;
        if near_resistance {
            return Ok(StrategySignal {
                strategy_name: "Volume Surge".to_string(),
                signal_type: SignalType::Sell,
                confidence: 0.65,
                position_size_multiplier: 1.15,
                rationale: format!(
                    "Volume Distribution: {:.1}x surge at resistance (${:.2}). Institutional selling detected.",
                    volume_ratio,
                    current.resistance_level
                ),
                target_price: Some(current.close * 0.98),
                stop_loss_pct: 1.5,
            });
        }
    }

    // Neutral - no significant volume signal
    Ok(StrategySignal {
        strategy_name: "Volume Surge".to_string(),
        signal_type: SignalType::Neutral,
        confidence: 0.0,
        position_size_multiplier: 1.0,
        rationale: format!(
            "No volume surge detected. Current volume ratio: {:.2}x previous.",
            volume_ratio
        ),
        target_price: None,
        stop_loss_pct: 2.0,
    })
}
