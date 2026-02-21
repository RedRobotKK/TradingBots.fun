use anyhow::Result;
use serde::{Deserialize, Serialize};
use crate::data::PriceData;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TechnicalIndicators {
    pub rsi: f64,
    pub bollinger_upper: f64,
    pub bollinger_middle: f64,
    pub bollinger_lower: f64,
    pub macd: f64,
    pub macd_signal: f64,
    pub atr: f64,
}

pub fn calculate_all(price_data: &PriceData) -> Result<TechnicalIndicators> {
    // Simplified for now - in production, track 50+ candles
    let rsi = calculate_rsi(price_data.close, 14);
    let (upper, middle, lower) = calculate_bollinger(price_data.close, 20, 2.0);
    let (macd, signal) = calculate_macd(price_data.close);
    let atr = calculate_atr(price_data.high, price_data.low, price_data.close, 14);

    Ok(TechnicalIndicators {
        rsi,
        bollinger_upper: upper,
        bollinger_middle: middle,
        bollinger_lower: lower,
        macd,
        macd_signal: signal,
        atr,
    })
}

fn calculate_rsi(price: f64, period: i32) -> f64 {
    // Simplified RSI - in production, use proper calculation
    if price > 100.0 {
        30.0 + (price - 100.0) as i32.min(70) as f64
    } else {
        50.0
    }
}

fn calculate_bollinger(price: f64, period: i32, std_dev: f64) -> (f64, f64, f64) {
    // Simplified - in production, calculate properly from multiple candles
    let middle = price;
    let offset = price * 0.02 * std_dev;
    (middle + offset, middle, middle - offset)
}

fn calculate_macd(price: f64) -> (f64, f64) {
    // Simplified - in production, use proper MACD calculation
    (price * 0.001, price * 0.0008)
}

fn calculate_atr(high: f64, low: f64, close: f64, period: i32) -> f64 {
    // Simplified ATR
    ((high - low) + (high - close).abs() + (low - close).abs()) / 3.0
}
