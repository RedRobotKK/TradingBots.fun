-- Migration 011: Collective intelligence — cross-user learning
--
-- Two tables power the shared knowledge layer:
--
--   trade_outcomes — every closed trade from every tenant, with signal alignment
--       encoded as +1 (signal agreed with trade direction), -1 (opposed), 0 (absent).
--       Used nightly to recalculate collective signal weights.
--
--   hot_positions — currently open positions across all tenants.
--       Upserted on open, deleted on close.  Powers the per-symbol "crowd signal"
--       that the trading loop reads before entering a new position.

-- ── trade_outcomes ────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS trade_outcomes (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID,                                   -- NULL = single-op mode
    symbol          TEXT        NOT NULL,
    side            TEXT        NOT NULL,                   -- 'LONG' | 'SHORT'
    entry_price     DOUBLE PRECISION NOT NULL,
    exit_price      DOUBLE PRECISION NOT NULL,
    pnl_pct         DOUBLE PRECISION NOT NULL,              -- % gain/loss on margin
    r_multiple      DOUBLE PRECISION,                       -- realised R (pnl / initial risk)
    hold_cycles     INTEGER,                                -- number of 30s cycles held
    -- Signal alignment at entry: +1 agreed with direction, -1 opposed, 0 absent
    sig_rsi         SMALLINT,
    sig_bollinger   SMALLINT,
    sig_macd        SMALLINT,
    sig_ema_cross   SMALLINT,
    sig_order_flow  SMALLINT,
    sig_z_score     SMALLINT,
    sig_volume      SMALLINT,
    sig_sentiment   SMALLINT,
    sig_funding     SMALLINT,
    sig_trend       SMALLINT,
    sig_candle      SMALLINT,
    sig_chart       SMALLINT,
    regime          TEXT,                                   -- 'bull' | 'bear' | 'neutral' | 'ranging'
    outcome         TEXT        NOT NULL,                   -- 'win' | 'loss' | 'breakeven'
    closed_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_to_symbol    ON trade_outcomes(symbol);
CREATE INDEX IF NOT EXISTS idx_to_closed_at ON trade_outcomes(closed_at DESC);
CREATE INDEX IF NOT EXISTS idx_to_outcome   ON trade_outcomes(outcome);
CREATE INDEX IF NOT EXISTS idx_to_tenant    ON trade_outcomes(tenant_id);

-- ── hot_positions ─────────────────────────────────────────────────────────────
-- Live snapshot of every open position across all tenants.
-- PRIMARY KEY (tenant_id, symbol) guarantees one row per tenant-symbol pair.
CREATE TABLE IF NOT EXISTS hot_positions (
    tenant_id           UUID             NOT NULL,
    symbol              TEXT             NOT NULL,
    side                TEXT             NOT NULL,    -- 'LONG' | 'SHORT'
    entry_price         DOUBLE PRECISION NOT NULL,
    size_usd            DOUBLE PRECISION NOT NULL DEFAULT 0,
    unrealised_pnl_pct  DOUBLE PRECISION NOT NULL DEFAULT 0,
    opened_at           TIMESTAMPTZ      NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, symbol)
);

CREATE INDEX IF NOT EXISTS idx_hp_symbol ON hot_positions(symbol);
