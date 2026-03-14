use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─────────────────────────── Retry configuration ─────────────────────────────
/// Maximum number of HTTP request attempts before giving up.
/// 5 attempts covers transient 502 windows: delays = 1s, 2s, 4s, 8s → ~15s total.
const MAX_RETRIES: u32 = 5;
/// Base delay for exponential back-off in milliseconds (doubles each attempt).
/// 1 000 ms base → 1 s, 2 s, 4 s, 8 s between retries.
const RETRY_BASE_MS: u64 = 1_000;

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
///
/// Returns `None` for HL-specific instrument types that have no Binance listing:
///   • `@N` symbols — HL price-level derivative contracts (e.g. @232, @7)
///   • Any symbol containing `/`  — HL spot market pairs
pub fn hl_to_binance(hl: &str) -> Option<String> {
    // Price-level derivatives: HL uses @<price> as the symbol name.
    // These are not listed on Binance and will always return HTTP 400.
    if hl.starts_with('@') { return None; }
    // Spot pairs (e.g. PURR/USDC) — not on Binance perps
    if hl.contains('/') { return None; }
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

    /// Internal helper: execute an async closure with exponential back-off retry.
    ///
    /// Retries up to `MAX_RETRIES` times on any `Err`, with delays of
    /// `RETRY_BASE_MS`, `2×RETRY_BASE_MS`, `4×RETRY_BASE_MS` (etc.) between
    /// attempts.  Returns the last error if all attempts fail.
    async fn with_retry<F, Fut, T>(op: F) -> Result<T>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut last_err = anyhow::anyhow!("unreachable");
        for attempt in 0..MAX_RETRIES {
            match op().await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    last_err = e;
                    if attempt + 1 < MAX_RETRIES {
                        let delay_ms = RETRY_BASE_MS * (1 << attempt); // 300, 600, 1200 ms
                        log::warn!("HTTP attempt {}/{} failed — retrying in {}ms: {}",
                            attempt + 1, MAX_RETRIES, delay_ms, last_err);
                        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    }
                }
            }
        }
        Err(last_err)
    }

    /// Fetch all mid-prices from Hyperliquid in a single API call (weight = 2).
    ///
    /// Returns symbol → price map for every perp traded on the exchange.
    /// Retries up to `MAX_RETRIES` times with exponential back-off on failure.
    pub async fn fetch_all_mids(&self) -> Result<HashMap<String, f64>> {
        let url  = format!("{}/info", self.hl_base);
        let body = serde_json::json!({ "type": "allMids" });
        let client = self.client.clone();

        Self::with_retry(|| {
            let url    = url.clone();
            let body   = body.clone();
            let client = client.clone();
            async move {
                let resp = client.post(&url).json(&body).send().await?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    match status.as_u16() {
                        502..=504 => anyhow::bail!(
                            "hl_api_502: Hyperliquid servers temporarily unavailable (HTTP {}). \
                             This is a normal transient blip — the bot will retry automatically \
                             and resume as soon as the API recovers. No action needed.",
                            status
                        ),
                        429 => anyhow::bail!(
                            "hl_api_429: Hyperliquid rate limit hit (HTTP 429). \
                             The bot is sending requests too quickly. Backing off.",
                        ),
                        _ => anyhow::bail!(
                            "hl_api_error: Hyperliquid allMids returned HTTP {} — \
                             if this persists check https://status.hyperliquid.xyz",
                            status
                        ),
                    }
                }
                let raw: HashMap<String, String> = resp.json().await?;
                let mids: HashMap<String, f64> = raw
                    .into_iter()
                    .filter_map(|(k, v)| v.parse::<f64>().ok().map(|p| (k, p)))
                    .collect();
                Ok(mids)
            }
        }).await
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

    /// Fetch OHLCV candles from Binance for the given Hyperliquid symbol.
    ///
    /// - `hl_symbol` — Hyperliquid ticker (e.g. `"BTC"`, `"kBONK"`)
    /// - `interval`  — Binance interval string, e.g. `"1h"` or `"4h"`
    /// - `limit`     — Number of candles to fetch (newest candle is last)
    ///
    /// Returns `Err` if the symbol has no Binance mapping (`@N` symbols, spot
    /// pairs) or if all retry attempts fail.
    async fn fetch_klines(&self, hl_symbol: &str, interval: &str, limit: u32) -> Result<Vec<PriceData>> {
        let bn_sym = hl_to_binance(hl_symbol)
            .ok_or_else(|| anyhow::anyhow!("No Binance mapping for {}", hl_symbol))?;

        let url = format!(
            "{}/api/v3/klines?symbol={}&interval={}&limit={}",
            self.bn_base, bn_sym, interval, limit
        );
        let client     = self.client.clone();
        let hl_sym_str = hl_symbol.to_string();

        Self::with_retry(|| {
            let url        = url.clone();
            let client     = client.clone();
            let bn_sym     = bn_sym.clone();
            let hl_sym_str = hl_sym_str.clone();
            async move {
                let resp = client.get(&url).send().await?;
                if !resp.status().is_success() {
                    anyhow::bail!("Binance {} {} → HTTP {}", bn_sym, interval, resp.status());
                }
                let raw: Vec<Vec<serde_json::Value>> = resp.json().await?;
                if raw.is_empty() {
                    anyhow::bail!("No candle data for {} {}", bn_sym, interval);
                }
                let candles: Vec<PriceData> = raw
                    .iter()
                    .map(|c| PriceData {
                        symbol:    hl_sym_str.clone(),
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
        }).await
    }

    /// Fetch 50 hourly candles from Binance for the given Hyperliquid symbol.
    pub async fn fetch_market_data(&self, hl_symbol: &str) -> Result<Vec<PriceData>> {
        self.fetch_klines(hl_symbol, "1h", 50).await
    }

    /// Fetch 50 four-hour candles from Binance for the given Hyperliquid symbol.
    /// Used for multi-timeframe confirmation (HTF indicators).
    /// 50 × 4h = 200 hours ≈ 8 days of context.
    pub async fn fetch_market_data_4h(&self, hl_symbol: &str) -> Result<Vec<PriceData>> {
        self.fetch_klines(hl_symbol, "4h", 50).await
    }

    /// Fetch top-20 order book depth from Binance for the given Hyperliquid symbol.
    ///
    /// Returns bids and asks as `(price, quantity)` pairs, sorted best-first.
    /// Retries up to `MAX_RETRIES` times with exponential back-off on failure.
    pub async fn fetch_order_book(&self, hl_symbol: &str) -> Result<OrderBook> {
        let bn_sym = hl_to_binance(hl_symbol)
            .ok_or_else(|| anyhow::anyhow!("No Binance mapping for {}", hl_symbol))?;

        let url    = format!("{}/api/v3/depth?symbol={}&limit=20", self.bn_base, bn_sym);
        let client = self.client.clone();
        let sym_str = hl_symbol.to_string();

        Self::with_retry(|| {
            let url     = url.clone();
            let client  = client.clone();
            let sym_str = sym_str.clone();
            async move {
                let resp = client.get(&url).send().await?;
                if !resp.status().is_success() {
                    anyhow::bail!("Binance depth {} → HTTP {}", sym_str, resp.status());
                }
                let book: serde_json::Value = resp.json().await?;

                let parse_side = |key: &str| -> Vec<(f64, f64)> {
                    book[key].as_array().map_or(vec![], |list| {
                        list.iter().filter_map(|entry| {
                            let p = entry[0].as_str()?.parse().ok()?;
                            let q = entry[1].as_str()?.parse().ok()?;
                            Some((p, q))
                        }).collect()
                    })
                };

                Ok(OrderBook {
                    symbol:    sym_str,
                    timestamp: chrono::Utc::now().timestamp_millis(),
                    bids:      parse_side("bids"),
                    asks:      parse_side("asks"),
                })
            }
        }).await
    }
}

// =============================================================================
//  Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ── hl_to_binance ─────────────────────────────────────────────────────────

    #[test]
    fn hl_to_binance_standard_symbols() {
        assert_eq!(hl_to_binance("BTC"),  Some("BTCUSDT".to_string()));
        assert_eq!(hl_to_binance("ETH"),  Some("ETHUSDT".to_string()));
        assert_eq!(hl_to_binance("SOL"),  Some("SOLUSDT".to_string()));
        assert_eq!(hl_to_binance("AVAX"), Some("AVAXUSDT".to_string()));
    }

    #[test]
    fn hl_to_binance_k_prefix_becomes_1000() {
        // HL uses "k" prefix for coins that trade in 1000-unit lots on Binance
        assert_eq!(hl_to_binance("kBONK"), Some("1000BONKUSDT".to_string()));
        assert_eq!(hl_to_binance("kPEPE"), Some("1000PEPEUSDT".to_string()));
        assert_eq!(hl_to_binance("kSHIB"), Some("1000SHIBUSDT".to_string()));
    }

    #[test]
    fn hl_to_binance_at_symbols_rejected() {
        // Price-level derivatives have no Binance listing
        assert_eq!(hl_to_binance("@232"),  None);
        assert_eq!(hl_to_binance("@7"),    None);
        assert_eq!(hl_to_binance("@1000"), None);
    }

    #[test]
    fn hl_to_binance_spot_pairs_rejected() {
        // Spot pairs (HL-specific) have no Binance perp listing
        assert_eq!(hl_to_binance("PURR/USDC"), None);
        assert_eq!(hl_to_binance("BTC/USDC"),  None);
    }

    // ── filter_candidates ─────────────────────────────────────────────────────

    fn make_mids(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn filter_candidates_always_includes_anchors() {
        let current  = make_mids(&[("BTC", 50000.0), ("ETH", 3000.0), ("SOL", 100.0),
                                    ("DOGE", 0.1), ("AVAX", 30.0)]);
        let previous = make_mids(&[("BTC", 50000.0), ("ETH", 3000.0), ("SOL", 100.0),
                                    ("DOGE", 0.1), ("AVAX", 30.0)]);
        let market = MarketClient::new();
        let result = market.filter_candidates(&current, &previous);
        assert!(result.contains(&"BTC".to_string()), "BTC must always be a candidate");
        assert!(result.contains(&"ETH".to_string()), "ETH must always be a candidate");
        assert!(result.contains(&"SOL".to_string()), "SOL must always be a candidate");
    }

    #[test]
    fn filter_candidates_caps_at_18() {
        // Build 30 symbols all with different % moves
        let current: HashMap<String, f64> = (0..30)
            .map(|i| (format!("COIN{i}"), 100.0 + i as f64))
            .chain([("BTC".to_string(), 50000.0), ("ETH".to_string(), 3000.0),
                    ("SOL".to_string(), 100.0)])
            .collect();
        let previous: HashMap<String, f64> = (0..30)
            .map(|i| (format!("COIN{i}"), 100.0))
            .chain([("BTC".to_string(), 50000.0), ("ETH".to_string(), 3000.0),
                    ("SOL".to_string(), 100.0)])
            .collect();
        let market = MarketClient::new();
        let result = market.filter_candidates(&current, &previous);
        assert!(result.len() <= 18, "Candidate list must be capped at 18, got {}", result.len());
    }

    #[test]
    fn filter_candidates_top_movers_are_included() {
        // MOON has 100% move — should be picked up over stable coins
        let current  = make_mids(&[("BTC", 50000.0), ("ETH", 3000.0), ("SOL", 100.0),
                                    ("MOON", 2.0), ("STABLE", 1.0)]);
        let previous = make_mids(&[("BTC", 50000.0), ("ETH", 3000.0), ("SOL", 100.0),
                                    ("MOON", 1.0), ("STABLE", 1.0)]);  // MOON +100%
        let market = MarketClient::new();
        let result = market.filter_candidates(&current, &previous);
        assert!(result.contains(&"MOON".to_string()),
            "Top mover MOON should be included");
    }

    #[test]
    fn filter_candidates_rejects_at_symbols() {
        // @-symbols are HL price-level derivatives — no Binance mapping, must be excluded
        let current  = make_mids(&[("BTC", 50000.0), ("ETH", 3000.0), ("SOL", 100.0),
                                    ("@232", 232.0), ("@7", 7.0)]);
        let previous = make_mids(&[("BTC", 50000.0), ("ETH", 3000.0), ("SOL", 100.0),
                                    ("@232", 100.0), ("@7", 1.0)]);  // big moves but invalid
        let market = MarketClient::new();
        let result = market.filter_candidates(&current, &previous);
        assert!(!result.contains(&"@232".to_string()), "@232 must be filtered out");
        assert!(!result.contains(&"@7".to_string()),   "@7 must be filtered out");
    }

    #[test]
    fn filter_candidates_empty_previous_gives_zero_change() {
        // Cycle 1: no previous prices, all anchors still returned
        let current  = make_mids(&[("BTC", 50000.0), ("ETH", 3000.0), ("SOL", 100.0)]);
        let previous: HashMap<String, f64> = HashMap::new();
        let market = MarketClient::new();
        let result = market.filter_candidates(&current, &previous);
        // Anchors are always included regardless
        assert!(result.contains(&"BTC".to_string()));
    }

    #[test]
    fn filter_candidates_no_duplicates() {
        // BTC is both an anchor and a top mover — should appear once
        let current  = make_mids(&[("BTC", 60000.0), ("ETH", 3000.0), ("SOL", 100.0)]);
        let previous = make_mids(&[("BTC", 50000.0), ("ETH", 3000.0), ("SOL", 100.0)]);
        let market = MarketClient::new();
        let result = market.filter_candidates(&current, &previous);
        let btc_count = result.iter().filter(|s| s.as_str() == "BTC").count();
        assert_eq!(btc_count, 1, "BTC must appear exactly once, got {}", btc_count);
    }
}
