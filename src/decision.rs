use anyhow::Result;
use serde::{Deserialize, Serialize};
use crate::data::PriceData;
use crate::indicators::TechnicalIndicators;
use crate::signals::OrderFlowSignal;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub action: String, // BUY, SELL, SKIP
    pub confidence: f64,
    pub position_size: f64,
    pub leverage: f64,
    pub entry_price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub strategy: String,
    pub rationale: String,
}

pub fn make_decision(
    market_data: &PriceData,
    indicators: &TechnicalIndicators,
    order_flow: &OrderFlowSignal,
) -> Result<Decision> {
    let mut signal_count = 0;
    let mut total_confidence = 0.0;

    // Signal 1: RSI (oversold/overbought)
    if indicators.rsi < 30.0 && order_flow.direction == "LONG" {
        signal_count += 1;
        total_confidence += 0.20;
    }

    // Signal 2: Bollinger Bands
    if market_data.close < indicators.bollinger_lower && order_flow.direction == "LONG" {
        signal_count += 1;
        total_confidence += 0.15;
    }

    // Signal 3: Order Flow
    if order_flow.confidence > 0.70 {
        signal_count += 1;
        total_confidence += order_flow.confidence * 0.3;
    }

    // Signal 4: MACD
    if indicators.macd > indicators.macd_signal && order_flow.direction == "LONG" {
        signal_count += 1;
        total_confidence += 0.15;
    }

    let mut base_confidence = 0.65 + (signal_count as f64 * 0.08);
    base_confidence = f64::min(base_confidence, 0.95);

    let action = if base_confidence >= 0.70 && order_flow.direction == "LONG" {
        "BUY".to_string()
    } else if base_confidence >= 0.70 && order_flow.direction == "SHORT" {
        "SELL".to_string()
    } else {
        "SKIP".to_string()
    };

    let entry_price = market_data.close;
    let stop_loss = entry_price - (indicators.atr * 2.0);
    let take_profit = entry_price + (indicators.atr * 3.0);

    Ok(Decision {
        action,
        confidence: base_confidence,
        position_size: 100.0, // Will be calculated by risk module
        leverage: 10.0,
        entry_price,
        stop_loss,
        take_profit,
        strategy: format!("Multi-Signal ({}signals)", signal_count),
        rationale: format!(
            "RSI: {:.1}, OrderFlow: {:.2}x, Confidence: {:.2}%",
            indicators.rsi,
            order_flow.imbalance_ratio,
            base_confidence * 100.0
        ),
    })
}
