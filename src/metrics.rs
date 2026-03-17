//! Real-time risk-adjusted performance metrics.
//!
//! Calculated on every cycle from the closed-trade history.  Results are
//! surfaced on the dashboard and fed back into position sizing.

use serde::{Deserialize, Serialize};
use crate::web_dashboard::ClosedTrade;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PerformanceMetrics {
    // Risk-adjusted return ratios
    pub sharpe:         f64,  // mean_ret / std_ret  (higher = better risk-adjusted)
    pub sortino:        f64,  // mean_ret / downside_std  (ignores upside volatility)

    // Expectancy & edge
    pub expectancy:     f64,  // avg expected P&L per trade as % of position
    pub profit_factor:  f64,  // gross profit / gross loss  (>1.5 = good)
    pub avg_win_pct:    f64,
    pub avg_loss_pct:   f64,
    pub win_rate:       f64,

    // Drawdown
    pub max_drawdown:   f64,  // peak-to-trough cumulative P&L %
    pub current_dd:     f64,  // drawdown from most recent peak

    // Summary counts
    pub total_trades:   usize,
    pub wins:           usize,
    pub losses:         usize,
}

impl PerformanceMetrics {
    /// Re-compute all metrics from the full closed-trade history.
    pub fn calculate(trades: &[ClosedTrade]) -> Self {
        if trades.is_empty() {
            return Self::default();
        }

        let n = trades.len();

        // Returns as fraction of position size (e.g. +0.05 = +5%)
        let returns: Vec<f64> = trades.iter().map(|t| t.pnl_pct / 100.0).collect();

        let mean_ret = returns.iter().sum::<f64>() / n as f64;

        // Total std-dev (Sharpe denominator)
        let variance = returns.iter().map(|r| (r - mean_ret).powi(2)).sum::<f64>() / n as f64;
        let std_dev  = variance.sqrt();

        // Downside std-dev (Sortino denominator) – only uses negative returns
        let downside: Vec<f64> = returns.iter().filter(|&&r| r < 0.0).copied().collect();
        let downside_dev = if downside.is_empty() {
            1e-9 // avoid div-by-zero; effectively infinite Sortino
        } else {
            let dv = downside.iter().map(|r| r.powi(2)).sum::<f64>() / downside.len() as f64;
            dv.sqrt()
        };

        let sharpe  = if std_dev > 1e-10   { mean_ret / std_dev }    else { 0.0 };
        let sortino = if downside_dev > 1e-10 { mean_ret / downside_dev } else { 0.0 };

        // Win/loss buckets
        let win_pcts:  Vec<f64> = trades.iter().filter(|t| t.pnl > 0.0).map(|t| t.pnl_pct).collect();
        let loss_pcts: Vec<f64> = trades.iter().filter(|t| t.pnl <= 0.0).map(|t| t.pnl_pct.abs()).collect();

        let wins   = win_pcts.len();
        let losses = loss_pcts.len();

        let win_rate     = wins as f64 / n as f64;
        let avg_win_pct  = if wins   > 0 { win_pcts.iter().sum::<f64>()  / wins   as f64 } else { 0.0 };
        let avg_loss_pct = if losses > 0 { loss_pcts.iter().sum::<f64>() / losses as f64 } else { 0.0 };

        // Expectancy: per-trade expected return %
        let expectancy = win_rate * avg_win_pct - (1.0 - win_rate) * avg_loss_pct;

        // Profit factor: gross profit / gross loss
        let gross_profit: f64 = win_pcts.iter().sum();
        let gross_loss:   f64 = loss_pcts.iter().sum();
        let profit_factor = if gross_loss > 0.0 { gross_profit / gross_loss } else { f64::INFINITY };

        // Max drawdown from cumulative P&L curve
        let mut cum = 0.0f64;
        let mut peak = 0.0f64;
        let mut max_dd = 0.0f64;
        for t in trades {
            cum += t.pnl_pct;
            if cum > peak { peak = cum; }
            let dd = peak - cum;
            if dd > max_dd { max_dd = dd; }
        }
        // Current drawdown (from last peak)
        let current_dd = (peak - cum).max(0.0);

        PerformanceMetrics {
            sharpe,
            sortino,
            expectancy,
            profit_factor,
            avg_win_pct,
            avg_loss_pct,
            win_rate,
            max_drawdown: max_dd,
            current_dd,
            total_trades: n,
            wins,
            losses,
        }
    }

    /// Half-Kelly optimal fraction of capital to risk per trade.
    ///
    /// Formula: Kelly = p − q/b, where:
    ///   p = win rate, q = loss rate, b = avg_win% / avg_loss%
    ///
    /// Returns −1.0 when insufficient data (<5 trades).
    /// Positive result = fraction of equity to size, clamped [1%, 15%].
    pub fn kelly_fraction(&self) -> f64 {
        if self.total_trades < 5 || self.avg_loss_pct < 0.001 {
            return -1.0; // sentinel: not enough history yet
        }
        let b           = self.avg_win_pct / self.avg_loss_pct; // win-to-loss size ratio
        let full_kelly  = self.win_rate - (1.0 - self.win_rate) / b;
        let half_kelly  = full_kelly / 2.0; // half-Kelly for robustness
        half_kelly.clamp(0.01, 0.15)
    }

    /// True when drawdown from peak exceeds 8% — triggers defensive sizing.
    pub fn in_circuit_breaker(&self) -> bool {
        self.current_dd > 8.0
    }

    /// Sharpe-based size multiplier. Layered on top of Kelly (or fallback tiers).
    pub fn size_multiplier(&self) -> f64 {
        if self.total_trades < 3 {
            return 1.0; // not enough data yet, use neutral
        }
        // Circuit breaker overrides everything
        if self.in_circuit_breaker() {
            return 0.35;
        }
        match self.sharpe {
            s if s > 2.5  => 1.25,
            s if s > 1.5  => 1.10,
            s if s > 0.5  => 1.00,
            s if s > 0.0  => 0.80,
            s if s > -0.5 => 0.60,
            _             => 0.40,
        }
    }

    /// Colour class for dashboard display.
    pub fn sharpe_class(&self) -> &'static str {
        match self.sharpe {
            s if s > 1.0  => "positive",
            s if s > 0.0  => "neutral",
            _             => "negative",
        }
    }

    /// Dynamic confidence floor based on performance metrics.
    ///
    /// Starts with `min_confidence` and adds penalties if metrics are weak:
    /// - If profit_factor < 1.2: +0.07 (not enough upside per downside)
    /// - If expectancy < 0.003: +0.05 (expected return too small)
    /// - If sortino < 0.5: +0.03 (poor downside-adjusted return)
    ///
    /// Total adjustment capped at 0.12, final result capped at 0.92.
    pub fn confidence_floor(&self, min_confidence: f64) -> f64 {
        let mut adjustment = 0.0;

        if self.profit_factor < 1.2 {
            adjustment += 0.07;
        }
        if self.expectancy < 0.003 {
            adjustment += 0.05;
        }
        if self.sortino < 0.5 {
            adjustment += 0.03;
        }

        adjustment = adjustment.min(0.12);
        (min_confidence + adjustment).min(0.92)
    }
}
