-- Migration 010: Per-tenant investment thesis constraints
--
-- Allows users to override the AI's symbol selection and leverage
-- via natural-language commands entered in the consumer app floating bar.
--
-- All columns are nullable — NULL means "use the AI defaults".

ALTER TABLE tenants
    ADD COLUMN IF NOT EXISTS investment_thesis    TEXT,        -- Free-form text for display, e.g. "Only meme coins, max 3× leverage"
    ADD COLUMN IF NOT EXISTS symbol_whitelist     TEXT,        -- Comma-separated uppercase symbols, e.g. "BTC,ETH,SOL"
    ADD COLUMN IF NOT EXISTS sector_filter        TEXT,        -- Named sector, e.g. "meme", "l1", "defi", "l2"
    ADD COLUMN IF NOT EXISTS max_leverage_override DOUBLE PRECISION; -- User-requested leverage cap, e.g. 3.0
