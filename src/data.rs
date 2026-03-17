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

/// Maximum number of candidate symbols to analyse per cycle.
/// With HL's native candle API (one POST per coin, no external rate-limit),
/// 40 pairs fit comfortably within a 30-second cycle budget.
const MAX_CANDIDATES: usize = 40;

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

/// Returns `true` for vanilla Hyperliquid perp symbols that have native candle data.
///
/// Excludes two HL-specific instrument types the candle API cannot serve:
///   • `@N` symbols — HL price-level derivative contracts (e.g. @232, @7)
///   • Any symbol containing `/`  — HL spot market pairs (e.g. PURR/USDC)
#[inline]
pub fn is_hl_perp(sym: &str) -> bool {
    !sym.starts_with('@') && !sym.contains('/')
}

/// Single-source market data client — Hyperliquid only.
///
/// Tier 1: `allMids`        — one POST returns ALL ~150+ perp prices
/// Tier 2: `candleSnapshot` — one POST per coin, no Binance rate-limit concern
/// Tier 3: `l2Book`         — one POST per coin for order book depth
pub struct MarketClient {
    client:  reqwest::Client,
    hl_base: String,
}

impl MarketClient {
    pub fn new() -> Self {
        MarketClient {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            hl_base: "https://api.hyperliquid.xyz".to_string(),
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
                        let delay_ms = RETRY_BASE_MS * (1 << attempt);
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
        let url    = format!("{}/info", self.hl_base);
        let body   = serde_json::json!({ "type": "allMids" });
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
    ///
    /// Always includes BTC/ETH/SOL anchors, then fills to `MAX_CANDIDATES` with
    /// the top movers by absolute % change since the previous cycle.
    /// Filters out HL-specific non-perp instruments (`@N` and `/` symbols).
    pub fn filter_candidates(
        &self,
        current: &HashMap<String, f64>,
        previous: &HashMap<String, f64>,
    ) -> Vec<String> {
        let anchors = ["BTC", "ETH", "SOL"];

        // Score each valid HL perp by absolute % change since last cycle
        let mut movers: Vec<(String, f64)> = current
            .iter()
            .filter(|(sym, _)| is_hl_perp(sym))
            .filter_map(|(sym, &cur)| {
                let prev = previous.get(sym.as_str()).copied().unwrap_or(cur);
                if prev == 0.0 { return None; }
                let pct = ((cur - prev) / prev).abs();
                Some((sym.clone(), pct))
            })
            .collect();

        movers.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Start with anchors (always included regardless of move size)
        let mut candidates: Vec<String> = anchors
            .iter()
            .filter(|&&s| current.contains_key(s))
            .map(|&s| s.to_string())
            .collect();

        // Fill to MAX_CANDIDATES with top movers
        for (sym, _) in &movers {
            if candidates.len() >= MAX_CANDIDATES { break; }
            if !candidates.contains(sym) {
                candidates.push(sym.clone());
            }
        }

        candidates
    }

    /// Fetch OHLCV candles from Hyperliquid's native `candleSnapshot` endpoint.
    ///
    /// - `coin`     — Hyperliquid ticker (e.g. `"BTC"`, `"kBONK"`)
    /// - `interval` — HL interval string: `"1h"`, `"4h"`, `"15m"`, etc.
    /// - `limit`    — Number of candles to return (newest last)
    ///
    /// `startTime` is derived as `now − limit × interval_ms` so we always get
    /// exactly `limit` closed candles regardless of the current time-of-day.
    /// History on HL goes back to ~2023 — sufficient for 14-period RSI / 20-period MACD.
    async fn fetch_hl_candles(&self, coin: &str, interval: &str, limit: u32) -> Result<Vec<PriceData>> {
        let interval_ms: i64 = match interval {
            "1m"  =>       60_000,
            "5m"  =>      300_000,
            "15m" =>      900_000,
            "1h"  =>    3_600_000,
            "4h"  =>   14_400_000,
            "1d"  =>   86_400_000,
            other => anyhow::bail!("Unknown interval: {}", other),
        };

        let now_ms    = chrono::Utc::now().timestamp_millis();
        // Extra +2 candles of buffer so the window always contains `limit` closed bars
        let start_ms  = now_ms - (limit as i64 + 2) * interval_ms;

        let url    = format!("{}/info", self.hl_base);
        let body   = serde_json::json!({
            "type": "candleSnapshot",
            "req": {
                "coin":      coin,
                "interval":  interval,
                "startTime": start_ms,
                "endTime":   now_ms
            }
        });
        let client   = self.client.clone();
        let coin_str = coin.to_string();

        Self::with_retry(|| {
            let url      = url.clone();
            let body     = body.clone();
            let client   = client.clone();
            let coin_str = coin_str.clone();
            async move {
                let resp = client.post(&url).json(&body).send().await?;
                if !resp.status().is_success() {
                    anyhow::bail!("HL candleSnapshot {} {} → HTTP {}", coin_str, interval, resp.status());
                }
                // HL returns an array of candle objects:
                // {"t":<open_ms>,"T":<close_ms>,"s":"BTC","i":"1h",
                //  "o":"50000","h":"51000","l":"49500","c":"50800","v":"1234","n":500}
                let raw: Vec<serde_json::Value> = resp.json().await?;
                if raw.is_empty() {
                    anyhow::bail!("No candle data for {} {}", coin_str, interval);
                }

                let parse = |v: &serde_json::Value, key: &str| -> f64 {
                    v[key].as_str()
                        .and_then(|s| s.parse().ok())
                        .or_else(|| v[key].as_f64())
                        .unwrap_or(0.0)
                };

                let mut candles: Vec<PriceData> = raw
                    .iter()
                    .map(|c| PriceData {
                        symbol:    coin_str.clone(),
                        timestamp: c["t"].as_i64().unwrap_or(0),
                        open:      parse(c, "o"),
                        high:      parse(c, "h"),
                        low:       parse(c, "l"),
                        close:     parse(c, "c"),
                        volume:    parse(c, "v"),
                    })
                    .collect();

                // Trim to exactly `limit` (drop the oldest if buffer over-fetched)
                if candles.len() > limit as usize {
                    let drop = candles.len() - limit as usize;
                    candles.drain(..drop);
                }

                Ok(candles)
            }
        }).await
    }

    /// Fetch 50 hourly candles from Hyperliquid for the given symbol.
    /// 50 × 1h = ~2 days of 1h context (sufficient for RSI-14, MACD-26).
    pub async fn fetch_market_data(&self, coin: &str) -> Result<Vec<PriceData>> {
        self.fetch_hl_candles(coin, "1h", 50).await
    }

    /// Fetch 50 four-hour candles from Hyperliquid for the given symbol.
    /// Used for multi-timeframe confirmation (HTF indicators).
    /// 50 × 4h = 200 hours ≈ 8 days of context.
    pub async fn fetch_market_data_4h(&self, coin: &str) -> Result<Vec<PriceData>> {
        self.fetch_hl_candles(coin, "4h", 50).await
    }

    /// Fetch order book depth from Hyperliquid's `l2Book` endpoint.
    ///
    /// Returns top-20 bids and asks as `(price, quantity)` pairs, sorted best-first.
    /// Retries up to `MAX_RETRIES` times with exponential back-off on failure.
    pub async fn fetch_order_book(&self, coin: &str) -> Result<OrderBook> {
        let url     = format!("{}/info", self.hl_base);
        let body    = serde_json::json!({ "type": "l2Book", "coin": coin });
        let client  = self.client.clone();
        let sym_str = coin.to_string();

        Self::with_retry(|| {
            let url     = url.clone();
            let body    = body.clone();
            let client  = client.clone();
            let sym_str = sym_str.clone();
            async move {
                let resp = client.post(&url).json(&body).send().await?;
                if !resp.status().is_success() {
                    anyhow::bail!("HL l2Book {} → HTTP {}", sym_str, resp.status());
                }
                // HL l2Book response:
                // {"coin":"BTC","levels":[[bids…],[asks…]],"time":…}
                // Each entry: {"n":<orders>,"px":"price","sz":"size"}
                let book: serde_json::Value = resp.json().await?;

                let parse_side = |idx: usize| -> Vec<(f64, f64)> {
                    book["levels"][idx].as_array().map_or(vec![], |list| {
                        list.iter().take(20).filter_map(|entry| {
                            let p: f64 = entry["px"].as_str()?.parse().ok()?;
                            let q: f64 = entry["sz"].as_str()?.parse().ok()?;
                            Some((p, q))
                        }).collect()
                    })
                };

                Ok(OrderBook {
                    symbol:    sym_str,
                    timestamp: book["time"].as_i64()
                                   .unwrap_or_else(|| chrono::Utc::now().timestamp_millis()),
                    bids: parse_side(0),
                    asks: parse_side(1),
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

    // ── is_hl_perp ─────────────────────────────────────────────────────────────

    #[test]
    fn is_hl_perp_accepts_standard_perps() {
        assert!(is_hl_perp("BTC"),   "BTC should be a valid HL perp");
        assert!(is_hl_perp("ETH"),   "ETH should be a valid HL perp");
        assert!(is_hl_perp("kBONK"),"kBONK should be a valid HL perp");
        assert!(is_hl_perp("AVAX"), "AVAX should be a valid HL perp");
    }

    #[test]
    fn is_hl_perp_rejects_at_symbols() {
        assert!(!is_hl_perp("@232"),  "@232 is a price-level derivative");
        assert!(!is_hl_perp("@7"),    "@7 is a price-level derivative");
        assert!(!is_hl_perp("@1000"),"@1000 is a price-level derivative");
    }

    #[test]
    fn is_hl_perp_rejects_spot_pairs() {
        assert!(!is_hl_perp("PURR/USDC"), "PURR/USDC is a spot pair");
        assert!(!is_hl_perp("BTC/USDC"),  "BTC/USDC is a spot pair");
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
    fn filter_candidates_caps_at_max() {
        // Build 60 symbols all with different % moves — should be capped at MAX_CANDIDATES
        let current: HashMap<String, f64> = (0..60)
            .map(|i| (format!("COIN{i}"), 100.0 + i as f64))
            .chain([("BTC".to_string(), 50000.0), ("ETH".to_string(), 3000.0),
                    ("SOL".to_string(), 100.0)])
            .collect();
        let previous: HashMap<String, f64> = (0..60)
            .map(|i| (format!("COIN{i}"), 100.0))
            .chain([("BTC".to_string(), 50000.0), ("ETH".to_string(), 3000.0),
                    ("SOL".to_string(), 100.0)])
            .collect();
        let market = MarketClient::new();
        let result = market.filter_candidates(&current, &previous);
        assert!(
            result.len() <= MAX_CANDIDATES,
            "Candidate list must be capped at {MAX_CANDIDATES}, got {}",
            result.len()
        );
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
        assert!(result.contains(&"MOON".to_string()), "Top mover MOON should be included");
    }

    #[test]
    fn filter_candidates_rejects_at_symbols() {
        // @-symbols are HL price-level derivatives — must be excluded
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
    fn filter_candidates_rejects_spot_pairs() {
        // Spot pairs have a "/" — must be excluded even with large moves
        let current  = make_mids(&[("BTC", 50000.0), ("ETH", 3000.0), ("SOL", 100.0),
                                    ("PURR/USDC", 2.0)]);
        let previous = make_mids(&[("BTC", 50000.0), ("ETH", 3000.0), ("SOL", 100.0),
                                    ("PURR/USDC", 0.01)]);  // 200× move but spot pair
        let market = MarketClient::new();
        let result = market.filter_candidates(&current, &previous);
        assert!(!result.contains(&"PURR/USDC".to_string()), "Spot pairs must be filtered out");
    }

    #[test]
    fn filter_candidates_empty_previous_gives_zero_change() {
        // Cycle 1: no previous prices — anchors still returned
        let current  = make_mids(&[("BTC", 50000.0), ("ETH", 3000.0), ("SOL", 100.0)]);
        let previous: HashMap<String, f64> = HashMap::new();
        let market = MarketClient::new();
        let result = market.filter_candidates(&current, &previous);
        assert!(result.contains(&"BTC".to_string()));
    }

    #[test]
    fn filter_candidates_no_duplicates() {
        // BTC is both an anchor and a top mover — should appear exactly once
        let current  = make_mids(&[("BTC", 60000.0), ("ETH", 3000.0), ("SOL", 100.0)]);
        let previous = make_mids(&[("BTC", 50000.0), ("ETH", 3000.0), ("SOL", 100.0)]);
        let market = MarketClient::new();
        let result = market.filter_candidates(&current, &previous);
        let btc_count = result.iter().filter(|s| s.as_str() == "BTC").count();
        assert_eq!(btc_count, 1, "BTC must appear exactly once, got {}", btc_count);
    }
}
