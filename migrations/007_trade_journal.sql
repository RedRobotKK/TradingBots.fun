-- Migration 007: Trade journal
-- Stores operator notes for closed trades.
-- The `trade_index` matches the in-memory index into `bot_state.closed_trades`
-- (0 = oldest trade in the 100-trade ring buffer).
-- Notes survive restarts and are loaded back into memory by the persistence layer.

CREATE TABLE IF NOT EXISTS closed_trade_notes (
    trade_index   BIGINT      PRIMARY KEY,  -- position in the closed_trades ring buffer
    note          TEXT        NOT NULL,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
