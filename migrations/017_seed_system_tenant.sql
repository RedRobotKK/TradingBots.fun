-- Seed the single-operator system tenant used in non-multi-tenant mode.
-- tenant_id 00000000-0000-0000-0000-000000000001 is hardcoded throughout the
-- codebase (main.rs single_op_tenant(), hl_wallet.rs, web_dashboard.rs).
-- Without this row, equity_snapshot FK violations fire every ~30 seconds.
INSERT INTO tenants (id, display_name, tier)
VALUES (
    '00000000-0000-0000-0000-000000000001',
    'System Operator',
    'Internal'
)
ON CONFLICT (id) DO NOTHING;
