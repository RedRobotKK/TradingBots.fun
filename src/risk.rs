//! Runtime risk guards — account health checks before live order placement.
//!
//! This module is intentionally lean: position-sizing and leverage calculations
//! live in `main.rs` (Kelly / heat model) and `decision.rs` (confidence-scaled
//! leverage).  `should_trade()` is the final gate in non-paper mode.

use crate::decision::Decision;
use anyhow::Result;
use serde::{Deserialize, Serialize};

// ─────────────────────────── Account health ───────────────────────────────────

/// Snapshot of account-level risk metrics fetched from the exchange.
///
/// In paper mode these values are ignored; in live mode they gate every order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    /// Total account equity in USD.
    pub equity: f64,
    /// Margin currently in use (USD).
    pub margin: f64,
    /// Health factor: `equity / margin`.  Safe threshold is > 2.0.
    pub health_factor: f64,
    /// Realised + unrealised P&L for the current trading day (UTC).
    pub daily_pnl: f64,
    /// Maximum allowed daily loss before trading is halted.
    pub daily_loss_limit: f64,
}

impl Account {
    /// Returns `true` when the account is safe to trade.
    ///
    /// Requires `health_factor > 2.0` AND `daily_pnl > -daily_loss_limit`.
    pub fn is_healthy(&self) -> bool {
        self.health_factor > 2.0 && self.daily_pnl > -self.daily_loss_limit
    }
}

// ─────────────────────────── Live-mode gate ───────────────────────────────────

/// Final pre-order gate for live (non-paper) trading.
///
/// Returns `Ok(true)` only when ALL of the following hold:
///   - Signal confidence ≥ 0.68 (matches the paper-mode threshold in `main.rs`)
///   - Account health factor > 2.0
///   - Daily loss limit has NOT been breached
///
/// On `Ok(false)` the caller should silently skip the order.
pub fn should_trade(decision: &Decision, account: &Account) -> Result<bool> {
    // Confidence gate — must match the paper-mode threshold in execute_paper_trade().
    // Historical back-test: signals below 0.68 yielded 0W/14L in choppy markets.
    if decision.confidence < 0.68 {
        return Ok(false);
    }

    // Account health: reject if health factor or daily loss limit is breached
    if !account.is_healthy() {
        return Ok(false);
    }

    // Redundant daily-loss check (is_healthy covers it, but explicit is clearer)
    if account.daily_pnl < -account.daily_loss_limit {
        return Ok(false);
    }

    Ok(true)
}
