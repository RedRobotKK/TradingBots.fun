-- Migration 0018: Create bot_sessions table
-- This table stores API sessions created via the /session endpoint.
-- Each row represents one active or historic session with its own
-- risk controls, venue, and paper-trading balance.

CREATE TABLE IF NOT EXISTS bot_sessions (
    id              TEXT        PRIMARY KEY,
    token           TEXT        NOT NULL,
    tx_hash         TEXT        NOT NULL DEFAULT '',
    plan            TEXT        NOT NULL DEFAULT 'standard_30d',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at      TIMESTAMPTZ NOT NULL DEFAULT (NOW() + INTERVAL '30 days'),
    venue           TEXT        NOT NULL DEFAULT 'internal',
    leverage_max            INTEGER,
    risk_mode               TEXT,
    symbols_whitelist       JSONB,
    max_drawdown_pct        DOUBLE PRECISION,
    performance_fee_pct     INTEGER         NOT NULL DEFAULT 0,
    hyperliquid_address     TEXT,
    webhook_url             TEXT,
    paused                  BOOLEAN         NOT NULL DEFAULT FALSE,
    latency_enabled             BOOLEAN NOT NULL DEFAULT TRUE,
    latency_alert_threshold_ms  INTEGER NOT NULL DEFAULT 800,
    name            TEXT,
    balance_usd     DOUBLE PRECISION    NOT NULL DEFAULT 200.0,
    session_pnl     DOUBLE PRECISION    NOT NULL DEFAULT 0.0,
    tenant_id       TEXT
);

CREATE INDEX IF NOT EXISTS idx_bot_sessions_tenant_id  ON bot_sessions (tenant_id);
CREATE INDEX IF NOT EXISTS idx_bot_sessions_expires_at ON bot_sessions (expires_at);
