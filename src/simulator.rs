//! 🎬 Historical Data Simulator
//! Replays 3-7 days of historical market data to test strategies in isolation

use crate::backtest::{Backtester, BacktestConfig, TradeAction};
use crate::strategies::{self, StrategyContext, SignalType, MarketSnapshot};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationResults {
    pub symbol: String,
    pub duration_days: usize,
    pub initial_capital: f64,
    pub final_equity: f64,
    pub return_pct: f64,
    pub total_trades: usize,
    pub winning_trades: usize,
    pub losing_trades: usize,
    pub win_rate: f64,
    pub avg_win: f64,
    pub avg_loss: f64,
    pub profit_factor: f64,
    pub max_drawdown_pct: f64,
    pub best_trade: f64,
    pub worst_trade: f64,
    pub strategy_performance: std::collections::HashMap<String, StrategyMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyMetrics {
    pub name: String,
    pub trades_triggered: usize,
    pub trades_won: usize,
    pub trades_lost: usize,
    pub win_rate: f64,
    pub total_pnl: f64,
    pub avg_confidence: f64,
}

pub struct Simulator {
    config: BacktestConfig,
}

impl Simulator {
    pub fn new(initial_capital: f64) -> Self {
        Self {
            config: BacktestConfig {
                initial_capital,
                max_position_pct: 0.15,
                max_leverage: 15.0,
                daily_loss_limit: initial_capital * 0.05,  // 5% daily limit
                slippage_pct: 0.0005,  // 0.05% slippage
            },
        }
    }

    /// Simulate trading on historical data points
    pub fn run_simulation(
        &self,
        symbol: &str,
        market_data: Vec<MarketSnapshot>,
        cex_signals: Vec<(i64, SignalType)>,  // timestamps and CEX signals
    ) -> SimulationResults {
        let mut backtester = Backtester::new(self.config.clone());
        let mut strategy_metrics: std::collections::HashMap<String, StrategyMetrics> =
            std::collections::HashMap::new();

        // Process each candle
        let mut prev_snapshot: Option<MarketSnapshot> = None;

        for (i, current_snapshot) in market_data.iter().enumerate() {
            // Get CEX signal for this timestamp
            let cex_signal = cex_signals
                .iter()
                .find(|(ts, _)| *ts == current_snapshot.timestamp)
                .map(|(_, signal)| *signal)
                .unwrap_or(SignalType::Neutral);

            // Build context
            let ctx = StrategyContext {
                current: current_snapshot.clone(),
                previous: prev_snapshot.clone(),
                cex_imbalance_ratio: 1.5,  // Assume moderate imbalance
                cex_signal_type: cex_signal,
                portfolio_equity: backtester.equity(),
                portfolio_drawdown_pct: backtester.max_drawdown_pct(),
                position_open: backtester.open_positions() > 0,
            };

            // Evaluate all strategies
            let signals = strategies::evaluate_all_strategies(&ctx);

            // Calculate confluence score
            let confluence = strategies::calculate_confluence_score(&signals);

            // Execute trade if high confidence
            if confluence > 0.70 && backtester.open_positions() == 0 {
                // Determine signal direction
                let mut buy_count = 0;
                let mut sell_count = 0;
                let mut total_confidence = 0.0;

                for signal in &signals {
                    total_confidence += signal.confidence;
                    match signal.signal_type {
                        SignalType::StrongBuy => buy_count += 2,
                        SignalType::Buy => buy_count += 1,
                        SignalType::StrongSell => sell_count += 2,
                        SignalType::Sell => sell_count += 1,
                        _ => {}
                    }
                }

                let avg_confidence = total_confidence / signals.len() as f64;

                // Determine position size based on confluence
                let position_size_pct = if confluence > 0.85 {
                    0.12
                } else if confluence > 0.75 {
                    0.08
                } else {
                    0.05
                };

                let position_size = backtester.cash() * position_size_pct;

                // Determine action
                if buy_count > sell_count {
                    let leverage = if confluence > 0.85 { 12.0 } else { 8.0 };
                    let stop_loss = current_snapshot.close * 0.97;  // 3% below entry
                    let take_profit = Some(current_snapshot.close * 1.08);  // 8% above entry

                    let strategy_names = signals
                        .iter()
                        .map(|s| s.strategy_name.clone())
                        .collect::<Vec<_>>()
                        .join(" + ");

                    let rationale = format!(
                        "Multi-signal confluence ({} signals): {}",
                        signals.len(),
                        strategy_names
                    );

                    if let Ok(_trade) = backtester.execute_trade(
                        symbol.to_string(),
                        TradeAction::Buy,
                        position_size / current_snapshot.close,
                        current_snapshot.close,
                        leverage,
                        stop_loss,
                        take_profit,
                        "Multi-Strategy Confluence".to_string(),
                        avg_confidence,
                        rationale.clone(),
                        current_snapshot.timestamp,
                    ) {
                        // Track strategy metrics
                        for signal in signals {
                            let metrics = strategy_metrics
                                .entry(signal.strategy_name.clone())
                                .or_insert_with(|| StrategyMetrics {
                                    name: signal.strategy_name.clone(),
                                    trades_triggered: 0,
                                    trades_won: 0,
                                    trades_lost: 0,
                                    win_rate: 0.0,
                                    total_pnl: 0.0,
                                    avg_confidence: 0.0,
                                });
                            metrics.trades_triggered += 1;
                        }
                    }
                } else if sell_count > buy_count {
                    let leverage = if confluence > 0.85 { 12.0 } else { 8.0 };
                    let stop_loss = current_snapshot.close * 1.03;  // 3% above entry
                    let take_profit = Some(current_snapshot.close * 0.92);  // 8% below entry

                    let strategy_names = signals
                        .iter()
                        .map(|s| s.strategy_name.clone())
                        .collect::<Vec<_>>()
                        .join(" + ");

                    let rationale = format!(
                        "Multi-signal confluence ({} signals): {}",
                        signals.len(),
                        strategy_names
                    );

                    if let Ok(_trade) = backtester.execute_trade(
                        symbol.to_string(),
                        TradeAction::Short,
                        position_size / current_snapshot.close,
                        current_snapshot.close,
                        leverage,
                        stop_loss,
                        take_profit,
                        "Multi-Strategy Confluence".to_string(),
                        avg_confidence,
                        rationale.clone(),
                        current_snapshot.timestamp,
                    ) {
                        // Track strategy metrics
                        for signal in signals {
                            let metrics = strategy_metrics
                                .entry(signal.strategy_name.clone())
                                .or_insert_with(|| StrategyMetrics {
                                    name: signal.strategy_name.clone(),
                                    trades_triggered: 0,
                                    trades_won: 0,
                                    trades_lost: 0,
                                    win_rate: 0.0,
                                    total_pnl: 0.0,
                                    avg_confidence: 0.0,
                                });
                            metrics.trades_triggered += 1;
                        }
                    }
                }
            }

            // Update prices for all positions
            backtester.update_price(symbol, current_snapshot.close, current_snapshot.timestamp);

            // Reset daily at end of day (simple: every 24 candles on 1h)
            if (i + 1) % 24 == 0 {
                backtester.reset_daily();
            }

            prev_snapshot = Some(current_snapshot.clone());
        }

        // Compile results
        let trades = backtester.closed_trades();
        let winning_trades = trades.iter().filter(|t| t.pnl.is_some_and(|p| p > 0.0)).count();
        let losing_trades = trades.iter().filter(|t| t.pnl.is_some_and(|p| p < 0.0)).count();

        let best_trade = trades
            .iter()
            .filter_map(|t| t.pnl)
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(0.0);

        let worst_trade = trades
            .iter()
            .filter_map(|t| t.pnl)
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(0.0);

        // Update strategy metrics with final P&L
        for trade in trades {
            if let Some(metrics) = strategy_metrics.get_mut(&trade.strategy) {
                if trade.pnl.map_or(false, |p| p > 0.0) {
                    metrics.trades_won += 1;
                } else {
                    metrics.trades_lost += 1;
                }
                if let Some(pnl) = trade.pnl {
                    metrics.total_pnl += pnl;
                }
                metrics.avg_confidence = (metrics.avg_confidence + trade.confidence) / 2.0;
            }
        }

        SimulationResults {
            symbol: symbol.to_string(),
            duration_days: market_data.len() / 24,  // Assuming 1h candles
            initial_capital: self.config.initial_capital,
            final_equity: backtester.equity(),
            return_pct: backtester.return_pct(),
            total_trades: trades.len(),
            winning_trades,
            losing_trades,
            win_rate: backtester.win_rate(),
            avg_win: backtester.avg_win(),
            avg_loss: backtester.avg_loss(),
            profit_factor: backtester.profit_factor(),
            max_drawdown_pct: backtester.max_drawdown_pct(),
            best_trade,
            worst_trade,
            strategy_performance: strategy_metrics,
        }
    }
}
