-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 024: Seed 5 000 scale-test paper wallets into the tenants table.
--
-- These wallets exercise the full trading algo at scale without any real
-- exchange exposure (tier = 'Pro', live_trading = false in Rust land).
--
-- ID scheme mirrors tenant.rs seed_scale_wallets() so the in-memory and
-- DB representations are always in sync and idempotent:
--
--   i = 1    → 5c000001-0000-0000-0000-000000000001
--   i = 5000 → 5c001388-0000-0000-0000-000000001388
--
-- The 5c00 prefix is a mnemonic for "scale" and lets snapshot_daily()'s
-- Uuid::parse_str() succeed cleanly.  Scale wallets are excluded from the
-- public leaderboard by a WHERE id NOT LIKE '5c00%' filter.
-- ─────────────────────────────────────────────────────────────────────────────

INSERT INTO tenants (id, display_name, tier, initial_capital)
SELECT
    format(
        '5c00%s-0000-0000-0000-%s',
        lpad(to_hex(gs.i), 4, '0'),   -- 4-hex index (first group)
        lpad(to_hex(gs.i), 12, '0')   -- 12-hex index (last group)
    )::uuid,
    format('ScaleWallet-%05s', gs.i),
    'Pro',
    200.0
FROM generate_series(1, 5000) AS gs(i)
ON CONFLICT (id) DO NOTHING;

-- Index to quickly identify and filter scale-test tenants
CREATE INDEX IF NOT EXISTS idx_tenants_scale_test
    ON tenants (id)
    WHERE id::text LIKE '5c00%';

-- Lightweight view: aggregate scale-test performance for monitoring
CREATE OR REPLACE VIEW scale_test_summary AS
SELECT
    COUNT(*)                                          AS wallet_count,
    SUM(initial_capital)                              AS total_capital_seeded,
    ROUND(AVG(initial_capital)::numeric, 2)           AS avg_capital,
    (SELECT COUNT(*) FROM equity_snapshots es
     JOIN tenants t ON t.id = es.tenant_id
     WHERE t.id::text LIKE '5c00%'
       AND es.recorded_at > now() - interval '1 hour') AS snapshots_last_hour
FROM tenants
WHERE id::text LIKE '5c00%';

COMMENT ON VIEW scale_test_summary IS
    'Quick health check for the 5000-wallet scale test — check with: SELECT * FROM scale_test_summary;';
