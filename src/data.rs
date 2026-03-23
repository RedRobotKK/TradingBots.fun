use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};

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

/// How long a cached allMids or order-book snapshot is considered fresh.
///
/// All 9 tenant loops share a single `Arc<MarketClient>`.  Without a shared
/// cache they would fire identical HTTP requests to Hyperliquid in a tight
/// burst every 30-second cycle, triggering HTTP 429 rate-limit responses.
///
/// With a 25-second TTL the first tenant that needs a snapshot fetches it;
/// every other tenant that arrives within the same cycle window gets the
/// in-memory copy.  Net result: at most 1 allMids request and ~40 l2Book
/// requests per 30-second cycle — down from up to 9× each.
const PRICE_ORACLE_TTL: Duration = Duration::from_secs(25);

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

// ─────────────────────────── Candle cache ────────────────────────────────────

/// One cached interval's worth of candles for a single coin.
///
/// `bar_epoch` = `floor(fetch_time_ms / interval_ms)` — the integer index of
/// the 1h (or 4h) bar that was current when we last fetched.  When the next
/// cycle runs we compare the current bar epoch; if it's advanced we know a new
/// bar has closed and the candles need refreshing.
struct CachedCandles {
    candles: Vec<PriceData>,
    bar_epoch: i64, // floor(fetch_time_ms / interval_ms)
}

// ─────────────────────────── Helpers ─────────────────────────────────────────

/// Returns `true` for vanilla Hyperliquid perp symbols that have native candle data.
///
/// Excludes two HL-specific instrument types the candle API cannot serve:
///   • `@N` symbols — HL price-level derivative contracts (e.g. @232, @7)
///   • Any symbol containing `/`  — HL spot market pairs (e.g. PURR/USDC)
#[inline]
pub fn is_hl_perp(sym: &str) -> bool {
    !sym.starts_with('@') && !sym.contains('/')
}

/// Convert an interval string to its duration in milliseconds.
#[inline]
fn interval_ms(interval: &str) -> Result<i64> {
    match interval {
        "1m" => Ok(60_000),
        "5m" => Ok(300_000),
        "15m" => Ok(900_000),
        "1h" => Ok(3_600_000),
        "4h" => Ok(14_400_000),
        "1d" => Ok(86_400_000),
        other => anyhow::bail!("Unknown interval: {}", other),
    }
}

// ─────────────────────────── Market client ───────────────────────────────────

/// Single-source market data client — Hyperliquid only.
///
/// # Rate budget (HL REST, 1 200 weight/min aggregate)
///
/// | Call              | Weight | Frequency          |
/// |-------------------|--------|--------------------|
/// | `allMids`         |      2 | every cycle (30 s) |
/// | `l2Book` × 40    |     80 | every cycle (30 s) |
/// | `candleSnapshot`  |     20 | once per new bar   |
///
/// Per-cycle budget: allMids(2) + 40×l2Book(80) = **82 weight** ≈ 164/min
/// Candle refreshes: 40×1h = 800 weight once/hour, 40×4h = 800 weight once/4h
/// Both well within the 1 200/min ceiling.
pub struct MarketClient {
    client: reqwest::Client,
    hl_base: String,
    /// Candle cache keyed by `"COIN:interval"` (e.g. `"BTC:1h"`).
    /// Interior-mutable so `&self` methods can update it.
    candle_cache: RwLock<HashMap<String, CachedCandles>>,

    // ── Shared pricing oracle ─────────────────────────────────────────────────
    // All 9 tenant loops share one Arc<MarketClient>.  These caches ensure that
    // only ONE HTTP request is sent to Hyperliquid per PRICE_ORACLE_TTL window
    // regardless of how many tenants ask for the same data simultaneously.

    /// allMids oracle: (fetched_at, symbol → mid_price)
    mids_oracle: Mutex<Option<(Instant, HashMap<String, f64>)>>,
    /// Order-book oracle: symbol → (fetched_at, OrderBook)
    book_oracle: RwLock<HashMap<String, (Instant, OrderBook)>>,
}

impl MarketClient {
    pub fn new() -> Self {
        MarketClient {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            hl_base: "https://api.hyperliquid.xyz".to_string(),
            candle_cache: RwLock::new(HashMap::new()),
            mids_oracle: Mutex::new(None),
            book_oracle: RwLock::new(HashMap::new()),
        }
    }

    // ── Retry helper ──────────────────────────────────────────────────────────

    /// Execute an async closure with exponential back-off retry.
    ///
    /// Retries up to `MAX_RETRIES` times on any `Err`, with delays of
    /// `RETRY_BASE_MS × 2^attempt` between attempts.
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
                        log::warn!(
                            "HTTP attempt {}/{} failed — retrying in {}ms: {}",
                            attempt + 1,
                            MAX_RETRIES,
                            delay_ms,
                            last_err
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    }
                }
            }
        }
        Err(last_err)
    }

    // ── Tier 1: allMids ───────────────────────────────────────────────────────

    /// Fetch all mid-prices from Hyperliquid (weight = 2), with oracle caching.
    ///
    /// If a snapshot was fetched within `PRICE_ORACLE_TTL` (25 s) it is returned
    /// immediately — no HTTP round-trip.  This means all 9 tenant loops that
    /// share this `Arc<MarketClient>` share the same fresh snapshot and only the
    /// *first* caller within each 30-second window pays the network cost.
    pub async fn fetch_all_mids(&self) -> Result<HashMap<String, f64>> {
        // Fast path: return cached snapshot if still fresh.
        {
            let guard = self.mids_oracle.lock().await;
            if let Some((fetched_at, ref mids)) = *guard {
                if fetched_at.elapsed() < PRICE_ORACLE_TTL {
                    return Ok(mids.clone());
                }
            }
        }

        // Slow path: fetch from Hyperliquid and update the oracle.
        let url = format!("{}/info", self.hl_base);
        let body = serde_json::json!({ "type": "allMids" });
        let client = self.client.clone();

        let mids = Self::with_retry(|| {
            let url = url.clone();
            let body = body.clone();
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
        })
        .await?;

        // Store in oracle so subsequent callers within the TTL skip the HTTP call.
        *self.mids_oracle.lock().await = Some((Instant::now(), mids.clone()));
        Ok(mids)
    }

    // ── Candidate selection ───────────────────────────────────────────────────

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

        let mut movers: Vec<(String, f64)> = current
            .iter()
            .filter(|(sym, _)| is_hl_perp(sym))
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

        let mut candidates: Vec<String> = anchors
            .iter()
            .filter(|&&s| current.contains_key(s))
            .map(|&s| s.to_string())
            .collect();

        for (sym, _) in &movers {
            if candidates.len() >= MAX_CANDIDATES {
                break;
            }
            if !candidates.contains(sym) {
                candidates.push(sym.clone());
            }
        }

        candidates
    }

    // ── Tier 2: candles (cached) ──────────────────────────────────────────────

    /// Fetch OHLCV candles, returning cached data when the current bar has not
    /// yet closed since the last HTTP fetch.
    ///
    /// # Cache logic
    /// A new bar closes when `floor(now_ms / interval_ms)` advances past the
    /// value recorded at fetch time.  Until then every 30-second cycle reuses
    /// the in-memory copy — saving ~800 HL weight units per hour for 40 coins.
    ///
    /// **Staggered refresh (thundering-herd prevention):**
    /// All 40 coins would otherwise miss their cache simultaneously at every
    /// bar boundary, producing a burst of ~80 HL API calls in the same 30-second
    /// window.  To spread these across the minute, each `(coin, interval)` pair
    /// has a deterministic jitter `hash(coin) % 60` seconds applied to `now_ms`
    /// before the epoch comparison.  A coin with jitter=45 sees the new bar 45
    /// seconds *later* than a coin with jitter=0, spreading 40 fetches evenly
    /// across a full 60-second window.
    ///
    /// An unconditional HTTP fetch is always made on the first call for a
    /// `(coin, interval)` pair (cold start / bot restart).
    async fn fetch_hl_candles_cached(
        &self,
        coin: &str,
        interval: &str,
        limit: u32,
    ) -> Result<Vec<PriceData>> {
        let ivl_ms = interval_ms(interval)?;
        let now_ms = chrono::Utc::now().timestamp_millis();

        // Per-coin jitter: deterministic hash-based offset in [0, 59] seconds.
        // We subtract this from now_ms before computing the epoch, which delays
        // when this (coin, interval) pair "notices" a new bar has opened.
        // Using a simple djb2-style hash over the coin bytes is fast and stable.
        let jitter_ms: i64 = {
            let hash: u64 = coin.bytes().fold(5381u64, |acc, b| {
                acc.wrapping_mul(33).wrapping_add(b as u64)
            });
            (hash % 60) as i64 * 1_000 // 0..=59 seconds in milliseconds
        };
        let jittered_ms = now_ms - jitter_ms;
        let cur_epoch = jittered_ms / ivl_ms; // integer bar index for this coin

        let cache_key = format!("{}:{}", coin, interval);

        // Fast path: cache hit — same bar epoch for this coin's jitter slot.
        {
            let r = self.candle_cache.read().await;
            if let Some(entry) = r.get(&cache_key) {
                if entry.bar_epoch == cur_epoch {
                    log::debug!(
                        "candle_cache HIT  {}:{} epoch={}",
                        coin,
                        interval,
                        cur_epoch
                    );
                    return Ok(entry.candles.clone());
                }
            }
        }

        // Slow path: fetch from HL (first call, or new bar has opened for this coin).
        log::debug!(
            "candle_cache MISS {}:{} epoch={} jitter={}ms — fetching",
            coin,
            interval,
            cur_epoch,
            jitter_ms
        );
        let candles = self.fetch_hl_candles_raw(coin, interval, limit).await?;

        // Write back; grab write lock only after the HTTP round trip.
        {
            let mut w = self.candle_cache.write().await;
            w.insert(
                cache_key,
                CachedCandles {
                    candles: candles.clone(),
                    bar_epoch: cur_epoch,
                },
            );
        }

        Ok(candles)
    }

    /// Raw HTTP fetch from Hyperliquid `candleSnapshot` — no caching.
    ///
    /// - `coin`     — Hyperliquid ticker (e.g. `"BTC"`, `"kBONK"`)
    /// - `interval` — HL interval string: `"1h"`, `"4h"`, `"15m"`, etc.
    /// - `limit`    — Number of candles to return (newest last)
    async fn fetch_hl_candles_raw(
        &self,
        coin: &str,
        interval: &str,
        limit: u32,
    ) -> Result<Vec<PriceData>> {
        let ivl_ms = interval_ms(interval)?;
        let now_ms = chrono::Utc::now().timestamp_millis();
        // +2 bar buffer so the window always contains `limit` fully-closed bars.
        let start_ms = now_ms - (limit as i64 + 2) * ivl_ms;

        let url = format!("{}/info", self.hl_base);
        let body = serde_json::json!({
            "type": "candleSnapshot",
            "req": {
                "coin":      coin,
                "interval":  interval,
                "startTime": start_ms,
                "endTime":   now_ms
            }
        });
        let client = self.client.clone();
        let coin_str = coin.to_string();

        Self::with_retry(|| {
            let url = url.clone();
            let body = body.clone();
            let client = client.clone();
            let coin_str = coin_str.clone();
            async move {
                let resp = client.post(&url).json(&body).send().await?;
                if !resp.status().is_success() {
                    anyhow::bail!(
                        "HL candleSnapshot {} {} → HTTP {}",
                        coin_str,
                        interval,
                        resp.status()
                    );
                }
                // HL candle shape:
                // {"t":<open_ms>,"T":<close_ms>,"s":"BTC","i":"1h",
                //  "o":"50000","h":"51000","l":"49500","c":"50800","v":"1234","n":500}
                let raw: Vec<serde_json::Value> = resp.json().await?;
                if raw.is_empty() {
                    anyhow::bail!("No candle data for {} {}", coin_str, interval);
                }

                let parse = |v: &serde_json::Value, key: &str| -> f64 {
                    v[key]
                        .as_str()
                        .and_then(|s| s.parse().ok())
                        .or_else(|| v[key].as_f64())
                        .unwrap_or(0.0)
                };

                let mut candles: Vec<PriceData> = raw
                    .iter()
                    .map(|c| PriceData {
                        symbol: coin_str.clone(),
                        timestamp: c["t"].as_i64().unwrap_or(0),
                        open: parse(c, "o"),
                        high: parse(c, "h"),
                        low: parse(c, "l"),
                        close: parse(c, "c"),
                        volume: parse(c, "v"),
                    })
                    .collect();

                // Trim to exactly `limit` (the buffer may return a few extra).
                if candles.len() > limit as usize {
                    candles.drain(..candles.len() - limit as usize);
                }

                Ok(candles)
            }
        })
        .await
    }

    /// Fetch 50 hourly candles (cached — refetches at most once per 1h bar close).
    pub async fn fetch_market_data(&self, coin: &str) -> Result<Vec<PriceData>> {
        self.fetch_hl_candles_cached(coin, "1h", 50).await
    }

    /// Fetch 50 four-hour candles (cached — refetches at most once per 4h bar close).
    pub async fn fetch_market_data_4h(&self, coin: &str) -> Result<Vec<PriceData>> {
        self.fetch_hl_candles_cached(coin, "4h", 50).await
    }

    // ── Tier 3: order book (per-cycle, not cached) ────────────────────────────

    /// Fetch order book depth from Hyperliquid's `l2Book` endpoint (weight = 2),
    /// with per-symbol oracle caching (TTL = `PRICE_ORACLE_TTL`).
    ///
    /// Returns top-20 bids and asks as `(price, quantity)` pairs, sorted best-first.
    /// Multiple tenant loops requesting the same symbol within the TTL window
    /// share a single cached snapshot — only one HTTP request is sent.
    pub async fn fetch_order_book(&self, coin: &str) -> Result<OrderBook> {
        // Fast path: return cached book if still fresh.
        {
            let guard = self.book_oracle.read().await;
            if let Some((fetched_at, ref book)) = guard.get(coin) {
                if fetched_at.elapsed() < PRICE_ORACLE_TTL {
                    return Ok(book.clone());
                }
            }
        }

        // Slow path: fetch from Hyperliquid.
        let url = format!("{}/info", self.hl_base);
        let body = serde_json::json!({ "type": "l2Book", "coin": coin });
        let client = self.client.clone();
        let sym_str = coin.to_string();

        let book = Self::with_retry(|| {
            let url = url.clone();
            let body = body.clone();
            let client = client.clone();
            let sym_str = sym_str.clone();
            async move {
                let resp = client.post(&url).json(&body).send().await?;
                if !resp.status().is_success() {
                    anyhow::bail!("HL l2Book {} → HTTP {}", sym_str, resp.status());
                }
                // HL l2Book:
                // {"coin":"BTC","levels":[[bids…],[asks…]],"time":…}
                // Each entry: {"n":<orders>,"px":"price","sz":"size"}
                let book: serde_json::Value = resp.json().await?;

                let parse_side = |idx: usize| -> Vec<(f64, f64)> {
                    book["levels"][idx].as_array().map_or(vec![], |list| {
                        list.iter()
                            .take(20)
                            .filter_map(|entry| {
                                let p: f64 = entry["px"].as_str()?.parse().ok()?;
                                let q: f64 = entry["sz"].as_str()?.parse().ok()?;
                                Some((p, q))
                            })
                            .collect()
                    })
                };

                Ok(OrderBook {
                    symbol: sym_str,
                    timestamp: book["time"]
                        .as_i64()
                        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis()),
                    bids: parse_side(0),
                    asks: parse_side(1),
                })
            }
        })
        .await?;

        // Cache the freshly-fetched book for subsequent callers.
        self.book_oracle
            .write()
            .await
            .insert(coin.to_string(), (Instant::now(), book.clone()));
        Ok(book)
    }

    /// Evict all cached candles (e.g. after a bot restart or manual reset).
    /// Normal operation never needs to call this.
    #[allow(dead_code)]
    pub async fn clear_candle_cache(&self) {
        self.candle_cache.write().await.clear();
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
        assert!(is_hl_perp("BTC"), "BTC should be a valid HL perp");
        assert!(is_hl_perp("ETH"), "ETH should be a valid HL perp");
        assert!(is_hl_perp("kBONK"), "kBONK should be a valid HL perp");
        assert!(is_hl_perp("AVAX"), "AVAX should be a valid HL perp");
    }

    #[test]
    fn is_hl_perp_rejects_at_symbols() {
        assert!(!is_hl_perp("@232"), "@232 is a price-level derivative");
        assert!(!is_hl_perp("@7"), "@7 is a price-level derivative");
        assert!(!is_hl_perp("@1000"), "@1000 is a price-level derivative");
    }

    #[test]
    fn is_hl_perp_rejects_spot_pairs() {
        assert!(!is_hl_perp("PURR/USDC"), "PURR/USDC is a spot pair");
        assert!(!is_hl_perp("BTC/USDC"), "BTC/USDC is a spot pair");
    }

    // ── interval_ms ───────────────────────────────────────────────────────────

    #[test]
    fn interval_ms_known_intervals() {
        assert_eq!(interval_ms("1m").unwrap(), 60_000);
        assert_eq!(interval_ms("1h").unwrap(), 3_600_000);
        assert_eq!(interval_ms("4h").unwrap(), 14_400_000);
        assert_eq!(interval_ms("1d").unwrap(), 86_400_000);
    }

    #[test]
    fn interval_ms_unknown_returns_err() {
        assert!(interval_ms("3h").is_err());
        assert!(interval_ms("").is_err());
    }

    // ── filter_candidates ─────────────────────────────────────────────────────

    fn make_mids(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn filter_candidates_always_includes_anchors() {
        let current = make_mids(&[
            ("BTC", 50000.0),
            ("ETH", 3000.0),
            ("SOL", 100.0),
            ("DOGE", 0.1),
            ("AVAX", 30.0),
        ]);
        let previous = current.clone();
        let market = MarketClient::new();
        let result = market.filter_candidates(&current, &previous);
        assert!(
            result.contains(&"BTC".to_string()),
            "BTC must always be a candidate"
        );
        assert!(
            result.contains(&"ETH".to_string()),
            "ETH must always be a candidate"
        );
        assert!(
            result.contains(&"SOL".to_string()),
            "SOL must always be a candidate"
        );
    }

    #[test]
    fn filter_candidates_caps_at_max() {
        let current: HashMap<String, f64> = (0..60)
            .map(|i| (format!("COIN{i}"), 100.0 + i as f64))
            .chain([
                ("BTC".to_string(), 50000.0),
                ("ETH".to_string(), 3000.0),
                ("SOL".to_string(), 100.0),
            ])
            .collect();
        let previous: HashMap<String, f64> = (0..60)
            .map(|i| (format!("COIN{i}"), 100.0))
            .chain([
                ("BTC".to_string(), 50000.0),
                ("ETH".to_string(), 3000.0),
                ("SOL".to_string(), 100.0),
            ])
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
        let current = make_mids(&[
            ("BTC", 50000.0),
            ("ETH", 3000.0),
            ("SOL", 100.0),
            ("MOON", 2.0),
            ("STABLE", 1.0),
        ]);
        let previous = make_mids(&[
            ("BTC", 50000.0),
            ("ETH", 3000.0),
            ("SOL", 100.0),
            ("MOON", 1.0),
            ("STABLE", 1.0),
        ]);
        let market = MarketClient::new();
        let result = market.filter_candidates(&current, &previous);
        assert!(
            result.contains(&"MOON".to_string()),
            "Top mover MOON should be included"
        );
    }

    #[test]
    fn filter_candidates_rejects_at_symbols() {
        let current = make_mids(&[
            ("BTC", 50000.0),
            ("ETH", 3000.0),
            ("SOL", 100.0),
            ("@232", 232.0),
            ("@7", 7.0),
        ]);
        let previous = make_mids(&[
            ("BTC", 50000.0),
            ("ETH", 3000.0),
            ("SOL", 100.0),
            ("@232", 100.0),
            ("@7", 1.0),
        ]);
        let market = MarketClient::new();
        let result = market.filter_candidates(&current, &previous);
        assert!(
            !result.contains(&"@232".to_string()),
            "@232 must be filtered out"
        );
        assert!(
            !result.contains(&"@7".to_string()),
            "@7 must be filtered out"
        );
    }

    #[test]
    fn filter_candidates_rejects_spot_pairs() {
        let current = make_mids(&[
            ("BTC", 50000.0),
            ("ETH", 3000.0),
            ("SOL", 100.0),
            ("PURR/USDC", 2.0),
        ]);
        let previous = make_mids(&[
            ("BTC", 50000.0),
            ("ETH", 3000.0),
            ("SOL", 100.0),
            ("PURR/USDC", 0.01),
        ]);
        let market = MarketClient::new();
        let result = market.filter_candidates(&current, &previous);
        assert!(
            !result.contains(&"PURR/USDC".to_string()),
            "Spot pairs must be filtered out"
        );
    }

    #[test]
    fn filter_candidates_no_duplicates() {
        let current = make_mids(&[("BTC", 60000.0), ("ETH", 3000.0), ("SOL", 100.0)]);
        let previous = make_mids(&[("BTC", 50000.0), ("ETH", 3000.0), ("SOL", 100.0)]);
        let market = MarketClient::new();
        let result = market.filter_candidates(&current, &previous);
        let btc_count = result.iter().filter(|s| s.as_str() == "BTC").count();
        assert_eq!(
            btc_count, 1,
            "BTC must appear exactly once, got {}",
            btc_count
        );
    }

    #[test]
    fn filter_candidates_empty_previous_gives_zero_change() {
        let current = make_mids(&[("BTC", 50000.0), ("ETH", 3000.0), ("SOL", 100.0)]);
        let previous = HashMap::new();
        let market = MarketClient::new();
        let result = market.filter_candidates(&current, &previous);
        assert!(result.contains(&"BTC".to_string()));
    }

    // ── Candle cache jitter ───────────────────────────────────────────────────

    /// The jitter function must produce values in [0, 59] seconds (inclusive).
    #[test]
    fn candle_jitter_in_range() {
        let symbols = &[
            "BTC", "ETH", "SOL", "AVAX", "kBONK", "kPEPE", "DOGE", "WIF", "ARB", "OP", "LINK",
            "AAVE", "UNI", "MKR",
        ];
        for sym in symbols {
            let jitter_ms: i64 = {
                let hash: u64 = sym.bytes().fold(5381u64, |acc, b| {
                    acc.wrapping_mul(33).wrapping_add(b as u64)
                });
                (hash % 60) as i64 * 1_000
            };
            assert!(jitter_ms >= 0, "jitter must be non-negative for {sym}");
            assert!(jitter_ms < 60_000, "jitter must be < 60s for {sym}");
        }
    }

    /// Different coins must produce different jitter values (avoid everyone
    /// mapping to the same slot — a degenerate hash would defeat the purpose).
    #[test]
    fn candle_jitter_distributes_across_coins() {
        let symbols = &[
            "BTC", "ETH", "SOL", "AVAX", "kBONK", "DOGE", "WIF", "ARB", "OP", "LINK", "AAVE",
            "UNI", "MKR", "CRV", "INJ", "TIA", "SEI", "SUI", "APT", "NEAR",
        ];
        let jitters: std::collections::HashSet<i64> = symbols
            .iter()
            .map(|sym| {
                let hash: u64 = sym.bytes().fold(5381u64, |acc, b| {
                    acc.wrapping_mul(33).wrapping_add(b as u64)
                });
                (hash % 60) as i64
            })
            .collect();
        // With 20 coins mapped into 60 slots the collision probability is
        // non-trivial, but we should see at least 10 distinct values.
        assert!(
            jitters.len() >= 10,
            "jitter values should spread across multiple slots, got {} unique: {:?}",
            jitters.len(),
            jitters
        );
    }
}
