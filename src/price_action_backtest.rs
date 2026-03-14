//! 📊 PRICE ACTION PATTERN BACKTESTING FRAMEWORK
//!
//! Backtests institutional price action patterns against historical data
//! Measures win rate, profit factor, and pattern reliability

use crate::price_action::{PriceActionDetector, Candle, PriceActionPattern, PatternType};
use serde::{Deserialize, Serialize};

/// Result of a single pattern trade
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternTrade {
    /// Pattern type that triggered
    pub pattern_type: PatternType,

    /// Entry price
    pub entry_price: f64,

    /// Exit price
    pub exit_price: f64,

    /// Profit/Loss in pips or absolute
    pub pnl: f64,

    /// Win or loss
    pub is_win: bool,

    /// Risk taken (distance to stop)
    pub risk: f64,

    /// Reward gained
    pub reward: f64,

    /// Risk/Reward ratio
    pub rr_ratio: f64,

    /// Pattern confidence
    pub confidence: f64,

    /// Candle at entry
    pub entry_candle: usize,

    /// Candle at exit
    pub exit_candle: usize,

    /// Bars held
    pub bars_held: usize,
}

/// Statistics for a specific pattern type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternStatistics {
    /// Pattern type
    pub pattern_type: PatternType,

    /// Total trades
    pub total_trades: usize,

    /// Winning trades
    pub winning_trades: usize,

    /// Losing trades
    pub losing_trades: usize,

    /// Win rate (0-100)
    pub win_rate: f64,

    /// Profit factor (wins / losses)
    pub profit_factor: f64,

    /// Average risk
    pub avg_risk: f64,

    /// Average reward
    pub avg_reward: f64,

    /// Average R:R ratio
    pub avg_rr_ratio: f64,

    /// Total P&L
    pub total_pnl: f64,

    /// Largest win
    pub largest_win: f64,

    /// Largest loss
    pub largest_loss: f64,

    /// Average bars held
    pub avg_bars_held: f64,

    /// Average confidence
    pub avg_confidence: f64,
}

/// Complete backtest results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceActionBacktestResults {
    /// All trades
    pub trades: Vec<PatternTrade>,

    /// Statistics by pattern type
    pub pattern_stats: Vec<PatternStatistics>,

    /// Overall statistics
    pub overall_stats: PatternStatistics,

    /// Period analyzed
    pub period: String,

    /// Total candles analyzed
    pub total_candles: usize,
}

/// Price Action Pattern Backtester
pub struct PriceActionBacktester {
    /// Historical candle data
    candles: Vec<Candle>,

    /// Pattern detector
    detector: PriceActionDetector,

    /// All trades
    trades: Vec<PatternTrade>,

    /// Current pattern (if any)
    current_pattern: Option<PriceActionPattern>,

    /// Entry candle (if trade active)
    entry_candle_idx: Option<usize>,
}

impl PriceActionBacktester {
    pub fn new(candles: Vec<Candle>) -> Self {
        Self {
            candles,
            detector: PriceActionDetector::new(),
            trades: Vec::new(),
            current_pattern: None,
            entry_candle_idx: None,
        }
    }

    /// Run the backtest
    pub fn run(&mut self) -> PriceActionBacktestResults {
        self.trades.clear();

        for (idx, &candle) in self.candles.iter().enumerate() {
            // Detect patterns
            let new_patterns = self.detector.add_candle(candle);

            // Check if current trade should exit
            if let Some(ref pattern) = self.current_pattern {
                if let Some(entry_idx) = self.entry_candle_idx {
                    let exit_price = self.evaluate_exit(pattern, candle);

                    if let Some(exit) = exit_price {
                        let trade = self.create_trade(pattern, entry_idx, idx, exit);
                        self.trades.push(trade);
                        self.current_pattern = None;
                        self.entry_candle_idx = None;
                    }
                }
            }

            // Enter new pattern if no active trade
            if self.current_pattern.is_none() && !new_patterns.is_empty() {
                let best = new_patterns
                    .iter()
                    .max_by(|a, b| a.confidence.partial_cmp(&b.confidence).unwrap())
                    .unwrap();

                // Only take patterns with minimum confidence
                if best.confidence >= best.pattern_type.min_confidence() {
                    self.current_pattern = Some(best.clone());
                    self.entry_candle_idx = Some(idx);
                }
            }
        }

        // Close any remaining open trade
        if let Some(ref pattern) = self.current_pattern {
            if let Some(entry_idx) = self.entry_candle_idx {
                let last_candle = self.candles[self.candles.len() - 1];
                let trade = self.create_trade(pattern, entry_idx, self.candles.len() - 1, last_candle.close);
                self.trades.push(trade);
            }
        }

        self.calculate_results()
    }

    /// Evaluate if trade should exit
    fn evaluate_exit(&self, pattern: &PriceActionPattern, candle: Candle) -> Option<f64> {
        // Exit on stop loss hit
        if (pattern.stop_zone - candle.low).abs() < 0.01
            || (pattern.stop_zone - candle.high).abs() < 0.01
        {
            return Some(pattern.stop_zone);
        }

        // Exit on target hit
        for target in &pattern.targets {
            if (candle.high >= *target && candle.close < *target) || candle.close >= *target {
                return Some(*target);
            }
        }

        // Exit on reversal (opposite direction move)
        if pattern.entry_price > pattern.stop_zone {
            // Long trade
            if candle.close < pattern.entry_price - (pattern.entry_price - pattern.stop_zone) {
                return Some(candle.close);
            }
        } else {
            // Short trade
            if candle.close > pattern.entry_price + (pattern.stop_zone - pattern.entry_price) {
                return Some(candle.close);
            }
        }

        None
    }

    /// Create a completed trade
    fn create_trade(
        &self,
        pattern: &PriceActionPattern,
        entry_idx: usize,
        exit_idx: usize,
        exit_price: f64,
    ) -> PatternTrade {
        let entry = pattern.entry_price;
        let pnl = exit_price - entry;
        let risk = (pattern.stop_zone - entry).abs();
        let reward = (exit_price - entry).abs();
        let rr_ratio = if risk > 0.0 { reward / risk } else { 0.0 };

        PatternTrade {
            pattern_type: pattern.pattern_type,
            entry_price: entry,
            exit_price,
            pnl,
            is_win: pnl > 0.0,
            risk,
            reward,
            rr_ratio,
            confidence: pattern.confidence,
            entry_candle: entry_idx,
            exit_candle: exit_idx,
            bars_held: exit_idx.saturating_sub(entry_idx),
        }
    }

    /// Calculate backtest results
    fn calculate_results(&self) -> PriceActionBacktestResults {
        let mut pattern_stats_map: std::collections::HashMap<PatternType, Vec<&PatternTrade>> =
            std::collections::HashMap::new();

        for trade in &self.trades {
            pattern_stats_map
                .entry(trade.pattern_type)
                .or_default()
                .push(trade);
        }

        let mut pattern_stats = Vec::new();

        for (pattern_type, trades) in pattern_stats_map {
            let stats = self.calculate_pattern_stats(pattern_type, trades);
            pattern_stats.push(stats);
        }

        let overall_stats = self.calculate_overall_stats();

        PriceActionBacktestResults {
            trades: self.trades.clone(),
            pattern_stats,
            overall_stats,
            period: format!("{} candles", self.candles.len()),
            total_candles: self.candles.len(),
        }
    }

    /// Calculate statistics for a specific pattern
    fn calculate_pattern_stats(&self, pattern_type: PatternType, trades: Vec<&PatternTrade>) -> PatternStatistics {
        if trades.is_empty() {
            return PatternStatistics {
                pattern_type,
                total_trades: 0,
                winning_trades: 0,
                losing_trades: 0,
                win_rate: 0.0,
                profit_factor: 0.0,
                avg_risk: 0.0,
                avg_reward: 0.0,
                avg_rr_ratio: 0.0,
                total_pnl: 0.0,
                largest_win: 0.0,
                largest_loss: 0.0,
                avg_bars_held: 0.0,
                avg_confidence: 0.0,
            };
        }

        let total = trades.len();
        let wins = trades.iter().filter(|t| t.is_win).count();
        let losses = total - wins;

        let total_pnl: f64 = trades.iter().map(|t| t.pnl).sum();
        let total_wins: f64 = trades.iter().filter(|t| t.is_win).map(|t| t.pnl).sum();
        let total_losses: f64 = trades
            .iter()
            .filter(|t| !t.is_win)
            .map(|t| t.pnl.abs())
            .sum();

        let profit_factor = if total_losses > 0.0 {
            total_wins / total_losses
        } else {
            0.0
        };

        let avg_risk: f64 = trades.iter().map(|t| t.risk).sum::<f64>() / total as f64;
        let avg_reward: f64 = trades.iter().map(|t| t.reward).sum::<f64>() / total as f64;
        let avg_rr_ratio: f64 = trades.iter().map(|t| t.rr_ratio).sum::<f64>() / total as f64;

        let largest_win = trades
            .iter()
            .map(|t| t.pnl)
            .fold(f64::NEG_INFINITY, f64::max);
        let largest_loss = trades
            .iter()
            .map(|t| t.pnl)
            .filter(|p| *p < 0.0)
            .fold(f64::NEG_INFINITY, f64::max)
            .abs();

        let avg_bars_held: f64 = trades.iter().map(|t| t.bars_held as f64).sum::<f64>() / total as f64;
        let avg_confidence: f64 = trades.iter().map(|t| t.confidence).sum::<f64>() / total as f64;

        PatternStatistics {
            pattern_type,
            total_trades: total,
            winning_trades: wins,
            losing_trades: losses,
            win_rate: (wins as f64 / total as f64) * 100.0,
            profit_factor,
            avg_risk,
            avg_reward,
            avg_rr_ratio,
            total_pnl,
            largest_win,
            largest_loss,
            avg_bars_held,
            avg_confidence,
        }
    }

    /// Calculate overall statistics
    fn calculate_overall_stats(&self) -> PatternStatistics {
        if self.trades.is_empty() {
            return PatternStatistics {
                pattern_type: PatternType::CompressionExpansion,
                total_trades: 0,
                winning_trades: 0,
                losing_trades: 0,
                win_rate: 0.0,
                profit_factor: 0.0,
                avg_risk: 0.0,
                avg_reward: 0.0,
                avg_rr_ratio: 0.0,
                total_pnl: 0.0,
                largest_win: 0.0,
                largest_loss: 0.0,
                avg_bars_held: 0.0,
                avg_confidence: 0.0,
            };
        }

        let total = self.trades.len();
        let wins = self.trades.iter().filter(|t| t.is_win).count();
        let losses = total - wins;

        let total_pnl: f64 = self.trades.iter().map(|t| t.pnl).sum();
        let total_wins: f64 = self.trades.iter().filter(|t| t.is_win).map(|t| t.pnl).sum();
        let total_losses: f64 = self.trades
            .iter()
            .filter(|t| !t.is_win)
            .map(|t| t.pnl.abs())
            .sum();

        let profit_factor = if total_losses > 0.0 {
            total_wins / total_losses
        } else {
            0.0
        };

        let avg_risk: f64 = self.trades.iter().map(|t| t.risk).sum::<f64>() / total as f64;
        let avg_reward: f64 = self.trades.iter().map(|t| t.reward).sum::<f64>() / total as f64;
        let avg_rr_ratio: f64 = self.trades.iter().map(|t| t.rr_ratio).sum::<f64>() / total as f64;

        let largest_win = self
            .trades
            .iter()
            .map(|t| t.pnl)
            .fold(f64::NEG_INFINITY, f64::max);
        let largest_loss = self
            .trades
            .iter()
            .map(|t| t.pnl)
            .filter(|p| *p < 0.0)
            .fold(f64::NEG_INFINITY, f64::max)
            .abs();

        let avg_bars_held: f64 = self.trades.iter().map(|t| t.bars_held as f64).sum::<f64>() / total as f64;
        let avg_confidence: f64 = self.trades.iter().map(|t| t.confidence).sum::<f64>() / total as f64;

        PatternStatistics {
            pattern_type: PatternType::CompressionExpansion, // Dummy for overall
            total_trades: total,
            winning_trades: wins,
            losing_trades: losses,
            win_rate: (wins as f64 / total as f64) * 100.0,
            profit_factor,
            avg_risk,
            avg_reward,
            avg_rr_ratio,
            total_pnl,
            largest_win,
            largest_loss,
            avg_bars_held,
            avg_confidence,
        }
    }

    /// Print backtest summary
    pub fn print_summary(&self, results: &PriceActionBacktestResults) {
        println!("\n╔════════════════════════════════════════════════════════╗");
        println!("║   PRICE ACTION PATTERN BACKTEST RESULTS                 ║");
        println!("╚════════════════════════════════════════════════════════╝");

        println!("\n📊 OVERALL STATISTICS:");
        println!("  Total Trades:     {}", results.overall_stats.total_trades);
        println!("  Win Rate:         {:.2}%", results.overall_stats.win_rate);
        println!("  Profit Factor:    {:.2}", results.overall_stats.profit_factor);
        println!("  Total P&L:        {:.2}", results.overall_stats.total_pnl);
        println!("  Avg Risk/Reward:  {:.2}", results.overall_stats.avg_rr_ratio);

        println!("\n📈 BY PATTERN TYPE:");
        for stats in &results.pattern_stats {
            println!(
                "  {} - Trades: {}, Win%: {:.1}%, PF: {:.2}",
                stats.pattern_type.as_str(),
                stats.total_trades,
                stats.win_rate,
                stats.profit_factor
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backtest_creation() {
        let candles = vec![
            Candle {
                timestamp: 1000,
                open: 100.0,
                high: 105.0,
                low: 95.0,
                close: 102.0,
                volume: 1000.0,
            },
        ];

        let backtester = PriceActionBacktester::new(candles);
        assert_eq!(backtester.trades.len(), 0);
    }

    #[test]
    fn test_trade_creation() {
        let trade = PatternTrade {
            pattern_type: PatternType::CompressionExpansion,
            entry_price: 100.0,
            exit_price: 110.0,
            pnl: 10.0,
            is_win: true,
            risk: 5.0,
            reward: 10.0,
            rr_ratio: 2.0,
            confidence: 80.0,
            entry_candle: 0,
            exit_candle: 5,
            bars_held: 5,
        };

        assert_eq!(trade.pnl, 10.0);
        assert!(trade.is_win);
        assert_eq!(trade.rr_ratio, 2.0);
    }
}
