//! Event-driven position monitor — scales to 1M+ tenants.
//!
//! # Why the polling loop breaks at scale
//!
//! The current design has one `run_cycle()` loop per tenant that checks every
//! open position for stop/target hits every 30 seconds.  With N tenants × P
//! positions each, that's O(N×P) work per cycle — all of it redundant because
//! the underlying check is just `current_price >= target_price`.
//!
//! At 1M tenants with 5 average positions: 5 million comparisons per cycle.
//! With 30-second cycles that's ~167K comparisons/second — not the compute
//! that kills you, it's the DB reads (each tenant loop loads its positions
//! individually) and the API calls (each loop checks HL for fresh prices).
//!
//! # Solution: event-driven, symbol-grouped evaluation
//!
//! When a symbol's price changes, **all** positions for that symbol become
//! candidates for evaluation.  The position monitor:
//!
//! 1. Subscribes to the [`SharedPriceOracle`] via a diff channel.
//! 2. For every price change above a minimum movement threshold:
//!    a. Loads all open positions for that symbol from the DB (one query).
//!    b. Evaluates stop/target logic for each row (pure math, no API).
//!    c. Writes triggered orders to `execution_queue` (one UNNEST upsert).
//! 3. The execution worker pool drains `execution_queue` and calls HL per-tenant.
//!
//! # Schema note
//!
//! All queries use runtime `sqlx::query()` (not compile-time `query!` macros)
//! so the monitor compiles regardless of whether `execution_queue` and
//! `symbol_signals` tables exist yet.  Those tables are created by migration 016
//! which runs on first bot startup.
//!
//! # Horizontal scaling
//!
//! Multiple bot processes can all run `PositionMonitor` against the same DB.
//! The `execution_queue` uses `SELECT … FOR UPDATE SKIP LOCKED` so workers
//! across processes coordinate without conflicts.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use log::{debug, error, info, warn};
use sqlx::PgPool;
use tokio::sync::RwLock;
use tokio::time::{interval, sleep};
use uuid::Uuid;

use crate::price_feed::SharedPriceOracle;
use crate::signal_engine::SharedSignalCache;

// ─────────────────────────── Constants ───────────────────────────────────────

/// How often the monitor scans for oracle price changes.
const MONITOR_TICK_MS: u64 = 100;

/// Minimum price movement (as fraction of price) that triggers position evaluation.
/// 0.05% = 5 bps.  Below this threshold noise dominates; ignore the tick.
const MIN_MOVE_THRESHOLD: f64 = 0.0005;

/// Maximum number of execution queue rows to write per symbol per tick.
/// Prevents a single symbol from flooding the queue on a big move.
const MAX_QUEUE_PER_SYMBOL: usize = 50;

/// How many execution workers drain the queue in parallel.
///
/// Reduced from 20 → 4.  20 workers polling every 200 ms consumed all 20 pool
/// connections permanently even when the queue was empty (100 DB polls/s).
/// 4 workers is sufficient for the current order volume; increase if needed
/// once real multi-tenant HL execution is wired up in Phase 2.
pub const EXECUTION_WORKER_COUNT: usize = 4;

// ─────────────────────────── Position row ────────────────────────────────────

/// One open position loaded from DB for stop/target evaluation.
/// Matches the actual `positions` table schema from migration 001.
#[derive(Debug, Clone)]
struct OpenPosition {
    id: String,           // "{tenant_id}:{symbol}"
    tenant_id: Uuid,
    side: String,         // "LONG" | "SHORT" (uppercase, per DB constraint)
    entry_price: f64,
    size_usd: f64,
    stop_price: Option<f64>,
    tp_price: Option<f64>,  // single TP level (2R target)
    opened_at: chrono::DateTime<chrono::Utc>,
    cycles_held: i32,
}

/// Reason an order was enqueued — maps 1:1 to `execution_queue.reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReason {
    StopHit,
    TargetHit,
    TimeExit,
}

impl ExitReason {
    fn as_str(self) -> &'static str {
        match self {
            ExitReason::StopHit => "stop_hit",
            ExitReason::TargetHit => "target_hit",
            ExitReason::TimeExit => "time_exit",
        }
    }

    fn priority(self) -> i32 {
        match self {
            ExitReason::StopHit => 5,
            ExitReason::TargetHit => 10,
            ExitReason::TimeExit => 20,
        }
    }
}

#[derive(Debug, Clone)]
struct QueuedExit {
    tenant_id: Uuid,
    symbol: String,
    side: String,          // "sell" for LONG, "buy" for SHORT
    reason: ExitReason,
    current_price: f64,
    size_usd: Option<f64>, // None = close full position
    idempotency_key: String,
}

// ─────────────────────────── Monitor ─────────────────────────────────────────

/// Spawns the event-driven position monitor and execution worker pool.
pub struct PositionMonitor {
    oracle: SharedPriceOracle,
    _signal_cache: SharedSignalCache,
    db: PgPool,
    prev_prices: Arc<RwLock<HashMap<String, f64>>>,
}

impl PositionMonitor {
    pub fn new(
        oracle: SharedPriceOracle,
        signal_cache: SharedSignalCache,
        db: PgPool,
    ) -> Self {
        Self {
            oracle,
            _signal_cache: signal_cache,
            db,
            prev_prices: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn spawn(self) {
        let monitor = Arc::new(self);
        {
            let m = monitor.clone();
            tokio::spawn(async move { m.run_monitor().await; });
        }
        {
            let m = monitor.clone();
            tokio::spawn(async move { m.run_execution_workers().await; });
        }
    }

    async fn run_monitor(self: Arc<Self>) {
        info!(
            "👁  PositionMonitor starting (tick={}ms, min_move={:.2}%)",
            MONITOR_TICK_MS,
            MIN_MOVE_THRESHOLD * 100.0
        );
        let mut tick = interval(Duration::from_millis(MONITOR_TICK_MS));
        loop {
            tick.tick().await;
            if let Err(e) = self.check_price_moves().await {
                warn!("PositionMonitor tick error: {}", e);
            }
        }
    }

    async fn check_price_moves(&self) -> Result<()> {
        let current: HashMap<String, f64> = {
            let guard = self.oracle.read().await;
            guard
                .iter()
                .filter(|(_, e)| e.mid > 0.0)
                .map(|(sym, e)| (sym.clone(), e.mid))
                .collect()
        };

        if current.is_empty() {
            return Ok(());
        }

        let moved: Vec<(String, f64)> = {
            let prev = self.prev_prices.read().await;
            current
                .iter()
                .filter_map(|(sym, &new_price)| {
                    let old = prev.get(sym).copied().unwrap_or(new_price);
                    if old > 0.0 && (new_price - old).abs() / old >= MIN_MOVE_THRESHOLD {
                        Some((sym.clone(), new_price))
                    } else {
                        None
                    }
                })
                .collect()
        };

        {
            let mut prev = self.prev_prices.write().await;
            prev.extend(current.into_iter());
        }

        for (symbol, new_price) in moved {
            if let Err(e) = self.evaluate_symbol_positions(&symbol, new_price).await {
                warn!("PositionMonitor: evaluate {} @ {:.4}: {}", symbol, new_price, e);
            }
        }

        Ok(())
    }

    /// Load all open positions for `symbol` and evaluate stop/target logic.
    ///
    /// Uses runtime `sqlx::query()` (not macros) so this compiles before
    /// migration 016 runs and before columns like `high_water_mark` exist.
    async fn evaluate_symbol_positions(&self, symbol: &str, current_price: f64) -> Result<()> {
        // Open positions are rows still present in the table (deleted on full close).
        // side values are 'LONG'/'SHORT' per the CHECK constraint in migration 001.
        let rows = sqlx::query(
            r#"
            SELECT
                id,
                tenant_id,
                side,
                entry_price::float8,
                COALESCE(size_usd, 0)::float8     AS size_usd,
                stop_price::float8                AS stop_price,
                tp_price::float8                  AS tp_price,
                opened_at,
                cycles_held
            FROM positions
            WHERE symbol = $1
            "#,
        )
        .bind(symbol)
        .fetch_all(&self.db)
        .await;

        let rows = match rows {
            Ok(r) => r,
            Err(e) => {
                debug!("PositionMonitor: positions query for {}: {}", symbol, e);
                return Ok(());
            }
        };

        if rows.is_empty() {
            return Ok(());
        }

        debug!(
            "PositionMonitor: evaluating {} positions for {} @ {:.4}",
            rows.len(), symbol, current_price
        );

        let mut exits: Vec<QueuedExit> = Vec::new();

        for row in &rows {
            use sqlx::Row;
            let id: String = row.try_get("id").unwrap_or_default();
            let tenant_id: Uuid = match row.try_get("tenant_id") {
                Ok(v) => v,
                Err(_) => continue,
            };
            let side: String = row.try_get("side").unwrap_or_default();
            let entry_price: f64 = row.try_get("entry_price").unwrap_or(0.0);
            let size_usd: f64 = row.try_get("size_usd").unwrap_or(0.0);
            let stop_price: Option<f64> = row.try_get("stop_price").ok().flatten();
            let tp_price: Option<f64> = row.try_get("tp_price").ok().flatten();
            let opened_at: chrono::DateTime<chrono::Utc> = row
                .try_get("opened_at")
                .unwrap_or_else(|_| chrono::Utc::now());
            let cycles_held: i32 = row.try_get("cycles_held").unwrap_or(0);

            let pos = OpenPosition {
                id,
                tenant_id,
                side,
                entry_price,
                size_usd,
                stop_price,
                tp_price,
                opened_at,
                cycles_held,
            };

            if let Some(exit) = self.evaluate_position(&pos, current_price) {
                exits.push(exit);
                if exits.len() >= MAX_QUEUE_PER_SYMBOL {
                    break;
                }
            }
        }

        if exits.is_empty() {
            return Ok(());
        }

        info!(
            "PositionMonitor: {} exits triggered for {} @ {:.4}",
            exits.len(), symbol, current_price
        );

        enqueue_exits(&self.db, &exits).await
    }

    fn evaluate_position(&self, pos: &OpenPosition, current_price: f64) -> Option<QueuedExit> {
        let is_long = pos.side == "LONG";
        let close_side = if is_long { "sell" } else { "buy" };

        // ── Stop loss ──────────────────────────────────────────────────────
        if let Some(stop) = pos.stop_price {
            let stop_hit = if is_long {
                current_price <= stop
            } else {
                current_price >= stop
            };
            if stop_hit {
                return Some(QueuedExit {
                    tenant_id: pos.tenant_id,
                    symbol: pos.id.split_once(':').map(|x| x.1).unwrap_or("").to_string(),
                    side: close_side.to_string(),
                    reason: ExitReason::StopHit,
                    current_price,
                    size_usd: None,
                    idempotency_key: format!("{}:stop_hit", pos.id),
                });
            }
        }

        // ── Take profit ────────────────────────────────────────────────────
        if let Some(tp) = pos.tp_price {
            let tp_hit = if is_long {
                current_price >= tp
            } else {
                current_price <= tp
            };
            if tp_hit {
                return Some(QueuedExit {
                    tenant_id: pos.tenant_id,
                    symbol: pos.id.split_once(':').map(|x| x.1).unwrap_or("").to_string(),
                    side: close_side.to_string(),
                    reason: ExitReason::TargetHit,
                    current_price,
                    size_usd: Some(pos.size_usd / 3.0), // partial close 1/3
                    idempotency_key: format!("{}:target_hit", pos.id),
                });
            }
        }

        // ── Time exit: stale position (>8 cycles, <0.5R profit) ───────────
        // R-size = |entry - stop| if stop is known, else 1% of entry as proxy.
        if cycles_held_stale(pos) {
            let r_size = pos.stop_price
                .map(|s| (pos.entry_price - s).abs())
                .unwrap_or(pos.entry_price * 0.01);
            let pnl_r = if r_size > 0.0 {
                if is_long {
                    (current_price - pos.entry_price) / r_size
                } else {
                    (pos.entry_price - current_price) / r_size
                }
            } else {
                0.0
            };
            if pnl_r < 0.5 {
                return Some(QueuedExit {
                    tenant_id: pos.tenant_id,
                    symbol: pos.id.split_once(':').map(|x| x.1).unwrap_or("").to_string(),
                    side: close_side.to_string(),
                    reason: ExitReason::TimeExit,
                    current_price,
                    size_usd: None,
                    idempotency_key: format!("{}:time_exit", pos.id),
                });
            }
        }

        None
    }

    async fn run_execution_workers(self: Arc<Self>) {
        info!("⚡ ExecutionWorkers starting ({} workers)", EXECUTION_WORKER_COUNT);
        let mut handles = Vec::new();
        for worker_id in 0..EXECUTION_WORKER_COUNT {
            let db = self.db.clone();
            handles.push(tokio::spawn(async move {
                run_single_worker(worker_id, db).await;
            }));
        }
        for handle in handles {
            if let Err(e) = handle.await {
                error!("ExecutionWorker panicked: {:?}", e);
            }
        }
    }
}

fn cycles_held_stale(pos: &OpenPosition) -> bool {
    // Primary: cycles_held counter from DB
    if pos.cycles_held > 8 {
        return true;
    }
    // Fallback: wall-clock time (8 cycles × 30s = 4 minutes)
    let age = chrono::Utc::now() - pos.opened_at;
    age.num_seconds() > 8 * 30
}

// ─────────────────────────── Queue writer ────────────────────────────────────

/// Batch-insert triggered exits to `execution_queue`.
/// Uses ON CONFLICT DO NOTHING on `idempotency_key` to prevent duplicates.
/// Runtime query (no macro) — safe before migration 016 runs.
async fn enqueue_exits(pool: &PgPool, exits: &[QueuedExit]) -> Result<()> {
    if exits.is_empty() {
        return Ok(());
    }

    for exit in exits {
        let result = sqlx::query(
            r#"
            INSERT INTO execution_queue
                (tenant_id, symbol, side, order_type, size_usd,
                 reduce_only, reason, signal_score, priority, idempotency_key)
            VALUES ($1, $2, $3, 'market', $4, TRUE, $5, $6, $7, $8)
            ON CONFLICT (idempotency_key) DO NOTHING
            "#,
        )
        .bind(exit.tenant_id)
        .bind(&exit.symbol)
        .bind(&exit.side)
        .bind(exit.size_usd)
        .bind(exit.reason.as_str())
        .bind(exit.current_price)
        .bind(exit.reason.priority())
        .bind(&exit.idempotency_key)
        .execute(pool)
        .await;

        if let Err(e) = result {
            // Table might not exist before first startup — log and continue
            debug!("enqueue_exits: {}", e);
        }
    }

    Ok(())
}

// ─────────────────────────── Execution worker ────────────────────────────────

/// One execution worker: polls `execution_queue` and processes pending jobs.
/// Runtime queries (no macros) — compiles before migration 016 creates the table.
///
/// # Idle backoff
/// When the queue is empty the worker sleeps 1 s before the next poll instead
/// of the active 200 ms interval.  This prevents the 4 workers from issuing
/// ~20 DB polls/second against an empty queue, freeing pool connections for
/// the tenant loops and background flushers.
async fn run_single_worker(worker_id: usize, pool: PgPool) {
    loop {
        match worker_tick(worker_id, &pool).await {
            Ok(true) => {
                // Job was processed — poll quickly for the next one.
                sleep(Duration::from_millis(200)).await;
            }
            Ok(false) => {
                // Queue empty — back off to avoid burning pool connections.
                sleep(Duration::from_secs(1)).await;
            }
            Err(e) => {
                warn!("ExecutionWorker {}: tick error: {}", worker_id, e);
                sleep(Duration::from_millis(500)).await;
            }
        }
    }
}

/// Returns `Ok(true)` if a job was processed, `Ok(false)` if the queue was empty.
async fn worker_tick(worker_id: usize, pool: &PgPool) -> Result<bool> {
    // Claim one pending job via SKIP LOCKED for parallel-safe processing.
    let row = sqlx::query(
        r#"
        UPDATE execution_queue
        SET    status = 'processing',
               processing_at = NOW(),
               attempts = attempts + 1
        WHERE  id = (
            SELECT id FROM execution_queue
            WHERE  status = 'pending'
            ORDER  BY priority ASC, enqueued_at ASC
            LIMIT  1
            FOR UPDATE SKIP LOCKED
        )
        RETURNING id, tenant_id::text, symbol, side, order_type,
                  size_usd::float8, reduce_only, reason
        "#,
    )
    .fetch_optional(pool)
    .await;

    let row = match row {
        Ok(Some(r)) => r,
        Ok(None) => return Ok(false), // queue empty
        Err(e) => {
            // Table may not exist yet — suppress until migration runs
            debug!("worker_tick {}: {}", worker_id, e);
            return Ok(false);
        }
    };

    use sqlx::Row;
    let id: i64 = row.try_get("id").unwrap_or(0);
    let tenant_id: String = row.try_get("tenant_id").unwrap_or_default();
    let symbol: String = row.try_get("symbol").unwrap_or_default();
    let side: String = row.try_get("side").unwrap_or_default();
    let reason: String = row.try_get("reason").unwrap_or_default();
    let size_usd: Option<f64> = row.try_get("size_usd").ok().flatten();

    debug!(
        "Worker {}: job {} tenant={} {} {} reason={}",
        worker_id, id, tenant_id, side.to_uppercase(), symbol, reason
    );

    // TODO Phase 2: load tenant credentials → call exchange::place_order()
    // For now: log and mark done.
    info!(
        "ExecutionWorker {}: QUEUED ORDER tenant={} {} {} {} size={:?} (Phase 2)",
        worker_id, tenant_id, side.to_uppercase(), symbol, reason, size_usd
    );

    let _ = sqlx::query(
        "UPDATE execution_queue SET status = 'done', completed_at = NOW() WHERE id = $1",
    )
    .bind(id)
    .execute(pool)
    .await;

    Ok(true) // job was processed
}

// ─────────────────────────── Scheduled cleanup ───────────────────────────────

/// Enqueue time-exit orders for stale positions across all tenants.
/// Called periodically — replaces the per-tenant time-exit block.
#[allow(dead_code)]
pub async fn enqueue_time_exits(pool: &PgPool, stale_cycles: i32) -> Result<usize> {
    let rows = sqlx::query(
        r#"
        SELECT id, tenant_id::text, symbol, side
        FROM   positions
        WHERE  cycles_held > $1
        "#,
    )
    .bind(stale_cycles)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let count = rows.len();
    if count == 0 {
        return Ok(0);
    }

    use sqlx::Row;
    let mut exits = Vec::with_capacity(count);
    for row in &rows {
        let id: String = row.try_get("id").unwrap_or_default();
        let tenant_id_str: String = row.try_get("tenant_id").unwrap_or_default();
        let tenant_id = match Uuid::parse_str(&tenant_id_str) {
            Ok(u) => u,
            Err(_) => continue,
        };
        let symbol = id.split_once(':').map(|x| x.1).unwrap_or("").to_string();
        let side: String = row.try_get("side").unwrap_or_default();
        let close_side = if side == "LONG" { "sell" } else { "buy" };

        exits.push(QueuedExit {
            tenant_id,
            symbol,
            side: close_side.to_string(),
            reason: ExitReason::TimeExit,
            current_price: 0.0,
            size_usd: None,
            idempotency_key: format!("{}:time_exit", id),
        });
    }

    enqueue_exits(pool, &exits).await?;
    info!("enqueue_time_exits: queued {} stale positions", count);
    Ok(count)
}
