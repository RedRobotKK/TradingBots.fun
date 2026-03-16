-- 006 — Add email column to tenants
--
-- The promo-email job (db.rs fetch_expired_trial_tenants) selects email
-- directly from the tenants table. Privy provides the user's email at
-- login time; web_dashboard.rs stamps it into this column on session creation.

ALTER TABLE tenants
    ADD COLUMN IF NOT EXISTS email TEXT DEFAULT NULL;

COMMENT ON COLUMN tenants.email IS
    'Email address captured from Privy at login. NULL if the user authenticated '
    'via wallet only. Used by the promo-email job to send trial-expiry offers.';

CREATE INDEX IF NOT EXISTS idx_tenants_email
    ON tenants (email)
    WHERE email IS NOT NULL;
