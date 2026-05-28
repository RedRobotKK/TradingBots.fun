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
    /// Health factor: `equity / margin`.
    pub health_factor: f64,
    /// Minimum health factor before trading is halted. Sourced from config
    /// (`MIN_HEALTH_FACTOR` env var) so small accounts can set a tighter threshold.
    pub min_health_factor: f64,
    /// Realised + unrealised P&L for the current trading day (UTC).
    pub daily_pnl: f64,
    /// Maximum allowed daily loss before trading is halted.
    pub daily_loss_limit: f64,
}

impl Account {
    /// Returns `true` when the account is safe to trade.
    ///
    /// Requires `health_factor > min_health_factor` AND `daily_pnl > -daily_loss_limit`.
    /// Both thresholds are sourced from config so they scale with account size.
    pub fn is_healthy(&self) -> bool {
        self.health_factor > self.min_health_factor && self.daily_pnl > -self.daily_loss_limit
    }
}

// ─────────────────────────── Live-mode gate ───────────────────────────────────

/// Minimum confidence for a LONG entry in live mode.
const MIN_LONG_CONFIDENCE: f64 = 0.72;  // matches paper-mode MIN_CONFIDENCE in main.rs

/// Minimum confidence for a SHORT entry in live mode.
/// Raised vs LONG: alts bounce fast, catching shorts wrong-footed.
/// Live data: 11,603 SHORT trades, 50.0% WR, -$282K (Jan–May 2026).
const MIN_SHORT_CONFIDENCE: f64 = 0.76;

/// Final pre-order gate for live (non-paper) trading.
///
/// Returns `Ok(true)` only when ALL of the following hold:
///   - Confidence ≥ MIN_LONG_CONFIDENCE (BUY) or MIN_SHORT_CONFIDENCE (SELL)
///   - Account health factor > `account.min_health_factor`
///   - Daily loss limit has NOT been breached
pub fn should_trade(decision: &Decision, account: &Account) -> Result<bool> {
    let min_conf = if decision.action == "SELL" { MIN_SHORT_CONFIDENCE } else { MIN_LONG_CONFIDENCE };
    if decision.confidence < min_conf {
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
