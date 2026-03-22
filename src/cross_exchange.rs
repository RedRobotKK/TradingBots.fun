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

/// How often to refresh CEX prices.
/// 1 minute keeps data fresh enough to catch fast-moving anomalies.
/// Rate cost is negligible: each exchange is ONE bulk call (all symbols),
/// so 3 calls/min vs limits of 1,200/min (Binance), 120/min (ByBit),
/// 20/2s (OKX) — well under every limit.
const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// Minimum divergence (%) that triggers any signal processing.
/// Below this threshold the difference is exchange-spread / fee noise.
const ANOMALY_THRESHOLD_PCT: f64 = 0.25;

/// How many consecutive bot-cycles an anomaly must persist before the
/// MOMENTUM signal (small divergences) fires.  At 1-min CEX refresh and
/// 30-second bot cycles this is approximately 1.5 minutes.
const PERSISTENCE_CYCLES: u32 = 3;

/// Above this divergence (%) the signal flips from momentum to mean-reversion.
///
/// Small divergence (< REVERSION_THRESHOLD):
///   HL > CEX → HL is leading price discovery → mild BULL
///   HL < CEX → local sell pressure building    → mild BEAR
///
/// Large divergence (≥ REVERSION_THRESHOLD):
///   HL > CEX → HL has overshot; arb bots will sell HL → BEAR (contrarian)
///   HL < CEX → HL has undershot; liquidations exhausted → BULL (contrarian)
const REVERSION_THRESHOLD_PCT: f64 = 1.5;

/// Above this divergence (%) the anomaly is extreme enough to:
///   1. Bypass the 3-cycle persistence gate (activate immediately).
///   2. Emit a WARN-level log so the operator sees it in real time.
const EXTREME_THRESHOLD_PCT: f64 = 2.0;

/// Maximum signal weight for small-divergence MOMENTUM signals.
/// Intentionally tiny — confirms direction but doesn't override core signals.
const MAX_MOMENTUM_WEIGHT: f64 = 0.03;

/// Maximum signal weight for large-divergence MEAN-REVERSION signals.
/// Larger than momentum weight — a 3% dislocation is meaningful — but still
/// bounded so a single data-feed glitch can't flip a decision on its own.
const MAX_REVERSION_WEIGHT: f64 = 0.12;

// ─────────────────────────── Public types ────────────────────────────────────

/// How the divergence is being interpreted this cycle.
#[derive(Debug, Clone, PartialEq)]
pub enum DivergenceMode {
    /// Sub-threshold noise — no signal.
    Inactive,
    /// Small divergence: HL is leading CEX price discovery.
    /// HL > CEX → bull,  HL < CEX → bear.
    Momentum,
    /// Large divergence: HL has overshot and will snap back to CEX.
    /// HL > CEX → bear (contrarian), HL < CEX → bull (contrarian).
    MeanReversion,
}

/// Cross-exchange divergence signal for a single symbol.
#[derive(Debug, Clone)]
pub struct CrossExchangeSignal {
    pub symbol: String,
    /// HL price − reference CEX price, as % of reference price.
    /// Positive = HL trading above CEX.
    /// Negative = HL trading below CEX.
    pub hl_premium_pct: f64,
    /// Number of consecutive cycles this divergence has persisted.
    pub persistence: u32,
    /// True when the signal is active (persistence gate met, or extreme bypass).
    pub active: bool,
    /// How the signal is being interpreted — changes with magnitude.
    pub mode: DivergenceMode,
}

impl CrossExchangeSignal {
    /// Contribution to bull/bear score.
    ///
    /// Returns `(bull_add, bear_add)`.  Only non-zero when `active == true`.
    ///
    /// # Signal logic
    ///
    /// **Momentum mode** (divergence 0.25 – 1.5%):
    ///   HL > CEX → HL is leading price discovery → mild BULL
    ///   HL < CEX → local sell pressure building  → mild BEAR
    ///   Weight: scales 0→MAX_MOMENTUM_WEIGHT (0.03) linearly with divergence.
    ///
    /// **Mean-reversion mode** (divergence ≥ 1.5%):
    ///   HL > CEX → HL has overshot; arb closes it by selling HL → BEAR
    ///   HL < CEX → HL has undershot; buying pressure restores parity → BULL
    ///   Weight: scales toward MAX_REVERSION_WEIGHT (0.12) with divergence.
    ///   A 3% anomaly gets the full cap; a 1.5% anomaly gets ~half.
    pub fn score_contribution(&self) -> (f64, f64) {
        if !self.active {
            return (0.0, 0.0);
        }

        let abs_pct = self.hl_premium_pct.abs();

        match self.mode {
            DivergenceMode::Inactive => (0.0, 0.0),

            DivergenceMode::Momentum => {
                // Linear scale up to the momentum cap.
                let w = ((abs_pct / 0.50) * MAX_MOMENTUM_WEIGHT).min(MAX_MOMENTUM_WEIGHT);
                if self.hl_premium_pct > 0.0 {
                    (w, 0.0)
                } else {
                    (0.0, w)
                }
            }

            DivergenceMode::MeanReversion => {
                // Scale toward reversion cap; 3% = full weight, 1.5% = ~half.
                // Direction is FLIPPED vs momentum — HL above CEX is now bearish.
                let w = ((abs_pct / 3.0) * MAX_REVERSION_WEIGHT).min(MAX_REVERSION_WEIGHT);
                if self.hl_premium_pct > 0.0 {
                    (0.0, w) // HL above CEX → will snap down → BEAR
                } else {
                    (w, 0.0) // HL below CEX → will snap up  → BULL
                }
            }
        }
    }

    /// Direction emoji for rationale / dashboard.
    pub fn emoji(&self) -> &'static str {
        match self.mode {
            DivergenceMode::Inactive => "⚪",
            DivergenceMode::Momentum => {
                if self.hl_premium_pct > 0.0 {
                    "🟢"
                } else {
                    "🔴"
                }
            }
            // Reversion: direction of the expected MOVE, not the current premium.
            DivergenceMode::MeanReversion => {
                if self.hl_premium_pct > 0.0 {
                    "🔴⟳"
                } else {
                    "🟢⟳"
                }
            }
        }
    }

    /// Short mode label for rationale string.
    pub fn mode_label(&self) -> &'static str {
        match self.mode {
            DivergenceMode::Inactive => "",
            DivergenceMode::Momentum => "MOM",
            DivergenceMode::MeanReversion => "REV",
        }
    }
}

// ─────────────────────────── Internal state ──────────────────────────────────

/// Last-seen CEX prices for a symbol, blended across available exchanges.
#[derive(Clone)]
struct CexSnapshot {
    /// Weighted-average price across Binance, ByBit, OKX.
    ref_price: f64,
    /// How many exchanges contributed to this snapshot.
    source_count: u8,
}

/// Per-symbol persistence tracker.
#[derive(Default, Clone)]
struct PersistenceEntry {
    /// Number of consecutive cycles the anomaly has been in the same direction.
    cycles: u32,
    /// Direction of the anomaly: +1 for HL > CEX, -1 for HL < CEX, 0 for none.
    direction: i8,
}

struct CacheInner {
    /// Latest CEX snapshot per HL symbol.
    cex_prices: HashMap<String, CexSnapshot>,
    /// Persistence tracker per symbol.
    persistence: HashMap<String, PersistenceEntry>,
    /// Timestamp of the last successful CEX fetch.
    last_fetch: Option<Instant>,
}

// ─────────────────────────── Main struct ─────────────────────────────────────

/// Thread-safe, auto-refreshing cross-exchange price monitor.
/// Clone the `Arc` freely — one instance per bot.
pub struct CrossExchangeMonitor {
    client: Client,
    inner: RwLock<CacheInner>,
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
                cex_prices: HashMap::new(),
                persistence: HashMap::new(),
                last_fetch: None,
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
    pub async fn evaluate(&self, symbol: &str, hl_mid: f64) -> Option<CrossExchangeSignal> {
        // Refresh if stale.
        {
            let r = self.inner.read().await;
            let stale = r
                .last_fetch
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
        if snap.ref_price <= 0.0 {
            return None;
        }

        let premium_pct = (hl_mid - snap.ref_price) / snap.ref_price * 100.0;
        let abs_pct = premium_pct.abs();
        let persistence = r.persistence.get(symbol).cloned().unwrap_or_default();

        // Determine activation.
        // Note: tick_persistence() is called AFTER evaluate() in main.rs, so
        // `persistence.cycles` holds the count from PREVIOUS cycles only.
        // We add +1 to count the current cycle, making PERSISTENCE_CYCLES = 3
        // activate exactly on the 3rd consecutive cycle (not the 4th).
        let normal_active =
            (persistence.cycles + 1) >= PERSISTENCE_CYCLES && abs_pct >= ANOMALY_THRESHOLD_PCT;
        // Extreme bypass: divergence ≥2% activates immediately (no persistence wait)
        // because the arb window may close before 3 cycles have elapsed.
        let extreme_bypass = abs_pct >= EXTREME_THRESHOLD_PCT;
        let active = normal_active || extreme_bypass;

        // Choose interpretation mode based on magnitude.
        let mode = if !active || abs_pct < ANOMALY_THRESHOLD_PCT {
            DivergenceMode::Inactive
        } else if abs_pct >= REVERSION_THRESHOLD_PCT {
            DivergenceMode::MeanReversion
        } else {
            DivergenceMode::Momentum
        };

        // Emit a prominent warning for extreme anomalies so the operator
        // sees it in logs / monitoring even if no trade fires immediately.
        if extreme_bypass {
            log::warn!(
                "🚨 CrossExchange EXTREME anomaly: {} HL{:+.2}% vs CEX \
                 (sources: {}) — {} signal activated immediately",
                symbol,
                premium_pct,
                snap.source_count,
                if premium_pct > 0.0 {
                    "BEAR/reversion"
                } else {
                    "BULL/reversion"
                }
            );
        }

        Some(CrossExchangeSignal {
            symbol: symbol.to_string(),
            hl_premium_pct: premium_pct,
            persistence: persistence.cycles,
            active,
            mode,
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
            entry.cycles = 0;
            entry.direction = 0;
            return;
        }

        let cur_dir: i8 = if premium_pct > 0.0 { 1 } else { -1 };

        if cur_dir != entry.direction {
            // Direction flip: reset to 1 (this cycle is the first in new direction).
            entry.cycles = 1;
            entry.direction = cur_dir;
        } else {
            entry.cycles = entry.cycles.saturating_add(1);
        }
    }

    /// Number of symbols with an active (persistent) anomaly.
    /// Uses the same `+1` offset as `evaluate()` so counts stay in sync.
    #[allow(dead_code)]
    pub async fn active_anomaly_count(&self) -> usize {
        let r = self.inner.read().await;
        r.persistence
            .values()
            .filter(|e| (e.cycles + 1) >= PERSISTENCE_CYCLES && e.direction != 0)
            .count()
    }

    /// Age of the CEX price cache in seconds.  `None` if never populated.
    #[allow(dead_code)]
    pub async fn cache_age_secs(&self) -> Option<u64> {
        self.inner
            .read()
            .await
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
        let (bn_res, bb_res, okx_res) =
            tokio::join!(self.fetch_binance(), self.fetch_bybit(), self.fetch_okx(),);

        let bn = bn_res.unwrap_or_else(|e| {
            log::warn!("📡 Binance CEX fetch: {}", e);
            HashMap::new()
        });
        let bb = bb_res.unwrap_or_else(|e| {
            log::warn!("📡 ByBit CEX fetch: {}", e);
            HashMap::new()
        });
        let okx = okx_res.unwrap_or_else(|e| {
            log::warn!("📡 OKX CEX fetch: {}", e);
            HashMap::new()
        });

        // Merge: union of all symbols seen, average across available exchanges.
        let mut merged: HashMap<String, (f64, u8)> = HashMap::new(); // symbol → (sum, count)

        for (sym, price) in &bn {
            let e = merged.entry(sym.clone()).or_insert((0.0, 0));
            e.0 += price;
            e.1 += 1;
        }
        for (sym, price) in &bb {
            let e = merged.entry(sym.clone()).or_insert((0.0, 0));
            e.0 += price;
            e.1 += 1;
        }
        for (sym, price) in &okx {
            let e = merged.entry(sym.clone()).or_insert((0.0, 0));
            e.0 += price;
            e.1 += 1;
        }

        let populated = merged.len();

        let mut w = self.inner.write().await;
        for (sym, (sum, cnt)) in merged {
            if cnt > 0 {
                w.cex_prices.insert(
                    sym,
                    CexSnapshot {
                        ref_price: sum / (cnt as f64),
                        source_count: cnt,
                    },
                );
            }
        }
        w.last_fetch = Some(Instant::now());

        log::info!(
            "📡 CrossExchange: {} symbols (Binance={} ByBit={} OKX={})",
            populated,
            bn.len(),
            bb.len(),
            okx.len()
        );

        Ok(populated)
    }

    /// Binance: `GET /api/v3/ticker/price` — returns all symbols in one call.
    /// Response: `[{"symbol": "BTCUSDT", "price": "50000.00"}, ...]`
    async fn fetch_binance(&self) -> Result<HashMap<String, f64>> {
        let resp = self
            .client
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

            if price <= 0.0 {
                continue;
            }

            if let Some(hl_sym) = binance_to_hl(sym_raw) {
                map.insert(hl_sym, price);
            }
        }

        Ok(map)
    }

    /// ByBit: `GET /v5/market/tickers?category=linear` — all linear perpetuals.
    /// Response: `{"result": {"list": [{"symbol": "BTCUSDT", "lastPrice": "50000"}, ...]}}`
    async fn fetch_bybit(&self) -> Result<HashMap<String, f64>> {
        let resp = self
            .client
            .get("https://api.bybit.com/v5/market/tickers")
            .query(&[("category", "linear")])
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("ByBit tickers HTTP {}", resp.status());
        }

        let body: serde_json::Value = resp.json().await?;
        let list = body["result"]["list"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("ByBit: unexpected response shape"))?;

        let mut map = HashMap::new();

        for item in list {
            let sym_raw = item["symbol"].as_str().unwrap_or("");
            let price_str = item["lastPrice"].as_str().unwrap_or("0");
            let price: f64 = price_str.parse().unwrap_or(0.0);

            if price <= 0.0 {
                continue;
            }

            if let Some(hl_sym) = bybit_to_hl(sym_raw) {
                map.insert(hl_sym, price);
            }
        }

        Ok(map)
    }

    /// OKX: `GET /api/v5/market/tickers?instType=SWAP` — all swap (perp) instruments.
    /// Response: `{"data": [{"instId": "BTC-USDT-SWAP", "last": "50000"}, ...]}`
    async fn fetch_okx(&self) -> Result<HashMap<String, f64>> {
        let resp = self
            .client
            .get("https://www.okx.com/api/v5/market/tickers")
            .query(&[("instType", "SWAP")])
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("OKX tickers HTTP {}", resp.status());
        }

        let body: serde_json::Value = resp.json().await?;
        let list = body["data"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("OKX: unexpected response shape"))?;

        let mut map = HashMap::new();

        for item in list {
            let inst_id = item["instId"].as_str().unwrap_or("");
            let price_str = item["last"].as_str().unwrap_or("0");
            let price: f64 = price_str.parse().unwrap_or(0.0);

            if price <= 0.0 {
                continue;
            }

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
        assert_eq!(binance_to_hl("BTCUSDT"), Some("BTC".into()));
        assert_eq!(binance_to_hl("ETHUSDT"), Some("ETH".into()));
        assert_eq!(binance_to_hl("SOLUSDT"), Some("SOL".into()));
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
        assert_eq!(binance_to_hl("DEFIUSDT"), None);
    }

    #[test]
    fn binance_non_usdt_filtered() {
        // Not a USDT pair — should return None.
        assert_eq!(binance_to_hl("BTCBUSD"), None);
        assert_eq!(binance_to_hl("ETHBTC"), None);
    }

    #[test]
    fn okx_swap_to_hl() {
        assert_eq!(okx_to_hl("BTC-USDT-SWAP"), Some("BTC".into()));
        assert_eq!(okx_to_hl("ETH-USDT-SWAP"), Some("ETH".into()));
        assert_eq!(okx_to_hl("SOL-USDT-SWAP"), Some("SOL".into()));
        assert_eq!(okx_to_hl("BTC-USD-SWAP"), None); // coin-margined, not USDT
    }

    // ── Persistence / signal tests ────────────────────────────────────────────

    #[test]
    fn signal_inactive_below_threshold() {
        let sig = CrossExchangeSignal {
            symbol: "BTC".into(),
            hl_premium_pct: 0.10,
            persistence: 5,
            active: false,
            mode: DivergenceMode::Inactive,
        };
        let (bull, bear) = sig.score_contribution();
        assert_eq!(bull, 0.0);
        assert_eq!(bear, 0.0);
    }

    /// Small positive premium → momentum → bull signal
    #[test]
    fn signal_momentum_positive_premium_gives_bull() {
        let sig = CrossExchangeSignal {
            symbol: "ETH".into(),
            hl_premium_pct: 0.50,
            persistence: 3,
            active: true,
            mode: DivergenceMode::Momentum,
        };
        let (bull, bear) = sig.score_contribution();
        assert!(
            bull > 0.0,
            "positive momentum premium should give bull score"
        );
        assert_eq!(bear, 0.0);
        assert!(
            bull <= MAX_MOMENTUM_WEIGHT + 1e-9,
            "momentum bull must not exceed cap"
        );
    }

    /// Small negative premium → momentum → bear signal
    #[test]
    fn signal_momentum_negative_premium_gives_bear() {
        let sig = CrossExchangeSignal {
            symbol: "SOL".into(),
            hl_premium_pct: -0.50,
            persistence: 4,
            active: true,
            mode: DivergenceMode::Momentum,
        };
        let (bull, bear) = sig.score_contribution();
        assert_eq!(bull, 0.0);
        assert!(
            bear > 0.0,
            "negative momentum premium should give bear score"
        );
        assert!(
            bear <= MAX_MOMENTUM_WEIGHT + 1e-9,
            "momentum bear must not exceed cap"
        );
    }

    /// Large positive premium (HL overshot) → mean-reversion → BEAR signal (direction flipped)
    #[test]
    fn signal_reversion_large_positive_premium_gives_bear() {
        let sig = CrossExchangeSignal {
            symbol: "BTC".into(),
            hl_premium_pct: 3.0, // HL 3% above CEX — will snap down
            persistence: 1,      // extreme bypass — only 1 cycle
            active: true,
            mode: DivergenceMode::MeanReversion,
        };
        let (bull, bear) = sig.score_contribution();
        assert_eq!(
            bull, 0.0,
            "HL overshooting CEX should be BEAR in reversion mode"
        );
        assert!(
            bear > 0.0,
            "HL overshooting should give bear reversion signal"
        );
        assert!(
            bear <= MAX_REVERSION_WEIGHT + 1e-9,
            "reversion bear must not exceed cap"
        );
    }

    /// Large negative premium (HL undershot) → mean-reversion → BULL signal (direction flipped)
    #[test]
    fn signal_reversion_large_negative_premium_gives_bull() {
        let sig = CrossExchangeSignal {
            symbol: "ETH".into(),
            hl_premium_pct: -3.0, // HL 3% below CEX — will snap up
            persistence: 1,
            active: true,
            mode: DivergenceMode::MeanReversion,
        };
        let (bull, bear) = sig.score_contribution();
        assert!(
            bull > 0.0,
            "HL undershooting CEX should be BULL in reversion mode"
        );
        assert_eq!(bear, 0.0);
        assert!(
            bull <= MAX_REVERSION_WEIGHT + 1e-9,
            "reversion bull must not exceed cap"
        );
    }

    /// Reversion weight grows with magnitude up to the cap
    #[test]
    fn signal_reversion_weight_scales_with_magnitude_and_caps() {
        let make = |pct: f64| CrossExchangeSignal {
            symbol: "BTC".into(),
            hl_premium_pct: -pct,
            persistence: 1,
            active: true,
            mode: DivergenceMode::MeanReversion,
        };

        let (bull_15, _) = make(1.5).score_contribution();
        let (bull_30, _) = make(3.0).score_contribution();
        let (bull_50, _) = make(5.0).score_contribution();

        assert!(
            bull_15 < bull_30,
            "larger divergence should have larger reversion signal"
        );
        assert!(
            (bull_30 - MAX_REVERSION_WEIGHT).abs() < 1e-9,
            "3% divergence should reach full cap"
        );
        assert!(
            (bull_50 - MAX_REVERSION_WEIGHT).abs() < 1e-9,
            "5% divergence should still be capped"
        );
    }

    /// Reversion signal is always larger than momentum signal for same magnitude
    #[test]
    fn reversion_weight_exceeds_momentum_weight_for_same_magnitude() {
        let momentum_sig = CrossExchangeSignal {
            symbol: "SOL".into(),
            hl_premium_pct: -0.50,
            persistence: 3,
            active: true,
            mode: DivergenceMode::Momentum,
        };
        let reversion_sig = CrossExchangeSignal {
            symbol: "SOL".into(),
            hl_premium_pct: -2.0,
            persistence: 1,
            active: true,
            mode: DivergenceMode::MeanReversion,
        };
        let (mom_bull, _) = momentum_sig.score_contribution();
        let (rev_bull, _) = reversion_sig.score_contribution();
        assert!(
            rev_bull > mom_bull,
            "reversion signal ({:.4}) should be larger than momentum ({:.4})",
            rev_bull,
            mom_bull
        );
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
            assert_eq!(
                count_after, 1,
                "direction flip should reset persistence to 1"
            );
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
