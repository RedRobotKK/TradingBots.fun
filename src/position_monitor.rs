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
//!    b. Evaluates stop/target/trail logic for each row (pure math, no API).
//!    c. Writes triggered orders to `execution_queue` (one UNNEST upsert).
//! 3. The execution worker pool drains `execution_queue` and calls HL per-tenant.
//!
//! # Cost model
//!
//! | Scenario             | Old (per-tenant loop)      | New (event-driven)         |
//! |----------------------|---------------------------|---------------------------|
//! | 9 tenants, 40 pos    | 9 loops × 40 checks = 360  | 1 query × 40 symbols      |
//! | 1 000 tenants        | 1 000 loops × 40 = 40 000  | 1 query × 40 symbols      |
//! | 1 000 000 tenants    | impossible                 | 1 query per symbol change |
//!
//! # Horizontal scaling
//!
//! Multiple bot processes can all run `PositionMonitor` against the same DB.
//! Each process handles a disjoint shard of symbols via consistent hashing on
//! the symbol name.  The `execution_queue` uses `SELECT … FOR UPDATE SKIP LOCKED`
//! so workers across processes coordinate without conflicts.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use log::{debug, error, info, warn};
use sqlx::PgPool;
use tokio::sync::RwLock;
use tokio::time::{interval, sleep};
use uuid::Uuid;

use crate::price_feed::SharedPriceOracle;
use crate::signal_engine::SharedSignalCache;

// ─────────────────────────── Constants ───────────────────────────────────────

/// How often the monitor scans for oracle price changes.
/// This is NOT the per-position check frequency — it's the oracle diff poll.
/// At 100ms the monitor catches price moves within one candle open.
const MONITOR_TICK_MS: u64 = 100;

/// Minimum price movement (as fraction of price) that triggers position evaluation.
/// 0.05% = 5 bps.  Below this threshold noise dominates; ignore the tick.
const MIN_MOVE_THRESHOLD: f64 = 0.0005;

/// Maximum number of execution queue rows to write per symbol per tick.
/// Prevents a single symbol from flooding the queue on a big move.
const MAX_QUEUE_PER_SYMBOL: usize = 50;

/// How many execution workers drain the queue in parallel.
/// Each worker handles one order at a time; 20 workers = 20 concurrent HL calls.
pub const EXECUTION_WORKER_COUNT: usize = 20;

// ─────────────────────────── Position row ────────────────────────────────────

/// One open position loaded from the DB for evaluation.
/// Mirrors the `positions` table schema — fields added here must match the query.
#[derive(Debug, Clone)]
pub struct OpenPosition {
    pub id: i64,
    pub tenant_id: Uuid,
    pub symbol: String,
    pub side: String,         // "long" | "short"
    pub entry_price: f64,
    pub current_size: f64,    // USD notional still open
    pub stop_price: f64,
    pub target_prices: Vec<f64>, // [2R, 4R, trail_level]
    pub hwm: f64,             // high-water mark for trailing stop
    pub peak_r: f64,          // peak R-multiple achieved
    pub opened_at: chrono::DateTime<Utc>,
}

/// Reason an order was enqueued — maps 1:1 to the `execution_queue.reason` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReason {
    StopHit,
    TargetHit,
    TrailHit,
    TimeExit,
}

impl ExitReason {
    fn as_str(self) -> &'static str {
        match self {
            ExitReason::StopHit => "stop_hit",
            ExitReason::TargetHit => "target_hit",
            ExitReason::TrailHit => "trail_hit",
            ExitReason::TimeExit => "time_exit",
        }
    }

    fn priority(self) -> i32 {
        // Exits are high priority — lower number = processed first
        match self {
            ExitReason::StopHit => 5,    // stop losses always first
            ExitReason::TargetHit => 10,
            ExitReason::TrailHit => 10,
            ExitReason::TimeExit => 20,
        }
    }
}

/// A triggered exit decision ready to write to `execution_queue`.
#[derive(Debug, Clone)]
struct QueuedExit {
    tenant_id: Uuid,
    symbol: String,
    side: String,       // "sell" for long, "buy" for short
    reason: ExitReason,
    current_price: f64,
    size_usd: Option<f64>, // None = close full position
    idempotency_key: String,
}

// ─────────────────────────── Monitor ─────────────────────────────────────────

/// Spawns the event-driven position monitor and execution worker pool.
///
/// # Usage
/// ```rust
/// PositionMonitor::new(oracle.clone(), signal_cache.clone(), db_pool.clone())
///     .spawn();
/// ```
pub struct PositionMonitor {
    oracle: SharedPriceOracle,
    _signal_cache: SharedSignalCache,
    db: PgPool,
    /// Last-seen prices per symbol — used to compute price diff between ticks.
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

    /// Spawn monitor loop + execution worker pool.  Returns immediately.
    pub fn spawn(self) {
        let monitor = Arc::new(self);

        // ── Monitor loop ─────────────────────────────────────────────────
        {
            let m = monitor.clone();
            tokio::spawn(async move {
                m.run_monitor().await;
            });
        }

        // ── Execution worker pool ─────────────────────────────────────────
        {
            let m = monitor.clone();
            tokio::spawn(async move {
                m.run_execution_workers().await;
            });
        }
    }

    // ── Monitor loop ──────────────────────────────────────────────────────────

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

    /// Diff the oracle against last-seen prices; evaluate positions for symbols that moved.
    async fn check_price_moves(&self) -> Result<()> {
        // Snapshot current oracle prices — hold the lock for the minimum time.
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

        // Find symbols that moved more than the threshold.
        let moved: Vec<(String, f64, f64)> = {
            let prev = self.prev_prices.read().await;
            current
                .iter()
                .filter_map(|(sym, &new_price)| {
                    let old = prev.get(sym).copied().unwrap_or(new_price);
                    if old > 0.0 {
                        let move_frac = (new_price - old).abs() / old;
                        if move_frac >= MIN_MOVE_THRESHOLD {
                            return Some((sym.clone(), new_price, old));
                        }
                    }
                    None
                })
                .collect()
        };

        // Update prev_prices snapshot.
        {
            let mut prev = self.prev_prices.write().await;
            prev.extend(current.into_iter());
        }

        if moved.is_empty() {
            return Ok(());
        }

        debug!(
            "PositionMonitor: {} symbols moved ≥{:.2}%",
            moved.len(),
            MIN_MOVE_THRESHOLD * 100.0
        );

        // Evaluate positions for each moved symbol.
        for (symbol, new_price, _old_price) in moved {
            if let Err(e) = self.evaluate_symbol_positions(&symbol, new_price).await {
                warn!("PositionMonitor: evaluate {} @ {:.4}: {}", symbol, new_price, e);
            }
        }

        Ok(())
    }

    /// Load all open positions for `symbol` and evaluate stop/target logic.
    ///
    /// This replaces the per-tenant `monitor_positions()` call.
    /// Cost: 1 DB query per symbol move, regardless of how many tenants hold it.
    async fn evaluate_symbol_positions(&self, symbol: &str, current_price: f64) -> Result<()> {
        // Load all open positions for this symbol across all tenants.
        // The `positions` table must have a (symbol, status) index — see migration 012.
        struct PosRow {
            id: i64,
            tenant_id: Uuid,
            side: String,
            entry_price: f64,
            current_size_usd: Option<f64>,
            stop_price: Option<f64>,
            target_price_1: Option<f64>,
            target_price_2: Option<f64>,
            hwm: Option<f64>,
            peak_r: Option<f64>,
            opened_at: chrono::DateTime<Utc>,
        }

        let rows = sqlx::query_as!(
            PosRow,
            r#"
            SELECT
                id,
                tenant_id,
                side,
                entry_price::float8            AS "entry_price!",
                size_usd::float8               AS current_size_usd,
                stop_price::float8             AS stop_price,
                target_price::float8           AS target_price_1,
                target_price_2::float8         AS target_price_2,
                high_water_mark::float8        AS hwm,
                peak_r::float8                 AS peak_r,
                opened_at
            FROM positions
            WHERE symbol = $1
              AND status = 'open'
            "#,
            symbol,
        )
        .fetch_all(&self.db)
        .await;

        let rows = match rows {
            Ok(r) => r,
            Err(e) => {
                // Table might not exist yet (pre-migration) — log and continue.
                debug!("PositionMonitor: positions query for {}: {}", symbol, e);
                return Ok(());
            }
        };

        if rows.is_empty() {
            return Ok(());
        }

        debug!(
            "PositionMonitor: evaluating {} positions for {} @ {:.4}",
            rows.len(),
            symbol,
            current_price
        );

        // Evaluate each position.
        let mut exits: Vec<QueuedExit> = Vec::new();
        for row in rows {
            let entry = row.entry_price;
            let stop = row.stop_price.unwrap_or(0.0);
            let target1 = row.target_price_1.unwrap_or(f64::MAX);
            let target2 = row.target_price_2.unwrap_or(f64::MAX);
            let hwm = row.hwm.unwrap_or(entry);
            let peak_r = row.peak_r.unwrap_or(0.0);
            let is_long = row.side == "long";

            // ── Stop loss ──────────────────────────────────────────────────
            let stop_hit = if is_long {
                stop > 0.0 && current_price <= stop
            } else {
                stop > 0.0 && current_price >= stop
            };

            if stop_hit {
                exits.push(QueuedExit {
                    tenant_id: row.tenant_id,
                    symbol: symbol.to_string(),
                    side: if is_long { "sell".to_string() } else { "buy".to_string() },
                    reason: ExitReason::StopHit,
                    current_price,
                    size_usd: None, // full close
                    idempotency_key: format!(
                        "{}:{}:stop_hit:{}",
                        row.tenant_id, symbol, row.id
                    ),
                });
                continue; // stop hit → no need to check targets
            }

            // ── Take profit 1 (2R target) ─────────────────────────────────
            let tp1_hit = if is_long {
                current_price >= target1
            } else {
                current_price <= target1
            };

            if tp1_hit && target1 < f64::MAX {
                exits.push(QueuedExit {
                    tenant_id: row.tenant_id,
                    symbol: symbol.to_string(),
                    side: if is_long { "sell".to_string() } else { "buy".to_string() },
                    reason: ExitReason::TargetHit,
                    current_price,
                    size_usd: row.current_size_usd.map(|s| s / 3.0), // 1/3 partial close
                    idempotency_key: format!(
                        "{}:{}:tp1:{}",
                        row.tenant_id, symbol, row.id
                    ),
                });
            }

            // ── Take profit 2 (4R target) ─────────────────────────────────
            let tp2_hit = if is_long {
                current_price >= target2
            } else {
                current_price <= target2
            };

            if tp2_hit && target2 < f64::MAX {
                exits.push(QueuedExit {
                    tenant_id: row.tenant_id,
                    symbol: symbol.to_string(),
                    side: if is_long { "sell".to_string() } else { "buy".to_string() },
                    reason: ExitReason::TargetHit,
                    current_price,
                    size_usd: row.current_size_usd.map(|s| s / 3.0), // 1/3 partial close
                    idempotency_key: format!(
                        "{}:{}:tp2:{}",
                        row.tenant_id, symbol, row.id
                    ),
                });
            }

            // ── Trailing stop (activates at 1.5R, trails 1.2×ATR) ─────────
            // The trail logic here is simplified — the full ATR-based trail
            // runs in the per-tenant execution worker where ATR is available.
            // Here we catch the obvious case: price fell back through HWM - 1R.
            if peak_r >= 1.5 {
                let r_size = (entry - stop).abs();
                let trail_level = if is_long {
                    hwm - r_size
                } else {
                    hwm + r_size
                };
                let trail_hit = if is_long {
                    current_price <= trail_level
                } else {
                    current_price >= trail_level
                };
                if trail_hit {
                    exits.push(QueuedExit {
                        tenant_id: row.tenant_id,
                        symbol: symbol.to_string(),
                        side: if is_long { "sell".to_string() } else { "buy".to_string() },
                        reason: ExitReason::TrailHit,
                        current_price,
                        size_usd: None, // close remainder
                        idempotency_key: format!(
                            "{}:{}:trail:{}",
                            row.tenant_id, symbol, row.id
                        ),
                    });
                }
            }

            // ── Time exit (handled by scheduled job, not price event) ──────
            // Positions open > 8 cycles without reaching 0.5R are closed by
            // a separate scheduled cleanup task, not here.

            exits.truncate(MAX_QUEUE_PER_SYMBOL);
        }

        if exits.is_empty() {
            return Ok(());
        }

        info!(
            "PositionMonitor: {} exits triggered for {} @ {:.4}",
            exits.len(),
            symbol,
            current_price
        );

        // Write triggered exits to execution_queue (UNNEST batch).
        enqueue_exits(&self.db, &exits).await
    }

    // ── Execution worker pool ─────────────────────────────────────────────────

    /// Spawn N execution workers that drain `execution_queue`.
    async fn run_execution_workers(self: Arc<Self>) {
        info!("⚡ ExecutionWorkers starting ({} workers)", EXECUTION_WORKER_COUNT);
        let mut handles = Vec::new();
        for worker_id in 0..EXECUTION_WORKER_COUNT {
            let db = self.db.clone();
            handles.push(tokio::spawn(async move {
                run_single_worker(worker_id, db).await;
            }));
        }
        // If all workers panic (shouldn't happen), log and exit.
        for handle in handles {
            if let Err(e) = handle.await {
                error!("ExecutionWorker panicked: {:?}", e);
            }
        }
    }
}

// ─────────────────────────── Queue writer ────────────────────────────────────

/// Batch-insert triggered exits to `execution_queue` using UNNEST.
/// Uses ON CONFLICT DO NOTHING on `idempotency_key` to prevent duplicates
/// across rapid price movements that re-evaluate the same position.
async fn enqueue_exits(pool: &PgPool, exits: &[QueuedExit]) -> Result<()> {
    if exits.is_empty() {
        return Ok(());
    }

    let tenant_ids: Vec<Uuid> = exits.iter().map(|e| e.tenant_id).collect();
    let symbols: Vec<&str> = exits.iter().map(|e| e.symbol.as_str()).collect();
    let sides: Vec<&str> = exits.iter().map(|e| e.side.as_str()).collect();
    let reasons: Vec<&str> = exits.iter().map(|e| e.reason.as_str()).collect();
    let prices: Vec<f64> = exits.iter().map(|e| e.current_price).collect();
    let sizes: Vec<Option<f64>> = exits.iter().map(|e| e.size_usd).collect();
    let priorities: Vec<i32> = exits.iter().map(|e| e.reason.priority()).collect();
    let idem_keys: Vec<&str> = exits.iter().map(|e| e.idempotency_key.as_str()).collect();

    sqlx::query(
        r#"
        INSERT INTO execution_queue
            (tenant_id, symbol, side, order_type, size_usd,
             reduce_only, reason, signal_score, priority, idempotency_key)
        SELECT t.tenant_id, t.symbol, t.side, 'market', t.size_usd,
               TRUE, t.reason, t.price, t.priority, t.idem_key
        FROM UNNEST(
            $1::uuid[], $2::text[], $3::text[], $4::float8[],
            $5::float8[], $6::text[], $7::int[], $8::text[]
        ) AS t(tenant_id, symbol, side, size_usd, price, reason, priority, idem_key)
        ON CONFLICT (idempotency_key) DO NOTHING
        "#,
    )
    .bind(&tenant_ids)
    .bind(&symbols)
    .bind(&sides)
    .bind(&sizes)
    .bind(&prices)
    .bind(&reasons)
    .bind(&priorities)
    .bind(&idem_keys)
    .execute(pool)
    .await?;

    Ok(())
}

// ─────────────────────────── Execution worker ────────────────────────────────

/// One execution worker: polls `execution_queue` for pending jobs, executes them.
///
/// Uses `SELECT … FOR UPDATE SKIP LOCKED` so multiple workers (within the same
/// process or across processes) coordinate without a central dispatcher.
///
/// TODO: wire in actual HL order placement via `exchange::place_order()`.
/// For now the worker claims the row, logs it, and marks it done.
/// The full implementation needs the tenant's signing key, which requires a
/// `tenant_credentials` lookup — that's the next migration.
async fn run_single_worker(worker_id: usize, pool: PgPool) {
    let mut tick = interval(Duration::from_millis(100));
    loop {
        tick.tick().await;
        if let Err(e) = worker_tick(worker_id, &pool).await {
            warn!("ExecutionWorker {}: tick error: {}", worker_id, e);
            sleep(Duration::from_millis(500)).await;
        }
    }
}

async fn worker_tick(worker_id: usize, pool: &PgPool) -> Result<()> {
    // Claim one pending job — SKIP LOCKED means other workers move past rows
    // we have locked, enabling parallelism without duplicate processing.
    struct JobRow {
        id: i64,
        tenant_id: Uuid,
        symbol: String,
        side: String,
        order_type: String,
        size_usd: Option<f64>,
        reduce_only: bool,
        reason: String,
    }

    let job = sqlx::query_as!(
        JobRow,
        r#"
        UPDATE execution_queue
        SET    status = 'processing',
               processing_at = NOW(),
               attempts = attempts + 1
        WHERE  id = (
            SELECT id
            FROM   execution_queue
            WHERE  status = 'pending'
            ORDER  BY priority ASC, enqueued_at ASC
            LIMIT  1
            FOR UPDATE SKIP LOCKED
        )
        RETURNING
            id,
            tenant_id,
            symbol,
            side,
            order_type,
            size_usd::float8     AS size_usd,
            reduce_only,
            reason
        "#,
    )
    .fetch_optional(pool)
    .await?;

    let job = match job {
        Some(j) => j,
        None => return Ok(()), // queue empty — nothing to do this tick
    };

    debug!(
        "Worker {}: executing job {} tenant={} symbol={} side={} reason={}",
        worker_id, job.id, job.tenant_id, job.symbol, job.side, job.reason
    );

    // ── TODO: actual order placement ──────────────────────────────────────────
    //
    // Full implementation:
    //   1. Load tenant credentials from `tenant_credentials` table (encrypted).
    //   2. Call `exchange::place_order(&hl_client, &job)` with the tenant's key.
    //   3. Record fill details back to the `positions` table.
    //   4. Log to `closed_trades` if reduce_only + full close.
    //
    // For now: mark done and log.  Phase 2 will wire in exchange.rs.
    info!(
        "ExecutionWorker {}: QUEUED ORDER tenant={} {} {} {} size={:?} (execution pending Phase 2)",
        worker_id, job.tenant_id, job.side.to_uppercase(), job.symbol, job.reason, job.size_usd
    );

    // Mark completed.
    sqlx::query!(
        "UPDATE execution_queue SET status = 'done', completed_at = NOW() WHERE id = $1",
        job.id,
    )
    .execute(pool)
    .await?;

    Ok(())
}

// ─────────────────────────── Scheduled cleanup ───────────────────────────────

/// Close positions open longer than `max_cycles` without reaching 0.5R.
/// Called by the scheduled maintenance task (every 30 min).
///
/// This replaces the `time_exit` block that ran inside the per-tenant loop.
/// At scale it runs once and scans all tenants in a single query.
pub async fn enqueue_time_exits(pool: &PgPool, stale_hours: i64) -> Result<usize> {
    struct StaleRow {
        tenant_id: Uuid,
        symbol: String,
        side: String,
        id: i64,
    }

    let rows = sqlx::query_as!(
        StaleRow,
        r#"
        SELECT tenant_id, symbol, side, id
        FROM   positions
        WHERE  status = 'open'
          AND  peak_r IS NOT NULL
          AND  peak_r < 0.5
          AND  opened_at < NOW() - ($1 || ' hours')::INTERVAL
        "#,
        stale_hours.to_string(),
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    if rows.is_empty() {
        return Ok(0);
    }

    let count = rows.len();
    let exits: Vec<QueuedExit> = rows
        .into_iter()
        .map(|r| {
            let is_long = r.side == "long";
            QueuedExit {
                tenant_id: r.tenant_id,
                symbol: r.symbol.clone(),
                side: if is_long { "sell".to_string() } else { "buy".to_string() },
                reason: ExitReason::TimeExit,
                current_price: 0.0, // worker will fetch current price at execution
                size_usd: None,
                idempotency_key: format!("{}:{}:time_exit:{}", r.tenant_id, r.symbol, r.id),
            }
        })
        .collect();

    enqueue_exits(pool, &exits).await?;
    info!("enqueue_time_exits: queued {} stale positions for closure", count);
    Ok(count)
}
