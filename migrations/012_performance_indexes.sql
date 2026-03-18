-- Migration 012: Performance indexes for high-traffic query paths
--
-- Covers:
--   • collective learning tables (hot_positions, trade_outcomes)
--   • closed_trades per-tenant lookups used by snapshot_daily
--   • leaderboard_snapshots ordered scans
--   • aum_snapshots (fix ASC→DESC for most-recent-first reads)

-- ─── hot_positions ────────────────────────────────────────────────────────────
-- Used by get_crowd_signal() to find all open positions on a given symbol+side
CREATE INDEX IF NOT EXISTS idx_hp_symbol_side
    ON hot_positions (symbol, side);

-- ─── trade_outcomes ───────────────────────────────────────────────────────────
-- Used by recalculate_collective_weights() which scans the last 90 days
-- and aggregates by outcome; also used by cleanup tasks
CREATE INDEX IF NOT EXISTS idx_to_closed_at_outcome
    ON trade_outcomes (closed_at DESC, outcome);

-- ─── closed_trades ────────────────────────────────────────────────────────────
-- Used by snapshot_daily batch query: WHERE tenant_id = ANY($) AND closed_at >= campaign_start
-- Also used by the per-symbol trade history lookups in the dashboard
CREATE INDEX IF NOT EXISTS idx_trades_tenant_closed
    ON closed_trades (tenant_id, closed_at DESC);

-- Composite covering index for the dashboard symbol filter
CREATE INDEX IF NOT EXISTS idx_trades_tenant_symbol_closed
    ON closed_trades (tenant_id, symbol, closed_at DESC);

-- ─── leaderboard_snapshots ────────────────────────────────────────────────────
-- Used by snapshot_daily DISTINCT ON (tenant_id) ORDER BY snapshot_date ASC
-- and by leaderboard_live view aggregations
CREATE INDEX IF NOT EXISTS idx_lb_snap_tenant_campaign_date
    ON leaderboard_snapshots (tenant_id, campaign_id, snapshot_date ASC);

-- ─── aum_snapshots ────────────────────────────────────────────────────────────
-- Most queries read the most-recent snapshot first; ensure DESC index exists
CREATE INDEX IF NOT EXISTS idx_aum_snap_tenant_ts_desc
    ON aum_snapshots (tenant_id, recorded_at DESC);
