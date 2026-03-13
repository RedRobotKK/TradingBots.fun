//! 🧠 Professional Quant Frameworks
//! 8 frameworks used by institutional traders, embedded in every trade decision
//! Volatility Regime, Order Flow, Multi-Timeframe, Kelly Criterion, Drawdown Management,
//! Strategy Attribution, Volatility Scaling, Monte Carlo Robustness

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ============================================================================
// 1. VOLATILITY REGIME DETECTION
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum VolatilityRegime {
    Calm,        // ATR < 0.5%
    Normal,      // ATR 0.5-2%
    Volatile,    // ATR 2-5%
    Panic,       // ATR 5%+
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolatilityAnalysis {
    pub atr_pct: f64,                    // Current ATR as % of price
    pub regime: VolatilityRegime,
    pub sizing_multiplier: f64,          // How much to scale position
    pub should_trade: bool,              // Is it safe to trade?
    pub rationale: String,
}

impl VolatilityAnalysis {
    /// Classify market volatility and determine sizing
    pub fn analyze(atr_pct: f64) -> Self {
        let (regime, sizing_multiplier, should_trade, rationale) = match atr_pct {
            x if x < 0.5 => (
                VolatilityRegime::Calm,
                1.0, // Trade full size
                true,
                "Calm market - use full position sizing".to_string(),
            ),
            x if x < 2.0 => (
                VolatilityRegime::Normal,
                0.75, // Trade 75% normal size
                true,
                format!("Normal volatility ({:.2}% ATR) - standard sizing", atr_pct),
            ),
            x if x < 5.0 => (
                VolatilityRegime::Volatile,
                0.5, // Trade half normal size
                true,
                format!("High volatility ({:.2}% ATR) - reduce sizing 50%", atr_pct),
            ),
            _ => (
                VolatilityRegime::Panic,
                0.25, // Trade 25% normal size
                false, // Only extreme setups
                format!("PANIC volatility ({:.2}% ATR) - only trade extremes", atr_pct),
            ),
        };

        Self {
            atr_pct,
            regime,
            sizing_multiplier,
            should_trade,
            rationale,
        }
    }
}

// ============================================================================
// 2. MULTI-TIMEFRAME CONFLUENCE
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum TimeframeSignal {
    StrongBuy,
    Buy,
    Neutral,
    Sell,
    StrongSell,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiTimeframeAnalysis {
    pub daily_signal: TimeframeSignal,
    pub four_hour_signal: TimeframeSignal,
    pub one_hour_signal: TimeframeSignal,
    pub five_min_signal: TimeframeSignal,

    pub confluence_score: f64,           // 0-1: how aligned are signals?
    pub direction_bias: Option<bool>,    // true = bullish, false = bearish, None = conflict
    pub confidence_level: f64,           // Final confidence after alignment check
}

impl MultiTimeframeAnalysis {
    pub fn new(
        daily: TimeframeSignal,
        four_h: TimeframeSignal,
        one_h: TimeframeSignal,
        five_m: TimeframeSignal,
    ) -> Self {
        let mut bullish_count = 0;
        let mut bearish_count = 0;

        // Helper to count alignment
        let check_alignment = |sig1: TimeframeSignal, sig2: TimeframeSignal| -> (bool, bool) {
            let sig1_bullish = matches!(sig1, TimeframeSignal::StrongBuy | TimeframeSignal::Buy);
            let sig2_bullish = matches!(sig2, TimeframeSignal::StrongBuy | TimeframeSignal::Buy);
            let sig1_bearish = matches!(sig1, TimeframeSignal::StrongSell | TimeframeSignal::Sell);
            let sig2_bearish = matches!(sig2, TimeframeSignal::StrongSell | TimeframeSignal::Sell);

            if sig1_bullish && sig2_bullish {
                (true, false)
            } else if sig1_bearish && sig2_bearish {
                (true, false)
            } else if (sig1_bullish && sig2_bearish) || (sig1_bearish && sig2_bullish) {
                (false, true)
            } else {
                (false, false)
            }
        };

        // Count signals
        for sig in &[daily, four_h, one_h, five_m] {
            if matches!(sig, TimeframeSignal::StrongBuy | TimeframeSignal::Buy) {
                bullish_count += 1;
            } else if matches!(sig, TimeframeSignal::StrongSell | TimeframeSignal::Sell) {
                bearish_count += 1;
            }
        }

        // Check pairwise alignment
        let (a1, c1) = check_alignment(daily, four_h);
        let (a2, c2) = check_alignment(four_h, one_h);
        let (a3, c3) = check_alignment(one_h, five_m);

        let aligned_signals = (a1 as i32) + (a2 as i32) + (a3 as i32);
        let _conflicting_signals = (c1 as i32) + (c2 as i32) + (c3 as i32);

        // Determine direction bias
        let direction_bias = if bullish_count >= 3 {
            Some(true) // Bullish
        } else if bearish_count >= 3 {
            Some(false) // Bearish
        } else {
            None // Conflicted
        };

        // Calculate confluence score (0-1)
        let confluence_score = (aligned_signals as f64) / 3.0; // 0 to 1

        // Calculate confidence level
        let confidence_level = match direction_bias {
            Some(_) => {
                // Aligned signals: base 0.65 + bonus for alignment
                0.65 + (confluence_score * 0.25)
            }
            None => {
                // Conflicting: reduce confidence
                0.40 + (confluence_score * 0.15)
            }
        };

        Self {
            daily_signal: daily,
            four_hour_signal: four_h,
            one_hour_signal: one_h,
            five_min_signal: five_m,
            confluence_score,
            direction_bias,
            confidence_level,
        }
    }

    pub fn is_strong_confluence(&self) -> bool {
        self.confluence_score >= 0.66 && self.direction_bias.is_some()
    }
}

// ============================================================================
// 3. KELLY CRITERION SIZING
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KellyCriterion {
    pub win_rate: f64,                   // e.g., 0.72 (72%)
    pub loss_rate: f64,                  // e.g., 0.28 (28%)
    pub avg_win: f64,                    // e.g., 1.5 (1.5% per win)
    pub avg_loss: f64,                   // e.g., 1.0 (1.0% per loss)
    pub kelly_fraction: f64,              // e.g., 0.44 (44%)
    pub fractional_kelly: f64,            // e.g., 0.22 (22%, for safety)
    pub recommended_position_size_pct: f64,
}

impl KellyCriterion {
    /// Calculate Kelly Criterion based on historical performance
    /// Formula: f* = (win_rate * avg_win - loss_rate * avg_loss) / avg_win
    pub fn calculate(win_rate: f64, avg_win: f64, avg_loss: f64) -> Self {
        let loss_rate = 1.0 - win_rate;

        // Kelly Criterion formula
        let kelly_fraction = if avg_win > 0.0 {
            (win_rate * avg_win - loss_rate * avg_loss) / avg_win
        } else {
            0.0
        };

        // Safety: Use fractional Kelly (50% of Kelly)
        let fractional_kelly = (kelly_fraction * 0.5).clamp(0.0, 0.25); // Cap at 25%

        Self {
            win_rate,
            loss_rate,
            avg_win,
            avg_loss,
            kelly_fraction,
            fractional_kelly,
            recommended_position_size_pct: fractional_kelly * 100.0,
        }
    }

    pub fn rationale(&self) -> String {
        if self.kelly_fraction <= 0.0 {
            "Negative expectancy - skip trading".to_string()
        } else if self.fractional_kelly < 0.01 {
            format!(
                "Kelly suggests {:.1}% (too small). Use minimum viable position.",
                self.recommended_position_size_pct
            )
        } else {
            format!(
                "Win rate {:.0}% | Avg win {:.2}% | Avg loss {:.2}% | Kelly: {:.1}% (using 50% = {:.1}%)",
                self.win_rate * 100.0,
                self.avg_win,
                self.avg_loss,
                self.kelly_fraction * 100.0,
                self.recommended_position_size_pct
            )
        }
    }
}

// ============================================================================
// 4. DRAWDOWN MANAGEMENT / ANTI-RUIN
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrawdownTracker {
    pub initial_capital: f64,
    pub current_equity: f64,
    pub daily_pnl: f64,
    pub weekly_pnl: f64,
    pub monthly_pnl: f64,

    pub daily_limit: f64,                // e.g., -5% of initial
    pub weekly_limit: f64,               // e.g., -10% of initial
    pub monthly_limit: f64,              // e.g., -15% of initial

    pub daily_limit_hit: bool,
    pub weekly_limit_hit: bool,
    pub monthly_limit_hit: bool,

    pub can_trade: bool,
}

impl DrawdownTracker {
    pub fn new(initial_capital: f64) -> Self {
        Self {
            initial_capital,
            current_equity: initial_capital,
            daily_pnl: 0.0,
            weekly_pnl: 0.0,
            monthly_pnl: 0.0,
            daily_limit: -(initial_capital * 0.05),    // -5%
            weekly_limit: -(initial_capital * 0.10),   // -10%
            monthly_limit: -(initial_capital * 0.15),  // -15%
            daily_limit_hit: false,
            weekly_limit_hit: false,
            monthly_limit_hit: false,
            can_trade: true,
        }
    }

    pub fn update(&mut self, pnl: f64) {
        self.daily_pnl += pnl;
        self.weekly_pnl += pnl;
        self.monthly_pnl += pnl;
        self.current_equity += pnl;

        // Check limits
        self.daily_limit_hit = self.daily_pnl < self.daily_limit;
        self.weekly_limit_hit = self.weekly_pnl < self.weekly_limit;
        self.monthly_limit_hit = self.monthly_pnl < self.monthly_limit;

        // Can trade only if no limits hit
        self.can_trade = !self.daily_limit_hit && !self.weekly_limit_hit && !self.monthly_limit_hit;
    }

    pub fn reset_daily(&mut self) {
        self.daily_pnl = 0.0;
        self.daily_limit_hit = false;
    }

    pub fn reset_weekly(&mut self) {
        self.weekly_pnl = 0.0;
        self.weekly_limit_hit = false;
    }

    pub fn reset_monthly(&mut self) {
        self.monthly_pnl = 0.0;
        self.monthly_limit_hit = false;
    }

    pub fn status_report(&self) -> String {
        format!(
            "Daily: ${:.2}/{:.2} | Weekly: ${:.2}/{:.2} | Monthly: ${:.2}/{:.2}",
            self.daily_pnl,
            self.daily_limit,
            self.weekly_pnl,
            self.weekly_limit,
            self.monthly_pnl,
            self.monthly_limit
        )
    }
}

// ============================================================================
// 5. STRATEGY ATTRIBUTION & WEIGHTING
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyStats {
    pub name: String,
    pub trades_triggered: usize,
    pub trades_won: usize,
    pub trades_lost: usize,
    pub win_rate: f64,
    pub total_pnl: f64,
    pub avg_win: f64,
    pub avg_loss: f64,
    pub weight: f64,                     // 0-1: how much to trust this strategy
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyAttributor {
    pub strategies: HashMap<String, StrategyStats>,
    pub rebalance_frequency: usize,      // Reweight every N trades
    pub trades_since_rebalance: usize,
}

impl StrategyAttributor {
    pub fn new() -> Self {
        Self {
            strategies: HashMap::new(),
            rebalance_frequency: 50, // Reweight every 50 trades
            trades_since_rebalance: 0,
        }
    }

    pub fn record_trade(&mut self, strategy: &str, won: bool, pnl: f64) {
        let stats = self.strategies.entry(strategy.to_string()).or_insert(StrategyStats {
            name: strategy.to_string(),
            trades_triggered: 0,
            trades_won: 0,
            trades_lost: 0,
            win_rate: 0.0,
            total_pnl: 0.0,
            avg_win: 0.0,
            avg_loss: 0.0,
            weight: 0.5, // Start neutral
        });

        stats.trades_triggered += 1;
        if won {
            stats.trades_won += 1;
        } else {
            stats.trades_lost += 1;
        }
        stats.total_pnl += pnl;
        stats.win_rate = stats.trades_won as f64 / stats.trades_triggered as f64;

        self.trades_since_rebalance += 1;

        // Rebalance weights periodically
        if self.trades_since_rebalance >= self.rebalance_frequency {
            self.rebalance_weights();
            self.trades_since_rebalance = 0;
        }
    }

    fn rebalance_weights(&mut self) {
        let min_trades = 5; // Need at least 5 trades for meaningful weight

        // Calculate new weights based on win rate
        for stats in self.strategies.values_mut() {
            if stats.trades_triggered < min_trades {
                stats.weight = 0.5; // Neutral until enough data
            } else {
                // Weight = win rate, clamped between 0.1 and 0.9
                stats.weight = stats.win_rate.clamp(0.1, 0.9);
            }
        }
    }

    pub fn get_weight(&self, strategy: &str) -> f64 {
        self.strategies
            .get(strategy)
            .map(|s| s.weight)
            .unwrap_or(0.5)
    }

    pub fn get_top_strategies(&self, limit: usize) -> Vec<(String, f64)> {
        let mut sorted: Vec<_> = self
            .strategies
            .iter()
            .map(|(name, stats)| (name.clone(), stats.win_rate))
            .collect();
        sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        sorted.into_iter().take(limit).collect()
    }
}

// ============================================================================
// 6. ORDER FLOW ANALYSIS
// ============================================================================

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum OrderFlowSignal {
    StrongBuy,   // Bid/ask ratio > 2.0 (aggressive buyers)
    Buy,         // Bid/ask ratio > 1.5
    Neutral,     // Bid/ask ratio 0.8-1.2
    Sell,        // Bid/ask ratio < 0.67
    StrongSell,  // Bid/ask ratio < 0.5 (aggressive sellers)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderFlowAnalysis {
    pub bid_volume: f64,
    pub ask_volume: f64,
    pub bid_ask_ratio: f64,
    pub signal: OrderFlowSignal,
    pub strength: f64,                   // 0-1: how extreme is the imbalance?
    pub whale_movement: bool,            // Large block order detected?
}

impl OrderFlowAnalysis {
    pub fn analyze(bid_volume: f64, ask_volume: f64) -> Self {
        let bid_ask_ratio = if ask_volume > 0.0 {
            bid_volume / ask_volume
        } else {
            1.0
        };

        let (signal, strength) = match bid_ask_ratio {
            x if x > 2.0 => (OrderFlowSignal::StrongBuy, (x - 2.0).min(1.0)),
            x if x > 1.5 => (OrderFlowSignal::Buy, (x - 1.5) * 0.5),
            x if x < 0.5 => (OrderFlowSignal::StrongSell, (0.5 - x).min(1.0)),
            x if x < 0.67 => (OrderFlowSignal::Sell, (0.67 - x) * 0.5),
            _ => (OrderFlowSignal::Neutral, 0.0),
        };

        Self {
            bid_volume,
            ask_volume,
            bid_ask_ratio,
            signal,
            strength,
            whale_movement: bid_volume > 1000000.0 || ask_volume > 1000000.0,
        }
    }
}

// ============================================================================
// 7. VOLATILITY SCALING
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolatilityScaler {
    pub baseline_atr_pct: f64,           // e.g., 0.8% (normal market)
    pub current_atr_pct: f64,
    pub scaling_factor: f64,              // How much to multiply position size
}

impl VolatilityScaler {
    pub fn new(baseline_atr: f64) -> Self {
        Self {
            baseline_atr_pct: baseline_atr,
            current_atr_pct: baseline_atr,
            scaling_factor: 1.0,
        }
    }

    pub fn update(&mut self, current_atr: f64) {
        self.current_atr_pct = current_atr;
        // Scale inversely: higher volatility = smaller positions
        self.scaling_factor = (self.baseline_atr_pct / current_atr).clamp(0.25, 1.5);
    }

    pub fn scale_position(&self, base_position_size: f64) -> f64 {
        base_position_size * self.scaling_factor
    }

    pub fn rationale(&self) -> String {
        format!(
            "ATR: {:.2}% (baseline: {:.2}%) → scale position {:.0}%",
            self.current_atr_pct,
            self.baseline_atr_pct,
            self.scaling_factor * 100.0
        )
    }
}

// ============================================================================
// 8. MONTE CARLO ROBUSTNESS
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonteCarloResult {
    pub simulations_run: usize,
    pub profitable_simulations: usize,
    pub win_rate: f64,                   // % of sims profitable
    pub avg_return: f64,
    pub worst_return: f64,
    pub best_return: f64,
    pub avg_max_drawdown: f64,
    pub is_robust: bool,                 // > 80% sims profitable = robust
}

impl MonteCarloResult {
    pub fn new(returns: Vec<f64>, drawdowns: Vec<f64>) -> Self {
        let simulations_run = returns.len();
        let profitable_simulations = returns.iter().filter(|&&r| r > 0.0).count();
        let win_rate = profitable_simulations as f64 / simulations_run as f64;

        let avg_return = returns.iter().sum::<f64>() / simulations_run as f64;
        let worst_return = returns
            .iter()
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .copied()
            .unwrap_or(0.0);
        let best_return = returns
            .iter()
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .copied()
            .unwrap_or(0.0);

        let avg_max_drawdown = drawdowns.iter().sum::<f64>() / drawdowns.len() as f64;
        let is_robust = win_rate >= 0.80;

        Self {
            simulations_run,
            profitable_simulations,
            win_rate,
            avg_return,
            worst_return,
            best_return,
            avg_max_drawdown,
            is_robust,
        }
    }

    pub fn verdict(&self) -> String {
        if self.is_robust {
            format!(
                "✅ ROBUST: {:.0}% of {} sims profitable. Avg return: {:.2}%",
                self.win_rate * 100.0,
                self.simulations_run,
                self.avg_return
            )
        } else {
            format!(
                "⚠️  FRAGILE: Only {:.0}% of {} sims profitable. Reconsider strategy.",
                self.win_rate * 100.0,
                self.simulations_run
            )
        }
    }
}
