-- v0.2.1 — On-chain trade log table for Hyperliquid sessions
-- Every order placed through a venue="hyperliquid" session is stored here
-- for transparency, tax records, and performance attribution.

CREATE TABLE IF NOT EXISTS hyperliquid_trade_logs (
    id              BIGSERIAL    PRIMARY KEY,
    session_id      TEXT         NOT NULL,
    coin            TEXT         NOT NULL,
    is_buy          BOOLEAN      NOT NULL,
    size_usd        DOUBLE PRECISION NOT NULL,
    limit_px        DOUBLE PRECISION,          -- NULL = market order
    leverage        INTEGER      NOT NULL DEFAULT 1,
    tx_ref          TEXT         NOT NULL,     -- HL response signature / order ID
    status          TEXT         NOT NULL,     -- 'filled' | 'rejected' | 'error'
    pnl_delta       DOUBLE PRECISION,          -- realised P&L at close (NULL for opens)
    raw_response    JSONB,                     -- full HL API response for audit trail
    created_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

-- Fast lookups by session
CREATE INDEX IF NOT EXISTS idx_hl_trade_logs_session
    ON hyperliquid_trade_logs (session_id, created_at DESC);

-- Fast lookups by coin for attribution
CREATE INDEX IF NOT EXISTS idx_hl_trade_logs_coin
    ON hyperliquid_trade_logs (coin, created_at DESC);

COMMENT ON TABLE hyperliquid_trade_logs IS 'On-chain order records for Hyperliquid venue sessions (v0.2.1+)';
COMMENT ON COLUMN hyperliquid_trade_logs.tx_ref IS 'Hyperliquid order signature or fill ID — links to explorer';
COMMENT ON COLUMN hyperliquid_trade_logs.raw_response IS 'Full HL API JSON response stored for audit and debugging';
