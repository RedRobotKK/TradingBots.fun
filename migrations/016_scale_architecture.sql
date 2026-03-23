-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 016: Scalable architecture tables
--
-- Enables 1M+ tenant operation by decoupling three concerns that currently
-- all live inside the per-tenant `run_cycle()` loop:
--
--   1. Signal computation   → symbol_signals   (computed once per symbol)
--   2. Position monitoring  → execution_queue  (event-driven, not polled)
--   3. Tenant activity      → tenant_activity  (tracks last-active for LRU cache)
--
-- Architecture at scale:
--   ┌────────────────────────────────────────────────────────────────┐
--   │  WS Oracle (1 conn) → price_oracle → PositionMonitor          │
--   │  SignalEngine (per-symbol, 30s) → symbol_signals              │
--   │  PositionMonitor (price event → DB query) → execution_queue   │
--   │  ExecutionWorker pool (N workers) → HL API per tenant         │
--   └────────────────────────────────────────────────────────────────┘
--
-- At 1M tenants with 5 avg positions = 5M rows in positions table.
-- A BTC price event: query WHERE symbol='BTC' (~100K rows), evaluate
-- stop/target, enqueue triggered orders. 20 worker pool handles ~500 execs/s.
-- ─────────────────────────────────────────────────────────────────────────────

-- ── 1. symbol_signals ─────────────────────────────────────────────────────────
-- Stores the latest computed signal for every active symbol.
-- The SignalEngine writes here; the ExecutionWorker and new-entry allocator
-- read from here instead of each tenant recomputing independently.
--
-- Replaces: the signal computation block inside run_cycle() × N tenants.
-- Cost: 1 computation per symbol per cycle regardless of tenant count.
CREATE TABLE IF NOT EXISTS symbol_signals (
    symbol          TEXT PRIMARY KEY,

    -- Signal strength and direction (-1.0 … +1.0, positive = long bias)
    signal_score    FLOAT8 NOT NULL DEFAULT 0.0,
    direction       TEXT   NOT NULL DEFAULT 'neutral'    -- 'long' | 'short' | 'neutral'
        CHECK (direction IN ('long', 'short', 'neutral')),
    confidence      FLOAT8 NOT NULL DEFAULT 0.0          -- 0.0 … 1.0

        CHECK (confidence BETWEEN 0.0 AND 1.0),

    -- Raw indicator snapshot (stored as JSONB so we can add/remove indicators
    -- without schema changes — the AI/MCP layer can query this directly)
    indicators      JSONB  NOT NULL DEFAULT '{}',

    -- Derived market context
    atr_pct         FLOAT8,   -- ATR as % of price (volatility proxy)
    spread_bps      FLOAT8,   -- bid-ask spread in basis points
    funding_rate    FLOAT8,   -- current HL funding rate
    volume_24h_usd  FLOAT8,   -- 24h volume from Binance oracle

    -- Lifecycle
    computed_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    cycle_seq       BIGINT  NOT NULL DEFAULT 0   -- monotonic counter, wraps per symbol
);

CREATE INDEX IF NOT EXISTS idx_symbol_signals_score
    ON symbol_signals (confidence DESC, computed_at DESC)
    WHERE direction != 'neutral';

CREATE INDEX IF NOT EXISTS idx_symbol_signals_age
    ON symbol_signals (computed_at);

-- ── 2. execution_queue ────────────────────────────────────────────────────────
-- Decouples "decide to trade" from "execute the trade".
-- The PositionMonitor and SignalEngine write here; ExecutionWorkers drain it.
-- Per-tenant rate limiting is enforced at the worker level, not at signal time.
--
-- This table is the single coordination point for horizontal scaling:
-- multiple bot process instances can all drain the same queue safely via
-- SELECT ... FOR UPDATE SKIP LOCKED.
CREATE TABLE IF NOT EXISTS execution_queue (
    id              BIGSERIAL PRIMARY KEY,
    tenant_id       UUID    NOT NULL,
    symbol          TEXT    NOT NULL,
    side            TEXT    NOT NULL CHECK (side IN ('buy', 'sell')),

    -- Order intent
    order_type      TEXT    NOT NULL DEFAULT 'market'
        CHECK (order_type IN ('market', 'limit', 'stop_market')),
    size_usd        FLOAT8,          -- position size in USD (NULL = use full close)
    limit_price     FLOAT8,          -- only for order_type = 'limit'
    reduce_only     BOOLEAN NOT NULL DEFAULT FALSE,

    -- Why this order was enqueued
    reason          TEXT    NOT NULL  -- 'new_entry' | 'stop_hit' | 'target_hit' | 'trail' | 'time_exit' | 'dca' | 'manual'
        CHECK (reason IN ('new_entry', 'stop_hit', 'target_hit', 'trail', 'time_exit', 'dca', 'manual')),
    signal_score    FLOAT8,           -- signal score at time of enqueue
    stop_price      FLOAT8,           -- computed stop loss price for this order
    target_prices   FLOAT8[],         -- partial-take-profit levels [2R, 4R, trail]

    -- Queue management
    priority        INT     NOT NULL DEFAULT 50,  -- lower = higher priority; exits = 10, entries = 50
    status          TEXT    NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'processing', 'done', 'failed', 'cancelled')),
    attempts        INT     NOT NULL DEFAULT 0,
    last_error      TEXT,
    enqueued_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    processing_at   TIMESTAMPTZ,
    completed_at    TIMESTAMPTZ,

    -- Idempotency: prevent duplicate orders for the same position event
    idempotency_key TEXT UNIQUE   -- e.g. '<tenant_id>:<symbol>:<reason>:<cycle_seq>'
);

CREATE INDEX IF NOT EXISTS idx_execution_queue_pending
    ON execution_queue (priority ASC, enqueued_at ASC)
    WHERE status = 'pending';

CREATE INDEX IF NOT EXISTS idx_execution_queue_tenant
    ON execution_queue (tenant_id, status, enqueued_at DESC);

-- ── 3. tenant_activity ────────────────────────────────────────────────────────
-- Lightweight table tracking each tenant's last-active timestamp and current
-- mode. Used by the LRU hot-cache to decide which tenants to keep in memory
-- vs evict to DB-only access.
--
-- "Active" = has open positions OR generated a queue entry in last 24h.
-- Dormant tenants consume zero compute — they're revived on demand.
CREATE TABLE IF NOT EXISTS tenant_activity (
    tenant_id           UUID PRIMARY KEY,

    -- Cache priority signal
    last_active_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_cycle_at       TIMESTAMPTZ,
    last_order_at       TIMESTAMPTZ,
    open_position_count INT NOT NULL DEFAULT 0,
    pending_queue_count INT NOT NULL DEFAULT 0,

    -- Bot mode
    mode                TEXT NOT NULL DEFAULT 'paper'
        CHECK (mode IN ('paper', 'live', 'paused', 'suspended')),
    capital_usd         FLOAT8,

    -- Process affinity: which bot process instance "owns" this tenant
    -- NULL = unassigned (any worker can pick up)
    -- Set via SELECT ... FOR UPDATE in worker assignment
    assigned_worker     TEXT,
    assigned_at         TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_tenant_activity_hot
    ON tenant_activity (open_position_count DESC, last_active_at DESC)
    WHERE mode != 'paused' AND mode != 'suspended';

-- ── 4. symbol_orderbook_cache ─────────────────────────────────────────────────
-- Stores the latest order book snapshot per symbol.
-- Replaces MarketClient.book_oracle (in-memory, lost on restart).
-- At 400 symbols × ~1KB each = 400KB total — trivially small.
CREATE TABLE IF NOT EXISTS symbol_orderbook_cache (
    symbol          TEXT PRIMARY KEY,
    best_bid        FLOAT8 NOT NULL,
    best_ask        FLOAT8 NOT NULL,
    spread_bps      FLOAT8 NOT NULL,
    bid_depth_usd   FLOAT8,   -- sum of bids within 0.5% of mid
    ask_depth_usd   FLOAT8,   -- sum of asks within 0.5% of mid
    imbalance       FLOAT8,   -- (bid_depth - ask_depth) / (bid_depth + ask_depth)
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ── 5. Maintenance function ───────────────────────────────────────────────────
-- Purge completed/failed execution_queue rows older than 7 days.
-- Called by the existing cron infra (same pattern as purge_old_price_history).
CREATE OR REPLACE FUNCTION purge_old_execution_queue() RETURNS void
LANGUAGE plpgsql AS $$
BEGIN
    DELETE FROM execution_queue
    WHERE status IN ('done', 'failed', 'cancelled')
      AND completed_at < NOW() - INTERVAL '7 days';
END;
$$;
