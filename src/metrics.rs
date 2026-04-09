//! Real-time risk-adjusted performance metrics.
//!
//! Calculated on every cycle from the closed-trade history.  Results are
//! surfaced on the dashboard and fed back into position sizing.

use crate::web_dashboard::ClosedTrade;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PerformanceMetrics {
    // Risk-adjusted return ratios
    pub sharpe: f64,  // mean_ret / std_ret  (higher = better risk-adjusted)
    pub sortino: f64, // mean_ret / downside_std  (ignores upside volatility)

    // Expectancy & edge
    pub expectancy: f64,    // avg expected P&L per trade as % of position
    pub profit_factor: f64, // gross profit / gross loss  (>1.5 = good)
    pub avg_win_pct: f64,
    pub avg_loss_pct: f64,
    pub win_rate: f64,
    /// Win rate over the most recent 20 closed trades only.
    /// More reactive than the cumulative `win_rate` — catches intraday edge
    /// degradation that gets diluted when early high-quality trades pad the
    /// all-time count.
    pub rolling_win_rate: f64,

    // Drawdown
    pub max_drawdown: f64, // peak-to-trough cumulative P&L %
    pub current_dd: f64,   // drawdown from most recent peak

    // Summary counts
    pub total_trades: usize,
    pub wins: usize,
    pub losses: usize,
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
        let std_dev = variance.sqrt();

        // Downside std-dev (Sortino denominator) – only uses negative returns
        let downside: Vec<f64> = returns.iter().filter(|&&r| r < 0.0).copied().collect();
        let downside_dev = if downside.is_empty() {
            1e-9 // avoid div-by-zero; effectively infinite Sortino
        } else {
            let dv = downside.iter().map(|r| r.powi(2)).sum::<f64>() / downside.len() as f64;
            dv.sqrt()
        };

        let sharpe = if std_dev > 1e-10 {
            mean_ret / std_dev
        } else {
            0.0
        };
        let sortino = if downside_dev > 1e-10 {
            mean_ret / downside_dev
        } else {
            0.0
        };

        // Win/loss buckets
        let win_pcts: Vec<f64> = trades
            .iter()
            .filter(|t| t.pnl > 0.0)
            .map(|t| t.pnl_pct)
            .collect();
        let loss_pcts: Vec<f64> = trades
            .iter()
            .filter(|t| t.pnl <= 0.0)
            .map(|t| t.pnl_pct.abs())
            .collect();

        let wins = win_pcts.len();
        let losses = loss_pcts.len();

        let win_rate = wins as f64 / n as f64;

        // Rolling win rate — last 20 trades only.  More responsive than the
        // cumulative figure: early high-quality trades can inflate win_rate for
        // hundreds of cycles, masking a live edge collapse.
        let rolling_window = 20_usize;
        let recent: Vec<&ClosedTrade> = trades.iter().rev().take(rolling_window).collect();
        let recent_wins = recent.iter().filter(|t| t.pnl > 0.0).count();
        let rolling_win_rate = recent_wins as f64 / recent.len() as f64;
        let avg_win_pct = if wins > 0 {
            win_pcts.iter().sum::<f64>() / wins as f64
        } else {
            0.0
        };
        let avg_loss_pct = if losses > 0 {
            loss_pcts.iter().sum::<f64>() / losses as f64
        } else {
            0.0
        };

        // Expectancy: per-trade expected return %
        let expectancy = win_rate * avg_win_pct - (1.0 - win_rate) * avg_loss_pct;

        // Profit factor: gross profit / gross loss
        let gross_profit: f64 = win_pcts.iter().sum();
        let gross_loss: f64 = loss_pcts.iter().sum();
        let profit_factor = if gross_loss > 0.0 {
            gross_profit / gross_loss
        } else {
            f64::INFINITY
        };

        // Max drawdown from cumulative P&L curve
        let mut cum = 0.0f64;
        let mut peak = 0.0f64;
        let mut max_dd = 0.0f64;
        for t in trades {
            cum += t.pnl_pct;
            if cum > peak {
                peak = cum;
            }
            let dd = peak - cum;
            if dd > max_dd {
                max_dd = dd;
            }
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
            rolling_win_rate,
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
        // Guard covers two sentinel cases:
        // 1. < 5 trades: not enough history for meaningful Kelly.
        // 2. avg_loss_pct <= 0: either no losses yet (100% win rate, b = ∞) or
        //    a degenerate edge case — clamp to the max half-Kelly cap instead
        //    of dividing by zero and producing NaN/Infinity.
        if self.total_trades < 5 || self.avg_loss_pct <= 0.0 {
            return if self.total_trades < 5 { -1.0 } else { 0.15 }; // 100% WR → max cap
        }
        let b = self.avg_win_pct / self.avg_loss_pct; // win-to-loss size ratio
        let full_kelly = self.win_rate - (1.0 - self.win_rate) / b;
        let half_kelly = full_kelly / 2.0; // half-Kelly for robustness
        half_kelly.clamp(0.01, 0.15)
    }

    /// True when the circuit breaker should be active — triggers defensive sizing.
    ///
    /// Two independent conditions:
    /// 1. `current_dd > 8%` — active drawdown from the most recent equity peak.
    ///    Recovers automatically once drawdown falls below `CB_RESET_THRESHOLD` (5%).
    /// 2. `max_drawdown > 30%` — a historical peak-to-trough loss this large
    ///    indicates a structural risk event occurred (e.g. the 73.93% spike seen
    ///    in monitoring).  The breaker stays permanently on until the bot is
    ///    manually reviewed and the stat is reset; `current_dd` recovering to 0
    ///    does NOT clear this condition.
    ///
    /// For dashboard display purposes only — uses the conservative $500+ thresholds.
    /// For actual trading decisions use `in_circuit_breaker_for_capital()`.
    pub fn in_circuit_breaker(&self) -> bool {
        self.in_circuit_breaker_for_capital(1_000.0) // display: use standard thresholds
    }

    /// Circuit breaker check scaled to account size, matching `execute_paper_trade`.
    ///
    /// The `current_dd` check mirrors the scaled on/off thresholds used in the
    /// trading loop.  The permanent `max_drawdown` flag is also scaled:
    ///
    ///  ≤ $25  : max_dd alarm at 60%  (30% = $3 on a $10 account — too noisy)
    ///  $26–100: max_dd alarm at 50%
    ///  $101–500: max_dd alarm at 40%
    ///  $500+  : 30% — original conservative threshold unchanged
    pub fn in_circuit_breaker_for_capital(&self, initial_capital: f64) -> bool {
        let (current_dd_thresh, max_dd_thresh) = if initial_capital <= 25.0 {
            (20.0, 60.0)
        } else if initial_capital <= 100.0 {
            (15.0, 50.0)
        } else if initial_capital <= 500.0 {
            (12.0, 40.0)
        } else {
            (8.0, 30.0)
        };
        self.current_dd > current_dd_thresh || self.max_drawdown > max_dd_thresh
    }

    /// Sharpe-based size multiplier. Layered on top of Kelly (or fallback tiers).
    pub fn size_multiplier(&self) -> f64 {
        if self.total_trades < 10 {
            return 1.0; // not enough data for Sharpe to be meaningful — neutral
        }
        // Circuit breaker overrides everything
        if self.in_circuit_breaker() {
            return 0.35;
        }
        match self.sharpe {
            s if s > 2.5 => 1.25,
            s if s > 1.5 => 1.10,
            s if s > 0.5 => 1.00,
            s if s > 0.0 => 0.90, // was 0.80 — less punishing for neutral Sharpe
            s if s > -0.5 => 0.75, // was 0.60 — floor raised
            _ => 0.60,            // was 0.40 — never crush position size below 60%
        }
    }

    /// Colour class for dashboard display.
    pub fn sharpe_class(&self) -> &'static str {
        match self.sharpe {
            s if s > 1.0 => "positive",
            s if s > 0.0 => "neutral",
            _ => "negative",
        }
    }

    /// Dynamic confidence floor based on performance metrics.
    ///
    /// Only applies after 10+ trades — before that, metrics are noisy and
    /// punishing early trades creates a self-fulfilling bad-data loop.
    ///
    /// Starts with `min_confidence` and adds penalties if metrics are weak:
    /// - If profit_factor < 1.0:      +0.05 (consistently losing)
    /// - If expectancy < 0.001:       +0.03 (near-zero expected return)
    /// - If sortino < 0.3:            +0.02 (very poor risk-adjusted return)
    /// - If win_rate < 0.55:          +0.04 (cumulative edge below threshold)
    /// - If rolling_win_rate < 0.50:  +0.04 (recent 20-trade window is coin-flip)
    ///
    /// Total adjustment capped at 0.12, final result capped at 0.88.
    ///
    /// Rationale for the new win-rate conditions: the existing penalties only
    /// fired when profit_factor dropped below 1.0 (a loss-making bot).  Live
    /// monitoring showed the bot's win rate collapsing from 84% → 50% over
    /// ~19 hours while PF stayed above 2.7 the entire time — meaning the floor
    /// mechanism never activated once.  The rolling_win_rate check catches
    /// intraday degradation that cumulative win_rate masks due to early padding.
    pub fn confidence_floor(&self, min_confidence: f64) -> f64 {
        // Don't penalise until there's enough history for metrics to be meaningful
        if self.total_trades < 10 {
            return min_confidence;
        }

        let mut adjustment: f64 = 0.0;

        if self.profit_factor < 1.0 {
            adjustment += 0.05; // genuinely losing, not just early-phase
        }
        if self.expectancy < 0.001 {
            adjustment += 0.03;
        }
        if self.sortino < 0.3 {
            adjustment += 0.02;
        }
        // Win-rate gates — react to edge decay well before PF crosses 1.0.
        // These are the conditions that were blind to the 84%→50% collapse seen
        // in live monitoring: PF stayed above 2.7 the entire time so none of the
        // above fired, but the edge had clearly degraded.
        if self.win_rate < 0.55 {
            adjustment += 0.04; // cumulative edge has decayed significantly
        }
        if self.rolling_win_rate < 0.50 {
            adjustment += 0.04; // recent 20 trades are coin-flip or worse
        }

        adjustment = adjustment.min(0.12); // raised from 0.08 to cover new conditions
        (min_confidence + adjustment).min(0.88)
    }
}
