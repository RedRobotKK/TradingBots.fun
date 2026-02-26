//! LunarCrush sentiment layer.
//!
//! Fetches social sentiment data for all tracked coins once every 5 minutes
//! and caches it in-memory.  Individual lookups are near-instant (HashMap).
//!
//! Key metrics used in the decision engine:
//!   • `galaxy_score`    0-100 overall social health (higher = more credible signal)
//!   • `alt_rank`        lower = more social momentum relative to peers
//!   • `bullish_percent` 0-100 % of posts classified bullish  (primary signal)
//!   • `bearish_percent` 0-100 % of posts classified bearish
//!
//! API: LunarCrush v4  —  https://lunarcrush.com/api4/public
//! Auth: `Authorization: Bearer {api_key}` header

use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};
use serde_json;

// Refresh interval: 5 minutes (LunarCrush free tier is generous but not real-time)
const CACHE_TTL: Duration = Duration::from_secs(300);
const BASE_URL:  &str     = "https://lunarcrush.com/api4/public";

// ─────────────────────────── Public types ────────────────────────────────────

/// Per-coin sentiment snapshot, populated from LunarCrush v4.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentimentData {
    pub symbol:           String,
    /// Overall social health 0-100 (higher = more reliable signal)
    pub galaxy_score:     f64,
    /// Social rank vs peers — lower is better (1 = most discussed)
    pub alt_rank:         u32,
    /// Percentage of posts classified as bullish (0-100)
    pub bullish_percent:  f64,
    /// Percentage of posts classified as bearish (0-100)
    pub bearish_percent:  f64,
    /// Total social post volume in the last 24 h
    pub social_volume:    f64,
    /// % of total crypto social volume this coin occupies
    pub social_dominance: f64,
}

impl SentimentData {
    /// Net direction: +1.0 = fully bullish, -1.0 = fully bearish.
    pub fn direction_score(&self) -> f64 {
        (self.bullish_percent - self.bearish_percent) / 100.0
    }

    /// Galaxy score normalised to 0-1 (used as confidence multiplier).
    pub fn quality(&self) -> f64 {
        (self.galaxy_score / 100.0).clamp(0.0, 1.0)
    }

    /// Combined signal strength = direction × quality.
    /// Range: -1.0 (strong bear) … +1.0 (strong bull).
    pub fn signal_strength(&self) -> f64 {
        self.direction_score() * self.quality()
    }

    /// Emoji for dashboard: 🟢 bull / 🟡 neutral / 🔴 bear.
    pub fn emoji(&self) -> &'static str {
        if self.bullish_percent >= 65.0      { "🟢" }
        else if self.bullish_percent >= 45.0 { "🟡" }
        else                                 { "🔴" }
    }
}

// ─────────────────────────── API response shapes ─────────────────────────────

#[derive(Deserialize)]
struct CoinsListResponse {
    data: Vec<CoinItem>,
}

/// Loosely-typed coin item — tolerates extra fields and type variations
/// across LunarCrush API versions (e.g. alt_rank as int *or* float).
#[derive(Deserialize)]
struct CoinItem {
    symbol:           String,
    galaxy_score:     Option<f64>,
    /// API sometimes returns int, sometimes float — accept both via f64.
    alt_rank:         Option<f64>,
    social_volume:    Option<f64>,
    social_dominance: Option<f64>,

    // v4 may return either a 1-5 sentiment score or explicit bull/bear %s.
    // We accept all variant field names we've seen in the wild.
    #[serde(rename = "bullish_sentiment")]
    bullish_sentiment: Option<f64>,
    #[serde(rename = "bearish_sentiment")]
    bearish_sentiment: Option<f64>,
    // Some v4 responses use these names instead:
    #[serde(rename = "percent_bullish")]
    percent_bullish: Option<f64>,
    #[serde(rename = "percent_bearish")]
    percent_bearish: Option<f64>,
    /// 1-5 scale: 1 = very bearish, 5 = very bullish (fallback).
    sentiment:         Option<f64>,
}

// ─────────────────────────── Cache ───────────────────────────────────────────

struct CacheInner {
    data:       HashMap<String, SentimentData>,
    last_fetch: Option<Instant>,
}

/// Thread-safe, lazily-refreshed sentiment cache.
/// Clone the `Arc` freely — one instance per bot.
pub struct SentimentCache {
    client:         Client,
    api_key:        String,
    inner:          RwLock<CacheInner>,
    /// Prevents concurrent refresh attempts (thundering-herd protection).
    ///
    /// Without this, all 18 candidate symbols calling `get()` simultaneously
    /// would each detect a stale cache and fire their own `fetch_all()` request,
    /// exhausting the LunarCrush rate limit in seconds.
    ///
    /// Pattern: acquire → double-check TTL → fetch if still stale → release.
    /// Tasks waiting on the lock see a fresh cache after the leader updates it.
    refresh_lock:   Mutex<()>,
}

pub type SharedSentiment = Arc<SentimentCache>;

impl SentimentCache {
    pub fn new(api_key: String) -> SharedSentiment {
        Arc::new(SentimentCache {
            client:  Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            api_key,
            inner: RwLock::new(CacheInner {
                data:       HashMap::new(),
                last_fetch: None,
            }),
            refresh_lock: Mutex::new(()),
        })
    }

    /// Look up sentiment for `symbol`.
    /// Transparently refreshes the cache when the TTL has expired.
    /// Returns `None` if the coin is not in the LunarCrush dataset or on error.
    pub async fn get(&self, symbol: &str) -> Option<SentimentData> {
        // Fast path: cache is warm — no lock needed
        {
            let r = self.inner.read().await;
            if r.last_fetch
                .map(|t| t.elapsed() < CACHE_TTL)
                .unwrap_or(false)
            {
                return r.data.get(symbol).cloned();
            }
        }

        // Slow path: acquire the refresh mutex.
        // Only ONE task refreshes at a time; the others wait here and then
        // benefit from the fresh cache in the double-check below.
        let _refresh_guard = self.refresh_lock.lock().await;

        // Double-check: was the cache refreshed while we were waiting?
        {
            let r = self.inner.read().await;
            if r.last_fetch
                .map(|t| t.elapsed() < CACHE_TTL)
                .unwrap_or(false)
            {
                return r.data.get(symbol).cloned();
            }
        }

        // We hold the lock and the cache is still stale — we are the designated refresher.
        match self.fetch_all().await {
            Ok(map) => {
                let result = map.get(symbol).cloned();
                let mut w  = self.inner.write().await;
                w.data       = map;
                w.last_fetch = Some(Instant::now());
                result
            }
            Err(e) => {
                log::warn!("🌙 LunarCrush fetch error: {} — using stale cache", e);
                // Mark as "recently attempted" even on failure so the 5-minute backoff
                // prevents all 18 concurrent waiters from immediately re-attempting
                // after the lock is released.
                {
                    let mut w  = self.inner.write().await;
                    w.last_fetch = Some(Instant::now());
                }
                self.inner.read().await.data.get(symbol).cloned()
            }
        }
    }

    /// Pre-warm the cache (call once at startup).
    pub async fn warm(&self) {
        match self.fetch_all().await {
            Ok(map) => {
                log::info!("🌙 LunarCrush: pre-warmed {} coins", map.len());
                let mut w   = self.inner.write().await;
                w.data       = map;
                w.last_fetch = Some(Instant::now());
            }
            Err(e) => {
                log::warn!("🌙 LunarCrush warm-up failed: {} — setting backoff", e);
                // Set last_fetch on failure so the first cycle doesn't immediately retry.
                let mut w  = self.inner.write().await;
                w.last_fetch = Some(Instant::now());
            }
        }
    }

    // ── Private ───────────────────────────────────────────────────────────

    /// Number of coins currently in the cache (0 = not yet fetched or error).
    pub async fn coin_count(&self) -> usize {
        self.inner.read().await.data.len()
    }

    async fn fetch_all(&self) -> Result<HashMap<String, SentimentData>> {
        // Fetch top 200 coins sorted by galaxy_score (covers all major alts).
        // The default limit is only 10 which misses most trading symbols.
        let url = format!("{}/coins/list/v2?limit=200&sort=galaxy_score&desc=1", BASE_URL);

        let raw = self.client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .map_err(|e| { log::warn!("🌙 LunarCrush HTTP error: {}", e); e })?;

        let status = raw.status();
        if !status.is_success() {
            let body = raw.text().await.unwrap_or_default();
            anyhow::bail!("LunarCrush {} — {}", status, &body[..body.len().min(300)]);
        }

        // Read raw text first so we can log it on parse failure.
        let text = raw.text().await
            .map_err(|e| anyhow::anyhow!("LunarCrush read error: {}", e))?;

        log::debug!("🌙 LunarCrush raw response (first 200): {}", &text[..text.len().min(200)]);

        // Parse loosely — unknown fields are silently ignored by serde.
        let resp: CoinsListResponse = serde_json::from_str(&text)
            .map_err(|e| {
                log::warn!("🌙 LunarCrush JSON parse error: {} — response: {}", e,
                    &text[..text.len().min(300)]);
                e
            })?;

        let mut map = HashMap::with_capacity(resp.data.len());

        for item in resp.data {
            let galaxy_score     = item.galaxy_score.unwrap_or(50.0);
            // alt_rank is f64 in struct to accept both int and float from API
            let alt_rank         = item.alt_rank.map(|r| r as u32).unwrap_or(9999);
            let social_volume    = item.social_volume.unwrap_or(0.0);
            let social_dominance = item.social_dominance.unwrap_or(0.0);

            // Priority order for bull/bear %:
            //   1. bullish_sentiment / bearish_sentiment (v4 some endpoints)
            //   2. percent_bullish / percent_bearish (alternate field names)
            //   3. sentiment 1-5 scale (most common in v4 list endpoint)
            //   4. neutral default (50/50)
            let (bullish_percent, bearish_percent) =
                if let (Some(b), Some(br)) = (item.bullish_sentiment, item.bearish_sentiment) {
                    (b.clamp(0.0, 100.0), br.clamp(0.0, 100.0))
                } else if let (Some(b), Some(br)) = (item.percent_bullish, item.percent_bearish) {
                    (b.clamp(0.0, 100.0), br.clamp(0.0, 100.0))
                } else if let Some(s) = item.sentiment {
                    // 1-5 → 0-100%: s=1 → 0% bull, s=3 → 50%, s=5 → 100%
                    let bull = ((s - 1.0) / 4.0 * 100.0).clamp(0.0, 100.0);
                    (bull, 100.0 - bull)
                } else {
                    (50.0, 50.0)
                };

            map.insert(item.symbol.to_uppercase(), SentimentData {
                symbol:           item.symbol.to_uppercase(),
                galaxy_score,
                alt_rank,
                bullish_percent,
                bearish_percent,
                social_volume,
                social_dominance,
            });
        }

        log::info!("🌙 LunarCrush: fetched {} coins (sample BTC bull={:.0}%)",
            map.len(),
            map.get("BTC").map(|d| d.bullish_percent).unwrap_or(-1.0));
        Ok(map)
    }
}
