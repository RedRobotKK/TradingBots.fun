use anyhow::Result;
use serde::{Deserialize, Serialize};
use crate::data::OrderBook;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderFlowSignal {
    pub bid_volume: f64,
    pub ask_volume: f64,
    pub imbalance_ratio: f64,
    pub direction: String,
    pub confidence: f64,
}

pub fn detect_order_flow(orderbook: &OrderBook) -> Result<OrderFlowSignal> {
    let bid_volume: f64 = orderbook.bids.iter().map(|(_, vol)| vol).sum();
    let ask_volume: f64 = orderbook.asks.iter().map(|(_, vol)| vol).sum();
    
    let imbalance_ratio = if ask_volume > 0.0 {
        bid_volume / ask_volume
    } else {
        1.0
    };

    let direction = if imbalance_ratio > 1.5 {
        "LONG".to_string()
    } else if imbalance_ratio < 0.67 {
        "SHORT".to_string()
    } else {
        "NEUTRAL".to_string()
    };

    let confidence = match imbalance_ratio {
        r if r > 3.0 => 0.95,
        r if r > 2.0 => 0.85,
        r if r > 1.5 => 0.70,
        _ => 0.50,
    };

    Ok(OrderFlowSignal {
        bid_volume,
        ask_volume,
        imbalance_ratio,
        direction,
        confidence,
    })
}
