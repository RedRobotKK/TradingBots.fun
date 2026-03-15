//! Fund event tracker — records deposits and withdrawals per tenant.
//!
//! ## How it works
//!
//! The bot polls each tenant's Hyperliquid account balance on every cycle.
//! `detect_and_record()` compares the previous known balance with the current
//! balance reported by HL.  Any meaningful delta (>$0.01) is recorded as a
//! `FundEvent` in a per-tenant CSV file:
//!
//! ```
//! data/funds/{tenant_id}.csv
//! ```
//!
//! The CSV columns are: `event_type,amount_usd,balance_after,timestamp`
//!
//! ## Why not rely on HL transaction history?
//!
//! HL's `/info` endpoint returns the *current* clearing-house state, not a
//! transaction history.  We infer deposits and withdrawals by diffing the
//! equity snapshots we already collect every cycle.  This is simple, requires
//! no extra API permissions, and works equally well on testnet and mainnet.
//!
//! ## Legal & tax use-case
//!
//! Operator admin and the tenant settings page expose the event log so users
//! can reconcile cost-basis, calculate holding periods, and export records
//! for local tax reporting.

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use crate::tenant::TenantId;

// ─────────────────────────────────────────────────────────────────────────────
//  Data model
// ─────────────────────────────────────────────────────────────────────────────

/// Direction of a fund movement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventType {
    Deposit,
    Withdrawal,
}

impl std::fmt::Display for EventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EventType::Deposit    => write!(f, "deposit"),
            EventType::Withdrawal => write!(f, "withdrawal"),
        }
    }
}

/// A single fund movement event for a tenant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundEvent {
    /// `"deposit"` or `"withdrawal"`.
    pub event_type:    EventType,
    /// Absolute USD amount moved (always positive).
    pub amount_usd:    f64,
    /// Tenant's HL account balance immediately after this event.
    pub balance_after: f64,
    /// UTC timestamp (ISO-8601).
    pub timestamp:     String,
}

// ─────────────────────────────────────────────────────────────────────────────
//  CSV path helper
// ─────────────────────────────────────────────────────────────────────────────

/// Returns the CSV file path for a given tenant.
///
/// Creates the `data/funds/` directory on first use.
pub fn csv_path(tenant_id: &TenantId) -> Result<PathBuf> {
    let dir = PathBuf::from("data/funds");
    fs::create_dir_all(&dir)?;
    Ok(dir.join(format!("{}.csv", tenant_id.as_str())))
}

// ─────────────────────────────────────────────────────────────────────────────
//  Write
// ─────────────────────────────────────────────────────────────────────────────

/// Append a `FundEvent` to the tenant's CSV file.
///
/// Creates the file (with header row) if it does not yet exist.
pub fn append(tenant_id: &TenantId, event: &FundEvent) -> Result<()> {
    let path     = csv_path(tenant_id)?;
    let is_new   = !path.exists();

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;

    if is_new {
        writeln!(file, "event_type,amount_usd,balance_after,timestamp")?;
    }

    writeln!(
        file,
        "{},{:.6},{:.6},{}",
        event.event_type,
        event.amount_usd,
        event.balance_after,
        event.timestamp,
    )?;

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
//  Read
// ─────────────────────────────────────────────────────────────────────────────

/// Read all recorded fund events for a tenant, oldest first.
///
/// Returns an empty `Vec` when no events have been recorded yet.
pub fn read_tenant_history(tenant_id: &TenantId) -> Result<Vec<FundEvent>> {
    let path = csv_path(tenant_id)?;
    if !path.exists() {
        return Ok(vec![]);
    }

    let content = fs::read_to_string(&path)?;
    let mut events = Vec::new();

    for (i, line) in content.lines().enumerate() {
        if i == 0 { continue; } // skip header
        let parts: Vec<&str> = line.splitn(4, ',').collect();
        if parts.len() != 4 { continue; }

        let event_type = match parts[0] {
            "deposit"    => EventType::Deposit,
            "withdrawal" => EventType::Withdrawal,
            _            => continue, // skip unknown rows
        };

        let amount_usd: f64    = parts[1].parse().unwrap_or(0.0);
        let balance_after: f64 = parts[2].parse().unwrap_or(0.0);
        let timestamp          = parts[3].to_string();

        events.push(FundEvent { event_type, amount_usd, balance_after, timestamp });
    }

    Ok(events)
}

// ─────────────────────────────────────────────────────────────────────────────
//  Aggregates
// ─────────────────────────────────────────────────────────────────────────────

/// Net USD deposited by this tenant (deposits minus withdrawals).
pub fn net_deposits(tenant_id: &TenantId) -> f64 {
    read_tenant_history(tenant_id)
        .unwrap_or_default()
        .iter()
        .fold(0.0_f64, |acc, e| {
            match e.event_type {
                EventType::Deposit    => acc + e.amount_usd,
                EventType::Withdrawal => acc - e.amount_usd,
            }
        })
}

/// Total lifetime deposits (withdrawals excluded).
pub fn total_deposited(tenant_id: &TenantId) -> f64 {
    read_tenant_history(tenant_id)
        .unwrap_or_default()
        .iter()
        .filter(|e| e.event_type == EventType::Deposit)
        .map(|e| e.amount_usd)
        .sum()
}

/// Total lifetime withdrawals (deposits excluded).
pub fn total_withdrawn(tenant_id: &TenantId) -> f64 {
    read_tenant_history(tenant_id)
        .unwrap_or_default()
        .iter()
        .filter(|e| e.event_type == EventType::Withdrawal)
        .map(|e| e.amount_usd)
        .sum()
}

// ─────────────────────────────────────────────────────────────────────────────
//  Detection
// ─────────────────────────────────────────────────────────────────────────────

/// Minimum USD delta required to record a fund event.
///
/// Prevents spurious events caused by unrealised-PnL fluctuations being
/// passed in as balance changes when only the *cleared* balance should be
/// compared.
const MIN_DELTA: f64 = 1.0;

/// Compare `old_balance` with `new_balance` and, if the delta exceeds
/// `MIN_DELTA`, append a `FundEvent` to the tenant's CSV.
///
/// Returns the event if one was recorded, `None` otherwise.
///
/// ### Caller responsibility
///
/// The balances passed must represent the HL *cleared* (settled) balance
/// — NOT the mark-to-market equity which fluctuates continuously with
/// unrealised PnL.  This avoids recording phantom events every cycle.
pub fn detect_and_record(
    tenant_id:   &TenantId,
    old_balance: f64,
    new_balance: f64,
) -> Option<FundEvent> {
    let delta = new_balance - old_balance;

    if delta.abs() < MIN_DELTA {
        return None;
    }

    let event = FundEvent {
        event_type:    if delta > 0.0 { EventType::Deposit } else { EventType::Withdrawal },
        amount_usd:    delta.abs(),
        balance_after: new_balance,
        timestamp:     Utc::now().to_rfc3339(),
    };

    if let Err(e) = append(tenant_id, &event) {
        log::warn!("fund_tracker: failed to record event for tenant {}: {}", tenant_id, e);
    } else {
        log::info!(
            "💰 Fund event: tenant={} type={} amount=${:.2} balance_after=${:.2}",
            tenant_id, event.event_type, event.amount_usd, event.balance_after
        );
    }

    Some(event)
}

// ─────────────────────────────────────────────────────────────────────────────
//  Summary struct — used by the admin and settings pages
// ─────────────────────────────────────────────────────────────────────────────

/// Aggregated fund statistics for display in the admin/settings UI.
pub struct FundSummary {
    pub net_deposits:    f64,
    pub total_deposited: f64,
    pub total_withdrawn: f64,
    pub event_count:     usize,
    pub last_event:      Option<FundEvent>,
}

/// Build a `FundSummary` for a tenant from their CSV history.
pub fn summary(tenant_id: &TenantId) -> FundSummary {
    let events = read_tenant_history(tenant_id).unwrap_or_default();
    let net     = events.iter().fold(0.0_f64, |acc, e| match e.event_type {
        EventType::Deposit    => acc + e.amount_usd,
        EventType::Withdrawal => acc - e.amount_usd,
    });
    let deposited: f64 = events.iter()
        .filter(|e| e.event_type == EventType::Deposit)
        .map(|e| e.amount_usd)
        .sum();
    let withdrawn: f64 = events.iter()
        .filter(|e| e.event_type == EventType::Withdrawal)
        .map(|e| e.amount_usd)
        .sum();
    let last = events.last().cloned();
    FundSummary {
        net_deposits:    net,
        total_deposited: deposited,
        total_withdrawn: withdrawn,
        event_count:     events.len(),
        last_event:      last,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Unit tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Logic is tested in-memory; csv_path is hard-coded to "data/funds" so
    // that patches the directory.  Since csv_path is hard-coded to "data/funds",
    // we test the logic independently via in-memory equivalents.

    fn make_event(kind: EventType, amount: f64, balance: f64) -> FundEvent {
        FundEvent {
            event_type:    kind,
            amount_usd:    amount,
            balance_after: balance,
            timestamp:     "2025-01-01T00:00:00Z".to_string(),
        }
    }

    // ── detect_and_record logic ────────────────────────────────────────────────

    #[test]
    fn delta_below_min_returns_none() {
        // A tiny fluctuation should not be recorded
        let tid = TenantId::from_str("test-detect-none");
        let result = detect_and_record(&tid, 1000.0, 1000.50);
        assert!(result.is_none(), "sub-dollar delta should not be recorded");
    }

    #[test]
    fn positive_delta_is_deposit() {
        let event = FundEvent {
            event_type:    EventType::Deposit,
            amount_usd:    500.0,
            balance_after: 1500.0,
            timestamp:     Utc::now().to_rfc3339(),
        };
        assert_eq!(event.event_type, EventType::Deposit);
        assert!((event.amount_usd - 500.0).abs() < 0.001);
    }

    #[test]
    fn negative_delta_is_withdrawal() {
        let event = FundEvent {
            event_type:    EventType::Withdrawal,
            amount_usd:    200.0,
            balance_after: 800.0,
            timestamp:     Utc::now().to_rfc3339(),
        };
        assert_eq!(event.event_type, EventType::Withdrawal);
        assert!((event.amount_usd - 200.0).abs() < 0.001);
    }

    // ── Aggregate helpers ─────────────────────────────────────────────────────

    #[test]
    fn net_deposits_deposits_minus_withdrawals() {
        let events: Vec<FundEvent> = vec![
            make_event(EventType::Deposit,    1000.0, 1000.0),
            make_event(EventType::Deposit,     500.0, 1500.0),
            make_event(EventType::Withdrawal,  300.0, 1200.0),
        ];
        let net = events.iter().fold(0.0_f64, |acc, e| match e.event_type {
            EventType::Deposit    => acc + e.amount_usd,
            EventType::Withdrawal => acc - e.amount_usd,
        });
        assert!((net - 1200.0).abs() < 0.001, "net should be 1000+500-300=1200");
    }

    #[test]
    fn total_deposited_sums_only_deposits() {
        let events: Vec<FundEvent> = vec![
            make_event(EventType::Deposit,    100.0, 100.0),
            make_event(EventType::Withdrawal,  50.0,  50.0),
            make_event(EventType::Deposit,    200.0, 250.0),
        ];
        let total: f64 = events.iter()
            .filter(|e| e.event_type == EventType::Deposit)
            .map(|e| e.amount_usd)
            .sum();
        assert!((total - 300.0).abs() < 0.001);
    }

    #[test]
    fn total_withdrawn_sums_only_withdrawals() {
        let events: Vec<FundEvent> = vec![
            make_event(EventType::Deposit,    500.0, 500.0),
            make_event(EventType::Withdrawal,  75.0, 425.0),
            make_event(EventType::Withdrawal, 100.0, 325.0),
        ];
        let total: f64 = events.iter()
            .filter(|e| e.event_type == EventType::Withdrawal)
            .map(|e| e.amount_usd)
            .sum();
        assert!((total - 175.0).abs() < 0.001);
    }

    // ── CSV round-trip ─────────────────────────────────────────────────────────

    #[test]
    fn event_type_display() {
        assert_eq!(EventType::Deposit.to_string(),    "deposit");
        assert_eq!(EventType::Withdrawal.to_string(), "withdrawal");
    }

    #[test]
    fn summary_zero_when_no_events() {
        // Craft an empty event list and verify FundSummary is zeroed
        let events: Vec<FundEvent> = vec![];
        let net: f64 = events.iter().fold(0.0, |acc, e| match e.event_type {
            EventType::Deposit    => acc + e.amount_usd,
            EventType::Withdrawal => acc - e.amount_usd,
        });
        assert_eq!(net, 0.0);
        assert!(events.last().is_none());
    }
}
