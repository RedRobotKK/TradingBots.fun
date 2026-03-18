-- Migration 009: Fix ambiguous column reference in claim_invite_code()
--
-- PostgreSQL raises "column reference campaign_id is ambiguous" because the
-- function's RETURNS TABLE declares output columns (invite_id, campaign_id,
-- created_by) that share names with invite_codes table columns referenced in
-- the UPDATE...RETURNING clause.  Qualify with the table name to disambiguate.

CREATE OR REPLACE FUNCTION claim_invite_code(p_code TEXT)
RETURNS TABLE (
    invite_id   UUID,
    campaign_id UUID,
    created_by  UUID
) LANGUAGE plpgsql AS $$
DECLARE
    v_id        UUID;
    v_campaign  UUID;
    v_creator   UUID;
BEGIN
    UPDATE invite_codes
    SET    uses_count = uses_count + 1
    WHERE  code       = upper(p_code)
      AND  is_active  = TRUE
      AND  (expires_at IS NULL OR expires_at > now())
      AND  uses_count < max_uses
    RETURNING invite_codes.id,
              invite_codes.campaign_id,
              invite_codes.created_by
    INTO v_id, v_campaign, v_creator;

    IF v_id IS NULL THEN
        RETURN;   -- empty result set = invalid / exhausted code
    END IF;

    RETURN QUERY SELECT v_id, v_campaign, v_creator;
END;
$$;

COMMENT ON FUNCTION claim_invite_code IS
    'Atomically validates and claims one use of an invite code; returns empty if invalid/exhausted';
