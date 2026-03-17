//! Perpetual futures funding rate module.
//!
//! Funding rate = the periodic payment exchanged between longs and shorts on
//! perpetual futures, settled every 8 hours.  It is a direct, real-time measure
//! of market leverage and crowd positioning — and therefore a powerful
//! **contrarian** signal:
//!
//! | 8h rate       | Market state           | Signal          |
//! |---------------|------------------------|-----------------|
//! | > +0.10 %     | Extreme long crowding  | Strong BEAR     |
//! | +0.05–0.10 %  | Elevated long crowding | Moderate BEAR   |
//! | +0.02–0.05 %  | Mild long bias         | Slight BEAR     |
//! | ±0.02 %       | Neutral                | No signal       |
//! | −0.02–−0.05 % | Mild short bias        | Slight BULL     |
//! | −0.05–−0.10 % | Elevated short crowd   | Moderate BULL   |
//! | < −0.10 %     | Extreme short crowding | Strong BULL     |
//!
//! High positive funding → longs are overcrowded and paying shorts to stay open.
//! When long crowding unwinds, prices fall quickly as stops are hit in cascade.
//!
//! # Source: Hyperliquid `metaAndAssetCtxs` (single POST, all HL perps)
//!
//! Switched from Binance `premiumIndex` for three reasons:
//!   1. Native HL symbol names — no USDT-suffix stripping or k-prefix mapping.
//!   2. Provides **predicted next-period funding** alongside current rate.
//!   3. Eliminates the last external Binance dependency from the trade path.
//!
//! Response shape (asset context for each perp, indexed parallel to universe):
//! ```json
//! {"funding": "0.0000125", "premium": "0.0000089", "markPx": "50000", ...}
//! ```
//!   • `funding`  — current 8h rate (realised, same semantics as Binance rate)
//!   • `premium`  — predicted rate for the next period (mark − index / 8h)
//!
//! Cached for 3 minutes; refreshes transparently on `get()` call.
//! Exposes `is_stale()` for the funding gate in `decision.rs`.

use anyhow::Result;
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Refresh every 3 minutes.  Funding settles every 8 h but can spike quickly;
/// 3-minute granularity catches intra-period crowding build-ups.
const CACHE_TTL: Duration = Duration::from_secs(180);

/// Maximum age before `is_stale()` returns true and the funding gate fires.
/// Set to 10 minutes — two missed refreshes plus margin.  If funding is older
/// than this we have no reliable crowd-positioning data and should not open
/// new positions.
const STALE_THRESHOLD: Duration = Duration::from_secs(600);

// ─────────────────────────── Public types ────────────────────────────────────

/// Funding rate snapshot for a single perpetual symbol.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FundingData {
    pub symbol:        String,
    /// Current 8-hour funding rate (e.g. 0.0001 = 0.01 %).
    /// Positive = longs pay shorts.  Negative = shorts pay longs.
    pub funding_rate:  f64,
    /// Predicted funding for the next 8-hour period (mark − index / 8h).
    /// Positive = market expects rates to stay elevated.
    /// Use as a leading indicator: if current is neutral but predicted is high,
    /// longs are building positions ahead of the next settlement.
    pub predicted_rate: f64,
    /// Change in funding rate vs the previous cache refresh cycle.
    /// Positive = funding rising (longs becoming more crowded / expensive).
    /// Negative = funding falling (de-levering, shorts building up).
    pub funding_delta: f64,
}

impl FundingData {
    /// Annualised rate as a percentage (rate × 3 payments/day × 365 days × 100).
    #[allow(dead_code)]
    pub fn annualised_pct(&self) -> f64 {
        self.funding_rate * 3.0 * 365.0 * 100.0
    }

    /// Contrarian signal strength in **[−1.0, +1.0]**.
    ///
    /// Blends current funding (60 %) with predicted funding (40 %) so that
    /// a building position (predicted rising faster than current) weighs in
    /// before it fully hits the current-rate reading.
    ///
    /// * Positive → bullish lean (shorts overcrowded → squeeze risk).
    /// * Negative → bearish lean (longs overcrowded → liquidation risk).
    /// * Zero     → neutral / below noise threshold.
    pub fn signal_strength(&self) -> f64 {
        // Blend: 60 % current, 40 % predicted — rewards forward-looking signal.
        let blended = self.funding_rate * 0.60 + self.predicted_rate * 0.40;
        self.strength_from_rate(blended)
    }

    fn strength_from_rate(&self, r: f64) -> f64 {
        // Thresholds in 8h rate units:
        //   0.0010 = 0.10 %  (extreme)
        //   0.0005 = 0.05 %  (elevated)
        //   0.0002 = 0.02 %  (mild — outer edge of neutral band)
        if      r >  0.0010 { -1.00 }  // extreme long crowding  → strong bear lean
        else if r >  0.0005 { -0.65 }  // elevated long crowding → moderate bear lean
        else if r >  0.0002 { -0.30 }  // mild long bias         → slight bear lean
        else if r < -0.0010 {  1.00 }  // extreme short crowding → strong bull lean
        else if r < -0.0005 {  0.65 }  // elevated short crowd   → moderate bull lean
        else if r < -0.0002 {  0.30 }  // mild short bias         → slight bull lean
        else                {  0.00 }  // neutral band            → no signal
    }

    /// True when the rate is outside the neutral ±0.02 % band.
    pub fn is_significant(&self) -> bool {
        self.funding_rate.abs() > 0.0002
    }

    /// Emoji indicator for dashboard display.
    pub fn emoji(&self) -> &'static str {
        let r = self.funding_rate;
        if      r >  0.0005 { "🔴" }  // elevated longs → bearish
        else if r < -0.0005 { "🟢" }  // elevated shorts → bullish
        else                { "🟡" }  // neutral
    }
}

// ─────────────────────────── HL API response shapes ──────────────────────────

/// Top-level response from `metaAndAssetCtxs`:
/// `[universe_array, asset_ctx_array]`
/// `universe_array[i].name` corresponds to `asset_ctx_array[i]`.
#[derive(serde::Deserialize)]
struct HlMeta {
    universe: Vec<HlUniverseItem>,
}

#[derive(serde::Deserialize)]
struct HlUniverseItem {
    name: String,
}

#[derive(serde::Deserialize)]
struct HlAssetCtx {
    /// Current 8h funding rate (string float, e.g. "0.0000125").
    funding: String,
    /// Predicted next-period funding = (mark − index) / index / 8 ≈ premium.
    premium: String,
}

// ─────────────────────────── Cache ───────────────────────────────────────────

struct CacheInner {
    data:       HashMap<String, FundingData>,
    prev_rates: HashMap<String, f64>,
    last_fetch: Option<Instant>,
}

/// Thread-safe, auto-refreshing funding rate cache backed by HL `metaAndAssetCtxs`.
/// Clone the `Arc` freely — one instance per bot.
pub struct FundingCache {
    client: Client,
    inner:  RwLock<CacheInner>,
}

pub type SharedFunding = Arc<FundingCache>;

impl FundingCache {
    pub fn new() -> SharedFunding {
        Arc::new(FundingCache {
            client: Client::builder()
                .timeout(Duration::from_secs(8))
                .build()
                .unwrap_or_default(),
            inner: RwLock::new(CacheInner {
                data:       HashMap::new(),
                prev_rates: HashMap::new(),
                last_fetch: None,
            }),
        })
    }

    /// Look up funding data for `symbol` (Hyperliquid short form: "ETH", "SOL").
    /// Transparently refreshes when the TTL has expired.
    /// Returns `None` only for symbols not listed on HL (e.g. `@N` derivatives).
    pub async fn get(&self, symbol: &str) -> Option<FundingData> {
        // Fast path: cache is warm.
        {
            let r = self.inner.read().await;
            if r.last_fetch.map(|t| t.elapsed() < CACHE_TTL).unwrap_or(false) {
                return r.data.get(symbol).cloned();
            }
        }

        // Snapshot current rates as prev before refreshing.
        let prev_rates: HashMap<String, f64> = {
            let r = self.inner.read().await;
            r.data.iter().map(|(k, v)| (k.clone(), v.funding_rate)).collect()
        };

        match self.fetch_all(&prev_rates).await {
            Ok(map) => {
                let result = map.get(symbol).cloned();
                let mut w  = self.inner.write().await;
                w.prev_rates = prev_rates;
                w.data       = map;
                w.last_fetch = Some(Instant::now());
                result
            }
            Err(e) => {
                log::warn!("💰 Funding fetch error: {} — using stale cache", e);
                self.inner.read().await.data.get(symbol).cloned()
            }
        }
    }

    /// Returns `true` when the cache has never been populated, or the last
    /// successful fetch is older than `STALE_THRESHOLD` (10 minutes).
    ///
    /// Used by the funding gate in `analyse_symbol` to block new entries when
    /// crowd-positioning data is unavailable.
    pub async fn is_stale(&self) -> bool {
        let r = self.inner.read().await;
        r.last_fetch
            .map(|t| t.elapsed() > STALE_THRESHOLD)
            .unwrap_or(true) // never populated → stale
    }

    /// Age of the cache in seconds, or `None` if it has never been populated.
    pub async fn age_secs(&self) -> Option<u64> {
        self.inner.read().await
            .last_fetch
            .map(|t| t.elapsed().as_secs())
    }

    /// Pre-warm the cache at startup (avoids first-cycle fetch latency).
    /// First warm has no previous rates, so `funding_delta` will be 0.0.
    pub async fn warm(&self) {
        let empty_prev: HashMap<String, f64> = HashMap::new();
        match self.fetch_all(&empty_prev).await {
            Ok(map) => {
                log::info!(
                    "💰 Funding rates (HL): pre-warmed {} perps  \
                     (BTC={:+.4}%→pred{:+.4}%  ETH={:+.4}%  SOL={:+.4}%)",
                    map.len(),
                    map.get("BTC").map(|d| d.funding_rate   * 100.0).unwrap_or(0.0),
                    map.get("BTC").map(|d| d.predicted_rate * 100.0).unwrap_or(0.0),
                    map.get("ETH").map(|d| d.funding_rate   * 100.0).unwrap_or(0.0),
                    map.get("SOL").map(|d| d.funding_rate   * 100.0).unwrap_or(0.0),
                );
                let mut w  = self.inner.write().await;
                w.data       = map;
                w.last_fetch = Some(Instant::now());
            }
            Err(e) => log::warn!("💰 Funding warm-up failed: {}", e),
        }
    }

    // ── Private ───────────────────────────────────────────────────────────────

    /// Fetch funding rates for all HL perps via `metaAndAssetCtxs` (one call).
    ///
    /// Response: `[{universe: [{name: "BTC"}, ...]}, [{funding: "...", premium: "..."}, ...]]`
    /// Universe index i corresponds to asset context index i.
    async fn fetch_all(&self, prev: &HashMap<String, f64>) -> Result<HashMap<String, FundingData>> {
        let resp = self.client
            .post("https://api.hyperliquid.xyz/info")
            .json(&serde_json::json!({ "type": "metaAndAssetCtxs" }))
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("HL metaAndAssetCtxs HTTP {}", resp.status());
        }

        // Response is a 2-element array: [meta_object, asset_ctx_array]
        let raw: Vec<serde_json::Value> = resp.json().await?;
        if raw.len() < 2 {
            anyhow::bail!("HL metaAndAssetCtxs: unexpected response shape");
        }

        let meta: HlMeta = serde_json::from_value(raw[0].clone())
            .map_err(|e| anyhow::anyhow!("HL meta parse: {}", e))?;

        let ctxs: Vec<HlAssetCtx> = serde_json::from_value(raw[1].clone())
            .map_err(|e| anyhow::anyhow!("HL asset ctx parse: {}", e))?;

        let mut map = HashMap::with_capacity(meta.universe.len());

        for (item, ctx) in meta.universe.iter().zip(ctxs.iter()) {
            let sym  = item.name.clone();
            let rate: f64 = ctx.funding.parse().unwrap_or(0.0);
            let pred: f64 = ctx.premium.parse().unwrap_or(0.0);

            // Delta vs previous refresh (0 on first observation).
            let delta = prev.get(&sym).map(|&p| rate - p).unwrap_or(0.0);

            map.insert(sym.clone(), FundingData {
                symbol:        sym,
                funding_rate:  rate,
                predicted_rate: pred,
                funding_delta: delta,
            });
        }

        log::info!(
            "💰 Funding (HL): {} perps  \
             BTC={:+.4}%→{:+.4}%  ETH={:+.4}%  SOL={:+.4}%",
            map.len(),
            map.get("BTC").map(|d| d.funding_rate   * 100.0).unwrap_or(0.0),
            map.get("BTC").map(|d| d.predicted_rate * 100.0).unwrap_or(0.0),
            map.get("ETH").map(|d| d.funding_rate   * 100.0).unwrap_or(0.0),
            map.get("SOL").map(|d| d.funding_rate   * 100.0).unwrap_or(0.0),
        );
        Ok(map)
    }
}
