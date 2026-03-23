-- Migration 015: Shared price oracle + historical data pipeline
--
-- Provides a single source of truth for all token prices, fed by the
-- PriceFeedService background task (Hyperliquid WebSocket + Binance WebSocket).
--
-- price_oracle          — latest price per symbol (upserted every 5 seconds)
-- price_oracle_history  — rolling 30-day tick history for pattern analysis
--
-- All trading loops read from price_oracle rather than hitting exchange APIs
-- directly, decoupling request volume from wallet count.

-- ── Latest prices (one row per symbol) ────────────────────────────────────────
CREATE TABLE IF NOT EXISTS price_oracle (
    symbol        TEXT             NOT NULL PRIMARY KEY,

    -- Composite mid: HL primary, Binance fallback
    mid           DOUBLE PRECISION NOT NULL,
    bid           DOUBLE PRECISION NOT NULL DEFAULT 0,
    ask           DOUBLE PRECISION NOT NULL DEFAULT 0,

    -- (ask − bid) / mid × 10 000  —  tighter spread = better liquidity
    spread_bps    REAL             NOT NULL DEFAULT 0,

    -- Raw source prices (both stored for cross-exchange divergence detection)
    hl_mid        DOUBLE PRECISION,       -- Hyperliquid native mid
    binance_mid   DOUBLE PRECISION,       -- Binance USDT pair mid

    -- 24-hour volume in USD (from Binance ticker, updated every ~1s)
    volume_24h_usd DOUBLE PRECISION,

    -- Divergence alert: |hl_mid − binance_mid| / hl_mid × 100  (%)
    -- Populated when both sources present; >0.5% may signal arbitrage
    cross_spread_pct REAL,

    source        TEXT             NOT NULL DEFAULT 'hyperliquid'
                                   CHECK (source IN ('hyperliquid','binance','composite')),
    updated_at    TIMESTAMPTZ      NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_price_oracle_updated
    ON price_oracle (updated_at DESC);

COMMENT ON TABLE price_oracle IS
    'Shared in-database price oracle, fed by PriceFeedService WebSocket streams. '
    'All tenant trading loops read from here — rate limit cost is O(symbols), not O(wallets).';

-- ── Rolling tick history (30-day retention) ───────────────────────────────────
CREATE TABLE IF NOT EXISTS price_oracle_history (
    id            BIGSERIAL        PRIMARY KEY,
    symbol        TEXT             NOT NULL,
    mid           DOUBLE PRECISION NOT NULL,
    hl_mid        DOUBLE PRECISION,
    binance_mid   DOUBLE PRECISION,
    spread_bps    REAL,
    volume_24h_usd DOUBLE PRECISION,
    source        TEXT             NOT NULL,
    recorded_at   TIMESTAMPTZ      NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_price_oracle_history_sym_time
    ON price_oracle_history (symbol, recorded_at DESC);

CREATE INDEX IF NOT EXISTS idx_price_oracle_history_time
    ON price_oracle_history (recorded_at DESC);

COMMENT ON TABLE price_oracle_history IS
    'Time-series price log for every symbol, written every 5 seconds by PriceFeedService. '
    'Used for: pattern analysis, backtesting, AI context window, and cross-exchange spread monitoring. '
    'Rows older than 30 days should be purged by a nightly cron (or pg_partman partition rotation).';

-- ── Cleanup function (called nightly by cron or pg_cron) ─────────────────────
CREATE OR REPLACE FUNCTION purge_old_price_history() RETURNS void
    LANGUAGE sql AS
$$
    DELETE FROM price_oracle_history
    WHERE recorded_at < now() - INTERVAL '30 days';
$$;

COMMENT ON FUNCTION purge_old_price_history IS
    'Remove price history older than 30 days. Wire to pg_cron or call from the '
    'nightly maintenance task: SELECT purge_old_price_history();';
