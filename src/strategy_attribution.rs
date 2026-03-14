//! 🎯 Strategy Attribution & Performance Tracking
//! Tracks which strategies are actually making money in crypto trading
//! Essential for understanding which technical indicators work best
//!
//! Implements:
//! - Strategy signal tracking (logs each strategy's signals)
//! - Performance metrics (win rate, profit factor, Sharpe ratio)
//! - Market regime analysis (bullish/bearish/ranging performance)
//! - Strategy correlation (which combos work together)
//! - Attribution reports (P&L per strategy)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Individual trade with strategy attribution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttributedTrade {
    pub timestamp: i64,
    pub entry_price: f64,
    pub exit_price: f64,
    pub quantity: f64,
    pub pnl_usd: f64,
    pub pnl_pct: f64,
    pub duration_minutes: u32,
    pub is_win: bool,
    pub contributing_strategies: Vec<String>,  // Which strategies signaled this trade
    pub primary_strategy: String,               // Highest confidence signal
    pub confluence_count: usize,                // How many strategies aligned
    pub market_regime: MarketRegime,
}

/// Market regime classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MarketRegime {
    StrongBullish,   // RSI > 60, price > MA50, ADX > 30
    Bullish,         // Price above MA20, volume normal
    Neutral,         // No clear direction
    Bearish,         // Price below MA20, volume normal
    StrongBearish,   // RSI < 40, price < MA50, ADX > 30
    VeryHigh,        // Extreme fear/greed index
}

impl MarketRegime {
    pub fn as_string(&self) -> String {
        match self {
            MarketRegime::StrongBullish => "Strong Bullish".to_string(),
            MarketRegime::Bullish => "Bullish".to_string(),
            MarketRegime::Neutral => "Neutral".to_string(),
            MarketRegime::Bearish => "Bearish".to_string(),
            MarketRegime::StrongBearish => "Strong Bearish".to_string(),
            MarketRegime::VeryHigh => "Extreme Volatility".to_string(),
        }
    }
}

/// Performance metrics for a single strategy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyMetrics {
    pub name: String,
    pub total_signals: u32,           // Total times this strategy signaled
    pub winning_trades: u32,          // How many signals led to winning trades
    pub losing_trades: u32,           // How many signals led to losses
    pub win_rate: f64,                // Percentage (0.0-1.0)
    pub profit_factor: f64,           // Total wins / Total losses
    pub avg_win_usd: f64,             // Average profit per winning trade
    pub avg_loss_usd: f64,            // Average loss per losing trade
    pub avg_duration_minutes: u32,    // How long typical trade lasts
    pub sharpe_ratio: f64,            // Risk-adjusted return metric
    pub total_pnl_usd: f64,           // Total profit/loss contributed
    pub max_consecutive_wins: u32,    // Best winning streak
    pub max_consecutive_losses: u32,  // Worst losing streak
}

impl Default for StrategyMetrics {
    fn default() -> Self {
        StrategyMetrics {
            name: String::new(),
            total_signals: 0,
            winning_trades: 0,
            losing_trades: 0,
            win_rate: 0.0,
            profit_factor: 0.0,
            avg_win_usd: 0.0,
            avg_loss_usd: 0.0,
            avg_duration_minutes: 0,
            sharpe_ratio: 0.0,
            total_pnl_usd: 0.0,
            max_consecutive_wins: 0,
            max_consecutive_losses: 0,
        }
    }
}

/// Market regime specific metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegimeSpecificMetrics {
    pub regime: MarketRegime,
    pub total_signals: u32,
    pub win_rate: f64,
    pub avg_pnl: f64,
    pub best_strategy: String,
    pub worst_strategy: String,
}

/// Strategy correlation (which strategies work well together)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyCorrelation {
    pub strategy_pair: (String, String),     // Two strategies
    pub co_signal_count: u32,                // How often they signal together
    pub joint_win_rate: f64,                 // Win rate when both signal
    pub correlation_coefficient: f64,        // Statistical correlation (-1.0 to 1.0)
}

/// Main strategy attribution tracker
pub struct StrategyAttributor {
    trades: Vec<AttributedTrade>,
    strategy_metrics: HashMap<String, StrategyMetrics>,
    regime_metrics: HashMap<MarketRegime, RegimeSpecificMetrics>,
    correlations: Vec<StrategyCorrelation>,
}

impl StrategyAttributor {
    /// Create new attribution tracker
    pub fn new() -> Self {
        StrategyAttributor {
            trades: vec![],
            strategy_metrics: HashMap::new(),
            regime_metrics: HashMap::new(),
            correlations: vec![],
        }
    }

    /// Record a trade with attribution
    pub fn record_trade(
        &mut self,
        entry_price: f64,
        exit_price: f64,
        quantity: f64,
        duration_minutes: u32,
        contributing_strategies: Vec<String>,
        primary_strategy: String,
        confluence_count: usize,
        market_regime: MarketRegime,
        timestamp: i64,
    ) {
        let pnl_usd = (exit_price - entry_price) * quantity;
        let pnl_pct = ((exit_price - entry_price) / entry_price) * 100.0;
        let is_win = pnl_usd > 0.0;

        let trade = AttributedTrade {
            timestamp,
            entry_price,
            exit_price,
            quantity,
            pnl_usd,
            pnl_pct,
            duration_minutes,
            is_win,
            contributing_strategies: contributing_strategies.clone(),
            primary_strategy: primary_strategy.clone(),
            confluence_count,
            market_regime,
        };

        // Update strategy metrics
        for strategy_name in &contributing_strategies {
            let metrics = self
                .strategy_metrics
                .entry(strategy_name.clone())
                .or_insert_with(|| StrategyMetrics {
                    name: strategy_name.clone(),
                    ..Default::default()
                });

            metrics.total_signals += 1;
            if is_win {
                metrics.winning_trades += 1;
            } else {
                metrics.losing_trades += 1;
            }
            metrics.total_pnl_usd += pnl_usd;
        }

        self.trades.push(trade);
        self.recalculate_metrics();
    }

    /// Recalculate all performance metrics
    pub fn recalculate_metrics(&mut self) {
        for (strategy_name, metrics) in self.strategy_metrics.iter_mut() {
            if metrics.total_signals == 0 {
                continue;
            }

            // Win rate
            metrics.win_rate = metrics.winning_trades as f64 / metrics.total_signals as f64;

            // Profit factor
            let wins_sum: f64 = self
                .trades
                .iter()
                .filter(|t| t.contributing_strategies.contains(strategy_name) && t.is_win)
                .map(|t| t.pnl_usd)
                .sum();

            let losses_sum: f64 = self
                .trades
                .iter()
                .filter(|t| t.contributing_strategies.contains(strategy_name) && !t.is_win)
                .map(|t| t.pnl_usd.abs())
                .sum();

            if losses_sum > 0.0 {
                metrics.profit_factor = wins_sum / losses_sum;
            } else {
                metrics.profit_factor = if wins_sum > 0.0 { f64::INFINITY } else { 1.0 };
            }

            // Average win/loss
            if metrics.winning_trades > 0 {
                metrics.avg_win_usd = wins_sum / metrics.winning_trades as f64;
            }
            if metrics.losing_trades > 0 {
                metrics.avg_loss_usd = losses_sum / metrics.losing_trades as f64;
            }

            // Average duration
            let avg_duration: u32 = self
                .trades
                .iter()
                .filter(|t| t.contributing_strategies.contains(strategy_name))
                .map(|t| t.duration_minutes)
                .sum::<u32>()
                / metrics.total_signals.max(1);
            metrics.avg_duration_minutes = avg_duration;

            // Sharpe ratio (simplified: return / volatility)
            let strategy_returns: Vec<f64> = self
                .trades
                .iter()
                .filter(|t| t.contributing_strategies.contains(strategy_name))
                .map(|t| t.pnl_pct)
                .collect();

            if strategy_returns.len() > 1 {
                let mean_return = strategy_returns.iter().sum::<f64>() / strategy_returns.len() as f64;
                let variance = strategy_returns
                    .iter()
                    .map(|r| (r - mean_return).powi(2))
                    .sum::<f64>()
                    / strategy_returns.len() as f64;
                let std_dev = variance.sqrt();

                if std_dev > 0.0 {
                    metrics.sharpe_ratio = mean_return / std_dev;
                }
            }
        }
    }

    /// Analyze performance by market regime
    pub fn analyze_by_regime(&mut self) {
        let mut regime_data: HashMap<MarketRegime, Vec<&AttributedTrade>> = HashMap::new();

        for trade in &self.trades {
            regime_data
                .entry(trade.market_regime)
                .or_default()
                .push(trade);
        }

        for (regime, trades) in regime_data {
            if trades.is_empty() {
                continue;
            }

            let total_signals = trades.len() as u32;
            let winning_trades = trades.iter().filter(|t| t.is_win).count() as u32;
            let win_rate = winning_trades as f64 / total_signals as f64;
            let avg_pnl = trades.iter().map(|t| t.pnl_usd).sum::<f64>() / total_signals as f64;

            // Find best and worst strategy in this regime
            let mut strategy_wins: HashMap<String, (u32, u32)> = HashMap::new();
            for trade in &trades {
                let entry = strategy_wins
                    .entry(trade.primary_strategy.clone())
                    .or_insert((0, 0));
                entry.1 += 1;
                if trade.is_win {
                    entry.0 += 1;
                }
            }

            let best_strategy = strategy_wins
                .iter()
                .max_by(|(_, (wins1, total1)), (_, (wins2, total2))| {
                    ((*wins1 as f64) / (*total1 as f64)).partial_cmp(&((*wins2 as f64) / (*total2 as f64))).unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(name, _)| name.clone())
                .unwrap_or_else(|| "N/A".to_string());

            let worst_strategy = strategy_wins
                .iter()
                .min_by(|(_, (wins1, total1)), (_, (wins2, total2))| {
                    ((*wins1 as f64) / (*total1 as f64)).partial_cmp(&((*wins2 as f64) / (*total2 as f64))).unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(name, _)| name.clone())
                .unwrap_or_else(|| "N/A".to_string());

            let metrics = RegimeSpecificMetrics {
                regime,
                total_signals,
                win_rate,
                avg_pnl,
                best_strategy,
                worst_strategy,
            };

            self.regime_metrics.insert(regime, metrics);
        }
    }

    /// Calculate correlation between strategy pairs
    pub fn calculate_correlations(&mut self) {
        let strategy_names: Vec<String> = self.strategy_metrics.keys().cloned().collect();

        for i in 0..strategy_names.len() {
            for j in (i + 1)..strategy_names.len() {
                let strat_a = &strategy_names[i];
                let strat_b = &strategy_names[j];

                // Count co-signals
                let co_signals: Vec<&AttributedTrade> = self
                    .trades
                    .iter()
                    .filter(|t| {
                        t.contributing_strategies.contains(strat_a)
                            && t.contributing_strategies.contains(strat_b)
                    })
                    .collect();

                if co_signals.is_empty() {
                    continue;
                }

                let co_signal_count = co_signals.len() as u32;
                let co_win_rate = co_signals.iter().filter(|t| t.is_win).count() as f64
                    / co_signal_count as f64;

                // Correlation coefficient (simplified: compare individual win rates)
                let strat_a_wr = self
                    .strategy_metrics
                    .get(strat_a)
                    .map(|m| m.win_rate)
                    .unwrap_or(0.0);
                let strat_b_wr = self
                    .strategy_metrics
                    .get(strat_b)
                    .map(|m| m.win_rate)
                    .unwrap_or(0.0);
                let joint_wr = co_win_rate;

                // If combined win rate is higher than individual, positive correlation
                let correlation = if strat_a_wr + strat_b_wr > 0.0 {
                    (joint_wr - (strat_a_wr + strat_b_wr) / 2.0) * 100.0
                } else {
                    0.0
                };

                self.correlations.push(StrategyCorrelation {
                    strategy_pair: (strat_a.clone(), strat_b.clone()),
                    co_signal_count,
                    joint_win_rate: co_win_rate,
                    correlation_coefficient: correlation,
                });
            }
        }

        // Sort by correlation strength (strongest first)
        self.correlations
            .sort_by(|a, b| b.correlation_coefficient.abs().partial_cmp(&a.correlation_coefficient.abs()).unwrap());
    }

    /// Get strategy metrics report
    pub fn get_strategy_report(&self) -> Vec<(String, StrategyMetrics)> {
        let mut report: Vec<_> = self.strategy_metrics.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        // Sort by profit factor (best strategies first)
        report.sort_by(|a, b| b.1.profit_factor.partial_cmp(&a.1.profit_factor).unwrap());
        report
    }

    /// Get best performing strategy in current market
    pub fn best_strategy_for_regime(&self, regime: MarketRegime) -> Option<String> {
        self.regime_metrics
            .get(&regime)
            .map(|m| m.best_strategy.clone())
    }

    /// Get strategy pairs that work well together
    pub fn get_best_correlations(&self) -> Vec<StrategyCorrelation> {
        self.correlations
            .iter()
            .filter(|c| c.correlation_coefficient > 5.0)  // Positive correlation
            .take(5)  // Top 5 pairs
            .cloned()
            .collect()
    }

    /// Get worst performing strategies
    pub fn get_underperformers(&self) -> Vec<String> {
        self.strategy_metrics
            .iter()
            .filter(|(_, m)| m.total_signals > 10 && m.win_rate < 0.45)  // <45% win rate
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Summary statistics
    pub fn summary(&self) -> String {
        let total_trades = self.trades.len();
        let total_wins = self.trades.iter().filter(|t| t.is_win).count();
        let total_pnl: f64 = self.trades.iter().map(|t| t.pnl_usd).sum();

        let avg_confluence = if total_trades > 0 {
            self.trades.iter().map(|t| t.confluence_count).sum::<usize>() / total_trades
        } else {
            0
        };

        format!(
            "📊 STRATEGY ATTRIBUTION SUMMARY\n\
             ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n\
             Total Trades: {}\n\
             Wins: {} ({:.1}%)\n\
             Total P&L: ${:.2}\n\
             Avg Confluence: {} strategies\n\
             Active Strategies: {}\n",
            total_trades,
            total_wins,
            (total_wins as f64 / total_trades.max(1) as f64) * 100.0,
            total_pnl,
            avg_confluence,
            self.strategy_metrics.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strategy_attribution() {
        let mut attributor = StrategyAttributor::new();

        // Simulate some trades
        attributor.record_trade(
            82.0,   // entry
            84.0,   // exit (win)
            10.0,   // quantity
            30,     // duration
            vec!["Mean Reversion".to_string(), "MACD Momentum".to_string()],
            "Mean Reversion".to_string(),
            2,
            MarketRegime::Bullish,
            1708576800,
        );

        attributor.record_trade(
            84.0,   // entry
            83.0,   // exit (loss)
            10.0,
            20,
            vec!["RSI Divergence".to_string()],
            "RSI Divergence".to_string(),
            1,
            MarketRegime::Neutral,
            1708576860,
        );

        let report = attributor.get_strategy_report();
        assert!(!report.is_empty());
        println!("{}", attributor.summary());
    }
}
