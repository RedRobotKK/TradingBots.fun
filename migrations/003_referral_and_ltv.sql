-- ══════════════════════════════════════════════════════════════════════════════
-- TradingBots.fun — Migration 003: referral attribution + LTV cohort analytics
--
-- Purpose:
--   1. Track where each user came from (UTM source, HL referral link, direct).
--   2. Surface referred vs. organic LTV differences so marketing can see
--      which acquisition channels generate the highest long-term revenue.
--   3. Add a REFERRED funnel event for moment-of-signup attribution.
--
-- Revenue streams tracked:
--   builder_fee_usd   — 1 or 3 bps earned per HL fill (primary free-tier LTV)
--   sub_revenue_usd   — cumulative Stripe subscription payments
--   referral_rebate_usd — 10% of HL taker fees on referred users (secondary)
--
-- Design:
--   • referral_source / utm_* on tenants is set once at TRIAL_START and never
--     overwritten (first-touch attribution, standard SaaS practice).
--   • builder_fee_bps stored on tenant so historical cohort queries can compute
--     revenue correctly even after a tier change.
--   • ltv_by_cohort view aggregates across all three revenue streams.
-- ══════════════════════════════════════════════════════════════════════════════

-- ── 1. Referral / UTM attribution columns on tenants ─────────────────────────

ALTER TABLE tenants
    -- First-touch acquisition source (utm_source or "direct" / "hl_referral").
    -- Set once at TRIAL_START; never overwritten.
    ADD COLUMN IF NOT EXISTS referral_source   TEXT,

    -- Full UTM params captured at signup for campaign drill-down.
    ADD COLUMN IF NOT EXISTS utm_source        TEXT,
    ADD COLUMN IF NOT EXISTS utm_medium        TEXT,
    ADD COLUMN IF NOT EXISTS utm_campaign      TEXT,

    -- Whether this user was acquired via our Hyperliquid referral link.
    -- When TRUE the platform also earns 10% of their HL taker fee indefinitely.
    ADD COLUMN IF NOT EXISTS hl_referred       BOOLEAN NOT NULL DEFAULT FALSE,

    -- Builder fee bps applied to this tenant's orders.
    -- Snapshot at account creation; updated when tier changes.
    -- 3 = Free, 1 = Pro/Internal.
    ADD COLUMN IF NOT EXISTS builder_fee_bps   SMALLINT NOT NULL DEFAULT 3,

    -- Cumulative platform revenue from this tenant (denormalised for fast LTV queries).
    -- Updated by the trading loop on each fill and by the Stripe webhook on payment.
    ADD COLUMN IF NOT EXISTS builder_fee_usd   NUMERIC(18,8) NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS sub_revenue_usd   NUMERIC(18,8) NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS referral_rebate_usd NUMERIC(18,8) NOT NULL DEFAULT 0;

-- Set existing Pro tenants to 1 bps (they should never have been at 3)
UPDATE tenants SET builder_fee_bps = 1 WHERE tier IN ('Pro', 'Internal');

-- ── 2. Index for cohort queries ───────────────────────────────────────────────

CREATE INDEX IF NOT EXISTS idx_tenants_referral_source
    ON tenants (referral_source, created_at);

CREATE INDEX IF NOT EXISTS idx_tenants_hl_referred
    ON tenants (hl_referred, created_at)
    WHERE hl_referred = TRUE;

-- ── 3. REFERRED event in funnel_events CHECK constraint ──────────────────────
-- Extend the allowed event_type values to include REFERRED.
-- PostgreSQL requires DROP + re-ADD for CHECK constraints.

ALTER TABLE funnel_events DROP CONSTRAINT IF EXISTS funnel_events_event_type_check;

ALTER TABLE funnel_events ADD CONSTRAINT funnel_events_event_type_check
    CHECK (event_type IN (
        'PAGE_VIEW', 'LOGIN_CLICK', 'AUTH_SUCCESS',
        'TRIAL_START', 'TERMS_ACCEPTED', 'WALLET_LINKED', 'FIRST_POSITION',
        'UPGRADE_CLICK', 'CHECKOUT_STARTED', 'UPGRADED',
        'TRIAL_EXPIRED', 'CHURNED',
        'AD_IMPRESSION', 'AD_CLICK',
        'REFERRED'          -- user arrived via our HL referral link at signup
    ));

-- ── 4. LTV cohort view ────────────────────────────────────────────────────────
-- Query this to answer: "which acquisition channel has the highest 30-day LTV?"
-- and "do HL-referred users convert to Pro at a higher rate?"

CREATE OR REPLACE VIEW ltv_by_cohort AS
SELECT
    -- Acquisition cohort
    coalesce(t.referral_source, 'direct')           AS source,
    t.hl_referred,
    t.tier,
    date_trunc('month', t.created_at)               AS signup_month,

    -- User counts
    count(*)                                         AS users,
    count(*) FILTER (WHERE t.tier = 'Pro')           AS pro_users,
    round(
        100.0 * count(*) FILTER (WHERE t.tier = 'Pro')
        / nullif(count(*), 0), 1
    )                                                AS conversion_pct,

    -- Revenue totals
    round(sum(t.builder_fee_usd),   2)               AS total_builder_fee_usd,
    round(sum(t.sub_revenue_usd),   2)               AS total_sub_revenue_usd,
    round(sum(t.referral_rebate_usd), 2)             AS total_referral_rebate_usd,
    round(sum(t.builder_fee_usd + t.sub_revenue_usd + t.referral_rebate_usd), 2)
                                                     AS total_ltv_usd,

    -- Per-user averages
    round(avg(t.builder_fee_usd + t.sub_revenue_usd + t.referral_rebate_usd), 2)
                                                     AS avg_ltv_per_user_usd,
    round(avg(t.builder_fee_usd), 2)                 AS avg_builder_fee_per_user_usd

FROM tenants t
GROUP BY 1, 2, 3, 4
ORDER BY signup_month DESC, total_ltv_usd DESC NULLS LAST;

COMMENT ON VIEW ltv_by_cohort IS
    'LTV by acquisition source + tier + month; query via AI: "which source has highest avg LTV this quarter?"';

-- ── 5. Per-tenant revenue timeline view ───────────────────────────────────────
-- Useful for customer success: "show me this user's revenue history"

CREATE OR REPLACE VIEW tenant_revenue_timeline AS
SELECT
    t.id                                                     AS tenant_id,
    t.display_name,
    t.tier,
    t.referral_source,
    t.hl_referred,
    t.created_at,
    t.builder_fee_bps,
    -- Rolling 30-day trade volume proxy (sum of notional from closed trades)
    coalesce((
        SELECT sum(ct.size_usd)
        FROM   closed_trades ct
        WHERE  ct.tenant_id = t.id
          AND  ct.closed_at > now() - INTERVAL '30 days'
    ), 0)                                                    AS volume_30d_usd,
    -- Estimated 30-day builder fee from volume
    round(coalesce((
        SELECT sum(ct.size_usd)
        FROM   closed_trades ct
        WHERE  ct.tenant_id = t.id
          AND  ct.closed_at > now() - INTERVAL '30 days'
    ), 0) * (t.builder_fee_bps::numeric / 10000), 4)        AS est_builder_fee_30d_usd,
    -- Lifetime totals
    t.builder_fee_usd,
    t.sub_revenue_usd,
    t.referral_rebate_usd,
    (t.builder_fee_usd + t.sub_revenue_usd + t.referral_rebate_usd)
                                                             AS total_ltv_usd
FROM tenants t
ORDER BY total_ltv_usd DESC NULLS LAST;

COMMENT ON VIEW tenant_revenue_timeline IS
    'Per-tenant revenue summary for customer success and admin LTV drill-down';
