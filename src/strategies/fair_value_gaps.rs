//! 🎯 Fair Value Gap (FVG) Strategy
//! Identifies gaps in price movement where market filled unevenly
//! FVG = price jumped over where orders were unfilled
//! Market tends to return to "fill the gap"

use crate::utils::Error;
use super::{StrategySignal, SignalType, StrategyContext};

/// Fair Value Gap: Unbalanced move where market "gapped" over fair value
/// Creates trading opportunity when price inevitably retests the gap
pub fn evaluate(ctx: &StrategyContext) -> Result<StrategySignal, Error> {
    let current = &ctx.current;
    let previous = match &ctx.previous {
        Some(p) => p,
        None => {
            return Ok(StrategySignal {
                strategy_name: "Fair Value Gap".to_string(),
                signal_type: SignalType::Neutral,
                confidence: 0.0,
                position_size_multiplier: 1.0,
                rationale: "Insufficient data".to_string(),
                target_price: None,
                stop_loss_pct: 2.0,
            });
        }
    };

    // FVG occurs when there's a gap in high/low between candles
    // Bullish FVG: Previous high < Current low (gap up)
    // Bearish FVG: Previous low > Current high (gap down)

    let bullish_gap_size = if current.low > previous.high {
        ((current.low - previous.high) / previous.high) * 100.0
    } else {
        0.0
    };

    let bearish_gap_size = if previous.low > current.high {
        ((previous.low - current.high) / current.high) * 100.0
    } else {
        0.0
    };

    let price_change = ((current.close - previous.close) / previous.close) * 100.0;
    let volume_ratio = current.volume / (previous.volume.max(1.0));

    // BULLISH FVG: Gap up with follow-through
    // Price jumped above previous high, left a gap to fill
    // Trade: Price will likely retest the gap (previous high)
    if bullish_gap_size > 0.3 && price_change > 0.5 && volume_ratio > 1.2 {
        let _gap_midpoint = (previous.high + current.low) / 2.0;
        let gap_fill_target = previous.high;  // Market will fill the gap

        // Confidence based on gap size and volume
        let confidence = if bullish_gap_size > 1.5 && volume_ratio > 1.5 {
            0.78
        } else if bullish_gap_size > 1.0 {
            0.70
        } else {
            0.62
        };

        return Ok(StrategySignal {
            strategy_name: "Fair Value Gap".to_string(),
            signal_type: SignalType::Buy,
            confidence,
            position_size_multiplier: 1.2,
            rationale: format!(
                "Bullish FVG: Gap up {:.2}% created at ${:.2}. Market will retest gap. Enter long for gap fill.",
                bullish_gap_size,
                gap_fill_target
            ),
            target_price: Some(gap_fill_target),
            stop_loss_pct: bullish_gap_size * 1.2,
        });
    }

    // BEARISH FVG: Gap down with follow-through
    // Price jumped below previous low, left a gap to fill
    // Trade: Price will likely retest the gap (previous low)
    if bearish_gap_size > 0.3 && price_change < -0.5 && volume_ratio > 1.2 {
        let _gap_midpoint = (previous.low + current.high) / 2.0;
        let gap_fill_target = previous.low;  // Market will fill the gap

        // Confidence based on gap size and volume
        let confidence = if bearish_gap_size > 1.5 && volume_ratio > 1.5 {
            0.78
        } else if bearish_gap_size > 1.0 {
            0.70
        } else {
            0.62
        };

        return Ok(StrategySignal {
            strategy_name: "Fair Value Gap".to_string(),
            signal_type: SignalType::Sell,
            confidence,
            position_size_multiplier: 1.2,
            rationale: format!(
                "Bearish FVG: Gap down {:.2}% created at ${:.2}. Market will retest gap. Enter short for gap fill.",
                bearish_gap_size,
                gap_fill_target
            ),
            target_price: Some(gap_fill_target),
            stop_loss_pct: bearish_gap_size * 1.2,
        });
    }

    // TRADING THE FVG MITIGATED: Price is moving back into old gap
    // This is a continuation signal - gap is being "filled"
    let bullish_fvg_being_filled = previous.low > current.open && current.open > previous.high;
    let bearish_fvg_being_filled = previous.high < current.open && current.open < previous.low;

    if bullish_fvg_being_filled && current.close < current.open {
        return Ok(StrategySignal {
            strategy_name: "Fair Value Gap".to_string(),
            signal_type: SignalType::Buy,
            confidence: 0.68,
            position_size_multiplier: 1.1,
            rationale: "FVG Mitigation: Price returning to fill previous bullish gap. Continuation signal.".to_string(),
            target_price: Some(previous.high),
            stop_loss_pct: 1.5,
        });
    }

    if bearish_fvg_being_filled && current.close > current.open {
        return Ok(StrategySignal {
            strategy_name: "Fair Value Gap".to_string(),
            signal_type: SignalType::Sell,
            confidence: 0.68,
            position_size_multiplier: 1.1,
            rationale: "FVG Mitigation: Price returning to fill previous bearish gap. Continuation signal.".to_string(),
            target_price: Some(previous.low),
            stop_loss_pct: 1.5,
        });
    }

    // Neutral - no FVG signal
    Ok(StrategySignal {
        strategy_name: "Fair Value Gap".to_string(),
        signal_type: SignalType::Neutral,
        confidence: 0.0,
        position_size_multiplier: 1.0,
        rationale: "No significant Fair Value Gap detected".to_string(),
        target_price: None,
        stop_loss_pct: 2.0,
    })
}
