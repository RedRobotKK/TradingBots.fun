//! 🎯 ATR Breakout Strategy
//! Uses Average True Range to identify explosive volatility breakouts
//! ATR expansion = increased volatility and opportunity

use crate::utils::Error;
use super::{StrategySignal, SignalType, StrategyContext};

/// ATR Breakout: Detects when price moves beyond typical ATR range
/// Signals volatility expansion and potential trend initiation
pub fn evaluate(ctx: &StrategyContext) -> Result<StrategySignal, Error> {
    let current = &ctx.current;
    let previous = match &ctx.previous {
        Some(p) => p,
        None => {
            return Ok(StrategySignal {
                strategy_name: "ATR Breakout".to_string(),
                signal_type: SignalType::Neutral,
                confidence: 0.0,
                position_size_multiplier: 1.0,
                rationale: "Insufficient data".to_string(),
                target_price: None,
                stop_loss_pct: 2.0,
            });
        }
    };

    // ATR represents average volatility
    let atr_current = current.atr_14;
    let atr_previous = previous.atr_14;

    // Price range of current candle
    let candle_range = current.high - current.low;
    let range_to_atr = candle_range / atr_current;

    // ATR expansion: Current ATR > previous ATR (volatility increasing)
    let atr_is_expanding = atr_current > atr_previous;
    let atr_expansion_rate = ((atr_current - atr_previous) / atr_previous) * 100.0;

    // Breakout above resistance with ATR expansion
    if current.close > current.resistance_level && atr_is_expanding {
        let breakout_distance = ((current.close - current.resistance_level) / current.resistance_level) * 100.0;
        let volatility_boost = atr_expansion_rate;

        let is_strong_breakout = range_to_atr > 0.8;  // Large candle relative to ATR
        let expansion_significant = volatility_boost > 10.0;

        let confidence = if is_strong_breakout && expansion_significant {
            0.82
        } else if is_strong_breakout || expansion_significant {
            0.72
        } else if breakout_distance > 0.5 {
            0.62
        } else {
            0.55
        };

        let target = current.resistance_level + (atr_current * 2.0);

        return Ok(StrategySignal {
            strategy_name: "ATR Breakout".to_string(),
            signal_type: SignalType::Buy,
            confidence,
            position_size_multiplier: 1.3,
            rationale: format!(
                "ATR Breakout Up: Price broke resistance (${:.2}) by {:.2}%. ATR expanding +{:.1}%. Volatility surging.",
                current.resistance_level,
                breakout_distance,
                volatility_boost
            ),
            target_price: Some(target),
            stop_loss_pct: (atr_current / current.close) * 100.0 * 1.5,
        });
    }

    // Breakout below support with ATR expansion
    if current.close < current.support_level && atr_is_expanding {
        let breakout_distance = ((current.support_level - current.close) / current.support_level) * 100.0;
        let volatility_boost = atr_expansion_rate;

        let is_strong_breakout = range_to_atr > 0.8;
        let expansion_significant = volatility_boost > 10.0;

        let confidence = if is_strong_breakout && expansion_significant {
            0.82
        } else if is_strong_breakout || expansion_significant {
            0.72
        } else if breakout_distance > 0.5 {
            0.62
        } else {
            0.55
        };

        let target = current.support_level - (atr_current * 2.0);

        return Ok(StrategySignal {
            strategy_name: "ATR Breakout".to_string(),
            signal_type: SignalType::Sell,
            confidence,
            position_size_multiplier: 1.3,
            rationale: format!(
                "ATR Breakout Down: Price broke support (${:.2}) by {:.2}%. ATR expanding +{:.1}%. Volatility surging.",
                current.support_level,
                breakout_distance,
                volatility_boost
            ),
            target_price: Some(target),
            stop_loss_pct: (atr_current / current.close) * 100.0 * 1.5,
        });
    }

    // Volatility surge without directional breakout = setup forming
    if atr_is_expanding && atr_expansion_rate > 15.0 && current.close.abs() < atr_current {
        return Ok(StrategySignal {
            strategy_name: "ATR Breakout".to_string(),
            signal_type: SignalType::Neutral,
            confidence: 0.40,
            position_size_multiplier: 0.8,
            rationale: format!(
                "High ATR Expansion: Volatility surging (+{:.1}%) but no directional breakout yet. Setup forming.",
                atr_expansion_rate
            ),
            target_price: None,
            stop_loss_pct: (atr_current / current.close) * 100.0,
        });
    }

    // Neutral - no ATR breakout signal
    Ok(StrategySignal {
        strategy_name: "ATR Breakout".to_string(),
        signal_type: SignalType::Neutral,
        confidence: 0.0,
        position_size_multiplier: 1.0,
        rationale: format!(
            "No ATR breakout. Current ATR: ${:.4}, Expansion: {:.1}%",
            atr_current, atr_expansion_rate
        ),
        target_price: None,
        stop_loss_pct: 2.0,
    })
}
