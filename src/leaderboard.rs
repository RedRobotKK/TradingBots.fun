//! Weekly leaderboard and campaign management.
//!
//! ## Ranking strategy
//!
//! All accounts are ranked by **% return** — not absolute PnL.  This means a
//! user who deposits $20, runs two bots, and earns 12% ranks above a $10,000
//! account that earns 2%.  This is intentional: it makes the contest fair for
//! small-capital users and encourages the platform's core use case (try it with
//! $20, see the bots work, upgrade when convinced).
//!
//! ## Snapshot cadence
//!
//! `snapshot_daily()` should be called once per day (e.g. at midnight UTC via
//! the maintenance task in `main.rs`).  It writes one row per active tenant per
//! active campaign into `leaderboard_snapshots`.  The first snapshot for a
//! (tenant, campaign) pair also records `start_equity_usd` which anchors the
//! % return calculation for the rest of the week.
//!
//! ## Prize distribution
//!
//! `award_campaign_prizes()` is called when a campaign ends.  It reads the
//! final standings from `leaderboard_live`, computes prizes from the campaign's
//! `prizes` JSONB, writes rows into `campaign_prizes`, and logs prize amounts.
//! Actual payment (USDC transfer, Stripe credit, etc.) is handled out-of-band
//! by the operator — this module only records what was awarded.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::SharedDb;
use crate::tenant::SharedTenantManager;

// ─────────────────────────────────────────────────────────────────────────────

/// One row from the live leaderboard view.
#[derive(Debug, Clone, Serialize)]
pub struct LeaderboardEntry {
    pub rank:             i64,
    pub tenant_id:        Uuid,
    pub display_name:     String,
    /// Truncated wallet address shown on the leaderboard (privacy-preserving).
    pub wallet_short:     String,
    pub equity_usd:       f64,
    pub start_equity_usd: f64,
    pub pct_return:       f64,
    pub trades_in_period: i32,
}

/// Active campaign metadata returned to the leaderboard page.
#[derive(Debug, Clone, Serialize)]
pub struct CampaignInfo {
    pub id:             Uuid,
    pub slug:           String,
    pub title:          String,
    pub description:    Option<String>,
    pub starts_at:      String,
    pub ends_at:        String,
    pub prize_pool_usd: f64,
    /// Parsed prize tiers from the JSONB column.
    pub prizes:         Vec<PrizeTier>,
    /// Seconds remaining until the campaign ends (for countdown timer).
    pub seconds_left:   i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrizeTier {
    pub rank:  i32,
    pub label: String,
    pub usd:   f64,
}

// ─────────────────────────────────────────────────────────────────────────────

/// Fetch the currently active campaign metadata.
/// Returns `None` if no campaign is active.
pub async fn active_campaign(db: &SharedDb) -> Result<Option<CampaignInfo>> {
    let row = sqlx::query!(
        r#"SELECT id, slug, title, description,
                  starts_at, ends_at, prize_pool_usd, prizes,
                  EXTRACT(EPOCH FROM (ends_at - now()))::BIGINT AS seconds_left
           FROM campaigns
           WHERE is_active = TRUE
           LIMIT 1"#,
    )
    .fetch_optional(db.pool())
    .await
    .map_err(|e| anyhow!("active_campaign: {}", e))?;

    let Some(r) = row else { return Ok(None) };

    let prizes: Vec<PrizeTier> = r.prizes
        .as_ref()
        .and_then(|v: &serde_json::Value| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    Ok(Some(CampaignInfo {
        id:             r.id,
        slug:           r.slug,
        title:          r.title,
        description:    r.description,
        starts_at:      r.starts_at.to_rfc3339(),
        ends_at:        r.ends_at.to_rfc3339(),
        prize_pool_usd: r.prize_pool_usd.to_string().parse::<f64>().unwrap_or(0.0),
        prizes,
        seconds_left:   r.seconds_left.unwrap_or(0),
    }))
}

// ─────────────────────────────────────────────────────────────────────────────

/// Fetch the current standings for the active campaign.
///
/// Returns up to `limit` entries, ranked by % return descending.
/// Accounts with zero trades are excluded from the ranking.
pub async fn live_standings(
    db:    &SharedDb,
    limit: i64,
) -> Result<Vec<LeaderboardEntry>> {
    let rows = sqlx::query!(
        r#"SELECT rank, tenant_id, display_name, wallet_address,
                  equity_usd, start_equity_usd,
                  pct_return, trades_in_period
           FROM leaderboard_live
           LIMIT $1"#,
        limit,
    )
    .fetch_all(db.pool())
    .await
    .map_err(|e| anyhow!("live_standings: {}", e))?;

    Ok(rows.into_iter().filter_map(|r| {
        let tenant_id = r.tenant_id?; // skip rows without a tenant_id
        let wallet_short = r.wallet_address
            .map(|w: String| if w.len() >= 10 { format!("{}…{}", &w[..6], &w[w.len()-4..]) } else { w })
            .unwrap_or_else(|| "—".to_string());

        Some(LeaderboardEntry {
            rank:             r.rank.unwrap_or(0),
            tenant_id,
            display_name:     r.display_name.unwrap_or_else(|| "Anonymous".into()),
            wallet_short,
            equity_usd:       r.equity_usd.map(|d: rust_decimal::Decimal| d.to_string().parse::<f64>().unwrap_or(0.0)).unwrap_or(0.0),
            start_equity_usd: r.start_equity_usd.map(|d: rust_decimal::Decimal| d.to_string().parse::<f64>().unwrap_or(0.0)).unwrap_or(0.0),
            pct_return:       r.pct_return.map(|d: rust_decimal::Decimal| d.to_string().parse::<f64>().unwrap_or(0.0)).unwrap_or(0.0),
            trades_in_period: r.trades_in_period.unwrap_or(0),
        })
    }).collect())
}

// ─────────────────────────────────────────────────────────────────────────────

/// Write daily leaderboard snapshots for all active tenants.
///
/// Called once per day by the maintenance task.  For each tenant whose
/// `campaign_id` matches the active campaign, reads their current equity
/// from the latest `equity_snapshots` row and upserts a `leaderboard_snapshots`
/// row for today.
///
/// The `start_equity_usd` is set on the FIRST snapshot for each
/// (tenant, campaign) pair and never updated — it anchors the % return.
///
/// Also increments `trades_in_period` by the number of new `closed_trades`
/// rows since yesterday.
pub async fn snapshot_daily(
    db:      &SharedDb,
    tenants: &SharedTenantManager,
) -> Result<usize> {
    // Get active campaign
    let campaign_id: Option<Uuid> = sqlx::query_scalar!(
        "SELECT id FROM campaigns WHERE is_active = TRUE LIMIT 1",
    )
    .fetch_optional(db.pool())
    .await
    .map_err(|e| anyhow!("snapshot_daily campaign lookup: {}", e))?;

    let Some(campaign_id) = campaign_id else {
        log::debug!("snapshot_daily: no active campaign, skipping");
        return Ok(0);
    };

    let mut count = 0usize;
    let tenant_ids: Vec<(String, f64)> = {
        let mgr = tenants.read().await;
        mgr.all()
            .map(|h| (h.id.as_str().to_string(), h.config.hl_balance_usd))
            .collect()
    };

    // Parse all tenant UUIDs up-front, discard malformed ones
    let tenants_parsed: Vec<(Uuid, f64)> = tenant_ids
        .iter()
        .filter_map(|(id, eq)| Uuid::parse_str(id).ok().map(|u| (u, *eq)))
        .collect();

    if tenants_parsed.is_empty() {
        return Ok(0);
    }

    let tenant_uuids: Vec<Uuid> = tenants_parsed.iter().map(|(u, _)| *u).collect();

    // Batch 1: count trades per tenant for this campaign in a single query
    // Returns rows: (tenant_id, count)
    let trade_counts: Vec<(Uuid, i64)> = sqlx::query!(
        r#"SELECT ct.tenant_id, COUNT(*) AS cnt
           FROM closed_trades ct
           JOIN campaigns c ON c.id = $1
           WHERE ct.tenant_id = ANY($2)
             AND ct.closed_at >= c.starts_at
           GROUP BY ct.tenant_id"#,
        campaign_id,
        &tenant_uuids,
    )
    .fetch_all(db.pool())
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|r| (r.tenant_id, r.cnt.unwrap_or(0)))
    .collect();

    let trade_count_map: std::collections::HashMap<Uuid, i64> =
        trade_counts.into_iter().collect();

    // Batch 2: get earliest start_equity_usd per tenant for this campaign
    let start_equities: Vec<(Uuid, rust_decimal::Decimal)> = sqlx::query!(
        r#"SELECT DISTINCT ON (tenant_id) tenant_id, start_equity_usd
           FROM leaderboard_snapshots
           WHERE tenant_id = ANY($1)
             AND campaign_id = $2
           ORDER BY tenant_id, snapshot_date ASC"#,
        &tenant_uuids,
        campaign_id,
    )
    .fetch_all(db.pool())
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|r| (r.tenant_id, r.start_equity_usd))
    .collect();

    let start_equity_map: std::collections::HashMap<Uuid, rust_decimal::Decimal> =
        start_equities.into_iter().collect();

    for (tenant_uuid, current_equity) in &tenants_parsed {
        let trades_in_period = *trade_count_map.get(tenant_uuid).unwrap_or(&0);

        let start_equity = start_equity_map
            .get(tenant_uuid)
            .map(|d: &rust_decimal::Decimal| d.to_string().parse::<f64>().unwrap_or(*current_equity))
            .unwrap_or(*current_equity);  // first snapshot: anchor to current equity

        let result = sqlx::query!(
            r#"INSERT INTO leaderboard_snapshots
                   (tenant_id, campaign_id, snapshot_date, equity_usd, start_equity_usd, trades_in_period)
               VALUES ($1, $2, CURRENT_DATE, $3, $4, $5)
               ON CONFLICT (tenant_id, campaign_id, snapshot_date)
               DO UPDATE SET equity_usd = EXCLUDED.equity_usd,
                             trades_in_period = EXCLUDED.trades_in_period"#,
            tenant_uuid,
            campaign_id,
            rust_decimal::Decimal::try_from(*current_equity).unwrap_or_default(),
            rust_decimal::Decimal::try_from(start_equity).unwrap_or_default(),
            trades_in_period as i32,
        )
        .execute(db.pool())
        .await;

        match result {
            Ok(_) => count += 1,
            Err(e) => log::warn!("snapshot_daily failed for tenant {}: {}", tenant_uuid, e),
        }
    }

    log::info!("📊 Leaderboard: wrote {} daily snapshots for campaign {}", count, campaign_id);
    Ok(count)
}

// ─────────────────────────────────────────────────────────────────────────────

/// Award prizes at the end of a campaign.
///
/// Reads the final standings, matches against `campaign.prizes` JSONB, and
/// inserts rows into `campaign_prizes`.  Sets `is_active = FALSE` on the
/// campaign and optionally starts the next one.
///
/// Returns a list of `(rank, tenant_id, prize_usd)` for operator confirmation.
#[allow(dead_code)]
pub async fn award_campaign_prizes(
    db:          &SharedDb,
    campaign_id: Uuid,
) -> Result<Vec<(i64, Uuid, f64)>> {
    // Fetch campaign prize structure
    let prizes_json: Option<serde_json::Value> = sqlx::query_scalar!(
        "SELECT prizes FROM campaigns WHERE id = $1",
        campaign_id,
    )
    .fetch_optional(db.pool())
    .await
    .map_err(|e| anyhow!("award prizes campaign fetch: {}", e))?
    .flatten();

    let prize_tiers: Vec<PrizeTier> = prizes_json
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    if prize_tiers.is_empty() {
        return Ok(vec![]);
    }

    // Fetch final standings
    let standings = live_standings(db, prize_tiers.len() as i64).await?;
    let mut awarded = Vec::new();

    for tier in &prize_tiers {
        let Some(entry) = standings.get((tier.rank - 1) as usize) else { continue };

        let tenant_uuid = entry.tenant_id;

        sqlx::query!(
            r#"INSERT INTO campaign_prizes
                   (campaign_id, tenant_id, rank, prize_usd, pct_return, payment_method)
               VALUES ($1, $2, $3, $4, $5, 'pending')"#,
            campaign_id,
            tenant_uuid,
            tier.rank as i16,
            rust_decimal::Decimal::try_from(tier.usd).unwrap_or_default(),
            rust_decimal::Decimal::try_from(entry.pct_return).unwrap_or_default(),
        )
        .execute(db.pool())
        .await
        .map_err(|e| anyhow!("insert campaign_prize: {}", e))?;

        log::info!(
            "🏆 Campaign {} rank {} — tenant {} — +{:.2}% — prize ${}",
            campaign_id, tier.rank, tenant_uuid, entry.pct_return, tier.usd
        );
        awarded.push((entry.rank, tenant_uuid, tier.usd));
    }

    // Deactivate the campaign
    sqlx::query!(
        "UPDATE campaigns SET is_active = FALSE WHERE id = $1",
        campaign_id,
    )
    .execute(db.pool())
    .await
    .map_err(|e| anyhow!("deactivate campaign: {}", e))?;

    log::info!("✅ Campaign {} closed — {} prizes awarded", campaign_id, awarded.len());
    Ok(awarded)
}
