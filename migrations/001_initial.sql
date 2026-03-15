-- ══════════════════════════════════════════════════════════════════════════════
-- TradingBots.fun — PostgreSQL schema
-- Migration 001: initial tables
--
-- Design principles:
--   • Tenant isolation: every trading table has tenant_id FK — no cross-tenant
--     data leakage is structurally possible.
--   • JSONB for blobs (signal_contrib): binary, indexed, queryable by AI agents
--     via MCP without deserialising in Rust.
--   • Pre-aggregated aum_snapshots: admin analytics and landing-page TVL graph
--     never need cross-tenant GROUP BY — the trading loop writes one row/cycle.
--   • TIMESTAMPTZ everywhere: store UTC, render in any timezone at query time.
--   • Idempotent: all CREATE TABLE / CREATE INDEX use IF NOT EXISTS.
-- ══════════════════════════════════════════════════════════════════════════════

-- ── Extensions ────────────────────────────────────────────────────────────────
-- pgcrypto gives us gen_random_uuid() as a server-side UUID default.
CREATE EXTENSION IF NOT EXISTS pgcrypto;

-- ── Tenants ───────────────────────────────────────────────────────────────────
-- One row per registered user.  The internal UUID is never shown to the user;
-- wallet_address (0x…) is what users see in the dashboard header.
CREATE TABLE IF NOT EXISTS tenants (
    id               UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    privy_did        TEXT        UNIQUE,                         -- "did:privy:cl…"
    wallet_address   TEXT        UNIQUE,                         -- "0x…", NULL until linked
    display_name     TEXT,                                       -- email or Privy name
    tier             TEXT        NOT NULL DEFAULT 'Free',        -- Free | Pro | Internal
    initial_capital  NUMERIC(18,8) NOT NULL DEFAULT 0,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    terms_accepted   TIMESTAMPTZ,                                -- NULL = not accepted yet
    stripe_customer  TEXT,                                       -- cus_…
    trial_ends_at    TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_tenants_privy  ON tenants (privy_did);
CREATE INDEX IF NOT EXISTS idx_tenants_wallet ON tenants (wallet_address);

COMMENT ON TABLE  tenants                  IS 'One row per registered user account';
COMMENT ON COLUMN tenants.privy_did        IS 'Privy DID used as cross-device identity key';
COMMENT ON COLUMN tenants.wallet_address   IS 'Hyperliquid wallet address — required for live trading';
COMMENT ON COLUMN tenants.initial_capital  IS 'Capital at account creation; used as sparkline baseline';

-- ── Equity snapshots ──────────────────────────────────────────────────────────
-- Written once per trading cycle (every ~30 s) per tenant.
-- Powers the per-tenant sparkline on the dashboard.
-- Pruned to a 7-day rolling window via hourly maintenance query.
CREATE TABLE IF NOT EXISTS equity_snapshots (
    id          BIGSERIAL   PRIMARY KEY,
    tenant_id   UUID        NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    equity      NUMERIC(18,8) NOT NULL
);

-- Composite index: all dashboard queries filter by tenant + recent time.
CREATE INDEX IF NOT EXISTS idx_equity_tenant_time
    ON equity_snapshots (tenant_id, recorded_at DESC);

COMMENT ON TABLE equity_snapshots IS
    'Rolling 7-day equity curve per tenant; pruned hourly by maintenance task';

-- ── Open positions ─────────────────────────────────────────────────────────────
-- Mirrors BotState.positions.  Written on open, updated on DCA/partial close,
-- deleted on full close (a closed_trades row is inserted instead).
CREATE TABLE IF NOT EXISTS positions (
    -- Natural key: one active position per (tenant, symbol) at a time.
    id               TEXT        PRIMARY KEY,          -- "{tenant_id}:{symbol}"
    tenant_id        UUID        NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    symbol           TEXT        NOT NULL,
    side             TEXT        NOT NULL CHECK (side IN ('LONG', 'SHORT')),
    entry_price      NUMERIC(18,8),
    size_usd         NUMERIC(18,8),
    notional_usd     NUMERIC(18,8),
    leverage         NUMERIC(6,2),
    stop_price       NUMERIC(18,8),
    tp_price         NUMERIC(18,8),
    opened_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    dca_count        SMALLINT    NOT NULL DEFAULT 0,
    tranche          SMALLINT    NOT NULL DEFAULT 0,
    cycles_held      INTEGER     NOT NULL DEFAULT 0,
    -- JSONB: queryable by AI agent ("which open positions had rsi_bullish = true?")
    signal_contrib   JSONB
);

CREATE INDEX IF NOT EXISTS idx_positions_tenant ON positions (tenant_id);

COMMENT ON TABLE  positions              IS 'Currently open trading positions — deleted on full close';
COMMENT ON COLUMN positions.signal_contrib IS
    'SignalContribution captured at entry; JSONB for direct AI/MCP queries';

-- ── Closed trades ──────────────────────────────────────────────────────────────
-- Append-only ledger of every completed trade.
-- The signal_contrib JSONB enables learner replay and AI analytics:
--   SELECT avg(r_multiple) WHERE signal_contrib->>'rsi_bullish' = 'true'
CREATE TABLE IF NOT EXISTS closed_trades (
    id               BIGSERIAL   PRIMARY KEY,
    tenant_id        UUID        NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    symbol           TEXT        NOT NULL,
    side             TEXT        NOT NULL CHECK (side IN ('LONG', 'SHORT')),
    entry_price      NUMERIC(18,8),
    exit_price       NUMERIC(18,8),
    size_usd         NUMERIC(18,8),
    pnl_usd          NUMERIC(18,8),
    pnl_pct          NUMERIC(10,4),
    r_multiple       NUMERIC(10,4),
    fees_usd         NUMERIC(18,8) NOT NULL DEFAULT 0,
    opened_at        TIMESTAMPTZ,
    closed_at        TIMESTAMPTZ  NOT NULL DEFAULT now(),
    close_reason     TEXT,        -- HIT_STOP | HIT_TP | TRAILING | MANUAL | CIRCUIT_BREAK
    signal_contrib   JSONB        -- full SignalContribution snapshot for learner replay
);

-- Per-tenant history queries (dashboard, tax export)
CREATE INDEX IF NOT EXISTS idx_trades_tenant_time
    ON closed_trades (tenant_id, closed_at DESC);

-- Cross-tenant analytics (admin dashboard, signal quality studies via MCP)
CREATE INDEX IF NOT EXISTS idx_trades_time
    ON closed_trades (closed_at DESC);

-- JSONB path index — dramatically speeds up AI agent queries on signal fields
-- e.g.: WHERE signal_contrib->>'rsi_bullish' = 'true'
CREATE INDEX IF NOT EXISTS idx_trades_signal_contrib
    ON closed_trades USING GIN (signal_contrib);

COMMENT ON TABLE closed_trades IS
    'Immutable trade ledger; signal_contrib JSONB enables AI-driven signal analytics';

-- ── Fund events ────────────────────────────────────────────────────────────────
-- Deposit/withdrawal/fee events.  High-water mark for performance fee calc.
CREATE TABLE IF NOT EXISTS fund_events (
    id            BIGSERIAL   PRIMARY KEY,
    tenant_id     UUID        NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    recorded_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    event_type    TEXT        NOT NULL
                  CHECK (event_type IN ('DEPOSIT','WITHDRAWAL','PERFORMANCE_FEE','REFERRAL','ADJUSTMENT')),
    amount_usd    NUMERIC(18,8) NOT NULL,
    balance_after NUMERIC(18,8) NOT NULL,
    notes         TEXT
);

CREATE INDEX IF NOT EXISTS idx_fund_tenant_time ON fund_events (tenant_id, recorded_at DESC);
CREATE INDEX IF NOT EXISTS idx_fund_time        ON fund_events (recorded_at DESC);

COMMENT ON TABLE fund_events IS
    'Deposit/withdrawal ledger; used for high-water mark and performance fee calc';

-- ── Per-tenant signal weights ──────────────────────────────────────────────────
-- One row per tenant, upserted after every closed trade by the learner.
-- Starts from the global defaults; diverges as trade history accumulates.
CREATE TABLE IF NOT EXISTS signal_weights (
    tenant_id    UUID        PRIMARY KEY REFERENCES tenants (id) ON DELETE CASCADE,
    weights      JSONB       NOT NULL,   -- SignalWeights struct serialised
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

COMMENT ON TABLE  signal_weights         IS 'Adaptive per-tenant signal weights; upserted by learner on close';
COMMENT ON COLUMN signal_weights.weights IS
    'Full SignalWeights JSON; queryable via AI: are weights converging across tenants?';

-- ── Pre-aggregated AUM snapshots ───────────────────────────────────────────────
-- Written ONCE per trading cycle by the bot — never computed ad-hoc.
-- Admin dashboard and public TVL endpoint query this table only; they never
-- touch equity_snapshots or closed_trades for display purposes.
--
-- This is the core insight: admin analytics must be O(1) per page load,
-- not O(tenants × history_rows).
CREATE TABLE IF NOT EXISTS aum_snapshots (
    id                   BIGSERIAL   PRIMARY KEY,
    recorded_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    total_aum            NUMERIC(18,8) NOT NULL,   -- Σ tenant equity
    deposited_capital    NUMERIC(18,8) NOT NULL,   -- Σ initial_capital (baseline)
    total_pnl            NUMERIC(18,8) NOT NULL,   -- total_aum − deposited_capital
    pnl_pct              NUMERIC(10,4) NOT NULL,   -- pnl / deposited_capital × 100
    active_tenant_count  INTEGER     NOT NULL,     -- tenants with open positions
    total_tenant_count   INTEGER     NOT NULL,     -- all registered tenants
    open_position_count  INTEGER     NOT NULL,     -- Σ open positions across all tenants
    total_trades_today   INTEGER     NOT NULL DEFAULT 0,
    win_rate_today       NUMERIC(6,4)              -- NULL until first trade of day
);

CREATE INDEX IF NOT EXISTS idx_aum_time ON aum_snapshots (recorded_at DESC);

COMMENT ON TABLE aum_snapshots IS
    'Pre-aggregated TVL metrics; one row per trading cycle; drives admin dashboard and landing-page hero graph';

-- ── Maintenance helper: prune old equity snapshots ────────────────────────────
-- Call this from a cron job or the Rust maintenance task.
-- Keeps the last 7 days (604800 seconds).
CREATE OR REPLACE FUNCTION prune_equity_snapshots() RETURNS void
LANGUAGE sql AS $$
    DELETE FROM equity_snapshots
    WHERE  recorded_at < now() - INTERVAL '7 days';
$$;

COMMENT ON FUNCTION prune_equity_snapshots IS
    'Prune equity_snapshots older than 7 days; call hourly from maintenance task';
