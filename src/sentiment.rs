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

// Refresh interval: 5 minutes (LunarCrush free tier is generous but not real-time)
const CACHE_TTL: Duration = Duration::from_secs(300);
const BASE_URL: &str = "https://lunarcrush.com/api4/public";

// ─────────────────────────── Public types ────────────────────────────────────

/// Per-coin sentiment snapshot, populated from LunarCrush v4.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentimentData {
    pub symbol: String,
    /// Overall social health 0-100 (higher = more reliable signal)
    pub galaxy_score: f64,
    /// Social rank vs peers — lower is better (1 = most discussed)
    pub alt_rank: u32,
    /// Percentage of posts classified as bullish (0-100)
    pub bullish_percent: f64,
    /// Percentage of posts classified as bearish (0-100)
    pub bearish_percent: f64,
    /// Total social post volume in the last 24 h
    pub social_volume: f64,
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
    #[allow(dead_code)]
    pub fn emoji(&self) -> &'static str {
        if self.bullish_percent >= 65.0 {
            "🟢"
        } else if self.bullish_percent >= 45.0 {
            "🟡"
        } else {
            "🔴"
        }
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
    symbol: String,
    galaxy_score: Option<f64>,
    /// API sometimes returns int, sometimes float — accept both via f64.
    alt_rank: Option<f64>,
    social_volume: Option<f64>,
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
    sentiment: Option<f64>,
}

// ─────────────────────────── Cache ───────────────────────────────────────────

struct CacheInner {
    data: HashMap<String, SentimentData>,
    last_fetch: Option<Instant>,
}

/// Thread-safe, lazily-refreshed sentiment cache.
/// Clone the `Arc` freely — one instance per bot.
pub struct SentimentCache {
    client: Client,
    api_key: String,
    inner: RwLock<CacheInner>,
    /// Prevents concurrent refresh attempts (thundering-herd protection).
    ///
    /// Without this, all 18 candidate symbols calling `get()` simultaneously
    /// would each detect a stale cache and fire their own `fetch_all()` request,
    /// exhausting the LunarCrush rate limit in seconds.
    ///
    /// Pattern: acquire → double-check TTL → fetch if still stale → release.
    /// Tasks waiting on the lock see a fresh cache after the leader updates it.
    refresh_lock: Mutex<()>,
}

pub type SharedSentiment = Arc<SentimentCache>;

impl SentimentCache {
    pub fn new(api_key: String) -> SharedSentiment {
        Arc::new(SentimentCache {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            api_key,
            inner: RwLock::new(CacheInner {
                data: HashMap::new(),
                last_fetch: None,
            }),
            refresh_lock: Mutex::new(()),
        })
    }

    /// Look up sentiment for `symbol`.
    /// Transparently refreshes the cache when the TTL has expired.
    /// Returns `None` if the coin is not in the LunarCrush dataset or on error.
    ///
    /// Handles Hyperliquid k-prefix automatically:
    ///   `get("kBONK")` → looks up "BONK" in the LunarCrush cache,
    /// because HL prefixes low-value coins with 'k' but LunarCrush does not.
    pub async fn get(&self, symbol: &str) -> Option<SentimentData> {
        // Normalise HL k-prefix: "kBONK" → "BONK", "BTC" → "BTC"
        let key = Self::normalise_symbol(symbol);

        // Fast path: cache is warm — no lock needed
        {
            let r = self.inner.read().await;
            if r.last_fetch
                .map(|t| t.elapsed() < CACHE_TTL)
                .unwrap_or(false)
            {
                return r.data.get(key).cloned();
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
                return r.data.get(key).cloned();
            }
        }

        // We hold the lock and the cache is still stale — we are the designated refresher.
        match self.fetch_all().await {
            Ok(map) => {
                let result = map.get(key).cloned();
                let mut w = self.inner.write().await;
                w.data = map;
                w.last_fetch = Some(Instant::now());
                result
            }
            Err(e) => {
                log::warn!("🌙 LunarCrush fetch error: {} — using stale cache", e);
                // Mark as "recently attempted" even on failure so the 5-minute backoff
                // prevents all 18 concurrent waiters from immediately re-attempting
                // after the lock is released.
                {
                    let mut w = self.inner.write().await;
                    w.last_fetch = Some(Instant::now());
                }
                self.inner.read().await.data.get(key).cloned()
            }
        }
    }

    /// Strip Hyperliquid's k-prefix so HL symbols map to LunarCrush symbols.
    /// "kBONK" → "BONK", "kSHIB" → "SHIB", "BTC" → "BTC"
    pub fn normalise_symbol(symbol: &str) -> &str {
        symbol.strip_prefix('k').unwrap_or(symbol)
    }

    /// Pre-warm the cache (call once at startup).
    pub async fn warm(&self) {
        match self.fetch_all().await {
            Ok(map) => {
                log::info!("🌙 LunarCrush: pre-warmed {} coins", map.len());
                let mut w = self.inner.write().await;
                w.data = map;
                w.last_fetch = Some(Instant::now());
            }
            Err(e) => {
                log::warn!("🌙 LunarCrush warm-up failed: {} — setting backoff", e);
                // Set last_fetch on failure so the first cycle doesn't immediately retry.
                let mut w = self.inner.write().await;
                w.last_fetch = Some(Instant::now());
            }
        }
    }

    // ── Private ───────────────────────────────────────────────────────────

    /// Number of coins currently in the cache (0 = not yet fetched or error).
    #[allow(dead_code)]
    pub async fn coin_count(&self) -> usize {
        self.inner.read().await.data.len()
    }

    async fn fetch_all(&self) -> Result<HashMap<String, SentimentData>> {
        // Fetch top 200 coins sorted by galaxy_score (covers all major alts).
        // The default limit is only 10 which misses most trading symbols.
        let url = format!(
            "{}/coins/list/v2?limit=200&sort=galaxy_score&desc=1",
            BASE_URL
        );

        let raw = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .map_err(|e| {
                log::warn!("🌙 LunarCrush HTTP error: {}", e);
                e
            })?;

        let status = raw.status();
        if !status.is_success() {
            let body = raw.text().await.unwrap_or_default();
            anyhow::bail!("LunarCrush {} — {}", status, &body[..body.len().min(300)]);
        }

        // Read raw text first so we can log it on parse failure.
        let text = raw
            .text()
            .await
            .map_err(|e| anyhow::anyhow!("LunarCrush read error: {}", e))?;

        log::debug!(
            "🌙 LunarCrush raw response (first 200): {}",
            &text[..text.len().min(200)]
        );

        // Parse loosely — unknown fields are silently ignored by serde.
        let resp: CoinsListResponse = serde_json::from_str(&text).map_err(|e| {
            log::warn!(
                "🌙 LunarCrush JSON parse error: {} — response: {}",
                e,
                &text[..text.len().min(300)]
            );
            e
        })?;

        let mut map = HashMap::with_capacity(resp.data.len());

        for item in resp.data {
            let galaxy_score = item.galaxy_score.unwrap_or(50.0);
            // alt_rank is f64 in struct to accept both int and float from API
            let alt_rank = item.alt_rank.map(|r| r as u32).unwrap_or(9999);
            let social_volume = item.social_volume.unwrap_or(0.0);
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

            map.insert(
                item.symbol.to_uppercase(),
                SentimentData {
                    symbol: item.symbol.to_uppercase(),
                    galaxy_score,
                    alt_rank,
                    bullish_percent,
                    bearish_percent,
                    social_volume,
                    social_dominance,
                },
            );
        }

        log::info!(
            "🌙 LunarCrush: fetched {} coins (sample BTC bull={:.0}%)",
            map.len(),
            map.get("BTC").map(|d| d.bullish_percent).unwrap_or(-1.0)
        );
        Ok(map)
    }
}

// =============================================================================
//  Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sent(bullish: f64, bearish: f64, galaxy: f64) -> SentimentData {
        SentimentData {
            symbol: "TEST".to_string(),
            galaxy_score: galaxy,
            alt_rank: 10,
            bullish_percent: bullish,
            bearish_percent: bearish,
            social_volume: 1000.0,
            social_dominance: 5.0,
        }
    }

    // ── SentimentData calculations ────────────────────────────────────────────

    #[test]
    fn direction_score_fully_bullish() {
        let s = make_sent(100.0, 0.0, 80.0);
        assert!((s.direction_score() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn direction_score_fully_bearish() {
        let s = make_sent(0.0, 100.0, 80.0);
        assert!((s.direction_score() - (-1.0)).abs() < 1e-9);
    }

    #[test]
    fn direction_score_neutral() {
        let s = make_sent(50.0, 50.0, 80.0);
        assert!(s.direction_score().abs() < 1e-9);
    }

    #[test]
    fn signal_strength_is_direction_times_quality() {
        let s = make_sent(75.0, 25.0, 80.0);
        let expected = s.direction_score() * (80.0 / 100.0);
        assert!((s.signal_strength() - expected).abs() < 1e-9);
    }

    #[test]
    fn signal_strength_zero_galaxy_kills_signal() {
        // Even strongly bullish sentiment should produce near-zero signal if galaxy=0
        let s = make_sent(90.0, 10.0, 0.0);
        assert!(s.signal_strength().abs() < 1e-9);
    }

    #[test]
    fn quality_clamped_to_0_1() {
        assert!((make_sent(50.0, 50.0, 0.0).quality() - 0.0).abs() < 1e-9);
        assert!((make_sent(50.0, 50.0, 100.0).quality() - 1.0).abs() < 1e-9);
        // Values outside 0-100 should be clamped
        assert!((make_sent(50.0, 50.0, 150.0).quality() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn emoji_thresholds() {
        assert_eq!(make_sent(65.0, 35.0, 80.0).emoji(), "🟢"); // exactly 65 = green
        assert_eq!(make_sent(64.9, 35.1, 80.0).emoji(), "🟡"); // just below 65 = yellow
        assert_eq!(make_sent(45.0, 55.0, 80.0).emoji(), "🟡"); // exactly 45 = yellow
        assert_eq!(make_sent(44.9, 55.1, 80.0).emoji(), "🔴"); // just below 45 = red
    }

    // ── normalise_symbol (k-prefix) ───────────────────────────────────────────

    #[test]
    fn normalise_strips_k_prefix() {
        assert_eq!(SentimentCache::normalise_symbol("kBONK"), "BONK");
        assert_eq!(SentimentCache::normalise_symbol("kPEPE"), "PEPE");
        assert_eq!(SentimentCache::normalise_symbol("kSHIB"), "SHIB");
    }

    #[test]
    fn normalise_leaves_normal_symbols_unchanged() {
        assert_eq!(SentimentCache::normalise_symbol("BTC"), "BTC");
        assert_eq!(SentimentCache::normalise_symbol("ETH"), "ETH");
        assert_eq!(SentimentCache::normalise_symbol("SOL"), "SOL");
        assert_eq!(SentimentCache::normalise_symbol("AVAX"), "AVAX");
    }

    #[test]
    fn normalise_does_not_strip_non_k_prefix() {
        // "KAVA" starts with 'K' (uppercase) — should NOT be stripped
        // (HL k-prefix is always lowercase 'k')
        assert_eq!(SentimentCache::normalise_symbol("KAVA"), "KAVA");
    }

    // ── JSON deserialization shapes ───────────────────────────────────────────

    #[test]
    fn deserialise_explicit_bullish_bearish_fields() {
        let json = r#"{
            "symbol": "BTC",
            "galaxy_score": 80.0,
            "alt_rank": 1.0,
            "social_volume": 5000.0,
            "social_dominance": 12.0,
            "bullish_sentiment": 70.0,
            "bearish_sentiment": 30.0
        }"#;
        let item: CoinItem = serde_json::from_str(json).expect("parse failed");
        assert_eq!(item.bullish_sentiment, Some(70.0));
        assert_eq!(item.bearish_sentiment, Some(30.0));
        assert_eq!(item.sentiment, None);
    }

    #[test]
    fn deserialise_percent_bullish_bearish_fields() {
        let json = r#"{
            "symbol": "ETH",
            "galaxy_score": 75.0,
            "alt_rank": 2.0,
            "percent_bullish": 60.0,
            "percent_bearish": 40.0
        }"#;
        let item: CoinItem = serde_json::from_str(json).expect("parse failed");
        assert_eq!(item.percent_bullish, Some(60.0));
        assert_eq!(item.percent_bearish, Some(40.0));
    }

    #[test]
    fn deserialise_1_to_5_sentiment_scale() {
        let json = r#"{
            "symbol": "SOL",
            "galaxy_score": 70.0,
            "alt_rank": 5.0,
            "sentiment": 4.0
        }"#;
        let item: CoinItem = serde_json::from_str(json).expect("parse failed");
        assert_eq!(item.sentiment, Some(4.0));
        // sentiment=4 → (4-1)/4 * 100 = 75% bull
        let bull = ((4.0_f64 - 1.0) / 4.0 * 100.0).clamp(0.0, 100.0);
        assert!((bull - 75.0).abs() < 1e-9);
    }

    #[test]
    fn deserialise_minimal_fields_uses_defaults() {
        // API may omit optional fields — must not panic
        let json = r#"{"symbol": "DOGE"}"#;
        let item: CoinItem = serde_json::from_str(json).expect("parse failed");
        assert_eq!(item.symbol, "DOGE");
        assert_eq!(item.galaxy_score, None);
        assert_eq!(item.alt_rank, None);
        assert_eq!(item.sentiment, None);
    }

    #[test]
    fn sentiment_1_scale_boundary_values() {
        // s=1 → 0% bull, s=5 → 100% bull, s=3 → 50% bull
        let cases = [(1.0_f64, 0.0_f64), (3.0, 50.0), (5.0, 100.0)];
        for (s, expected_bull) in cases {
            let bull = ((s - 1.0) / 4.0 * 100.0).clamp(0.0, 100.0);
            assert!(
                (bull - expected_bull).abs() < 1e-9,
                "sentiment={s} → expected {expected_bull}% bull, got {bull}%"
            );
        }
    }
}
