-- v0.2.1 — Extended session fields for Hyperliquid venue support
-- Adds new columns to the bot_sessions / sessions table (if it exists).
-- All columns are nullable with sensible defaults for backward compatibility.

ALTER TABLE bot_sessions
    ADD COLUMN IF NOT EXISTS venue                TEXT    NOT NULL DEFAULT 'internal',
    ADD COLUMN IF NOT EXISTS leverage_max         INTEGER,
    ADD COLUMN IF NOT EXISTS risk_mode            TEXT,
    ADD COLUMN IF NOT EXISTS symbols_whitelist    JSONB,
    ADD COLUMN IF NOT EXISTS max_drawdown_pct     DOUBLE PRECISION,
    ADD COLUMN IF NOT EXISTS performance_fee_pct  INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS hyperliquid_address  TEXT,
    ADD COLUMN IF NOT EXISTS webhook_url          TEXT,
    ADD COLUMN IF NOT EXISTS paused               BOOLEAN NOT NULL DEFAULT FALSE;

-- Add latency tracking columns
ALTER TABLE bot_sessions
    ADD COLUMN IF NOT EXISTS latency_enabled               BOOLEAN NOT NULL DEFAULT TRUE,
    ADD COLUMN IF NOT EXISTS latency_alert_threshold_ms    INTEGER NOT NULL DEFAULT 800;

COMMENT ON COLUMN bot_sessions.venue IS 'internal | hyperliquid — execution venue for this session';
COMMENT ON COLUMN bot_sessions.leverage_max IS '1–50; NULL = use bot default';
COMMENT ON COLUMN bot_sessions.risk_mode IS 'conservative | balanced | aggressive';
COMMENT ON COLUMN bot_sessions.symbols_whitelist IS 'JSON array of allowed symbols; NULL = all pairs';
COMMENT ON COLUMN bot_sessions.max_drawdown_pct IS 'Auto-pause threshold as percentage; NULL = disabled';
COMMENT ON COLUMN bot_sessions.performance_fee_pct IS 'Performance fee on profits, 0 = disabled';
COMMENT ON COLUMN bot_sessions.hyperliquid_address IS 'Derived per-session HL deposit address (public key only)';
COMMENT ON COLUMN bot_sessions.webhook_url IS 'HTTP POST target for trade events';
COMMENT ON COLUMN bot_sessions.paused IS 'True when session is halted by drawdown guard or manual pause';
