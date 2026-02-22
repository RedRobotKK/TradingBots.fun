//! 🎯 RSI Divergence Strategy (distinct from Price Divergence)
//! Detects divergence specifically in RSI momentum
//! When price makes new high/low but RSI doesn't - trend may be weakening

use crate::utils::Error;
use super::{StrategySignal, SignalType, StrategyContext};

/// RSI Divergence detects weakness in momentum despite price moves
/// Bullish: Price lower but RSI higher (RSI divergence - recovery imminent)
/// Bearish: Price higher but RSI lower (RSI divergence - reversal warning)
pub fn evaluate(ctx: &StrategyContext) -> Result<StrategySignal, Error> {
    let current = &ctx.current;
    let previous = match &ctx.previous {
        Some(p) => p,
        None => {
            return Ok(StrategySignal {
                strategy_name: "RSI Divergence".to_string(),
                signal_type: SignalType::Neutral,
                confidence: 0.0,
                position_size_multiplier: 1.0,
                rationale: "Insufficient data".to_string(),
                target_price: None,
                stop_loss_pct: 2.0,
            });
        }
    };

    // Use RSI 14 for divergence (primary indicator)
    let rsi_change = current.rsi_14 - previous.rsi_14;
    let price_change = ((current.close - previous.close) / previous.close) * 100.0;

    // BULLISH RSI Divergence: Price lower but RSI higher (momentum strengthening during decline)
    if price_change < -1.0 && rsi_change > 2.0 {
        let divergence_strength = rsi_change.abs() + price_change.abs();

        // Strong signal if price near support
        let near_support = (current.close - current.support_level) / current.close < 0.03;
        let in_oversold = current.rsi_14 < 35.0;

        let confidence = if near_support && in_oversold && divergence_strength > 4.0 {
            0.80
        } else if in_oversold && divergence_strength > 4.0 {
            0.70
        } else if divergence_strength > 2.0 {
            0.60
        } else {
            0.45
        };

        return Ok(StrategySignal {
            strategy_name: "RSI Divergence".to_string(),
            signal_type: SignalType::Buy,
            confidence,
            position_size_multiplier: 1.2,
            rationale: format!(
                "Bullish RSI Divergence: Price down {:.2}% but RSI up {:.2}%. Strength: {:.2}. {}",
                price_change.abs(),
                rsi_change,
                divergence_strength,
                if near_support { "At support." } else { "" }
            ),
            target_price: Some(current.close * 1.02),
            stop_loss_pct: 2.0,
        });
    }

    // BEARISH RSI Divergence: Price higher but RSI lower (momentum weakening during rally)
    if price_change > 1.0 && rsi_change < -2.0 {
        let divergence_strength = rsi_change.abs() + price_change.abs();

        // Strong signal if price near resistance
        let near_resistance = (current.resistance_level - current.close) / current.close < 0.03;
        let in_overbought = current.rsi_14 > 65.0;

        let confidence = if near_resistance && in_overbought && divergence_strength > 4.0 {
            0.80
        } else if in_overbought && divergence_strength > 4.0 {
            0.70
        } else if divergence_strength > 2.0 {
            0.60
        } else {
            0.45
        };

        return Ok(StrategySignal {
            strategy_name: "RSI Divergence".to_string(),
            signal_type: SignalType::Sell,
            confidence,
            position_size_multiplier: 1.2,
            rationale: format!(
                "Bearish RSI Divergence: Price up {:.2}% but RSI down {:.2}%. Strength: {:.2}. {}",
                price_change,
                rsi_change.abs(),
                divergence_strength,
                if near_resistance { "At resistance." } else { "" }
            ),
            target_price: Some(current.close * 0.98),
            stop_loss_pct: 2.0,
        });
    }

    // Check RSI 7 (faster oscillator) for confirmation
    let rsi7_change = current.rsi_7 - previous.rsi_7;

    // Ultra-bullish: Both RSI 14 and RSI 7 higher despite price drop
    if price_change < -0.5 && rsi_change > 1.0 && rsi7_change > 2.0 && current.rsi_14 < 40.0 {
        return Ok(StrategySignal {
            strategy_name: "RSI Divergence".to_string(),
            signal_type: SignalType::Buy,
            confidence: 0.75,
            position_size_multiplier: 1.25,
            rationale: "Multi-timeframe RSI bullish divergence: Both RSI14 and RSI7 rising despite price decline.".to_string(),
            target_price: Some(current.close * 1.025),
            stop_loss_pct: 1.5,
        });
    }

    // Neutral
    Ok(StrategySignal {
        strategy_name: "RSI Divergence".to_string(),
        signal_type: SignalType::Neutral,
        confidence: 0.0,
        position_size_multiplier: 1.0,
        rationale: "No significant RSI divergence detected".to_string(),
        target_price: None,
        stop_loss_pct: 2.0,
    })
}
