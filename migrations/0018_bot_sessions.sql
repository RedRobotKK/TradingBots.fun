-- Migration 0018: Create bot_sessions table
-- This table stores API sessions created via the /session endpoint.
-- Each row represents one active or historic session with its own
-- risk controls, venue, and paper-trading balance.

CREATE TABLE IF NOT EXISTS bot_sessions (
    -- Primary key — random UUID string generated at session creation
    id              TEXT        PRIMARY KEY,

    -- Bearer token returned to the caller (hashed or raw depending on impl)
    token           TEXT        NOT NULL,

    -- Payment transaction hash that funded this session (x402 or internal)
    tx_hash         TEXT        NOT NULL DEFAULT '',

    -- Session plan/tier label (e.g. "burst_30d", "standard_30d")
    plan            TEXT        NOT NULL DEFAULT 'standard_30d',

    -- ISO-8601 timestamps
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at      TIMESTAMPTZ NOT NULL DEFAULT (NOW() + INTERVAL '30 days'),

    -- Execution venue: 'internal' (paper/sim) | 'hyperliquid' (live)
    venue           TEXT        NOT NULL DEFAULT 'internal',

    -- Risk controls (all nullable = use bot defaults)
    leverage_max            INTEGER,
    risk_mode               TEXT,
    symbols_whitelist       JSONB,
    max_drawdown_pct        DOUBLE PRECISION,
    performance_fee_pct     INTEGER         NOT NULL DEFAULT 0,
    hyperliquid_address     TEXT,
    webhook_url             TEXT,
    paused                  BOOLEAN         NOT NULL DEFAULT FALSE,

    -- Latency tracking
    latency_enabled             BOOLEAN NOT NULL DEFAULT TRUE,
    latency_alert_threshold_ms  INTEGER NOT NULL DEFAULT 800,

    -- Identity + paper capital
    name            TEXT,
    balance_usd     DOUBLE PRECISION    NOT NULL DEFAULT 200.0,
    session_pnl     DOUBLE PRECISION    NOT NULL DEFAULT 0.0,

    -- Optional: link to the tenant that owns this session
    tenant_id       TEXT
);

CREATE INDEX IF NOT EXISTS idx_bot_sessions_tenant_id  ON bot_sessions (tenant_id);
CREATE INDEX IF NOT EXISTS idx_bot_sessions_expires_at ON bot_sessions (expires_at);
