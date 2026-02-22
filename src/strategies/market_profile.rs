//! 🎯 Market Profile Strategy
//! Identifies Volume Point of Control (POC) and Value Area
//! POC = price level where most volume traded (most contested)
//! Value Area = price range containing 70% of volume

use crate::utils::Error;
use super::{StrategySignal, SignalType, StrategyContext};

/// Market Profile: Analyzes volume distribution across price levels
/// POC (Point of Control) = highest volume level = market fairness level
/// Above POC = Distribution (supply), Below POC = Accumulation (demand)
pub fn evaluate(ctx: &StrategyContext) -> Result<StrategySignal, Error> {
    let current = &ctx.current;
    let previous = match &ctx.previous {
        Some(p) => p,
        None => {
            return Ok(StrategySignal {
                strategy_name: "Market Profile".to_string(),
                signal_type: SignalType::Neutral,
                confidence: 0.0,
                position_size_multiplier: 1.0,
                rationale: "Insufficient data for market profile".to_string(),
                target_price: None,
                stop_loss_pct: 2.0,
            });
        }
    };

    let price_change = ((current.close - previous.close) / previous.close) * 100.0;
    let volume_ratio = current.volume / (previous.volume.max(1.0));

    // APPROXIMATION: Use VWAP as POC (price-weighted average)
    // POC = fairness level where buyers and sellers agree
    let poc = current.vwap;
    let distance_to_poc = ((current.close - poc) / poc) * 100.0;

    // Price well above POC = Supply dominates
    let well_above_poc = distance_to_poc > 1.0;
    // Price well below POC = Demand dominates
    let well_below_poc = distance_to_poc < -1.0;

    // VALUE AREA interpretation: Bollinger Bands approximate value area
    // Upper band = top of value area, Lower band = bottom of value area
    let in_value_area = current.close >= current.bollinger_lower
        && current.close <= current.bollinger_upper;
    let outside_value_area = !in_value_area;

    // TREND 1: Price trending above POC with high volume = strong uptrend
    // Buyers are winning, price drifting higher
    if well_above_poc && price_change > 0.5 && volume_ratio > 1.2 {
        let distance_above_poc = distance_to_poc;

        return Ok(StrategySignal {
            strategy_name: "Market Profile".to_string(),
            signal_type: SignalType::Buy,
            confidence: 0.72,
            position_size_multiplier: 1.2,
            rationale: format!(
                "Market Profile Bullish: Price {:.2}% above POC (${:.2}). \
                 Buyers in control, trending higher on {:.1}x volume.",
                distance_above_poc, poc, volume_ratio
            ),
            target_price: Some(current.close * 1.02),
            stop_loss_pct: 1.5,
        });
    }

    // TREND 2: Price trending below POC with high volume = strong downtrend
    // Sellers are winning, price drifting lower
    if well_below_poc && price_change < -0.5 && volume_ratio > 1.2 {
        let distance_below_poc = distance_to_poc.abs();

        return Ok(StrategySignal {
            strategy_name: "Market Profile".to_string(),
            signal_type: SignalType::Sell,
            confidence: 0.72,
            position_size_multiplier: 1.2,
            rationale: format!(
                "Market Profile Bearish: Price {:.2}% below POC (${:.2}). \
                 Sellers in control, trending lower on {:.1}x volume.",
                distance_below_poc, poc, volume_ratio
            ),
            target_price: Some(current.close * 0.98),
            stop_loss_pct: 1.5,
        });
    }

    // REVERSION 1: Price far above POC with declining volume = overextension
    // Expect pullback toward POC (fairness)
    if well_above_poc && volume_ratio < 1.0 {
        return Ok(StrategySignal {
            strategy_name: "Market Profile".to_string(),
            signal_type: SignalType::Sell,
            confidence: 0.65,
            position_size_multiplier: 1.15,
            rationale: format!(
                "Market Profile Mean Reversion Up: Price {:.2}% above POC with declining volume. \
                 Expect pullback to fairness level (${:.2}).",
                distance_to_poc, poc
            ),
            target_price: Some(poc),
            stop_loss_pct: 1.0,
        });
    }

    // REVERSION 2: Price far below POC with declining volume = oversold
    // Expect rebound toward POC
    if well_below_poc && volume_ratio < 1.0 {
        return Ok(StrategySignal {
            strategy_name: "Market Profile".to_string(),
            signal_type: SignalType::Buy,
            confidence: 0.65,
            position_size_multiplier: 1.15,
            rationale: format!(
                "Market Profile Mean Reversion Down: Price {:.2}% below POC with declining volume. \
                 Expect rebound to fairness level (${:.2}).",
                distance_to_poc.abs(), poc
            ),
            target_price: Some(poc),
            stop_loss_pct: 1.0,
        });
    }

    // BREAKOUT: Price outside value area with volume = trend formation
    if outside_value_area && volume_ratio > 1.3 {
        let above_value_area = current.close > current.bollinger_upper;

        if above_value_area {
            return Ok(StrategySignal {
                strategy_name: "Market Profile".to_string(),
                signal_type: SignalType::Buy,
                confidence: 0.68,
                position_size_multiplier: 1.2,
                rationale: format!(
                    "Market Profile Breakout Up: Price broke above value area (${:.2}) with {:.1}x volume. \
                     New trend initiating.",
                    current.bollinger_upper, volume_ratio
                ),
                target_price: Some(current.close * 1.025),
                stop_loss_pct: 2.0,
            });
        } else {
            return Ok(StrategySignal {
                strategy_name: "Market Profile".to_string(),
                signal_type: SignalType::Sell,
                confidence: 0.68,
                position_size_multiplier: 1.2,
                rationale: format!(
                    "Market Profile Breakout Down: Price broke below value area (${:.2}) with {:.1}x volume. \
                     New trend initiating.",
                    current.bollinger_lower, volume_ratio
                ),
                target_price: Some(current.close * 0.975),
                stop_loss_pct: 2.0,
            });
        }
    }

    // Neutral - price near POC in value area
    Ok(StrategySignal {
        strategy_name: "Market Profile".to_string(),
        signal_type: SignalType::Neutral,
        confidence: 0.0,
        position_size_multiplier: 1.0,
        rationale: format!(
            "Market Profile Balanced: Price at fairness. POC: ${:.2}, Distance: {:.2}%, In Value Area: {}",
            poc, distance_to_poc, in_value_area
        ),
        target_price: None,
        stop_loss_pct: 2.0,
    })
}
