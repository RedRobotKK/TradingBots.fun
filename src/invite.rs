//! Invite-code gating — all signups require a valid invite code.
//!
//! ## How it works
//!
//! 1. Operator (or an existing user) inserts rows into `invite_codes` via
//!    the admin panel or the `POST /admin/invite` endpoint.
//! 2. At login, the browser sends `{ token, invite_code, … }` to
//!    `POST /auth/session`.
//! 3. `claim_invite_code()` calls the DB function atomically: increments
//!    `uses_count` and returns the code row, or returns `None` if the code
//!    is invalid / exhausted / expired.
//! 4. On success the `tenant.invite_code_used`, `invited_by`, and
//!    `campaign_id` fields are stamped on the new tenant row.
//!
//! ## Referral codes (personal)
//!
//! Any Pro tenant can request a personal invite code from
//! `POST /app/invite/generate`.  That code has `created_by = tenant_id` and
//! `max_uses = 1` by default.  When someone claims it, the referrer gets
//! a `REFERRAL_CONVERTED` funnel event and (future) a fee-credit reward.
//!
//! ## Campaign blast codes
//!
//! Operator-generated codes with `max_uses = N` tied to a campaign.
//! Seeded in migration 004; new ones added via the admin panel.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::SharedDb;
use crate::tenant::TenantId;

// ─────────────────────────────────────────────────────────────────────────────

/// Result of a successful invite code claim.
#[derive(Debug, Clone)]
pub struct ClaimedInvite {
    /// PK of the invite_codes row (for audit linkage).
    #[allow(dead_code)]
    pub invite_id: Uuid,
    /// Campaign this code belongs to (may be NULL for uncampaigned codes).
    pub campaign_id: Option<Uuid>,
    /// Tenant who generated the code (None for operator blast codes).
    pub created_by: Option<Uuid>,
}

// ─────────────────────────────────────────────────────────────────────────────

/// Atomically claim one use of an invite code.
///
/// Calls the `claim_invite_code(p_code)` PL/pgSQL function defined in
/// migration 004.  That function:
///   1. UPDATEs `uses_count += 1` WHERE code matches AND is_active AND not expired
///      AND uses_count < max_uses — all in one statement (no TOCTOU race).
///   2. RETURNs the invite_id, campaign_id, created_by if the UPDATE touched a row.
///   3. Returns an empty result set if the code was invalid / exhausted.
///
/// # Errors
/// Returns `Err` only on DB failure.  An invalid / exhausted code returns
/// `Ok(None)` — the caller must surface this as a 400-level auth error.
pub async fn claim_invite_code(db: &SharedDb, code: &str) -> Result<Option<ClaimedInvite>> {
    let row = sqlx::query!(
        r#"SELECT invite_id, campaign_id, created_by
           FROM claim_invite_code($1)"#,
        code.trim().to_uppercase(),
    )
    .fetch_optional(db.pool())
    .await
    .map_err(|e| anyhow!("claim_invite_code DB error: {}", e))?;

    Ok(row.map(|r| ClaimedInvite {
        invite_id: r.invite_id.unwrap_or_default(),
        campaign_id: r.campaign_id,
        created_by: r.created_by,
    }))
}

// ─────────────────────────────────────────────────────────────────────────────

/// Generate a new personal referral invite code for an existing tenant.
///
/// Inserts one row into `invite_codes` with:
///   - `code`         = DB-generated `TB-XXXXXXXX`
///   - `created_by`   = the requesting tenant
///   - `campaign_id`  = the currently active campaign (if any)
///   - `max_uses`     = 1 (personal codes are single-use by default)
///   - `expires_at`   = 30 days from now
///
/// Returns the generated code string (e.g. `"TB-A1B2C3D4"`).
pub async fn generate_referral_code(db: &SharedDb, tenant_id: &TenantId) -> Result<String> {
    // Look up the currently active campaign (if any)
    let campaign_id: Option<Uuid> =
        sqlx::query_scalar!("SELECT id FROM campaigns WHERE is_active = TRUE LIMIT 1",)
            .fetch_optional(db.pool())
            .await
            .map_err(|e| anyhow!("campaign lookup: {}", e))?;

    let tenant_uuid =
        Uuid::parse_str(tenant_id.as_str()).map_err(|e| anyhow!("invalid tenant UUID: {}", e))?;

    // Generate code and insert atomically
    let code: String = sqlx::query_scalar!(
        r#"INSERT INTO invite_codes (code, campaign_id, created_by, max_uses, expires_at)
           VALUES (generate_invite_code(), $1, $2, 1, now() + INTERVAL '30 days')
           RETURNING code"#,
        campaign_id,
        tenant_uuid,
    )
    .fetch_one(db.pool())
    .await
    .map_err(|e| anyhow!("generate_referral_code insert: {}", e))?;

    log::info!(
        "🎟 Referral code {} generated for tenant {}",
        code,
        tenant_id
    );
    Ok(code)
}

// ─────────────────────────────────────────────────────────────────────────────

/// Admin: generate N blast codes for a campaign.
///
/// Returns the list of generated code strings.
#[allow(dead_code)]
pub async fn generate_blast_codes(
    db: &SharedDb,
    campaign_id: Uuid,
    count: u32,
    max_uses: i16,
    note: Option<&str>,
) -> Result<Vec<String>> {
    let mut codes = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let code: String = sqlx::query_scalar!(
            r#"INSERT INTO invite_codes (code, campaign_id, max_uses, note)
               VALUES (generate_invite_code(), $1, $2, $3)
               RETURNING code"#,
            campaign_id,
            max_uses,
            note,
        )
        .fetch_one(db.pool())
        .await
        .map_err(|e| anyhow!("generate_blast_codes insert: {}", e))?;
        codes.push(code);
    }
    log::info!(
        "🎟 Generated {} blast codes for campaign {}",
        count,
        campaign_id
    );
    Ok(codes)
}

// ─────────────────────────────────────────────────────────────────────────────

/// Look up a tenant's personal referral code (if they have one).
///
/// Returns None if they haven't generated one yet.
pub async fn get_referral_code_for_tenant(
    db: &SharedDb,
    tenant_id: &TenantId,
) -> Result<Option<String>> {
    let tenant_uuid =
        Uuid::parse_str(tenant_id.as_str()).map_err(|e| anyhow!("invalid tenant UUID: {}", e))?;

    let code = sqlx::query_scalar!(
        "SELECT code FROM invite_codes WHERE created_by = $1 AND max_uses = 1 ORDER BY created_at DESC LIMIT 1",
        tenant_uuid,
    )
    .fetch_optional(db.pool())
    .await
    .map_err(|e| anyhow!("get_referral_code: {}", e))?;

    Ok(code)
}

// ─────────────────────────────────────────────────────────────────────────────

/// Payload for POST /admin/invite/generate-blast
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct GenerateBlastRequest {
    pub campaign_id: Uuid,
    pub count: u32,
    pub max_uses: Option<i16>,
    pub note: Option<String>,
}

/// Response shape for invite code endpoints.
#[derive(Debug, Serialize)]
#[allow(dead_code)]
pub struct InviteCodeResponse {
    pub code: String,
    pub expires_at: Option<String>,
    pub campaign_id: Option<Uuid>,
    pub uses_left: Option<i16>,
}

/// Admin: list all invite codes for a campaign.
#[derive(Debug, Serialize)]
#[allow(dead_code)]
pub struct InviteCodeSummary {
    pub code: String,
    pub uses_count: i16,
    pub max_uses: i16,
    pub is_active: bool,
    pub created_at: String,
    pub note: Option<String>,
    pub created_by: Option<String>, // tenant display_name or "operator"
}

#[allow(dead_code)]
pub async fn list_codes_for_campaign(
    db: &SharedDb,
    campaign_id: Uuid,
) -> Result<Vec<InviteCodeSummary>> {
    let rows = sqlx::query!(
        r#"SELECT ic.code, ic.uses_count, ic.max_uses, ic.is_active,
                  ic.created_at, ic.note,
                  t.display_name AS creator_name
           FROM invite_codes ic
           LEFT JOIN tenants t ON t.id = ic.created_by
           WHERE ic.campaign_id = $1
           ORDER BY ic.created_at DESC"#,
        campaign_id,
    )
    .fetch_all(db.pool())
    .await
    .map_err(|e| anyhow!("list_codes_for_campaign: {}", e))?;

    Ok(rows
        .into_iter()
        .map(|r| InviteCodeSummary {
            code: r.code,
            uses_count: r.uses_count,
            max_uses: r.max_uses,
            is_active: r.is_active,
            created_at: r.created_at.to_rfc3339(),
            note: r.note,
            created_by: Some(r.creator_name.unwrap_or_else(|| "operator".into())),
        })
        .collect())
}
