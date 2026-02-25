use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

/// Convert a Hyperliquid symbol to a Binance USDT pair symbol.
/// e.g.  kBONK → 1000BONKUSDT,  BTC → BTCUSDT
pub fn hl_to_binance(hl: &str) -> Option<String> {
    // k-prefix on Hyperliquid means the coin trades in "1000x" units on Binance
    if let Some(base) = hl.strip_prefix('k') {
        Some(format!("1000{base}USDT"))
    } else {
        Some(format!("{hl}USDT"))
    }
}

/// Two-tier market data client.
/// Tier 1: Hyperliquid `allMids` – one call returns ALL prices (rate weight = 2)
/// Tier 2: Binance candles – fetched only for the ~18 top candidates
pub struct MarketClient {
    client: reqwest::Client,
    hl_base: String,
    bn_base: String,
}

impl MarketClient {
    pub fn new() -> Self {
        MarketClient {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            hl_base: "https://api.hyperliquid.xyz".to_string(),
            bn_base: "https://api.binance.com".to_string(),
        }
    }

    /// Fetch all mid-prices from Hyperliquid in a single API call (weight = 2).
    /// Returns symbol → price map for every perp traded on the exchange.
    pub async fn fetch_all_mids(&self) -> Result<HashMap<String, f64>> {
        let url = format!("{}/info", self.hl_base);
        let body = serde_json::json!({ "type": "allMids" });

        let resp = self.client.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("Hyperliquid allMids returned HTTP {}", resp.status());
        }

        let raw: HashMap<String, String> = resp.json().await?;
        let mids: HashMap<String, f64> = raw
            .into_iter()
            .filter_map(|(k, v)| v.parse::<f64>().ok().map(|p| (k, p)))
            .collect();

        Ok(mids)
    }

    /// Select trading candidates from the full mid-price universe.
    /// Always includes BTC, ETH, SOL anchors plus the top movers vs previous cycle.
    pub fn filter_candidates(
        &self,
        current: &HashMap<String, f64>,
        previous: &HashMap<String, f64>,
    ) -> Vec<String> {
        let anchors = ["BTC", "ETH", "SOL"];

        // Score each symbol by absolute % change since last cycle
        let mut movers: Vec<(String, f64)> = current
            .iter()
            .filter(|(sym, _)| hl_to_binance(sym).is_some())
            .filter_map(|(sym, &cur)| {
                let prev = previous.get(sym.as_str()).copied().unwrap_or(cur);
                if prev == 0.0 {
                    return None;
                }
                let pct = ((cur - prev) / prev).abs();
                Some((sym.clone(), pct))
            })
            .collect();

        movers.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Start with anchors
        let mut candidates: Vec<String> = anchors
            .iter()
            .filter(|&&s| current.contains_key(s))
            .map(|&s| s.to_string())
            .collect();

        // Fill up to 18 with top movers
        for (sym, _) in movers.iter().take(20) {
            if !candidates.contains(sym) {
                candidates.push(sym.clone());
            }
            if candidates.len() >= 18 {
                break;
            }
        }

        candidates
    }

    /// Fetch 50 hourly candles from Binance for the given Hyperliquid symbol.
    /// Returns the full candle series (newest candle last) as Vec<PriceData>.
    pub async fn fetch_market_data(&self, hl_symbol: &str) -> Result<Vec<PriceData>> {
        let bn_sym = hl_to_binance(hl_symbol)
            .ok_or_else(|| anyhow::anyhow!("No Binance mapping for {}", hl_symbol))?;

        let url = format!(
            "{}/api/v3/klines?symbol={}&interval=1h&limit=50",
            self.bn_base, bn_sym
        );

        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("Binance {} → HTTP {}", bn_sym, resp.status());
        }

        let raw: Vec<Vec<serde_json::Value>> = resp.json().await?;
        if raw.is_empty() {
            anyhow::bail!("No candle data returned for {}", bn_sym);
        }

        let candles: Vec<PriceData> = raw
            .iter()
            .map(|c| PriceData {
                symbol: hl_symbol.to_string(),
                timestamp: c[0].as_i64().unwrap_or(0),
                open:   c[1].as_str().unwrap_or("0").parse().unwrap_or(0.0),
                high:   c[2].as_str().unwrap_or("0").parse().unwrap_or(0.0),
                low:    c[3].as_str().unwrap_or("0").parse().unwrap_or(0.0),
                close:  c[4].as_str().unwrap_or("0").parse().unwrap_or(0.0),
                volume: c[7].as_str().unwrap_or("0").parse().unwrap_or(0.0),
            })
            .collect();

        Ok(candles)
    }

    /// Fetch top-20 order book depth from Binance for the given Hyperliquid symbol.
    pub async fn fetch_order_book(&self, hl_symbol: &str) -> Result<OrderBook> {
        let bn_sym = hl_to_binance(hl_symbol)
            .ok_or_else(|| anyhow::anyhow!("No Binance mapping for {}", hl_symbol))?;

        let url = format!(
            "{}/api/v3/depth?symbol={}&limit=20",
            self.bn_base, bn_sym
        );

        let resp = self.client.get(&url).send().await?;
        let book: serde_json::Value = resp.json().await?;

        let mut bids = Vec::new();
        let mut asks = Vec::new();

        if let Some(bid_list) = book["bids"].as_array() {
            for bid in bid_list {
                if let (Some(p), Some(q)) = (bid[0].as_str(), bid[1].as_str()) {
                    bids.push((p.parse().unwrap_or(0.0), q.parse().unwrap_or(0.0)));
                }
            }
        }

        if let Some(ask_list) = book["asks"].as_array() {
            for ask in ask_list {
                if let (Some(p), Some(q)) = (ask[0].as_str(), ask[1].as_str()) {
                    asks.push((p.parse().unwrap_or(0.0), q.parse().unwrap_or(0.0)));
                }
            }
        }

        Ok(OrderBook {
            symbol: hl_symbol.to_string(),
            timestamp: chrono::Utc::now().timestamp_millis(),
            bids,
            asks,
        })
    }
}
