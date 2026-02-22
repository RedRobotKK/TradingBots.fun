//! 🎯 Bollinger Band Breakout Strategy
//! Detects price breakouts from Bollinger Band extremes
//! Distinct from Mean Reversion - looks for breakout momentum, not bounce

use crate::utils::Error;
use super::{StrategySignal, SignalType, StrategyContext};

/// Bollinger Breakout strategy
/// - Long: Price breaks above upper band with volume
/// - Short: Price breaks below lower band with volume
pub fn evaluate(ctx: &StrategyContext) -> Result<StrategySignal, Error> {
    let current = &ctx.current;
    let previous = &ctx.previous;

    if previous.is_none() {
        return Ok(StrategySignal {
            strategy_name: "Bollinger Breakout".to_string(),
            signal_type: SignalType::Neutral,
            confidence: 0.0,
            position_size_multiplier: 1.0,
            rationale: "Insufficient data".to_string(),
            target_price: None,
            stop_loss_pct: 2.0,
        });
    }

    let prev = previous.unwrap();
    let band_width = current.bollinger_upper - current.bollinger_lower;
    let mid = current.bollinger_middle;

    // Breakout above upper band
    if current.close > current.bollinger_upper && prev.close <= prev.bollinger_upper {
        let distance_above = ((current.close - current.bollinger_upper) / current.close) * 100.0;
        let breakout_strength = (distance_above / band_width) * current.close;

        // Confirm with volume
        let volume_ratio = current.volume / (prev.volume.max(1.0));
        let has_volume = volume_ratio > 1.2;

        let confidence = if has_volume && breakout_strength > 0.5 {
            0.75
        } else if breakout_strength > 0.5 {
            0.60
        } else {
            0.45
        };

        let target = current.close + (band_width * 0.5);

        return Ok(StrategySignal {
            strategy_name: "Bollinger Breakout".to_string(),
            signal_type: SignalType::Buy,
            confidence,
            position_size_multiplier: 1.2,
            rationale: format!(
                "Breakout above upper band (${:.2}). Breakout strength: {:.2}%. {}",
                current.bollinger_upper,
                breakout_strength,
                if has_volume { "Volume confirmed." } else { "Lower volume." }
            ),
            target_price: Some(target),
            stop_loss_pct: band_width / current.close * 100.0,
        });
    }

    // Breakout below lower band
    if current.close < current.bollinger_lower && prev.close >= prev.bollinger_lower {
        let distance_below = ((current.bollinger_lower - current.close) / current.close) * 100.0;
        let breakout_strength = (distance_below / band_width) * current.close;

        let volume_ratio = current.volume / (prev.volume.max(1.0));
        let has_volume = volume_ratio > 1.2;

        let confidence = if has_volume && breakout_strength > 0.5 {
            0.75
        } else if breakout_strength > 0.5 {
            0.60
        } else {
            0.45
        };

        let target = current.close - (band_width * 0.5);

        return Ok(StrategySignal {
            strategy_name: "Bollinger Breakout".to_string(),
            signal_type: SignalType::Sell,
            confidence,
            position_size_multiplier: 1.2,
            rationale: format!(
                "Breakout below lower band (${:.2}). Breakout strength: {:.2}%. {}",
                current.bollinger_lower,
                breakout_strength,
                if has_volume { "Volume confirmed." } else { "Lower volume." }
            ),
            target_price: Some(target),
            stop_loss_pct: band_width / current.close * 100.0,
        });
    }

    // Neutral - no breakout
    Ok(StrategySignal {
        strategy_name: "Bollinger Breakout".to_string(),
        signal_type: SignalType::Neutral,
        confidence: 0.0,
        position_size_multiplier: 1.0,
        rationale: "Price contained within Bollinger Bands".to_string(),
        target_price: None,
        stop_loss_pct: 2.0,
    })
}
