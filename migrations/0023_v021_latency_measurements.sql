-- v0.2.1 — Per-trade latency measurement table
-- Records all 5 execution timing primitives for each trade:
--   signal → order_signed → order_sent → response_received → fill_confirmed
-- Used for p50/p95/p99 dashboard stats and SLA monitoring.

CREATE TABLE IF NOT EXISTS latency_measurements (
    id                      BIGSERIAL    PRIMARY KEY,
    session_id              TEXT         NOT NULL,
    trade_id                TEXT,                          -- internal correlation ID
    coin                    TEXT         NOT NULL,
    signal_received_at      TIMESTAMPTZ  NOT NULL,
    order_signed_at         TIMESTAMPTZ,
    order_sent_at           TIMESTAMPTZ,
    response_received_at    TIMESTAMPTZ,
    fill_confirmed_at       TIMESTAMPTZ,
    -- Pre-computed derived metrics (ms) for fast aggregation queries
    order_latency_ms        DOUBLE PRECISION
        GENERATED ALWAYS AS (
            EXTRACT(EPOCH FROM (response_received_at - order_sent_at)) * 1000.0
        ) STORED,
    fill_latency_ms         DOUBLE PRECISION
        GENERATED ALWAYS AS (
            EXTRACT(EPOCH FROM (fill_confirmed_at - order_sent_at)) * 1000.0
        ) STORED,
    total_latency_ms        DOUBLE PRECISION
        GENERATED ALWAYS AS (
            EXTRACT(EPOCH FROM (fill_confirmed_at - signal_received_at)) * 1000.0
        ) STORED,
    created_at              TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

-- Primary access pattern: all measurements for a session in time order
CREATE INDEX IF NOT EXISTS idx_latency_session_time
    ON latency_measurements (session_id, created_at DESC);

-- For cross-session percentile queries
CREATE INDEX IF NOT EXISTS idx_latency_total_ms
    ON latency_measurements (total_latency_ms)
    WHERE total_latency_ms IS NOT NULL;

COMMENT ON TABLE latency_measurements IS 'Per-trade execution latency primitives (v0.2.1+). 5 timing checkpoints per order.';
COMMENT ON COLUMN latency_measurements.order_latency_ms IS 'response_received - order_sent (ms). Computed column.';
COMMENT ON COLUMN latency_measurements.fill_latency_ms IS 'fill_confirmed - order_sent (ms). Computed column.';
COMMENT ON COLUMN latency_measurements.total_latency_ms IS 'fill_confirmed - signal_received (ms). Computed column.';
