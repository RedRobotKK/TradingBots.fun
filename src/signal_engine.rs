//! Per-symbol signal computation engine — scales to 1M+ tenants.
//!
//! # The problem with per-tenant signal computation
//!
//! The original `run_cycle()` computed signals for every candidate symbol on
//! every 30-second tick, **for every tenant independently**.  With N tenants
//! that means N identical RSI/MACD/order-book computations per symbol per cycle.
//! The computations are deterministic (same input, same output), so repeating
//! them per tenant is pure waste.
//!
//! # Solution: one engine, per-symbol workers
//!
//! [`SignalEngine`] runs a single background task that:
//!
//! 1. Reads the current symbol list from the price oracle (all live HL perps).
//! 2. For each active symbol, fetches candles + order book **once**.
//! 3. Computes RSI, MACD, ATR, volume z-score, and order-flow signals.
//! 4. Writes results to `symbol_signals` (Postgres) via UNNEST batch upsert.
//! 5. Publishes an in-memory snapshot to [`SharedSignalCache`] for zero-DB-latency
//!    reads inside the same process.
//!
//! # Fan-out
//!
//! Every tenant loop, position monitor, and execution worker reads from
//! [`SharedSignalCache`] or `symbol_signals`.  Tenants never compute signals
//! directly — they consume the pre-computed results.
//!
//! # Throughput
//!
//! At 400 HL perp symbols × 30 s cycle = 13 symbols/sec.
//! Each symbol: ~2 ms computation + async I/O (candles cached per bar).
//! Total engine CPU: <50 ms/cycle, 1 Postgres upsert per cycle (UNNEST).
//!
//! # Adding more signal types
//!
//! 1. Add a field to [`SignalSnapshot`].
//! 2. Compute it in [`compute_for_symbol`].
//! 3. Serialize into `indicators` JSONB or add a dedicated column if you need
//!    SQL filtering on it.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio::sync::RwLock;
use tokio::time::{interval, sleep};

use crate::data::{is_hl_perp, MarketClient};
use crate::indicators;
use crate::price_feed::{oracle_to_mids, SharedPriceOracle};
use crate::signals;

// ─────────────────────────── Constants ───────────────────────────────────────

/// How often the engine recomputes signals for all active symbols.
/// Must be ≥ the HL candle bar granularity (1 m) but small enough to catch
/// fast-moving setups.  30 s is the existing per-tenant cycle cadence.
const ENGINE_CYCLE_SECS: u64 = 30;

/// Maximum symbols to analyse per cycle.  Matches `MAX_CANDIDATES` in main.rs
/// but applied globally (not per-tenant) so the total cost is the same as
/// a single tenant used to pay.
const MAX_SYMBOLS: usize = 40;

/// Minimum 24-h USD volume for a symbol to be considered.
/// Below this the spread is typically too wide for reliable fills.
const MIN_VOLUME_USD: f64 = 5_000_000.0;

/// Minimum signal confidence to persist to `symbol_signals`.
/// Noise below this threshold is not worth storing.
const MIN_PERSIST_CONFIDENCE: f64 = 0.10;

// ─────────────────────────── Output type ─────────────────────────────────────

/// Computed signal snapshot for one symbol, valid for up to one engine cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalSnapshot {
    pub symbol: String,

    /// Net signal score in [-1.0, 1.0]. Positive = long bias, negative = short.
    pub signal_score: f64,

    /// Directional label derived from `signal_score`.
    pub direction: SignalDirection,

    /// Composite confidence [0.0, 1.0] — higher = more indicators agree.
    pub confidence: f64,

    // ── Indicators ────────────────────────────────────────────────────────
    pub rsi_14: Option<f64>,
    pub rsi_direction: Option<String>, // "bullish" | "bearish" | "neutral"
    pub macd_hist: Option<f64>,        // positive = bullish momentum
    pub atr_pct: Option<f64>,          // ATR as % of current price
    pub volume_z: Option<f64>,         // volume z-score vs 20-period mean

    // ── Order flow (from live L2 book) ────────────────────────────────────
    pub book_imbalance: Option<f64>,   // bid/(bid+ask) depth ratio
    pub spread_bps: Option<f64>,       // best bid-ask spread in bps
    pub has_bid_wall: bool,
    pub has_ask_wall: bool,

    // ── Market context ────────────────────────────────────────────────────
    pub mid_price: f64,
    pub volume_24h_usd: Option<f64>,
    pub funding_rate: Option<f64>,

    /// Full serialised indicator set — stored as JSONB in `symbol_signals`
    /// so MCP/Claude can query any field without a schema change.
    pub indicators_json: serde_json::Value,

    pub computed_at: chrono::DateTime<Utc>,
    pub cycle_seq: u64,
}

/// Directional bias derived from the composite signal score.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SignalDirection {
    Long,
    Short,
    Neutral,
}

impl SignalDirection {
    pub fn as_str(&self) -> &'static str {
        match self {
            SignalDirection::Long => "long",
            SignalDirection::Short => "short",
            SignalDirection::Neutral => "neutral",
        }
    }
}

impl From<f64> for SignalDirection {
    fn from(score: f64) -> Self {
        if score > 0.15 {
            SignalDirection::Long
        } else if score < -0.15 {
            SignalDirection::Short
        } else {
            SignalDirection::Neutral
        }
    }
}

// ─────────────────────────── Shared cache ────────────────────────────────────

/// In-memory snapshot of the latest computed signals, keyed by HL symbol.
///
/// This is the fast path for the same-process consumers (position monitor,
/// execution workers, web dashboard).  Cross-process consumers (separate
/// worker pods) read from `symbol_signals` in Postgres instead.
pub type SharedSignalCache = Arc<RwLock<HashMap<String, SignalSnapshot>>>;

/// Create a new empty signal cache — inject into all consumers at startup.
pub fn new_signal_cache() -> SharedSignalCache {
    Arc::new(RwLock::new(HashMap::new()))
}

// ── Convenience read helpers ──────────────────────────────────────────────────

/// Return the latest snapshot for `symbol`, or `None` if not yet computed.
pub async fn get_signal(cache: &SharedSignalCache, symbol: &str) -> Option<SignalSnapshot> {
    cache.read().await.get(symbol).cloned()
}

/// Return all current snapshots sorted by confidence (highest first).
pub async fn top_signals(
    cache: &SharedSignalCache,
    direction: Option<&SignalDirection>,
    min_confidence: f64,
) -> Vec<SignalSnapshot> {
    let guard = cache.read().await;
    let mut snaps: Vec<SignalSnapshot> = guard
        .values()
        .filter(|s| {
            s.confidence >= min_confidence
                && direction
                    .map(|d| &s.direction == d)
                    .unwrap_or(true)
        })
        .cloned()
        .collect();
    snaps.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));
    snaps
}

// ─────────────────────────── Engine ──────────────────────────────────────────

/// Spawns background signal computation.
///
/// # Usage
/// ```rust
/// let signal_cache = signal_engine::new_signal_cache();
/// SignalEngine::new(market.clone(), oracle.clone(), signal_cache.clone(), db_pool)
///     .spawn();
/// // ... inject signal_cache into position_monitor and execution workers
/// ```
pub struct SignalEngine {
    market: Arc<MarketClient>,
    oracle: SharedPriceOracle,
    cache: SharedSignalCache,
    db: Option<PgPool>,
}

impl SignalEngine {
    pub fn new(
        market: Arc<MarketClient>,
        oracle: SharedPriceOracle,
        cache: SharedSignalCache,
        db: Option<PgPool>,
    ) -> Self {
        Self { market, oracle, cache, db }
    }

    /// Spawn the engine loop.  Returns immediately; computation runs in the background.
    pub fn spawn(self) {
        tokio::spawn(async move {
            self.run().await;
        });
    }

    async fn run(self) {
        info!("📊 SignalEngine starting (cycle={}s, max_symbols={})", ENGINE_CYCLE_SECS, MAX_SYMBOLS);
        let mut tick = interval(Duration::from_secs(ENGINE_CYCLE_SECS));
        tick.tick().await; // skip first immediate tick — let oracle warm up first
        let mut cycle_seq: u64 = 0;
        loop {
            tick.tick().await;
            cycle_seq += 1;
            if let Err(e) = self.run_cycle(cycle_seq).await {
                error!("SignalEngine cycle {} failed: {}", cycle_seq, e);
                sleep(Duration::from_secs(5)).await;
            }
        }
    }

    async fn run_cycle(&self, cycle_seq: u64) -> Result<()> {
        let t0 = std::time::Instant::now();

        // ── Step 1: get ranked symbol list from oracle ─────────────────────
        let mids: HashMap<String, f64> = oracle_to_mids(&self.oracle).await;
        if mids.is_empty() {
            debug!("SignalEngine: oracle empty, skipping cycle {}", cycle_seq);
            return Ok(());
        }

        // Filter to HL perps with sufficient volume, rank by volume desc.
        // Volume comes from oracle (populated by Binance mini-ticker).
        let oracle_guard = self.oracle.read().await;
        let mut candidates: Vec<(String, f64)> = mids
            .keys()
            .filter(|sym| is_hl_perp(sym))
            .filter_map(|sym| {
                oracle_guard
                    .get(sym)
                    .and_then(|e| e.volume_24h_usd)
                    .filter(|&v| v >= MIN_VOLUME_USD)
                    .map(|v| (sym.clone(), v))
            })
            .collect();
        drop(oracle_guard);

        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        candidates.truncate(MAX_SYMBOLS);

        debug!(
            "SignalEngine cycle {}: {} candidates (of {} perps with volume)",
            cycle_seq,
            candidates.len(),
            mids.len()
        );

        // ── Step 2: compute signals for each symbol ────────────────────────
        let mut snapshots: Vec<SignalSnapshot> = Vec::with_capacity(candidates.len());
        for (symbol, _vol) in &candidates {
            match self.compute_for_symbol(symbol, &mids, cycle_seq).await {
                Ok(snap) => snapshots.push(snap),
                Err(e) => warn!("SignalEngine: {} compute error: {}", symbol, e),
            }
        }

        // ── Step 3: update in-memory cache ────────────────────────────────
        {
            let mut cache = self.cache.write().await;
            for snap in &snapshots {
                cache.insert(snap.symbol.clone(), snap.clone());
            }
        }

        // ── Step 4: batch-upsert to Postgres ──────────────────────────────
        if let Some(pool) = &self.db {
            if let Err(e) = flush_to_db(pool, &snapshots).await {
                warn!("SignalEngine: DB flush failed: {}", e);
            }
        }

        let elapsed = t0.elapsed();
        info!(
            "📊 SignalEngine cycle {} done: {} signals, {:.0} ms",
            cycle_seq,
            snapshots.len(),
            elapsed.as_millis()
        );
        Ok(())
    }

    /// Compute the full signal snapshot for one symbol.
    ///
    /// This replaces the signal computation block that previously ran inside
    /// `run_cycle()` for each tenant independently.  It runs once per symbol
    /// per engine cycle regardless of tenant count.
    async fn compute_for_symbol(
        &self,
        symbol: &str,
        mids: &HashMap<String, f64>,
        cycle_seq: u64,
    ) -> Result<SignalSnapshot> {
        let mid = *mids.get(symbol).ok_or_else(|| anyhow::anyhow!("no mid for {}", symbol))?;

        // ── Candles (1h, cached per bar epoch via MarketClient) ───────────
        let candles = self.market.fetch_market_data(symbol).await.unwrap_or_default();

        // ── Order book (25s cached in MarketClient) ───────────────────────
        let book = self.market.fetch_order_book(symbol).await.ok();

        // ── Indicator computation via calculate_all ────────────────────────
        // `calculate_all` requires ≥26 candles; returns all indicators in one call.
        let ind = if candles.len() >= 26 {
            indicators::calculate_all(&candles).ok()
        } else {
            None
        };

        let rsi_14 = ind.as_ref().map(|i| i.rsi);
        let macd_hist = ind.as_ref().map(|i| i.macd_histogram);
        let atr_pct = ind.as_ref().map(|i| {
            if mid > 0.0 { (i.atr / mid) * 100.0 } else { 0.0 }
        });
        // Volume z-score: current bar volume vs 20-bar mean (from volume_ratio)
        let volume_z = ind.as_ref().map(|i| {
            // volume_ratio = current_vol / mean_vol; convert to z-score approximation
            // Assumes log-normal volume: z ≈ (ratio - 1.0) / 0.5
            (i.volume_ratio - 1.0) / 0.5
        });

        let rsi_direction = rsi_14.map(|r| {
            if r > 60.0 { "bullish".to_string() }
            else if r < 40.0 { "bearish".to_string() }
            else { "neutral".to_string() }
        });

        // ── Order-flow signals ─────────────────────────────────────────────
        let (book_imbalance, spread_bps, has_bid_wall, has_ask_wall) =
            if let Some(ref ob) = book {
                if let Ok(ofs) = signals::detect_order_flow(ob) {
                    let imbalance = if ofs.bid_volume + ofs.ask_volume > 0.0 {
                        Some(ofs.bid_volume / (ofs.bid_volume + ofs.ask_volume))
                    } else {
                        None
                    };
                    // spread_pct is already percentage — convert to bps (×100)
                    let spr = Some(ofs.spread_pct * 100.0);
                    (imbalance, spr, ofs.bid_wall_near, ofs.ask_wall_near)
                } else {
                    (None, None, false, false)
                }
            } else {
                (None, None, false, false)
            };

        // ── Oracle context (Binance volume + cross-spread) ─────────────────
        let (volume_24h_usd, oracle_spread_bps) = {
            let g = self.oracle.read().await;
            if let Some(entry) = g.get(symbol) {
                (entry.volume_24h_usd, Some(entry.spread_bps as f64))
            } else {
                (None, None)
            }
        };

        let effective_spread_bps = spread_bps.or(oracle_spread_bps);

        // ── Composite score ────────────────────────────────────────────────
        //
        // Weights are intentionally simple and interpretable.
        // Score components (each contributing towards [-1, +1]):
        //   • RSI: overbought/oversold momentum (30%)
        //   • MACD histogram: trend direction and strength (35%)
        //   • Book imbalance: order-flow pressure (25%)
        //   • Volume z-score: participation / conviction (10%)
        let mut score: f64 = 0.0;
        let mut weight_sum: f64 = 0.0;

        if let Some(rsi) = rsi_14 {
            let rsi_contrib = (rsi - 50.0) / 50.0; // → [-1, +1]
            score += rsi_contrib * 0.30;
            weight_sum += 0.30;
        }

        if let Some(hist) = macd_hist {
            let atr_norm = atr_pct.unwrap_or(1.0) / 100.0 * mid;
            let macd_contrib = if atr_norm > 0.0 {
                (hist / atr_norm).clamp(-1.0, 1.0)
            } else {
                0.0
            };
            score += macd_contrib * 0.35;
            weight_sum += 0.35;
        }

        if let Some(imb) = book_imbalance {
            let imb_contrib = (imb - 0.5) * 2.0; // → [-1, +1]
            score += imb_contrib * 0.25;
            weight_sum += 0.25;
        }

        if let Some(vz) = volume_z {
            let vz_contrib = vz.clamp(-2.0, 2.0) / 2.0 * score.signum();
            score += vz_contrib * 0.10;
            weight_sum += 0.10;
        }

        if weight_sum > 0.0 {
            score /= weight_sum;
        }
        score = score.clamp(-1.0, 1.0);

        // Confidence: fraction of available indicators that agree with direction
        let confidence = {
            let agreement: f64 = [
                rsi_14.map(|r| if (r > 55.0) == (score > 0.0) { 1.0_f64 } else { 0.0 }),
                macd_hist.map(|h| if (h > 0.0) == (score > 0.0) { 1.0 } else { 0.0 }),
                book_imbalance.map(|i| if (i > 0.5) == (score > 0.0) { 1.0 } else { 0.0 }),
            ]
            .iter()
            .filter_map(|x| *x)
            .sum::<f64>();

            let count = [rsi_14.map(|_| ()), macd_hist.map(|_| ()), book_imbalance.map(|_| ())]
                .iter()
                .filter(|x| x.is_some())
                .count() as f64;

            if count > 0.0 { (agreement / count) * score.abs() } else { 0.0 }
        };

        // ── JSONB for DB storage / MCP queries ────────────────────────────
        let indicators_json = serde_json::json!({
            "rsi_14": rsi_14,
            "macd_histogram": macd_hist,
            "macd_line": ind.as_ref().map(|i| i.macd),
            "macd_signal": ind.as_ref().map(|i| i.macd_signal),
            "rsi_direction": rsi_direction,
            "atr": ind.as_ref().map(|i| i.atr),
            "atr_pct": atr_pct,
            "atr_expansion_ratio": ind.as_ref().map(|i| i.atr_expansion_ratio),
            "bb_width_pct": ind.as_ref().map(|i| i.bb_width_pct),
            "ema_cross_pct": ind.as_ref().map(|i| i.ema_cross_pct),
            "adx": ind.as_ref().map(|i| i.adx),
            "z_score": ind.as_ref().map(|i| i.z_score),
            "volume_ratio": ind.as_ref().map(|i| i.volume_ratio),
            "volume_z_approx": volume_z,
            "vwap_pct": ind.as_ref().map(|i| i.vwap_pct),
            "book_imbalance": book_imbalance,
            "spread_bps": effective_spread_bps,
            "has_bid_wall": has_bid_wall,
            "has_ask_wall": has_ask_wall,
            "volume_24h_usd": volume_24h_usd,
            "candle_count": candles.len(),
        });

        let direction = SignalDirection::from(score);

        Ok(SignalSnapshot {
            symbol: symbol.to_string(),
            signal_score: score,
            direction,
            confidence,
            rsi_14,
            rsi_direction,
            macd_hist,
            atr_pct,
            volume_z,
            book_imbalance,
            spread_bps: effective_spread_bps,
            has_bid_wall,
            has_ask_wall,
            mid_price: mid,
            volume_24h_usd,
            funding_rate: None,
            indicators_json,
            computed_at: Utc::now(),
            cycle_seq,
        })
    }
}

// ─────────────────────────── DB flush (UNNEST batch) ─────────────────────────

/// Write all signal snapshots to `symbol_signals` in two SQL statements.
///
/// Uses the same UNNEST pattern as `price_feed::flush_to_db` — the entire
/// batch is sent as arrays, not N individual upserts.  Cost = 2 round trips
/// regardless of symbol count.
async fn flush_to_db(pool: &PgPool, snaps: &[SignalSnapshot]) -> Result<()> {
    if snaps.is_empty() {
        return Ok(());
    }

    // Only persist signals above the noise floor.
    let snaps: Vec<&SignalSnapshot> = snaps
        .iter()
        .filter(|s| s.confidence >= MIN_PERSIST_CONFIDENCE)
        .collect();

    if snaps.is_empty() {
        return Ok(());
    }

    // Prepare parallel arrays for UNNEST
    let symbols: Vec<&str> = snaps.iter().map(|s| s.symbol.as_str()).collect();
    let scores: Vec<f64> = snaps.iter().map(|s| s.signal_score).collect();
    let directions: Vec<&str> = snaps.iter().map(|s| s.direction.as_str()).collect();
    let confidences: Vec<f64> = snaps.iter().map(|s| s.confidence).collect();
    let indicators: Vec<serde_json::Value> =
        snaps.iter().map(|s| s.indicators_json.clone()).collect();
    let atr_pcts: Vec<Option<f64>> = snaps.iter().map(|s| s.atr_pct).collect();
    let spread_bps: Vec<Option<f64>> = snaps.iter().map(|s| s.spread_bps).collect();
    let vol_24h: Vec<Option<f64>> = snaps.iter().map(|s| s.volume_24h_usd).collect();
    let cycle_seqs: Vec<i64> = snaps.iter().map(|s| s.cycle_seq as i64).collect();

    sqlx::query(
        r#"
        INSERT INTO symbol_signals
            (symbol, signal_score, direction, confidence, indicators,
             atr_pct, spread_bps, volume_24h_usd, computed_at, cycle_seq)
        SELECT * FROM UNNEST(
            $1::text[], $2::float8[], $3::text[], $4::float8[], $5::jsonb[],
            $6::float8[], $7::float8[], $8::float8[],
            (SELECT ARRAY(SELECT NOW() FROM generate_series(1, $9))),
            $10::bigint[]
        ) AS t(symbol, signal_score, direction, confidence, indicators,
               atr_pct, spread_bps, volume_24h_usd, computed_at, cycle_seq)
        ON CONFLICT (symbol) DO UPDATE SET
            signal_score   = EXCLUDED.signal_score,
            direction      = EXCLUDED.direction,
            confidence     = EXCLUDED.confidence,
            indicators     = EXCLUDED.indicators,
            atr_pct        = EXCLUDED.atr_pct,
            spread_bps     = EXCLUDED.spread_bps,
            volume_24h_usd = EXCLUDED.volume_24h_usd,
            computed_at    = EXCLUDED.computed_at,
            cycle_seq      = EXCLUDED.cycle_seq
        "#,
    )
    .bind(&symbols)
    .bind(&scores)
    .bind(&directions)
    .bind(&confidences)
    .bind(&indicators)
    .bind(&atr_pcts)
    .bind(&spread_bps)
    .bind(&vol_24h)
    .bind(snaps.len() as i32)
    .bind(&cycle_seqs)
    .execute(pool)
    .await?;

    debug!("SignalEngine: flushed {} symbol signals to DB", snaps.len());
    Ok(())
}
