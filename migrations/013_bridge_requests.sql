-- Migration 013: Bridge request persistence
--
-- Provides durable storage for the automated Hyperliquid → Arbitrum
-- withdrawals so the dashboard/agent console can query status even
-- after a service restart and so auditors can verify tenant-level flows.
CREATE TABLE IF NOT EXISTS bridge_requests (
    id            UUID        PRIMARY KEY,
    tenant_id     UUID        NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    amount_usd    NUMERIC(18,8) NOT NULL,
    destination   TEXT        NOT NULL,
    status        TEXT        NOT NULL CHECK (status IN ('Initiated','Pending','Completed','Failed')),
    status_reason TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_bridge_requests_tenant
    ON bridge_requests (tenant_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_bridge_requests_status
    ON bridge_requests (status);

COMMENT ON TABLE bridge_requests IS
    'Tenant-scoped Hyperliquid withdrawal requests managed by the bridge automation';
COMMENT ON COLUMN bridge_requests.destination IS
    'Target Arbitrum wallet, restricted by the trusted destination list';
