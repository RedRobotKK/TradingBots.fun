//! 📊 Backtesting Engine for Paper Trading
//! Simulates trading on historical data with full position tracking and performance metrics

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestConfig {
    pub initial_capital: f64,
    pub max_position_pct: f64,
    pub max_leverage: f64,
    pub daily_loss_limit: f64,
    pub slippage_pct: f64,  // 0.001 = 0.1%
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub symbol: String,
    pub size: f64,
    pub entry_price: f64,
    pub entry_time: i64,
    pub current_price: f64,
    pub leverage: f64,
    pub stop_loss: f64,
    pub take_profit: Option<f64>,
    pub strategy: String,  // Which strategy triggered it
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulatedTrade {
    pub id: String,
    pub symbol: String,
    pub action: TradeAction,
    pub entry_price: f64,
    pub entry_time: i64,
    pub exit_price: Option<f64>,
    pub exit_time: Option<i64>,
    pub size: f64,
    pub leverage: f64,
    pub pnl: Option<f64>,
    pub pnl_pct: Option<f64>,
    pub strategy: String,
    pub confidence: f64,
    pub status: TradeStatus,
    pub rationale: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum TradeAction {
    Buy,
    Sell,
    Short,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum TradeStatus {
    Open,
    Closed,
    StoppedOut,
    TakeProfitHit,
    Cancelled,
}

pub struct Backtester {
    config: BacktestConfig,
    equity: f64,
    initial_equity: f64,
    cash: f64,
    positions: HashMap<String, Position>,
    closed_trades: Vec<SimulatedTrade>,
    daily_pnl: f64,
    daily_loss_limit_hit: bool,
    max_drawdown: f64,
    peak_equity: f64,
}

impl Backtester {
    pub fn new(config: BacktestConfig) -> Self {
        let equity = config.initial_capital;
        Self {
            config,
            equity,
            initial_equity: equity,
            cash: equity,
            positions: HashMap::new(),
            closed_trades: Vec::new(),
            daily_pnl: 0.0,
            daily_loss_limit_hit: false,
            max_drawdown: 0.0,
            peak_equity: equity,
        }
    }

    /// Execute a simulated trade (market order with slippage)
    pub fn execute_trade(
        &mut self,
        symbol: String,
        action: TradeAction,
        size: f64,
        entry_price: f64,
        leverage: f64,
        stop_loss: f64,
        take_profit: Option<f64>,
        strategy: String,
        confidence: f64,
        rationale: String,
        timestamp: i64,
    ) -> Result<SimulatedTrade, String> {
        // Check daily loss limit
        if self.daily_loss_limit_hit {
            return Err("Daily loss limit hit - no more trading today".to_string());
        }

        // Apply slippage
        let slippage = entry_price * self.config.slippage_pct;
        let actual_entry_price = match action {
            TradeAction::Buy => entry_price + slippage,
            TradeAction::Sell | TradeAction::Short => entry_price - slippage,
        };

        // Check margin
        let required_margin = (size * actual_entry_price) / leverage;
        if required_margin > self.cash {
            return Err(format!(
                "Insufficient margin: need {:.2}, have {:.2}",
                required_margin, self.cash
            ));
        }

        // Create position
        let position = Position {
            symbol: symbol.clone(),
            size,
            entry_price: actual_entry_price,
            entry_time: timestamp,
            current_price: actual_entry_price,
            leverage,
            stop_loss,
            take_profit,
            strategy: strategy.clone(),
        };

        // Deduct margin from cash
        self.cash -= required_margin;

        // Create trade record
        let trade_id = format!("{}-{}-{}", symbol, timestamp, (self.closed_trades.len() + 1));
        let trade = SimulatedTrade {
            id: trade_id,
            symbol: symbol.clone(),
            action,
            entry_price: actual_entry_price,
            entry_time: timestamp,
            exit_price: None,
            exit_time: None,
            size,
            leverage,
            pnl: None,
            pnl_pct: None,
            strategy,
            confidence,
            status: TradeStatus::Open,
            rationale,
        };

        self.positions.insert(symbol, position);
        Ok(trade)
    }

    /// Update position with current market price
    pub fn update_price(&mut self, symbol: &str, price: f64, timestamp: i64) {
        let (should_close_stop, should_close_tp) = if let Some(position) = self.positions.get_mut(symbol) {
            position.current_price = price;

            // Extract values to avoid borrow issues
            let stop_loss = position.stop_loss;
            let take_profit = position.take_profit;
            (price <= stop_loss, take_profit.map(|tp| price >= tp).unwrap_or(false))
        } else {
            (false, false)
        };

        // Now call close_position without the borrow
        if should_close_stop {
            self.close_position(symbol, price, timestamp, TradeStatus::StoppedOut);
        } else if should_close_tp {
            self.close_position(symbol, price, timestamp, TradeStatus::TakeProfitHit);
        }

        // Update drawdown
        self.update_drawdown();
    }

    /// Close an open position
    pub fn close_position(
        &mut self,
        symbol: &str,
        exit_price: f64,
        timestamp: i64,
        status: TradeStatus,
    ) {
        if let Some(position) = self.positions.remove(symbol) {
            // Calculate P&L
            let pnl = match position {
                Position { size, entry_price, .. } => {
                    (exit_price - entry_price) * size * position.leverage
                }
            };

            let pnl_pct = (pnl / (position.entry_price * position.size)) * 100.0;

            // Update cash and daily PnL
            self.cash += (position.entry_price * position.size) / position.leverage;
            self.cash += pnl;
            self.daily_pnl += pnl;

            // Check daily loss limit
            if self.daily_pnl < -self.config.daily_loss_limit {
                self.daily_loss_limit_hit = true;
            }

            // Find and update trade
            if let Some(trade) = self.closed_trades.last_mut() {
                trade.exit_price = Some(exit_price);
                trade.exit_time = Some(timestamp);
                trade.pnl = Some(pnl);
                trade.pnl_pct = Some(pnl_pct);
                trade.status = status;
            }

            // Update equity
            self.equity = self.cash + self.get_open_positions_value();
            self.update_drawdown();
        }
    }

    pub fn get_open_positions_value(&self) -> f64 {
        self.positions
            .values()
            .map(|p| (p.current_price - p.entry_price) * p.size * p.leverage)
            .sum()
    }

    pub fn update_drawdown(&mut self) {
        let current_equity = self.equity;
        if current_equity > self.peak_equity {
            self.peak_equity = current_equity;
        }
        let drawdown = ((current_equity - self.peak_equity) / self.peak_equity).abs();
        if drawdown > self.max_drawdown {
            self.max_drawdown = drawdown;
        }
    }

    pub fn reset_daily(&mut self) {
        self.daily_pnl = 0.0;
        self.daily_loss_limit_hit = false;
    }

    // Getters
    pub fn equity(&self) -> f64 {
        self.equity
    }

    pub fn cash(&self) -> f64 {
        self.cash
    }

    pub fn return_pct(&self) -> f64 {
        ((self.equity - self.initial_equity) / self.initial_equity) * 100.0
    }

    pub fn max_drawdown_pct(&self) -> f64 {
        self.max_drawdown * 100.0
    }

    pub fn closed_trades(&self) -> &[SimulatedTrade] {
        &self.closed_trades
    }

    pub fn open_positions(&self) -> usize {
        self.positions.len()
    }

    pub fn win_rate(&self) -> f64 {
        let winners = self
            .closed_trades
            .iter()
            .filter(|t| t.pnl.map_or(false, |p| p > 0.0))
            .count();
        if self.closed_trades.is_empty() {
            0.0
        } else {
            (winners as f64 / self.closed_trades.len() as f64) * 100.0
        }
    }

    pub fn avg_win(&self) -> f64 {
        let winners: Vec<f64> = self
            .closed_trades
            .iter()
            .filter_map(|t| t.pnl.filter(|p| *p > 0.0))
            .collect();

        if winners.is_empty() {
            0.0
        } else {
            winners.iter().sum::<f64>() / winners.len() as f64
        }
    }

    pub fn avg_loss(&self) -> f64 {
        let losers: Vec<f64> = self
            .closed_trades
            .iter()
            .filter_map(|t| t.pnl.filter(|p| *p < 0.0))
            .collect();

        if losers.is_empty() {
            0.0
        } else {
            losers.iter().sum::<f64>() / losers.len() as f64
        }
    }

    pub fn profit_factor(&self) -> f64 {
        let gross_profit: f64 = self
            .closed_trades
            .iter()
            .filter_map(|t| t.pnl.filter(|p| *p > 0.0))
            .sum();

        let gross_loss: f64 = self
            .closed_trades
            .iter()
            .filter_map(|t| t.pnl.filter(|p| *p < 0.0))
            .map(|p| p.abs())
            .sum();

        if gross_loss == 0.0 {
            if gross_profit > 0.0 {
                f64::INFINITY
            } else {
                0.0
            }
        } else {
            gross_profit / gross_loss
        }
    }
}
