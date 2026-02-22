//! 🎯 MACD Divergence Strategy (distinct from MACD Momentum)
//! Detects MACD histogram divergence - when price/MACD momentum don't align
//! Signals momentum exhaustion before price reversal

use crate::utils::Error;
use super::{StrategySignal, SignalType, StrategyContext};

/// MACD Divergence: Analyzes MACD histogram changes
/// vs price movement to detect momentum weakness
pub fn evaluate(ctx: &StrategyContext) -> Result<StrategySignal, Error> {
    let current = &ctx.current;
    let previous = match &ctx.previous {
        Some(p) => p,
        None => {
            return Ok(StrategySignal {
                strategy_name: "MACD Divergence".to_string(),
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
    let macd_histogram_change = current.macd_histogram - previous.macd_histogram;
    let macd_level = current.macd_histogram;

    // BULLISH MACD Divergence: Price declining but MACD histogram strengthening (becoming less negative)
    if price_change < -1.0 && macd_histogram_change > 0.1 && macd_level < 0.0 {
        let histogram_recovery = (macd_histogram_change / previous.macd_histogram.abs().max(0.001)) * 100.0;
        let strength = histogram_recovery.abs() + price_change.abs();

        // Confirm with oversold RSI
        let rsi_oversold = current.rsi_14 < 35.0;
        let near_support = (current.close - current.support_level) / current.close < 0.03;

        let confidence = if rsi_oversold && near_support && strength > 5.0 {
            0.80
        } else if rsi_oversold && strength > 4.0 {
            0.72
        } else if strength > 3.0 {
            0.60
        } else {
            0.50
        };

        return Ok(StrategySignal {
            strategy_name: "MACD Divergence".to_string(),
            signal_type: SignalType::Buy,
            confidence,
            position_size_multiplier: 1.2,
            rationale: format!(
                "Bullish MACD Divergence: Price down {:.2}% but MACD histogram strengthening (+{:.2}%). {}",
                price_change.abs(),
                macd_histogram_change,
                if rsi_oversold { "RSI oversold." } else { "" }
            ),
            target_price: Some(current.close * 1.025),
            stop_loss_pct: 1.5,
        });
    }

    // BEARISH MACD Divergence: Price rallying but MACD histogram weakening (becoming less positive)
    if price_change > 1.0 && macd_histogram_change < -0.1 && macd_level > 0.0 {
        let histogram_weakness = (macd_histogram_change.abs() / previous.macd_histogram.max(0.001)) * 100.0;
        let strength = histogram_weakness + price_change;

        // Confirm with overbought RSI
        let rsi_overbought = current.rsi_14 > 65.0;
        let near_resistance = (current.resistance_level - current.close) / current.close < 0.03;

        let confidence = if rsi_overbought && near_resistance && strength > 5.0 {
            0.80
        } else if rsi_overbought && strength > 4.0 {
            0.72
        } else if strength > 3.0 {
            0.60
        } else {
            0.50
        };

        return Ok(StrategySignal {
            strategy_name: "MACD Divergence".to_string(),
            signal_type: SignalType::Sell,
            confidence,
            position_size_multiplier: 1.2,
            rationale: format!(
                "Bearish MACD Divergence: Price up {:.2}% but MACD histogram weakening ({:.2}%). {}",
                price_change,
                macd_histogram_change,
                if rsi_overbought { "RSI overbought." } else { "" }
            ),
            target_price: Some(current.close * 0.975),
            stop_loss_pct: 1.5,
        });
    }

    // Signal line crossover divergence: MACD crosses signal line while price moves opposite
    let macd_above_signal = current.macd > current.macd_signal;
    let prev_macd_above_signal = previous.macd > previous.macd_signal;

    // Bullish crossover with price weakness = strong setup
    if macd_above_signal && !prev_macd_above_signal && price_change < 0.0 {
        return Ok(StrategySignal {
            strategy_name: "MACD Divergence".to_string(),
            signal_type: SignalType::Buy,
            confidence: 0.68,
            position_size_multiplier: 1.15,
            rationale: "MACD bullish crossover despite price decline. Momentum reversal likely.".to_string(),
            target_price: Some(current.close * 1.02),
            stop_loss_pct: 1.5,
        });
    }

    // Bearish crossover with price strength = warning sign
    if !macd_above_signal && prev_macd_above_signal && price_change > 0.0 {
        return Ok(StrategySignal {
            strategy_name: "MACD Divergence".to_string(),
            signal_type: SignalType::Sell,
            confidence: 0.68,
            position_size_multiplier: 1.15,
            rationale: "MACD bearish crossover despite price strength. Momentum reversal warning.".to_string(),
            target_price: Some(current.close * 0.98),
            stop_loss_pct: 1.5,
        });
    }

    // Neutral
    Ok(StrategySignal {
        strategy_name: "MACD Divergence".to_string(),
        signal_type: SignalType::Neutral,
        confidence: 0.0,
        position_size_multiplier: 1.0,
        rationale: "No significant MACD divergence signal".to_string(),
        target_price: None,
        stop_loss_pct: 2.0,
    })
}
