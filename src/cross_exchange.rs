//! Cross-exchange price monitor.
//!
//! Fetches mark/last prices from Binance, ByBit, and OKX every 5 minutes
//! and compares them against Hyperliquid's mid-prices from `allMids`.
//!
//! # Why this matters
//!
//! HL is a closed ecosystem — its prices should track CEX prices closely
//! because arbitrageurs keep them aligned.  When HL diverges from CEX by
//! more than ~0.25 %, it signals one of:
//!
//! 1. **HL-local buying pressure** (HL > CEX) — new leveraged longs building
//!    before the information reaches CEXes; mild BULL signal for the symbol.
//! 2. **HL-local selling pressure** (HL < CEX) — leveraged shorts or panic
//!    selling not yet visible on CEXes; mild BEAR signal.
//! 3. **Arbitrage lag** — will normalise quickly; weaker signal.
//!
//! The signal is intentionally SMALL (max weight 0.03) and requires the
//! anomaly to persist across 3 consecutive 30-second cycles before adding
//! anything to the bull/bear score.  This prevents single-tick noise from
//! affecting decisions.
//!
//! # Data Sources
//!
//! | Exchange | Endpoint | Field used | Rate |
//! |----------|----------|-----------|------|
//! | Binance  | `/api/v3/ticker/price` (bulk) | `price` | 1 call / 5 min |
//! | ByBit    | `/v5/market/tickers?category=linear` | `lastPrice` | 1 call / 5 min |
//! | OKX      | `/api/v5/market/tickers?instType=SWAP` | `last` | 1 call / 5 min |
//! | HL mids  | Passed in from `allMids` call already made by main loop | - | free |
//!
//! # Symbol mapping
//!
//! HL uses bare names ("BTC", "ETH", "kBONK").
//! Binance uses "BTCUSDT", "ETHUSDT", "1000BONKUSDT".
//! ByBit uses "BTCUSDT", "ETHUSDT", "1000BONKUSDT".
//! OKX uses "BTC-USDT-SWAP", "ETH-USDT-SWAP".
//!
//! The mapper normalises all to HL-style names.
//!
//! # Noise controls
//!
//! - Minimum anomaly threshold: ±0.25 % — sub-threshold divergences ignored.
//! - 3-cycle persistence: anomaly must appear in 3 consecutive cycles.
//! - Direction flip resets persistence counter.
//! - Max signal contribution: 0.03 (smaller than any core signal weight).

use anyhow::Result;
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

// ─────────────────────────── Constants ───────────────────────────────────────

/// How often to refresh CEX prices.  5 minutes keeps weight near zero
/// while still catching meaningful divergences.
const REFRESH_INTERVAL: Duration = Duration::from_secs(300);

/// Minimum divergence (%) that triggers a signal.
/// Below this threshold the difference is exchange-spread noise.
const ANOMALY_THRESHOLD_PCT: f64 = 0.25;

/// How many consecutive 30-second bot cycles the anomaly must persist
/// before influencing bull/bear scores.
const PERSISTENCE_CYCLES: u32 = 3;

/// Maximum signal contribution to bull or bear score per cycle.
/// Intentionally tiny — anomalies confirm direction but don't override it.
const MAX_SIGNAL_WEIGHT: f64 = 0.03;

// ─────────────────────────── Public types ────────────────────────────────────

/// Cross-exchange divergence signal for a single symbol.
#[derive(Debug, Clone)]
pub struct CrossExchangeSignal {
    pub symbol: String,
    /// HL price − reference CEX price, as % of reference price.
    /// Positive = HL trading above CEX (local buying pressure).
    /// Negative = HL trading below CEX (local selling pressure).
    pub hl_premium_pct: f64,
    /// Number of consecutive cycles this divergence has persisted.
    pub persistence:    u32,
    /// True when persistence >= PERSISTENCE_CYCLES and anomaly exceeds threshold.
    pub active:         bool,
}

impl CrossExchangeSignal {
    /// Contribution to bull/bear score.
    ///
    /// Returns `(bull_add, bear_add)`.  Only non-zero when `active == true`.
    ///
    /// Positive premium (HL > CEX) → local buying → weak bull.
    /// Negative premium (HL < CEX) → local selling → weak bear.
    ///
    /// Magnitude scales with divergence up to MAX_SIGNAL_WEIGHT.
    pub fn score_contribution(&self) -> (f64, f64) {
        if !self.active { return (0.0, 0.0); }

        // Scale 0.25%→0.03, 0.50%→0.03 (capped), 1.0%→0.03 (capped).
        // Anything above 0.5% gets the full cap — larger divergences are
        // likely fat-finger / stale feed rather than meaningful signal.
        let abs_pct = self.hl_premium_pct.abs();
        let raw_w   = (abs_pct / 0.50) * MAX_SIGNAL_WEIGHT;
        let w       = raw_w.min(MAX_SIGNAL_WEIGHT);

        if self.hl_premium_pct > 0.0 {
            (w, 0.0) // HL above CEX → local buy pressure → bull
        } else {
            (0.0, w) // HL below CEX → local sell pressure → bear
        }
    }

    /// Direction as emoji string for dashboard/log display.
    pub fn emoji(&self) -> &'static str {
        if !self.active          { "⚪" }
        else if self.hl_premium_pct > 0.0 { "🟢" }
        else                     { "🔴" }
    }
}

// ─────────────────────────── Internal state ──────────────────────────────────

/// Last-seen CEX prices for a symbol, blended across available exchanges.
#[derive(Clone)]
struct CexSnapshot {
    /// Weighted-average price across Binance, ByBit, OKX.
    ref_price:   f64,
    /// How many exchanges contributed to this snapshot.
    source_count: u8,
}

/// Per-symbol persistence tracker.
#[derive(Default, Clone)]
struct PersistenceEntry {
    /// Number of consecutive cycles the anomaly has been in the same direction.
    cycles:    u32,
    /// Direction of the anomaly: +1 for HL > CEX, -1 for HL < CEX, 0 for none.
    direction: i8,
}

struct CacheInner {
    /// Latest CEX snapshot per HL symbol.
    cex_prices:  HashMap<String, CexSnapshot>,
    /// Persistence tracker per symbol.
    persistence: HashMap<String, PersistenceEntry>,
    /// Timestamp of the last successful CEX fetch.
    last_fetch:  Option<Instant>,
}

// ─────────────────────────── Main struct ─────────────────────────────────────

/// Thread-safe, auto-refreshing cross-exchange price monitor.
/// Clone the `Arc` freely — one instance per bot.
pub struct CrossExchangeMonitor {
    client: Client,
    inner:  RwLock<CacheInner>,
}

pub type SharedCrossExchange = Arc<CrossExchangeMonitor>;

impl CrossExchangeMonitor {
    pub fn new() -> SharedCrossExchange {
        Arc::new(CrossExchangeMonitor {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            inner: RwLock::new(CacheInner {
                cex_prices:  HashMap::new(),
                persistence: HashMap::new(),
                last_fetch:  None,
            }),
        })
    }

    // ── Public API ────────────────────────────────────────────────────────────

    /// Pre-warm the cache at startup.
    pub async fn warm(&self) {
        match self.refresh_cex_prices().await {
            Ok(count) => log::info!(
                "📡 CrossExchange: pre-warmed {} symbols from Binance/ByBit/OKX",
                count
            ),
            Err(e) => log::warn!(
                "📡 CrossExchange: warm-up failed ({}) — will retry on first evaluate()",
                e
            ),
        }
    }

    /// Evaluate a symbol against the cached CEX snapshot.
    ///
    /// `hl_mid` is the current HL mid-price (from `allMids`).
    /// Transparently refreshes CEX prices when the TTL has expired.
    ///
    /// Returns `None` when the symbol has no CEX counterpart
    /// (e.g. HL-native tokens not listed on major CEXes) or when
    /// the cache has never been successfully populated.
    pub async fn evaluate(
        &self,
        symbol: &str,
        hl_mid: f64,
    ) -> Option<CrossExchangeSignal> {
        // Refresh if stale.
        {
            let r = self.inner.read().await;
            let stale = r.last_fetch
                .map(|t| t.elapsed() >= REFRESH_INTERVAL)
                .unwrap_or(true);
            drop(r);
            if stale {
                if let Err(e) = self.refresh_cex_prices().await {
                    log::warn!("📡 CrossExchange refresh failed: {}", e);
                    // Continue with stale data if we have it; return None below if not.
                }
            }
        }

        let r = self.inner.read().await;
        let snap = r.cex_prices.get(symbol)?;
        if snap.ref_price <= 0.0 { return None; }

        let premium_pct = (hl_mid - snap.ref_price) / snap.ref_price * 100.0;
        let persistence  = r.persistence.get(symbol).cloned().unwrap_or_default();

        Some(CrossExchangeSignal {
            symbol:         symbol.to_string(),
            hl_premium_pct: premium_pct,
            persistence:    persistence.cycles,
            active:         persistence.cycles >= PERSISTENCE_CYCLES
                            && premium_pct.abs() >= ANOMALY_THRESHOLD_PCT,
        })
    }

    /// Update persistence counters for a symbol based on the current divergence.
    ///
    /// Call this once per bot cycle, AFTER calling `evaluate()`.
    /// The persistence gate requires the anomaly to recur in 3 successive
    /// calls before `active` is set to `true`.
    pub async fn tick_persistence(&self, symbol: &str, premium_pct: f64) {
        let mut w = self.inner.write().await;
        let entry = w.persistence.entry(symbol.to_string()).or_default();

        if premium_pct.abs() < ANOMALY_THRESHOLD_PCT {
            // Sub-threshold: reset counter.
            entry.cycles    = 0;
            entry.direction = 0;
            return;
        }

        let cur_dir: i8 = if premium_pct > 0.0 { 1 } else { -1 };

        if cur_dir != entry.direction {
            // Direction flip: reset to 1 (this cycle is the first in new direction).
            entry.cycles    = 1;
            entry.direction = cur_dir;
        } else {
            entry.cycles = entry.cycles.saturating_add(1);
        }
    }

    /// Number of symbols with an active (persistent) anomaly.
    pub async fn active_anomaly_count(&self) -> usize {
        let r = self.inner.read().await;
        r.persistence.values()
            .filter(|e| e.cycles >= PERSISTENCE_CYCLES && e.direction != 0)
            .count()
    }

    /// Age of the CEX price cache in seconds.  `None` if never populated.
    pub async fn cache_age_secs(&self) -> Option<u64> {
        self.inner.read().await
            .last_fetch
            .map(|t| t.elapsed().as_secs())
    }

    // ── Private ───────────────────────────────────────────────────────────────

    /// Fetch prices from all three CEXes in parallel, merge by symbol,
    /// and write into the cache.
    ///
    /// Returns the number of symbols successfully populated.
    async fn refresh_cex_prices(&self) -> Result<usize> {
        // Fire all three requests concurrently; a single exchange failure
        // doesn't block the others.
        let (bn_res, bb_res, okx_res) = tokio::join!(
            self.fetch_binance(),
            self.fetch_bybit(),
            self.fetch_okx(),
        );

        let bn  = bn_res.unwrap_or_else(|e| { log::warn!("📡 Binance CEX fetch: {}", e); HashMap::new() });
        let bb  = bb_res.unwrap_or_else(|e| { log::warn!("📡 ByBit CEX fetch: {}",  e); HashMap::new() });
        let okx = okx_res.unwrap_or_else(|e| { log::warn!("📡 OKX CEX fetch: {}",   e); HashMap::new() });

        // Merge: union of all symbols seen, average across available exchanges.
        let mut merged: HashMap<String, (f64, u8)> = HashMap::new(); // symbol → (sum, count)

        for (sym, price) in &bn  { let e = merged.entry(sym.clone()).or_insert((0.0, 0)); e.0 += price; e.1 += 1; }
        for (sym, price) in &bb  { let e = merged.entry(sym.clone()).or_insert((0.0, 0)); e.0 += price; e.1 += 1; }
        for (sym, price) in &okx { let e = merged.entry(sym.clone()).or_insert((0.0, 0)); e.0 += price; e.1 += 1; }

        let populated = merged.len();

        let mut w = self.inner.write().await;
        for (sym, (sum, cnt)) in merged {
            if cnt > 0 {
                w.cex_prices.insert(sym, CexSnapshot {
                    ref_price:    sum / (cnt as f64),
                    source_count: cnt,
                });
            }
        }
        w.last_fetch = Some(Instant::now());

        log::info!(
            "📡 CrossExchange: {} symbols (Binance={} ByBit={} OKX={})",
            populated, bn.len(), bb.len(), okx.len()
        );

        Ok(populated)
    }

    /// Binance: `GET /api/v3/ticker/price` — returns all symbols in one call.
    /// Response: `[{"symbol": "BTCUSDT", "price": "50000.00"}, ...]`
    async fn fetch_binance(&self) -> Result<HashMap<String, f64>> {
        let resp = self.client
            .get("https://api.binance.com/api/v3/ticker/price")
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("Binance ticker/price HTTP {}", resp.status());
        }

        let items: Vec<serde_json::Value> = resp.json().await?;
        let mut map = HashMap::new();

        for item in &items {
            let sym_raw = item["symbol"].as_str().unwrap_or("");
            let price_str = item["price"].as_str().unwrap_or("0");
            let price: f64 = price_str.parse().unwrap_or(0.0);

            if price <= 0.0 { continue; }

            if let Some(hl_sym) = binance_to_hl(sym_raw) {
                map.insert(hl_sym, price);
            }
        }

        Ok(map)
    }

    /// ByBit: `GET /v5/market/tickers?category=linear` — all linear perpetuals.
    /// Response: `{"result": {"list": [{"symbol": "BTCUSDT", "lastPrice": "50000"}, ...]}}`
    async fn fetch_bybit(&self) -> Result<HashMap<String, f64>> {
        let resp = self.client
            .get("https://api.bybit.com/v5/market/tickers")
            .query(&[("category", "linear")])
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("ByBit tickers HTTP {}", resp.status());
        }

        let body: serde_json::Value = resp.json().await?;
        let list = body["result"]["list"].as_array()
            .ok_or_else(|| anyhow::anyhow!("ByBit: unexpected response shape"))?;

        let mut map = HashMap::new();

        for item in list {
            let sym_raw   = item["symbol"].as_str().unwrap_or("");
            let price_str = item["lastPrice"].as_str().unwrap_or("0");
            let price: f64 = price_str.parse().unwrap_or(0.0);

            if price <= 0.0 { continue; }

            if let Some(hl_sym) = bybit_to_hl(sym_raw) {
                map.insert(hl_sym, price);
            }
        }

        Ok(map)
    }

    /// OKX: `GET /api/v5/market/tickers?instType=SWAP` — all swap (perp) instruments.
    /// Response: `{"data": [{"instId": "BTC-USDT-SWAP", "last": "50000"}, ...]}`
    async fn fetch_okx(&self) -> Result<HashMap<String, f64>> {
        let resp = self.client
            .get("https://www.okx.com/api/v5/market/tickers")
            .query(&[("instType", "SWAP")])
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("OKX tickers HTTP {}", resp.status());
        }

        let body: serde_json::Value = resp.json().await?;
        let list = body["data"].as_array()
            .ok_or_else(|| anyhow::anyhow!("OKX: unexpected response shape"))?;

        let mut map = HashMap::new();

        for item in list {
            let inst_id   = item["instId"].as_str().unwrap_or("");
            let price_str = item["last"].as_str().unwrap_or("0");
            let price: f64 = price_str.parse().unwrap_or(0.0);

            if price <= 0.0 { continue; }

            if let Some(hl_sym) = okx_to_hl(inst_id) {
                map.insert(hl_sym, price);
            }
        }

        Ok(map)
    }
}

// ─────────────────────────── Symbol mappers ──────────────────────────────────

/// Convert a Binance USDT-M futures symbol to HL bare name.
///
/// Binance → HL:
///   "BTCUSDT"      → "BTC"
///   "ETHUSDT"      → "ETH"
///   "1000BONKUSDT" → "kBONK"
///   "1000PEPEUSDT" → "kPEPE"
///   "1000SHIBUSDT" → "kSHIB"
///   "BTCDOMUSDT"   → None  (index, not a perp)
///   "DEFIUSDT"     → None  (composite index)
///
/// Returns `None` for symbols that don't have a plain USDT perp on HL.
fn binance_to_hl(sym: &str) -> Option<String> {
    // Only USDT-margined perps.
    let base = sym.strip_suffix("USDT")?;

    // Exclude composite indices, BTC dominance index, etc.
    if matches!(base, "BTCDOM" | "DEFI" | "ALTSEASON" | "BTCST") {
        return None;
    }

    // 1000x-denomination mapping.
    if let Some(name) = base.strip_prefix("1000") {
        return Some(format!("k{}", name));
    }

    // Plain name — BTC, ETH, SOL, etc.
    Some(base.to_string())
}

/// Convert a ByBit linear USDT symbol to HL bare name.
///
/// ByBit and Binance use the same convention for most symbols.
fn bybit_to_hl(sym: &str) -> Option<String> {
    // ByBit uses "BTCUSDT", "1000PEPEUSDT" — same convention as Binance.
    binance_to_hl(sym)
}

/// Convert an OKX SWAP instrument ID to HL bare name.
///
/// OKX format: "BTC-USDT-SWAP", "ETH-USDT-SWAP", "1000BONK-USDT-SWAP"
fn okx_to_hl(inst_id: &str) -> Option<String> {
    // Must end in "-USDT-SWAP".
    let base_with_suffix = inst_id.strip_suffix("-USDT-SWAP")?;
    let base = base_with_suffix.replace('-', "");

    // Reuse the Binance logic — add fake "USDT" suffix then strip it back off.
    let synthetic = format!("{}USDT", base);
    binance_to_hl(&synthetic)
}

// ─────────────────────────── Unit tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::runtime::Runtime;

    fn rt() -> Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    // ── Symbol mapper tests ───────────────────────────────────────────────────

    #[test]
    fn binance_plain_symbol() {
        assert_eq!(binance_to_hl("BTCUSDT"),  Some("BTC".into()));
        assert_eq!(binance_to_hl("ETHUSDT"),  Some("ETH".into()));
        assert_eq!(binance_to_hl("SOLUSDT"),  Some("SOL".into()));
        assert_eq!(binance_to_hl("AVAXUSDT"), Some("AVAX".into()));
    }

    #[test]
    fn binance_kilo_prefix_maps_to_k() {
        assert_eq!(binance_to_hl("1000BONKUSDT"), Some("kBONK".into()));
        assert_eq!(binance_to_hl("1000PEPEUSDT"), Some("kPEPE".into()));
        assert_eq!(binance_to_hl("1000SHIBUSDT"), Some("kSHIB".into()));
    }

    #[test]
    fn binance_composite_indices_filtered() {
        assert_eq!(binance_to_hl("BTCDOMUSDT"), None);
        assert_eq!(binance_to_hl("DEFIUSDT"),   None);
    }

    #[test]
    fn binance_non_usdt_filtered() {
        // Not a USDT pair — should return None.
        assert_eq!(binance_to_hl("BTCBUSD"),  None);
        assert_eq!(binance_to_hl("ETHBTC"),   None);
    }

    #[test]
    fn okx_swap_to_hl() {
        assert_eq!(okx_to_hl("BTC-USDT-SWAP"),  Some("BTC".into()));
        assert_eq!(okx_to_hl("ETH-USDT-SWAP"),  Some("ETH".into()));
        assert_eq!(okx_to_hl("SOL-USDT-SWAP"),  Some("SOL".into()));
        assert_eq!(okx_to_hl("BTC-USD-SWAP"),   None);  // coin-margined, not USDT
    }

    // ── Persistence / signal tests ────────────────────────────────────────────

    #[test]
    fn signal_inactive_below_threshold() {
        let sig = CrossExchangeSignal {
            symbol:         "BTC".into(),
            hl_premium_pct: 0.10,   // below 0.25% threshold
            persistence:    5,
            active:         false,  // not active even though persistent
        };
        let (bull, bear) = sig.score_contribution();
        assert_eq!(bull, 0.0);
        assert_eq!(bear, 0.0);
    }

    #[test]
    fn signal_positive_premium_gives_bull() {
        let sig = CrossExchangeSignal {
            symbol:         "ETH".into(),
            hl_premium_pct: 0.50,
            persistence:    3,
            active:         true,
        };
        let (bull, bear) = sig.score_contribution();
        assert!(bull  > 0.0, "positive premium should give bull score");
        assert_eq!(bear, 0.0);
        assert!(bull <= MAX_SIGNAL_WEIGHT + 1e-9, "bull score should not exceed cap");
    }

    #[test]
    fn signal_negative_premium_gives_bear() {
        let sig = CrossExchangeSignal {
            symbol:         "SOL".into(),
            hl_premium_pct: -0.50,
            persistence:    4,
            active:         true,
        };
        let (bull, bear) = sig.score_contribution();
        assert_eq!(bull, 0.0);
        assert!(bear > 0.0, "negative premium should give bear score");
        assert!(bear <= MAX_SIGNAL_WEIGHT + 1e-9, "bear score should not exceed cap");
    }

    #[test]
    fn signal_large_premium_capped_at_max_weight() {
        let sig = CrossExchangeSignal {
            symbol:         "BTC".into(),
            hl_premium_pct: 5.0,   // 5% divergence — still capped
            persistence:    10,
            active:         true,
        };
        let (bull, _) = sig.score_contribution();
        assert!((bull - MAX_SIGNAL_WEIGHT).abs() < 1e-9, "large premium capped at MAX_SIGNAL_WEIGHT");
    }

    #[test]
    fn persistence_counter_resets_on_direction_flip() {
        let rt = rt();
        rt.block_on(async {
            let mon = CrossExchangeMonitor::new();

            // 3 cycles bullish.
            for _ in 0..3 {
                mon.tick_persistence("BTC", 0.40).await;
            }
            let count = {
                let r = mon.inner.read().await;
                r.persistence.get("BTC").map(|e| e.cycles).unwrap_or(0)
            };
            assert_eq!(count, 3);

            // Direction flip — should reset to 1.
            mon.tick_persistence("BTC", -0.40).await;
            let count_after = {
                let r = mon.inner.read().await;
                r.persistence.get("BTC").map(|e| e.cycles).unwrap_or(0)
            };
            assert_eq!(count_after, 1, "direction flip should reset persistence to 1");
        });
    }

    #[test]
    fn persistence_counter_resets_on_sub_threshold() {
        let rt = rt();
        rt.block_on(async {
            let mon = CrossExchangeMonitor::new();

            for _ in 0..4 {
                mon.tick_persistence("ETH", 0.40).await;
            }
            // Sub-threshold cycle.
            mon.tick_persistence("ETH", 0.10).await;
            let count = {
                let r = mon.inner.read().await;
                r.persistence.get("ETH").map(|e| e.cycles).unwrap_or(0)
            };
            assert_eq!(count, 0, "sub-threshold should reset persistence counter");
        });
    }
}
