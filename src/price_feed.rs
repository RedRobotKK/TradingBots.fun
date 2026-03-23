//! Shared price oracle — Hyperliquid WebSocket + Binance WebSocket → Postgres.
//!
//! # Why this exists
//!
//! Without a shared oracle every tenant trading loop independently hits the
//! Hyperliquid REST API each cycle:
//!
//! ```text
//! 9 tenants × (1 allMids + 40 l2Books) × 2 weight = 738 weight/30s
//!                                                   = 1 476 weight/min
//! HL limit = 1 200 weight/min  →  OVER by 23% → HTTP 429s guaranteed
//! ```
//!
//! With this oracle:
//! - ONE persistent WebSocket connection per exchange feeds all wallets.
//! - REST weight consumed = 0 for mid-prices (WS replaces allMids poll).
//! - Lag = < 100 ms (HL) / < 50 ms (Binance).
//! - Tenants to 429 threshold: ~500+ (only l2Book calls scale with symbol count).
//!
//! # Architecture
//!
//! ```text
//!  Hyperliquid WS ──┐
//!                    ├──► SharedPriceOracle (Arc<RwLock<HashMap>>)
//!  Binance WS ───────┘         │
//!                              ├──► MarketClient.fetch_all_mids() (fast path)
//!                              └──► DB writer (price_oracle + history, every 5s)
//! ```
//!
//! # Binance symbol normalisation
//!
//! Binance uses `BTCUSDT`; Hyperliquid uses `BTC`.  Low-cap HL symbols use a
//! `k` prefix meaning × 1 000 (e.g. `kPEPE` = 1 000 PEPE).  The normaliser
//! strips the USDT/USDC/BUSD suffix and maps PEPE ↔ kPEPE, etc.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::RwLock;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::Message};

// ── Public types ───────────────────────────────────────────────────────────────

/// One entry in the shared price oracle, updated in real-time from WebSocket feeds.
#[derive(Debug, Clone)]
pub struct OracleEntry {
    /// Hyperliquid symbol (e.g. `"BTC"`, `"SOL"`, `"kPEPE"`).
    pub symbol: String,
    /// Composite mid — HL primary, Binance fallback if HL unavailable.
    pub mid: f64,
    /// Best bid from Binance book ticker (0 if unavailable).
    pub bid: f64,
    /// Best ask from Binance book ticker (0 if unavailable).
    pub ask: f64,
    /// `(ask − bid) / mid × 10 000`  in basis points.
    pub spread_bps: f32,
    /// Hyperliquid native mid (from allMids WebSocket).
    pub hl_mid: Option<f64>,
    /// Binance USDT pair mid `(bid + ask) / 2`.
    pub binance_mid: Option<f64>,
    /// 24-hour USD volume from Binance mini-ticker.
    pub volume_24h_usd: Option<f64>,
    /// Cross-exchange divergence `|hl − binance| / hl × 100` (%).
    /// Values > 0.5% can signal arbitrage or stale data.
    pub cross_spread_pct: Option<f32>,
    /// Which feed(s) populated this entry.
    pub source: &'static str, // "hyperliquid" | "binance" | "composite"
    /// Wall-clock time of last update (UTC, for DB storage and staleness checks).
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// `symbol → OracleEntry`, shared across all tenant trading loops.
pub type SharedPriceOracle = Arc<RwLock<HashMap<String, OracleEntry>>>;

/// Create an empty oracle — pass to `MarketClient::with_oracle` and
/// `PriceFeedService::new`.
pub fn new_oracle() -> SharedPriceOracle {
    Arc::new(RwLock::new(HashMap::new()))
}

// ── Constants ─────────────────────────────────────────────────────────────────

const HL_WS_URL: &str = "wss://api.hyperliquid.xyz/ws";
/// Binance combined stream: `!bookTicker` fires on every best-bid/ask change
/// (all symbols); `!miniTicker@arr` gives volume/price per symbol every 1 s.
/// We connect both: bookTicker for tight bid/ask, miniTicker for 24-h volume.
const BINANCE_BOOK_URL: &str = "wss://stream.binance.com:9443/ws/!bookTicker";
const BINANCE_MINI_URL: &str = "wss://stream.binance.com:9443/ws/!miniTicker@arr";

const RECONNECT_DELAY: Duration = Duration::from_secs(5);
/// How often to flush the in-memory oracle snapshot to Postgres.
const DB_FLUSH_INTERVAL: Duration = Duration::from_secs(5);
/// Price entries older than this are considered stale and bypassed.
pub const ORACLE_TTL_SECS: i64 = 30;

// ── PriceFeedService ──────────────────────────────────────────────────────────

/// Spawns background tasks that keep the [`SharedPriceOracle`] up to date.
///
/// Call [`PriceFeedService::spawn`] once at startup.  The service runs until
/// the process exits; individual WebSocket tasks auto-reconnect on disconnect.
pub struct PriceFeedService {
    oracle: SharedPriceOracle,
    db: Option<sqlx::PgPool>,
}

impl PriceFeedService {
    pub fn new(oracle: SharedPriceOracle, db: Option<sqlx::PgPool>) -> Self {
        Self { oracle, db }
    }

    /// Spawn all feed tasks.  Returns immediately; tasks run in the background.
    pub fn spawn(self) {
        let o1 = self.oracle.clone();
        let o2 = self.oracle.clone();
        let o3 = self.oracle.clone();
        let o4 = self.oracle.clone();
        let db = self.db;

        // ── Hyperliquid allMids WebSocket ──────────────────────────────────
        tokio::spawn(async move {
            loop {
                match run_hl_allmids(o1.clone()).await {
                    Ok(()) => log::info!("HL price WS ended cleanly — reconnecting"),
                    Err(e) => log::warn!("HL price WS error: {e} — reconnecting in 5s"),
                }
                sleep(RECONNECT_DELAY).await;
            }
        });

        // ── Binance book ticker WebSocket (bid/ask/spread) ─────────────────
        tokio::spawn(async move {
            loop {
                match run_binance_book(o2.clone()).await {
                    Ok(()) => log::info!("Binance book WS ended cleanly — reconnecting"),
                    Err(e) => log::warn!("Binance book WS error: {e} — reconnecting in 5s"),
                }
                sleep(RECONNECT_DELAY).await;
            }
        });

        // ── Binance mini-ticker WebSocket (24h volume) ─────────────────────
        tokio::spawn(async move {
            loop {
                match run_binance_mini(o3.clone()).await {
                    Ok(()) => log::info!("Binance mini WS ended cleanly — reconnecting"),
                    Err(e) => log::warn!("Binance mini WS error: {e} — reconnecting in 5s"),
                }
                sleep(RECONNECT_DELAY).await;
            }
        });

        // ── DB flush task ──────────────────────────────────────────────────
        if let Some(pool) = db {
            tokio::spawn(async move {
                loop {
                    sleep(DB_FLUSH_INTERVAL).await;
                    let snapshot = o4.read().await.values().cloned().collect::<Vec<_>>();
                    if let Err(e) = flush_to_db(&pool, &snapshot).await {
                        log::warn!("price_oracle DB flush failed: {e}");
                    }
                }
            });
        }

        log::info!("🔌 PriceFeedService spawned (HL WS + Binance WS + DB writer)");
    }
}

// ── Feed implementations ──────────────────────────────────────────────────────

/// Hyperliquid `allMids` WebSocket — pushed on every price change (~100 ms).
async fn run_hl_allmids(oracle: SharedPriceOracle) -> Result<()> {
    let (mut ws, _) = connect_async(HL_WS_URL).await?;
    log::info!("✅ Hyperliquid allMids WebSocket connected");

    // Subscribe to allMids feed
    ws.send(Message::Text(
        r#"{"method":"subscribe","subscription":{"type":"allMids"}}"#.to_string(),
    ))
    .await?;

    while let Some(msg) = ws.next().await {
        let msg = msg?;
        let Message::Text(txt) = msg else { continue };

        let v: serde_json::Value = match serde_json::from_str(&txt) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Expected: {"channel":"allMids","data":{"mids":{"BTC":"95000",...}}}
        if v["channel"].as_str() != Some("allMids") {
            continue;
        }
        let Some(mids_obj) = v["data"]["mids"].as_object() else {
            continue;
        };

        let now = Utc::now();
        let mut guard = oracle.write().await;
        for (sym, val) in mids_obj {
            let Some(mid) = val.as_str().and_then(|s| s.parse::<f64>().ok()) else {
                continue;
            };
            let entry = guard.entry(sym.clone()).or_insert_with(|| OracleEntry {
                symbol:          sym.clone(),
                mid,
                bid:             0.0,
                ask:             0.0,
                spread_bps:      0.0,
                hl_mid:          None,
                binance_mid:     None,
                volume_24h_usd:  None,
                cross_spread_pct: None,
                source:          "hyperliquid",
                updated_at:      now,
            });
            entry.hl_mid = Some(mid);
            // HL is the authoritative price for all trading decisions
            entry.mid = mid;
            entry.source = if entry.binance_mid.is_some() { "composite" } else { "hyperliquid" };
            entry.cross_spread_pct = compute_cross_spread(entry.hl_mid, entry.binance_mid);
            entry.updated_at = now;
        }
    }
    Ok(())
}

/// Binance `!bookTicker` — fired on every best-bid/ask change.
/// Provides: bid, ask, spread_bps.  High frequency but zero REST cost.
async fn run_binance_book(oracle: SharedPriceOracle) -> Result<()> {
    let (mut ws, _) = connect_async(BINANCE_BOOK_URL).await?;
    log::info!("✅ Binance bookTicker WebSocket connected");

    while let Some(msg) = ws.next().await {
        let msg = msg?;
        let Message::Text(txt) = msg else { continue };

        // {"e":"bookTicker","u":123,"s":"BTCUSDT","b":"95000","B":"0.5","a":"95001","A":"1.2"}
        #[derive(Deserialize)]
        struct BookTick {
            s: String, // symbol  e.g. "BTCUSDT"
            b: String, // best bid
            a: String, // best ask
        }
        let Ok(tick) = serde_json::from_str::<BookTick>(&txt) else {
            continue;
        };

        let Some(hl_sym) = binance_to_hl(&tick.s) else { continue };
        let bid: f64 = tick.b.parse().unwrap_or(0.0);
        let ask: f64 = tick.a.parse().unwrap_or(0.0);
        if bid <= 0.0 || ask <= 0.0 {
            continue;
        }
        let binance_mid = (bid + ask) / 2.0;
        let spread_bps = ((ask - bid) / binance_mid * 10_000.0) as f32;
        let now = Utc::now();

        let mut guard = oracle.write().await;
        let entry = guard.entry(hl_sym.clone()).or_insert_with(|| OracleEntry {
            symbol:          hl_sym.clone(),
            mid:             binance_mid,
            bid,
            ask,
            spread_bps,
            hl_mid:          None,
            binance_mid:     Some(binance_mid),
            volume_24h_usd:  None,
            cross_spread_pct: None,
            source:          "binance",
            updated_at:      now,
        });
        entry.bid         = bid;
        entry.ask         = ask;
        entry.spread_bps  = spread_bps;
        entry.binance_mid = Some(binance_mid);
        if entry.hl_mid.is_none() {
            // No HL data yet — use Binance as temporary mid
            entry.mid    = binance_mid;
            entry.source = "binance";
        } else {
            entry.source = "composite";
        }
        entry.cross_spread_pct = compute_cross_spread(entry.hl_mid, entry.binance_mid);
        entry.updated_at = now;
    }
    Ok(())
}

/// Binance `!miniTicker@arr` — full array of all symbols every 1 second.
/// Provides: 24h volume, 24h price change %.  Used for candidate ranking.
async fn run_binance_mini(oracle: SharedPriceOracle) -> Result<()> {
    let (mut ws, _) = connect_async(BINANCE_MINI_URL).await?;
    log::info!("✅ Binance miniTicker WebSocket connected");

    while let Some(msg) = ws.next().await {
        let msg = msg?;
        let Message::Text(txt) = msg else { continue };

        // Array of: {"e":"24hrMiniTicker","s":"BTCUSDT","c":"95000","v":"1234","q":"117000000",...}
        #[derive(Deserialize)]
        struct MiniTick {
            s: String, // symbol
            q: String, // 24h quote-asset volume (USD for USDT pairs)
        }
        let Ok(ticks) = serde_json::from_str::<Vec<MiniTick>>(&txt) else {
            continue;
        };

        let now = Utc::now();
        let mut guard = oracle.write().await;
        for tick in ticks {
            let Some(hl_sym) = binance_to_hl(&tick.s) else { continue };
            let vol: f64 = tick.q.parse().unwrap_or(0.0);
            if let Some(entry) = guard.get_mut(&hl_sym) {
                entry.volume_24h_usd = Some(vol);
                entry.updated_at = now;
            }
            // Note: we don't create entries here; book ticker does that.
        }
    }
    Ok(())
}

// ── DB writer ─────────────────────────────────────────────────────────────────

/// Flush the oracle snapshot to Postgres using UNNEST batch queries.
///
/// Two queries total per flush cycle (regardless of symbol count):
///   1. Batch upsert into `price_oracle`  (current state, one row per symbol)
///   2. Batch insert into `price_oracle_history` (time-series tick log)
///
/// Only symbols with at least one HL price (`hl_mid IS NOT NULL`) are written.
/// This excludes Binance-only symbols that HL doesn't trade, keeping the table
/// small and the history useful for trading decisions.
async fn flush_to_db(pool: &sqlx::PgPool, entries: &[OracleEntry]) -> Result<()> {
    // Only persist symbols that Hyperliquid knows about (have a hl_mid).
    let hl_entries: Vec<&OracleEntry> = entries
        .iter()
        .filter(|e| e.hl_mid.is_some())
        .collect();

    if hl_entries.is_empty() {
        return Ok(());
    }

    let now = Utc::now();

    // ── Build typed vectors for UNNEST ────────────────────────────────────────
    let mut symbols:         Vec<&str>        = Vec::with_capacity(hl_entries.len());
    let mut mids:            Vec<f64>         = Vec::with_capacity(hl_entries.len());
    let mut bids:            Vec<f64>         = Vec::with_capacity(hl_entries.len());
    let mut asks:            Vec<f64>         = Vec::with_capacity(hl_entries.len());
    let mut spreads:         Vec<f64>         = Vec::with_capacity(hl_entries.len());
    let mut hl_mids:         Vec<Option<f64>> = Vec::with_capacity(hl_entries.len());
    let mut binance_mids:    Vec<Option<f64>> = Vec::with_capacity(hl_entries.len());
    let mut volumes:         Vec<Option<f64>> = Vec::with_capacity(hl_entries.len());
    let mut cross_spreads:   Vec<Option<f64>> = Vec::with_capacity(hl_entries.len());
    let mut sources:         Vec<&str>        = Vec::with_capacity(hl_entries.len());

    for e in &hl_entries {
        symbols.push(&e.symbol);
        mids.push(e.mid);
        bids.push(e.bid);
        asks.push(e.ask);
        spreads.push(e.spread_bps as f64);
        hl_mids.push(e.hl_mid);
        binance_mids.push(e.binance_mid);
        volumes.push(e.volume_24h_usd);
        cross_spreads.push(e.cross_spread_pct.map(|v| v as f64));
        sources.push(e.source);
    }

    // ── Query 1: upsert price_oracle (1 query, N rows via UNNEST) ─────────────
    sqlx::query(
        r#"INSERT INTO price_oracle
               (symbol, mid, bid, ask, spread_bps, hl_mid, binance_mid,
                volume_24h_usd, cross_spread_pct, source, updated_at)
           SELECT * FROM UNNEST(
               $1::text[], $2::float8[], $3::float8[], $4::float8[],
               $5::float8[], $6::float8[], $7::float8[],
               $8::float8[], $9::float8[], $10::text[],
               $11::timestamptz[]
           ) AS t(symbol, mid, bid, ask, spread_bps, hl_mid, binance_mid,
                  volume_24h_usd, cross_spread_pct, source, updated_at)
           ON CONFLICT (symbol) DO UPDATE SET
               mid              = EXCLUDED.mid,
               bid              = EXCLUDED.bid,
               ask              = EXCLUDED.ask,
               spread_bps       = EXCLUDED.spread_bps,
               hl_mid           = EXCLUDED.hl_mid,
               binance_mid      = EXCLUDED.binance_mid,
               volume_24h_usd   = EXCLUDED.volume_24h_usd,
               cross_spread_pct = EXCLUDED.cross_spread_pct,
               source           = EXCLUDED.source,
               updated_at       = EXCLUDED.updated_at"#,
    )
    .bind(&symbols)
    .bind(&mids)
    .bind(&bids)
    .bind(&asks)
    .bind(&spreads)
    .bind(&hl_mids)
    .bind(&binance_mids)
    .bind(&volumes)
    .bind(&cross_spreads)
    .bind(&sources)
    .bind(vec![now; hl_entries.len()])
    .execute(pool)
    .await?;

    // ── Query 2: append price_oracle_history (1 query, N rows via UNNEST) ────
    sqlx::query(
        r#"INSERT INTO price_oracle_history
               (symbol, mid, hl_mid, binance_mid, spread_bps, volume_24h_usd,
                source, recorded_at)
           SELECT * FROM UNNEST(
               $1::text[], $2::float8[], $3::float8[], $4::float8[],
               $5::float8[], $6::float8[], $7::text[], $8::timestamptz[]
           ) AS t(symbol, mid, hl_mid, binance_mid, spread_bps, volume_24h_usd,
                  source, recorded_at)"#,
    )
    .bind(&symbols)
    .bind(&mids)
    .bind(&hl_mids)
    .bind(&binance_mids)
    .bind(&spreads)
    .bind(&volumes)
    .bind(&sources)
    .bind(vec![now; hl_entries.len()])
    .execute(pool)
    .await?;

    log::debug!(
        "price_oracle: flushed {} HL symbols to DB ({} total in oracle)",
        hl_entries.len(),
        entries.len(),
    );
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Convert a Binance trading symbol to its Hyperliquid equivalent.
///
/// Returns `None` for symbols that don't exist on Hyperliquid, including:
/// - Non-USDT/USDC/BUSD quote pairs (BTC/BNB, ETH/BTC, …)
/// - Leveraged tokens (`BTCUP`, `BTCDOWN`, `BEAR`, `BULL`)
/// - Structured products (`BTCDOM`, …)
///
/// Hyperliquid `k`-prefix mapping (1 token = 1 000 units, e.g. `kPEPE`):
/// - PEPEUSDT → `kPEPE`   (PEPE trades at ~$0.000010 → HL scales × 1 000)
/// - SHIBUSDT → `kSHIB`
/// - FLOKIUSDT → `kFLOKI`
/// - BONKUSDT → `kBONK`
/// - NEIROUSDT → `kNEIRO`
pub fn binance_to_hl(binance: &str) -> Option<String> {
    // Strip quote suffix
    let base = if let Some(s) = binance.strip_suffix("USDT") { s }
               else if let Some(s) = binance.strip_suffix("USDC") { s }
               else if let Some(s) = binance.strip_suffix("BUSD") { s }
               else { return None };

    // Reject leveraged/inverse/structured tokens
    if matches!(base,
        s if s.ends_with("UP")   || s.ends_with("DOWN") ||
             s.ends_with("BULL") || s.ends_with("BEAR") ||
             s.ends_with("DOM")  || s.ends_with("HEDGE"))
    {
        return None;
    }

    // HL k-prefix tokens (low-price tokens scaled × 1 000 on HL)
    let hl_sym = match base {
        "PEPE"  => "kPEPE",
        "SHIB"  => "kSHIB",
        "FLOKI" => "kFLOKI",
        "BONK"  => "kBONK",
        "NEIRO" => "kNEIRO",
        "LUNC"  => "kLUNC",
        other   => other,
    };

    Some(hl_sym.to_string())
}

/// `|hl − binance| / hl × 100` (%).  Returns `None` if either price missing.
fn compute_cross_spread(hl: Option<f64>, binance: Option<f64>) -> Option<f32> {
    let h = hl?;
    let b = binance?;
    if h <= 1e-10 {
        return None;
    }
    Some(((h - b).abs() / h * 100.0) as f32)
}

// ── Public convenience ────────────────────────────────────────────────────────

/// Extract a `symbol → mid` map from the oracle for use as a drop-in
/// replacement for `MarketClient::fetch_all_mids()`.
pub async fn oracle_to_mids(oracle: &SharedPriceOracle) -> HashMap<String, f64> {
    let guard = oracle.read().await;
    let cutoff = Utc::now() - chrono::Duration::seconds(ORACLE_TTL_SECS);
    guard
        .values()
        .filter(|e| e.updated_at > cutoff)
        .map(|e| (e.symbol.clone(), e.mid))
        .collect()
}

/// Return the best available mid for a single symbol, or `None` if stale/absent.
pub async fn oracle_mid(oracle: &SharedPriceOracle, symbol: &str) -> Option<f64> {
    let guard = oracle.read().await;
    let entry = guard.get(symbol)?;
    let cutoff = Utc::now() - chrono::Duration::seconds(ORACLE_TTL_SECS);
    if entry.updated_at > cutoff {
        Some(entry.mid)
    } else {
        None
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binance_to_hl_standard_symbols() {
        assert_eq!(binance_to_hl("BTCUSDT"),  Some("BTC".into()));
        assert_eq!(binance_to_hl("ETHUSDT"),  Some("ETH".into()));
        assert_eq!(binance_to_hl("SOLUSDT"),  Some("SOL".into()));
        assert_eq!(binance_to_hl("FETUSDT"),  Some("FET".into()));
    }

    #[test]
    fn binance_to_hl_k_prefix_tokens() {
        assert_eq!(binance_to_hl("PEPEUSDT"),  Some("kPEPE".into()));
        assert_eq!(binance_to_hl("SHIBUSDT"),  Some("kSHIB".into()));
        assert_eq!(binance_to_hl("BONKUSDT"),  Some("kBONK".into()));
        assert_eq!(binance_to_hl("FLOKIUSDT"), Some("kFLOKI".into()));
    }

    #[test]
    fn binance_to_hl_rejects_leveraged_tokens() {
        assert_eq!(binance_to_hl("BTCUPUSDT"),   None);
        assert_eq!(binance_to_hl("BTCDOWNUSDT"), None);
        assert_eq!(binance_to_hl("ETHBULLBUSD"), None);
        assert_eq!(binance_to_hl("BTCDOMUSDT"),  None);
    }

    #[test]
    fn binance_to_hl_rejects_non_usdt_pairs() {
        assert_eq!(binance_to_hl("ETHBTC"),  None);
        assert_eq!(binance_to_hl("BNBETH"),  None);
    }

    #[test]
    fn cross_spread_pct() {
        // 1% divergence
        let pct = compute_cross_spread(Some(100.0), Some(101.0));
        assert!(pct.is_some());
        assert!((pct.unwrap() - 1.0).abs() < 0.01);

        // Missing source
        assert!(compute_cross_spread(None, Some(100.0)).is_none());
        assert!(compute_cross_spread(Some(100.0), None).is_none());
    }
}
