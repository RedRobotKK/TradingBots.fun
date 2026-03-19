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

/// The funding cycle phase, computed from UTC time relative to the 8-hour
/// settlement windows (00:00, 08:00, 16:00 UTC on all major exchanges).
///
/// Settlement times create predictable price patterns:
///  • Pre-settlement:  paying side (longs if +ve funding) closes to avoid cost
///                     → price moves in the direction that hurts the payer
///  • Post-settlement: rate resets, crowd repositions, often brief reversal
///  • Mid-cycle:       no structural cycle pressure; signal unchanged
#[derive(Debug, Clone, PartialEq)]
pub enum FundingCyclePhase {
    /// < 90 minutes to next settlement — maximum closing pressure from payers.
    PreSettlement { hours_remaining: f64 },
    /// 0–30 minutes after settlement — repositioning window, potential flip.
    PostSettlement { minutes_elapsed: f64 },
    /// Everything else — no structural cycle pressure.
    MidCycle { hours_to_next: f64 },
}

impl FundingCyclePhase {
    /// Compute the current cycle phase from the given UTC timestamp (seconds since epoch).
    pub fn from_utc_secs(utc_secs: i64) -> Self {
        // 8-hour cycle: settlements at 0h, 8h, 16h UTC → every 28800 seconds
        const PERIOD: i64 = 8 * 3600; // 28800 seconds
        let seconds_into_cycle = utc_secs.rem_euclid(PERIOD);
        let seconds_to_next    = PERIOD - seconds_into_cycle;
        let hours_to_next      = seconds_to_next as f64 / 3600.0;

        if seconds_into_cycle < 1800 {
            // Within 30 min AFTER a settlement → post-settlement repositioning
            FundingCyclePhase::PostSettlement {
                minutes_elapsed: seconds_into_cycle as f64 / 60.0,
            }
        } else if seconds_to_next < 5400 {
            // Within 90 min BEFORE next settlement → pre-settlement closing pressure
            FundingCyclePhase::PreSettlement { hours_remaining: hours_to_next }
        } else {
            FundingCyclePhase::MidCycle { hours_to_next }
        }
    }

    /// Signal amplifier for the funding signal given current cycle phase.
    ///
    /// Returns a multiplier in [0.5, 1.8]:
    ///  • Pre-settlement with significant funding → amplify (closing pressure is real)
    ///  • Post-settlement → reduce (rate just reset, direction uncertain)
    ///  • Mid-cycle → neutral multiplier (1.0)
    pub fn signal_multiplier(&self, funding_rate: f64) -> f64 {
        match self {
            FundingCyclePhase::PreSettlement { hours_remaining } => {
                // Closer to settlement = stronger closing pressure
                // 0–30 min → 1.8×, 30–60 min → 1.5×, 60–90 min → 1.2×
                if funding_rate.abs() > 0.0002 {
                    if *hours_remaining < 0.5 { 1.80 }
                    else if *hours_remaining < 1.0 { 1.50 }
                    else { 1.20 }
                } else {
                    1.0 // neutral funding — cycle phase doesn't matter
                }
            }
            FundingCyclePhase::PostSettlement { .. } => {
                // Rate just reset; avoid being whipsawed by immediate repositioning
                0.60
            }
            FundingCyclePhase::MidCycle { .. } => 1.0,
        }
    }
}

/// Returns the current cycle phase based on the system clock (UTC).
pub fn current_cycle_phase() -> FundingCyclePhase {
    use std::time::{SystemTime, UNIX_EPOCH};
    let utc_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    FundingCyclePhase::from_utc_secs(utc_secs)
}

impl FundingData {
    /// Annualised rate as a percentage (rate × 3 payments/day × 365 days × 100).
    #[allow(dead_code)]
    pub fn annualised_pct(&self) -> f64 {
        self.funding_rate * 3.0 * 365.0 * 100.0
    }

    /// Contrarian signal strength in **[−1.0, +1.0]**, cycle-phase adjusted.
    ///
    /// Blends current funding (60 %) with predicted funding (40 %) for forward-
    /// looking sensitivity, then amplifies or reduces based on where we are in
    /// the 8-hour settlement cycle:
    ///
    ///  • Pre-settlement + significant funding → amplified (closing pressure)
    ///  • Post-settlement → reduced (rate just reset, uncertain)
    ///  • Mid-cycle → standard signal
    ///
    /// * Positive → bullish lean (shorts overcrowded → squeeze risk).
    /// * Negative → bearish lean (longs overcrowded → liquidation risk).
    /// * Zero     → neutral / below noise threshold.
    pub fn signal_strength(&self) -> f64 {
        let blended  = self.funding_rate * 0.60 + self.predicted_rate * 0.40;
        let raw      = self.strength_from_rate(blended);
        let phase    = current_cycle_phase();
        let mult     = phase.signal_multiplier(self.funding_rate);
        (raw * mult).clamp(-1.0, 1.0)
    }

    /// Raw signal strength without cycle-phase adjustment (for testing).
    #[allow(dead_code)]
    pub fn raw_signal_strength(&self) -> f64 {
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

    /// Phase-aware emoji: shows cycle context alongside funding direction.
    #[allow(dead_code)]
    pub fn cycle_emoji(&self) -> &'static str {
        match current_cycle_phase() {
            FundingCyclePhase::PreSettlement { .. }  => "⏰",
            FundingCyclePhase::PostSettlement { .. } => "🔄",
            FundingCyclePhase::MidCycle { .. }       => "·",
        }
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

// ─────────────────────────── Tests ───────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // 8-hour cycle: settlements at 0, 28800, 57600, 86400 seconds past epoch.
    // 00:00 UTC = 0s, 08:00 UTC = 28800s, 16:00 UTC = 57600s

    #[test]
    fn phase_mid_cycle_at_4h_mark() {
        // 4 hours into the cycle = 14400s — well away from settlement
        let phase = FundingCyclePhase::from_utc_secs(14400);
        match phase {
            FundingCyclePhase::MidCycle { hours_to_next } => {
                assert!((hours_to_next - 4.0).abs() < 0.01,
                    "4h into cycle should have 4h remaining, got {hours_to_next}");
            }
            other => panic!("expected MidCycle, got {other:?}"),
        }
    }

    #[test]
    fn phase_pre_settlement_at_45min_before() {
        // 45 min before settlement = cycle at 8h - 45min = 27000s into cycle
        let phase = FundingCyclePhase::from_utc_secs(27000);
        match phase {
            FundingCyclePhase::PreSettlement { hours_remaining } => {
                assert!((hours_remaining - 0.5).abs() < 0.01,
                    "45 min before settlement should give 0.5h remaining, got {hours_remaining}");
            }
            other => panic!("expected PreSettlement, got {other:?}"),
        }
    }

    #[test]
    fn phase_post_settlement_at_10min_after() {
        // 10 min after settlement = 600s into cycle
        let phase = FundingCyclePhase::from_utc_secs(600);
        match phase {
            FundingCyclePhase::PostSettlement { minutes_elapsed } => {
                assert!((minutes_elapsed - 10.0).abs() < 0.1,
                    "10 min after settlement, got {minutes_elapsed}");
            }
            other => panic!("expected PostSettlement, got {other:?}"),
        }
    }

    #[test]
    fn pre_settlement_amplifies_significant_funding() {
        // Positive funding 0.06% (elevated longs) + 30 min to settlement → 1.8× amplifier
        let phase = FundingCyclePhase::PreSettlement { hours_remaining: 0.4 };
        let mult = phase.signal_multiplier(0.0006); // 0.06% > 0.02% threshold
        assert_eq!(mult, 1.80, "< 30 min to settlement with significant funding → 1.8×");
    }

    #[test]
    fn pre_settlement_no_amp_for_neutral_funding() {
        // Neutral funding (0.01%) + near settlement → no amplification
        let phase = FundingCyclePhase::PreSettlement { hours_remaining: 0.2 };
        let mult = phase.signal_multiplier(0.0001); // below 0.02% threshold
        assert_eq!(mult, 1.0, "neutral funding rate should not be amplified by cycle phase");
    }

    #[test]
    fn post_settlement_reduces_signal() {
        let phase = FundingCyclePhase::PostSettlement { minutes_elapsed: 15.0 };
        let mult = phase.signal_multiplier(0.0008);
        assert_eq!(mult, 0.60, "post-settlement should dampen the signal to 0.6×");
    }

    #[test]
    fn signal_strength_clamped_to_unit_range() {
        // Even with large amplifier the output should stay in [-1, 1]
        let data = FundingData {
            symbol:        "SOL".to_string(),
            funding_rate:  0.0020, // very high positive → raw = -1.0
            predicted_rate: 0.0010,
            funding_delta: 0.0001,
        };
        let s = data.raw_signal_strength();
        assert!((-1.0..=1.0).contains(&s), "raw signal out of range: {s}");
    }
}
