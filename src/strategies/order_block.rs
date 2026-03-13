//! 🎯 Order Block Detection Strategy
//! Identifies institutional order footprints (price levels where big blocks were filled)
//! Recycled Supply/Demand: Previous turning points often hold resistance/support

use crate::utils::Error;
use super::{StrategySignal, SignalType, StrategyContext};

/// Order Block: Detects price levels where large institutional orders executed
/// High volume + reversal = order block formed
/// Price retest of old block = potential trading opportunity
pub fn evaluate(ctx: &StrategyContext) -> Result<StrategySignal, Error> {
    let current = &ctx.current;
    let previous = match &ctx.previous {
        Some(p) => p,
        None => {
            return Ok(StrategySignal {
                strategy_name: "Order Block".to_string(),
                signal_type: SignalType::Neutral,
                confidence: 0.0,
                position_size_multiplier: 1.0,
                rationale: "Insufficient data for order block analysis".to_string(),
                target_price: None,
                stop_loss_pct: 2.0,
            });
        }
    };

    let _price_change = ((current.close - previous.close) / previous.close) * 100.0;
    let volume_ratio = current.volume / (previous.volume.max(1.0));
    let candle_size = ((current.close - current.open).abs() / current.open) * 100.0;

    // ORDER BLOCK FORMATION: High volume + large candle = potential order block at this level
    let is_bullish_ob = current.close > current.open && candle_size > 1.0 && volume_ratio > 1.5;
    let is_bearish_ob = current.close < current.open && candle_size > 1.0 && volume_ratio > 1.5;

    // After order block forms, price tends to retest it
    // BULLISH: Price retesting upward to old bullish order block
    if is_bullish_ob && current.close > current.resistance_level * 0.98 {
        let ob_level = current.open;  // Order block formed at open of large candle
        let _distance_above_ob = ((current.close - ob_level) / ob_level) * 100.0;

        return Ok(StrategySignal {
            strategy_name: "Order Block".to_string(),
            signal_type: SignalType::Buy,
            confidence: 0.70,
            position_size_multiplier: 1.2,
            rationale: format!(
                "Bullish Order Block Formation: Large green candle ({}%) with volume surge ({}x). OB level: ${:.2}. Expect retest.",
                candle_size, volume_ratio, ob_level
            ),
            target_price: Some(ob_level * 1.02),
            stop_loss_pct: 1.5,
        });
    }

    // BEARISH: Price retesting downward to old bearish order block
    if is_bearish_ob && current.close < current.support_level * 1.02 {
        let ob_level = current.open;  // Order block formed at open of large candle
        let _distance_below_ob = ((ob_level - current.close) / ob_level) * 100.0;

        return Ok(StrategySignal {
            strategy_name: "Order Block".to_string(),
            signal_type: SignalType::Sell,
            confidence: 0.70,
            position_size_multiplier: 1.2,
            rationale: format!(
                "Bearish Order Block Formation: Large red candle ({}%) with volume surge ({}x). OB level: ${:.2}. Expect retest.",
                candle_size, volume_ratio, ob_level
            ),
            target_price: Some(ob_level * 0.98),
            stop_loss_pct: 1.5,
        });
    }

    // RETESTING ORDER BLOCK: Price near support/resistance with decreasing volume = testing old OB
    let volume_declining = current.volume < previous.volume * 0.9;
    let near_support = (current.close - current.support_level) / current.close < 0.025;
    let near_resistance = (current.resistance_level - current.close) / current.close < 0.025;

    // Bullish retest: Price touching support (old OB) with decreasing volume on pullback
    if near_support && volume_declining && current.rsi_14 < 45.0 {
        return Ok(StrategySignal {
            strategy_name: "Order Block".to_string(),
            signal_type: SignalType::Buy,
            confidence: 0.65,
            position_size_multiplier: 1.15,
            rationale: format!(
                "Order Block Retest (Bullish): Price retesting support OB (${:.2}) with declining volume. Mitigation likely.",
                current.support_level
            ),
            target_price: Some(current.close * 1.025),
            stop_loss_pct: 1.5,
        });
    }

    // Bearish retest: Price touching resistance (old OB) with decreasing volume on pullback
    if near_resistance && volume_declining && current.rsi_14 > 55.0 {
        return Ok(StrategySignal {
            strategy_name: "Order Block".to_string(),
            signal_type: SignalType::Sell,
            confidence: 0.65,
            position_size_multiplier: 1.15,
            rationale: format!(
                "Order Block Retest (Bearish): Price retesting resistance OB (${:.2}) with declining volume. Rejection likely.",
                current.resistance_level
            ),
            target_price: Some(current.close * 0.975),
            stop_loss_pct: 1.5,
        });
    }

    // Neutral - no order block signal
    Ok(StrategySignal {
        strategy_name: "Order Block".to_string(),
        signal_type: SignalType::Neutral,
        confidence: 0.0,
        position_size_multiplier: 1.0,
        rationale: "No significant order block signal detected".to_string(),
        target_price: None,
        stop_loss_pct: 2.0,
    })
}
