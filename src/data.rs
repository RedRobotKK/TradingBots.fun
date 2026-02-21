use anyhow::Result;
use serde::{Deserialize, Serialize};
use crate::config::Config;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceData {
    pub symbol: String,
    pub timestamp: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBook {
    pub symbol: String,
    pub timestamp: i64,
    pub bids: Vec<(f64, f64)>,
    pub asks: Vec<(f64, f64)>,
}

#[derive(Debug)]
pub struct CexClient {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl CexClient {
    pub fn new(config: &Config) -> Result<Self> {
        Ok(CexClient {
            client: reqwest::Client::new(),
            api_key: config.binance_api_key.clone(),
            base_url: "https://api.binance.com".to_string(),
        })
    }

    pub async fn fetch_market_data(&self, symbol: &str) -> Result<PriceData> {
        let symbol_clean = symbol.replace("/", "").to_uppercase();
        let url = format!(
            "{}/api/v3/klines?symbol={}USDT&interval=1h&limit=50",
            self.base_url, symbol_clean
        );

        let resp = self.client.get(&url).send().await?;
        let candles: Vec<Vec<serde_json::Value>> = resp.json().await?;

        if candles.is_empty() {
            anyhow::bail!("No candle data received");
        }

        let last = &candles[candles.len() - 1];
        Ok(PriceData {
            symbol: symbol.to_string(),
            timestamp: last[0].as_i64().unwrap_or(0),
            open: last[1].as_str().unwrap_or("0").parse().unwrap_or(0.0),
            high: last[2].as_str().unwrap_or("0").parse().unwrap_or(0.0),
            low: last[3].as_str().unwrap_or("0").parse().unwrap_or(0.0),
            close: last[4].as_str().unwrap_or("0").parse().unwrap_or(0.0),
            volume: last[7].as_str().unwrap_or("0").parse().unwrap_or(0.0),
        })
    }

    pub async fn fetch_order_book(&self, symbol: &str) -> Result<OrderBook> {
        let symbol_clean = symbol.replace("/", "").to_uppercase();
        let url = format!(
            "{}/api/v3/depth?symbol={}USDT&limit=20",
            self.base_url, symbol_clean
        );

        let resp = self.client.get(&url).send().await?;
        let book: serde_json::Value = resp.json().await?;

        let mut bids = Vec::new();
        let mut asks = Vec::new();

        if let Some(bid_list) = book["bids"].as_array() {
            for bid in bid_list {
                if let (Some(price), Some(qty)) = (bid[0].as_str(), bid[1].as_str()) {
                    bids.push((price.parse().unwrap_or(0.0), qty.parse().unwrap_or(0.0)));
                }
            }
        }

        if let Some(ask_list) = book["asks"].as_array() {
            for ask in ask_list {
                if let (Some(price), Some(qty)) = (ask[0].as_str(), ask[1].as_str()) {
                    asks.push((price.parse().unwrap_or(0.0), qty.parse().unwrap_or(0.0)));
                }
            }
        }

        Ok(OrderBook {
            symbol: symbol.to_string(),
            timestamp: chrono::Utc::now().timestamp_millis(),
            bids,
            asks,
        })
    }
}
