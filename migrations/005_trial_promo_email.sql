-- ══════════════════════════════════════════════════════════════════════════════
-- 005 — Trial monetisation: promo-email tracking + trial-tier ad policy
-- ══════════════════════════════════════════════════════════════════════════════
--
-- Goals:
--   1. Track when the post-trial promo email ($9.95/month intro offer) was sent
--      so the batch job never sends it twice.
--   2. Record TRIAL_EXPIRED and PROMO_SENT funnel events for conversion analysis.
--   3. Separate Stripe price ID for the $9.95 introductory offer is handled
--      entirely in app config (STRIPE_PROMO_PRICE_ID env var) — no DB column
--      needed.
--
-- Advertisement policy (reminder, enforced in code):
--   • tier = 'Free'  AND  trial ACTIVE    → ads shown  (trial is monetised)
--   • tier = 'Free'  AND  trial EXPIRED   → ads shown  (upsell pressure)
--   • tier = 'Pro'   (any state)          → NO ads
--   • tier = 'Internal'                   → NO ads
-- ══════════════════════════════════════════════════════════════════════════════

-- ── 1. Add promo email sent timestamp ────────────────────────────────────────
ALTER TABLE tenants
    ADD COLUMN IF NOT EXISTS promo_email_sent_at  TIMESTAMPTZ DEFAULT NULL;

COMMENT ON COLUMN tenants.promo_email_sent_at IS
    'Timestamp when the post-trial promo email ($9.95/month) was sent. '
    'NULL = not yet sent. Set by the hourly trial-expiry batch job. '
    'The job skips rows where this is non-NULL so emails are sent exactly once.';

-- ── 2. Extend funnel_events CHECK to include new event types ─────────────────
-- We must drop and recreate the constraint because PostgreSQL does not support
-- ALTER CONSTRAINT on CHECK constraints.
ALTER TABLE funnel_events
    DROP CONSTRAINT IF EXISTS funnel_events_event_type_check;

ALTER TABLE funnel_events
    ADD CONSTRAINT funnel_events_event_type_check CHECK (event_type IN (
        -- Acquisition
        'TRIAL_START',
        -- Activation
        'TERMS_ACCEPTED',
        'WALLET_LINKED',
        'FIRST_POSITION',
        -- Revenue
        'UPGRADED',
        'CHURNED',
        -- Referral
        'REFERRED',
        'INVITE_USED',
        -- Retention / re-engagement
        'TRIAL_EXPIRED',    -- fired when the batch job detects a newly-expired trial
        'PROMO_SENT'        -- fired when the $9.95 promo email is dispatched
    ));

-- ── 3. Index to speed up the hourly batch query ───────────────────────────────
-- Finds Free-tier tenants whose trial ended before now and who haven't been
-- emailed yet.  Without this index the batch job would do a sequential scan on
-- the full tenants table.
CREATE INDEX IF NOT EXISTS idx_tenants_trial_promo
    ON tenants (trial_ends_at, promo_email_sent_at)
    WHERE tier = 'Free'
      AND trial_ends_at IS NOT NULL
      AND promo_email_sent_at IS NULL;

-- ── 4. Helper view: conversion funnel with promo-email cohort ─────────────────
-- Shows how many trial users converted before vs after receiving the promo.
-- Query this from Claude/MCP: "promo email conversion rate this month"
CREATE OR REPLACE VIEW promo_email_conversion AS
SELECT
    date_trunc('week', t.promo_email_sent_at) AS promo_week,
    count(*)                                   AS emails_sent,
    count(*) FILTER (
        WHERE t.tier = 'Pro'
          AND EXISTS (
              SELECT 1 FROM funnel_events fe
               WHERE fe.tenant_id = t.id
                 AND fe.event_type = 'UPGRADED'
                 AND fe.occurred_at > t.promo_email_sent_at
          )
    )                                          AS converted_after_promo,
    round(
        100.0 * count(*) FILTER (
            WHERE t.tier = 'Pro'
              AND EXISTS (
                  SELECT 1 FROM funnel_events fe
                   WHERE fe.tenant_id = t.id
                     AND fe.event_type = 'UPGRADED'
                     AND fe.occurred_at > t.promo_email_sent_at
              )
        ) / nullif(count(*), 0),
        1
    )                                          AS conversion_pct
FROM tenants t
WHERE t.promo_email_sent_at IS NOT NULL
GROUP BY 1
ORDER BY 1 DESC;

COMMENT ON VIEW promo_email_conversion IS
    'Weekly cohort analysis of the post-trial $9.95 promo email. '
    'conversion_pct = users who upgraded after receiving the email.';
