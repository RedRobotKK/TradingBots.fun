use anyhow::Result;
use serde::{Deserialize, Serialize};
use crate::decision::Decision;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub equity: f64,
    pub margin: f64,
    pub health_factor: f64,
    pub daily_pnl: f64,
    pub daily_loss_limit: f64,
}

impl Account {
    pub fn is_healthy(&self) -> bool {
        self.health_factor > 2.0 && self.daily_pnl > -self.daily_loss_limit
    }
}

pub fn should_trade(decision: &Decision, account: &Account) -> Result<bool> {
    // Circuit breaker checks
    if decision.confidence < 0.65 {
        return Ok(false); // Too low confidence
    }

    if !account.is_healthy() {
        return Ok(false); // Account unhealthy
    }

    if account.daily_pnl < -account.daily_loss_limit {
        return Ok(false); // Daily loss limit hit
    }

    Ok(true)
}

pub fn calculate_position_size(
    confidence: f64,
    equity: f64,
    volatility: f64,
) -> f64 {
    let position_pct = match confidence {
        c if c > 0.90 => 0.15,
        c if c > 0.80 => 0.12,
        c if c > 0.70 => 0.08,
        c if c > 0.65 => 0.05,
        _ => 0.0,
    };

    equity * position_pct
}

pub fn calculate_leverage(volatility: f64) -> f64 {
    match volatility {
        v if v < 2.0 => 15.0,
        v if v < 4.0 => 10.0,
        _ => 5.0,
    }
}

pub fn calculate_stop_loss(
    entry: f64,
    support: f64,
    atr: f64,
) -> f64 {
    let atr_stop = entry - (atr * 1.5);
    f64::max(support, atr_stop)
}
