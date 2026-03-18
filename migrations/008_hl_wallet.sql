-- Migration 008: Per-user Hyperliquid trading wallet
--
-- Each consumer tenant gets a dedicated Hyperliquid wallet generated at
-- onboarding.  This is separate from their Privy authentication wallet and
-- is the address where they deposit USDC before the bot can trade.
--
-- hl_wallet_address  — EIP-55 checksum address (0x…), public, shown in UI.
-- hl_wallet_key_enc  — AES-256-GCM encrypted private key (nonce_hex:ct_hex).
--                      Keyed from SESSION_SECRET + tenant_id.  Never logged.
-- hl_setup_complete  — True once the user has acknowledged their private key
--                      and been shown the funding instructions.  Used to gate
--                      the redirect from /app/onboarding/accept → /app/setup
--                      vs /app on repeat visits.

ALTER TABLE tenants
    ADD COLUMN IF NOT EXISTS hl_wallet_address  TEXT    UNIQUE,
    ADD COLUMN IF NOT EXISTS hl_wallet_key_enc  TEXT,
    ADD COLUMN IF NOT EXISTS hl_setup_complete  BOOLEAN NOT NULL DEFAULT false;

CREATE INDEX IF NOT EXISTS idx_tenants_hl_wallet ON tenants (hl_wallet_address);
