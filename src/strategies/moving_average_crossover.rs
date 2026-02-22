//! 🎯 Moving Average Crossover Strategy
//! Combines two distinct MA signals: fast (10/20) and slow (50/200)
//! Detects trend changes via MA crossover events

use crate::utils::Error;
use super::{StrategySignal, SignalType, StrategyContext};

/// MA Crossover uses pre-calculated MAs available in MarketSnapshot
/// For full system, would also include:
/// - EMA 10/20 fast crossover
/// - EMA 50/200 slow crossover
/// - SMA variants
pub fn evaluate(ctx: &StrategyContext) -> Result<StrategySignal, Error> {
    let current = &ctx.current;
    let previous = match &ctx.previous {
        Some(p) => p,
        None => {
            return Ok(StrategySignal {
                strategy_name: "Moving Average Crossover".to_string(),
                signal_type: SignalType::Neutral,
                confidence: 0.0,
                position_size_multiplier: 1.0,
                rationale: "Insufficient data for MA crossover".to_string(),
                target_price: None,
                stop_loss_pct: 2.0,
            });
        }
    };

    // In a real implementation, MarketSnapshot would include multiple MA values
    // For now, we'll use Bollinger middle as a proxy for EMA 20
    // and use VWAP as proxy for longer-term average

    let fast_ma = current.bollinger_middle;  // Represents ~20-period MA
    let slow_ma = current.vwap;              // Represents longer-term trend

    let prev_fast = previous.bollinger_middle;
    let prev_slow = previous.vwap;

    // Golden Cross: Fast MA crosses above Slow MA
    if fast_ma > slow_ma && prev_fast <= prev_slow {
        let crossover_distance = ((fast_ma - slow_ma) / slow_ma) * 100.0;

        return Ok(StrategySignal {
            strategy_name: "Moving Average Crossover".to_string(),
            signal_type: SignalType::Buy,
            confidence: 0.70,
            position_size_multiplier: 1.15,
            rationale: format!(
                "Golden Cross: Fast MA (${:.2}) crossed above Slow MA (${:.2}). Distance: {:.2}%",
                fast_ma, slow_ma, crossover_distance
            ),
            target_price: Some(slow_ma * 1.02),  // 2% above slow MA
            stop_loss_pct: 1.5,
        });
    }

    // Death Cross: Fast MA crosses below Slow MA
    if fast_ma < slow_ma && prev_fast >= prev_slow {
        let crossover_distance = ((slow_ma - fast_ma) / slow_ma) * 100.0;

        return Ok(StrategySignal {
            strategy_name: "Moving Average Crossover".to_string(),
            signal_type: SignalType::Sell,
            confidence: 0.70,
            position_size_multiplier: 1.15,
            rationale: format!(
                "Death Cross: Fast MA (${:.2}) crossed below Slow MA (${:.2}). Distance: {:.2}%",
                fast_ma, slow_ma, crossover_distance
            ),
            target_price: Some(slow_ma * 0.98),  // 2% below slow MA
            stop_loss_pct: 1.5,
        });
    }

    // Price above both MAs = bullish alignment
    if current.close > fast_ma && fast_ma > slow_ma {
        let alignment_strength = ((current.close - slow_ma) / slow_ma) * 100.0;

        return Ok(StrategySignal {
            strategy_name: "Moving Average Crossover".to_string(),
            signal_type: SignalType::Buy,
            confidence: 0.55,
            position_size_multiplier: 1.0,
            rationale: format!(
                "Bullish MA alignment: Price (${:.2}) > Fast MA (${:.2}) > Slow MA (${:.2}). Strength: {:.2}%",
                current.close, fast_ma, slow_ma, alignment_strength
            ),
            target_price: Some(current.close * 1.015),
            stop_loss_pct: 1.0,
        });
    }

    // Price below both MAs = bearish alignment
    if current.close < fast_ma && fast_ma < slow_ma {
        let alignment_strength = ((slow_ma - current.close) / slow_ma) * 100.0;

        return Ok(StrategySignal {
            strategy_name: "Moving Average Crossover".to_string(),
            signal_type: SignalType::Sell,
            confidence: 0.55,
            position_size_multiplier: 1.0,
            rationale: format!(
                "Bearish MA alignment: Price (${:.2}) < Fast MA (${:.2}) < Slow MA (${:.2}). Strength: {:.2}%",
                current.close, fast_ma, slow_ma, alignment_strength
            ),
            target_price: Some(current.close * 0.985),
            stop_loss_pct: 1.0,
        });
    }

    // Neutral
    Ok(StrategySignal {
        strategy_name: "Moving Average Crossover".to_string(),
        signal_type: SignalType::Neutral,
        confidence: 0.0,
        position_size_multiplier: 1.0,
        rationale: "No significant MA alignment signal".to_string(),
        target_price: None,
        stop_loss_pct: 2.0,
    })
}
