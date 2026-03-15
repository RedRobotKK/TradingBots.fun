-- ══════════════════════════════════════════════════════════════════════════════
-- RedRobot HedgeBot — Migration 002: conversion funnel + ad impression tracking
--
-- Purpose:
--   Track every meaningful step from anonymous landing-page visit → trial
--   signup → Pro upgrade, plus ad impressions/clicks for trial-user monetisation.
--
-- Design:
--   • tenant_id is NULLABLE — pre-auth events (PAGE_VIEW, LOGIN_CLICK) fire
--     before a user exists in the tenants table.
--   • anon_id (cookie/localStorage uuid4) stitches anonymous and post-auth
--     events into a single journey for each browser session.
--   • event_type is an open TEXT with a CHECK constraint rather than an enum
--     so new steps can be added without a migration ALTER TYPE.
--   • properties JSONB holds arbitrary per-event context (referrer, UTM params,
--     ad network, CPM bid, etc.) without requiring schema changes.
--   • Indexed for the three most common query patterns:
--       1. Full funnel query  (group by event_type, count, conversion rate)
--       2. Per-tenant journey (select * where tenant_id = ? order by occurred_at)
--       3. Ad revenue roll-up (where event_type IN ('AD_IMPRESSION','AD_CLICK'))
-- ══════════════════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS funnel_events (
    id            BIGSERIAL    PRIMARY KEY,
    occurred_at   TIMESTAMPTZ  NOT NULL DEFAULT now(),

    -- Anonymous visitor ID (uuid4 from cookie / localStorage).
    -- Set on every page load; used to stitch the pre-auth journey to the
    -- post-auth tenant row once Privy login completes.
    anon_id       TEXT         NOT NULL,

    -- NULL for events that fire before the user authenticates.
    tenant_id     UUID         REFERENCES tenants (id) ON DELETE SET NULL,

    -- Funnel step identifier.
    -- Allowed values:
    --   PAGE_VIEW          — landing page rendered (server-side)
    --   LOGIN_CLICK        — "Connect Wallet" button clicked
    --   AUTH_SUCCESS       — Privy auth completed successfully
    --   TRIAL_START        — new tenant registered (14-day trial begins)
    --   TERMS_ACCEPTED     — onboarding T&C wall cleared
    --   WALLET_LINKED      — HL wallet address saved
    --   FIRST_POSITION     — tenant opened their first simulated position
    --   UPGRADE_CLICK      — "Upgrade to Pro" button clicked
    --   CHECKOUT_STARTED   — Stripe checkout session created
    --   UPGRADED           — Stripe payment succeeded (webhook)
    --   TRIAL_EXPIRED      — trial ended without upgrade (batch job)
    --   CHURNED            — Pro subscription cancelled (webhook)
    --   AD_IMPRESSION      — ad rendered in the UI
    --   AD_CLICK           — ad clicked
    event_type    TEXT         NOT NULL CHECK (event_type IN (
        'PAGE_VIEW', 'LOGIN_CLICK', 'AUTH_SUCCESS',
        'TRIAL_START', 'TERMS_ACCEPTED', 'WALLET_LINKED', 'FIRST_POSITION',
        'UPGRADE_CLICK', 'CHECKOUT_STARTED', 'UPGRADED',
        'TRIAL_EXPIRED', 'CHURNED',
        'AD_IMPRESSION', 'AD_CLICK'
    )),

    -- Arbitrary context: UTM params, referrer, ad network, CPM, page path, etc.
    -- Example for PAGE_VIEW:
    --   {"path": "/", "utm_source": "twitter", "utm_campaign": "launch", "referrer": "t.co"}
    -- Example for AD_IMPRESSION:
    --   {"network": "coinzilla", "ad_unit": "banner_300x250", "cpm_usd": 1.20}
    properties    JSONB
);

-- ── Indexes ───────────────────────────────────────────────────────────────────

-- Full funnel conversion report: SELECT event_type, count(*) GROUP BY event_type
CREATE INDEX IF NOT EXISTS idx_funnel_event_type
    ON funnel_events (event_type, occurred_at DESC);

-- Per-tenant journey: WHERE tenant_id = ? ORDER BY occurred_at
CREATE INDEX IF NOT EXISTS idx_funnel_tenant
    ON funnel_events (tenant_id, occurred_at DESC)
    WHERE tenant_id IS NOT NULL;

-- Anonymous → tenant stitch: WHERE anon_id = ? ORDER BY occurred_at
CREATE INDEX IF NOT EXISTS idx_funnel_anon
    ON funnel_events (anon_id, occurred_at DESC);

-- Ad revenue roll-up: WHERE event_type IN ('AD_IMPRESSION', 'AD_CLICK')
CREATE INDEX IF NOT EXISTS idx_funnel_ads
    ON funnel_events (occurred_at DESC)
    WHERE event_type IN ('AD_IMPRESSION', 'AD_CLICK');

-- JSONB queries: properties->>'utm_source', properties->>'network', etc.
CREATE INDEX IF NOT EXISTS idx_funnel_properties
    ON funnel_events USING GIN (properties);

-- ── Convenience views ─────────────────────────────────────────────────────────

-- Daily funnel summary — useful for admin dashboard and ad-hoc analysis via MCP.
CREATE OR REPLACE VIEW funnel_daily AS
SELECT
    date_trunc('day', occurred_at) AS day,
    event_type,
    count(*)                       AS events,
    count(DISTINCT anon_id)        AS unique_visitors,
    count(DISTINCT tenant_id)      AS unique_tenants
FROM funnel_events
GROUP BY 1, 2
ORDER BY 1 DESC, 2;

COMMENT ON VIEW funnel_daily IS
    'Daily funnel step counts; query via AI/MCP: "how many users reached TRIAL_START this week?"';

-- Ad revenue summary — total impressions, clicks, estimated revenue per network.
CREATE OR REPLACE VIEW ad_revenue_daily AS
SELECT
    date_trunc('day', occurred_at)      AS day,
    properties->>'network'              AS network,
    count(*) FILTER (WHERE event_type = 'AD_IMPRESSION') AS impressions,
    count(*) FILTER (WHERE event_type = 'AD_CLICK')      AS clicks,
    round(
        avg((properties->>'cpm_usd')::numeric)
        FILTER (WHERE event_type = 'AD_IMPRESSION'), 4
    )                                   AS avg_cpm,
    round(
        sum((properties->>'cpm_usd')::numeric / 1000)
        FILTER (WHERE event_type = 'AD_IMPRESSION'), 4
    )                                   AS estimated_revenue_usd
FROM funnel_events
WHERE event_type IN ('AD_IMPRESSION', 'AD_CLICK')
GROUP BY 1, 2
ORDER BY 1 DESC, estimated_revenue_usd DESC NULLS LAST;

COMMENT ON VIEW ad_revenue_daily IS
    'Daily ad revenue per network; query via AI/MCP: "which ad network earned most yesterday?"';

COMMENT ON TABLE funnel_events IS
    'Conversion funnel + ad impression ledger; anon_id stitches pre/post-auth events';
