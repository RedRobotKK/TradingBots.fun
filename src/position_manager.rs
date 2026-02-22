//! 📊 Position Manager: DCA/Pyramiding + Smart Exits
//! Tracks multiple entries, calculates optimal exits based on ATH/ATL

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Individual entry into a position
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionEntry {
    pub entry_number: u32,           // 1, 2, 3, 4
    pub entry_price: f64,
    pub entry_time: i64,
    pub quantity: f64,
    pub position_size_pct: f64,      // % of capital
    pub leverage: f64,
    pub confidence_at_entry: f64,
    pub confluence_signals: usize,   // Number of signals at entry
}

/// Aggregate position tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatePosition {
    pub symbol: String,
    pub entries: VecDeque<PositionEntry>,
    pub total_quantity: f64,
    pub average_entry_price: f64,
    pub total_capital_deployed: f64,
    pub current_price: f64,
    pub unrealized_pnl: f64,
    pub unrealized_pnl_pct: f64,
    pub position_status: PositionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PositionStatus {
    Single,        // Only 1 entry
    Pyramiding,    // 2+ entries, averaging up/down
    Closed,
}

impl AggregatePosition {
    pub fn new(symbol: String) -> Self {
        Self {
            symbol,
            entries: VecDeque::new(),
            total_quantity: 0.0,
            average_entry_price: 0.0,
            total_capital_deployed: 0.0,
            current_price: 0.0,
            unrealized_pnl: 0.0,
            unrealized_pnl_pct: 0.0,
            position_status: PositionStatus::Closed,
        }
    }

    /// Add a new entry to the position
    pub fn add_entry(&mut self, entry: PositionEntry) {
        // Update aggregate values
        let entry_cost = entry.quantity * entry.entry_price;
        self.total_capital_deployed += entry_cost;
        self.total_quantity += entry.quantity;

        // Recalculate average entry price
        if self.total_quantity > 0.0 {
            self.average_entry_price = self.total_capital_deployed / self.total_quantity;
        }

        // Update position status
        self.position_status = if self.entries.len() > 0 {
            PositionStatus::Pyramiding
        } else {
            PositionStatus::Single
        };

        self.entries.push_back(entry);
    }

    /// Update current price and calculate P&L
    pub fn update_price(&mut self, price: f64) {
        self.current_price = price;
        let position_value = self.total_quantity * price;
        self.unrealized_pnl = position_value - self.total_capital_deployed;
        self.unrealized_pnl_pct = if self.total_capital_deployed > 0.0 {
            (self.unrealized_pnl / self.total_capital_deployed) * 100.0
        } else {
            0.0
        };
    }

    /// Check if position is eligible for another pyramid entry
    pub fn can_add_entry(&self, max_entries: u32) -> bool {
        (self.entries.len() as u32) < max_entries
    }

    /// Get average leverage across all entries
    pub fn average_leverage(&self) -> f64 {
        if self.entries.is_empty() {
            return 0.0;
        }

        let total_leverage: f64 = self.entries.iter().map(|e| e.leverage).sum();
        total_leverage / self.entries.len() as f64
    }

    /// Get average confidence across all entries
    pub fn average_confidence(&self) -> f64 {
        if self.entries.is_empty() {
            return 0.0;
        }

        let total_conf: f64 = self.entries.iter().map(|e| e.confidence_at_entry).sum();
        total_conf / self.entries.len() as f64
    }
}

/// Smart exit analysis based on daily ATH/ATL
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyPriceContext {
    pub date: String,
    pub open: f64,
    pub high: f64,        // ATH (All-Time High for the day)
    pub low: f64,         // ATL (All-Time Low for the day)
    pub close: f64,
    pub volume: f64,
}

/// Exit strategy calculation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExitStrategy {
    pub ideal_exit_price: f64,
    pub conservative_exit_price: f64,
    pub aggressive_exit_price: f64,
    pub rationale: String,
    pub expected_return_pct: f64,
    pub risk_reward_ratio: f64,
    pub exit_trigger: ExitTrigger,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ExitTrigger {
    ATHTarget,            // Target ATH
    Midpoint,             // Midpoint between ATH and entry
    FibonacciLevel,       // 61.8% retracement
    SupportLevel,         // Nearest support
    RiskRewardRatio,      // 3:1 ratio from stop loss
}

pub struct ExitCalculator;

impl ExitCalculator {
    /// Calculate optimal exit prices based on ATH/ATL and position
    pub fn calculate_exit_strategy(
        position: &AggregatePosition,
        daily_context: &DailyPriceContext,
        stop_loss_price: f64,
    ) -> ExitStrategy {
        let entry = position.average_entry_price;
        let ath = daily_context.high;
        let atl = daily_context.low;
        let current = position.current_price;

        // Check if ATH is above entry (bullish day)
        let is_bullish_day = ath > entry;

        let (ideal_exit, aggressive_exit, conservative_exit, trigger) = if is_bullish_day {
            // Bullish day - target ATH or above
            let ath_target = ath * 1.005; // 0.5% above ATH (retest)

            // Conservative: 61.8% of ATH-to-entry move
            let fib_level = entry + (ath - entry) * 0.618;

            // Aggressive: ATH + risk-reward buffer
            let risk = entry - stop_loss_price;
            let rr_exit = ath + (risk * 3.0); // 3:1 reward

            (ath_target, rr_exit, fib_level, ExitTrigger::ATHTarget)
        } else {
            // Bearish day - use midpoint strategy
            let midpoint = (entry + atl) / 2.0;

            // Conservative: entry price (break even is OK)
            let conservative = entry;

            // Aggressive: Fib level
            let fib_level = atl + (entry - atl) * 0.382; // 38.2% retracement

            (midpoint, fib_level, conservative, ExitTrigger::Midpoint)
        };

        // Calculate expected returns
        let expected_return = ((ideal_exit - entry) / entry) * 100.0;
        let risk = (entry - stop_loss_price).abs();
        let reward = (ideal_exit - entry).abs();
        let risk_reward = if risk > 0.0 {
            reward / risk
        } else {
            0.0
        };

        ExitStrategy {
            ideal_exit_price: ideal_exit,
            conservative_exit_price: conservative_exit,
            aggressive_exit_price: aggressive_exit,
            rationale: format!(
                "ATH: ${:.4}, ATL: ${:.4}, Entry: ${:.4}. {}",
                ath,
                atl,
                entry,
                if is_bullish_day {
                    "Bullish day - target ATH with upside bias"
                } else {
                    "Bearish day - use midpoint / accumulation strategy"
                }
            ),
            expected_return_pct: expected_return,
            risk_reward_ratio: risk_reward,
            exit_trigger: trigger,
        }
    }

    /// Determine which exit level to use based on risk tolerance
    pub fn select_exit_level(
        strategy: &ExitStrategy,
        risk_tolerance: RiskTolerance,
    ) -> f64 {
        match risk_tolerance {
            RiskTolerance::Conservative => strategy.conservative_exit_price,
            RiskTolerance::Moderate => strategy.ideal_exit_price,
            RiskTolerance::Aggressive => strategy.aggressive_exit_price,
        }
    }

    /// Suggested exit when position hits key levels
    pub fn suggest_exit(
        position: &AggregatePosition,
        daily_context: &DailyPriceContext,
        stop_loss_price: f64,
    ) -> ExitRecommendation {
        let strategy = Self::calculate_exit_strategy(position, daily_context, stop_loss_price);
        let entry = position.average_entry_price;

        // Determine current level
        let distance_from_entry = ((position.current_price - entry) / entry) * 100.0;

        let recommendation = if position.current_price >= strategy.ideal_exit_price {
            RecommendationType::ExitNow("Reached ideal exit price".to_string())
        } else if position.current_price >= strategy.conservative_exit_price
            && strategy.risk_reward_ratio > 2.0
        {
            RecommendationType::PartialExit(
                "Good risk/reward, consider taking profits".to_string(),
            )
        } else if distance_from_entry > -5.0 && daily_context.close > entry {
            RecommendationType::Hold("Position improving, hold for ideal exit".to_string())
        } else if position.current_price < stop_loss_price {
            RecommendationType::StopOut("Stop loss triggered".to_string())
        } else {
            RecommendationType::Hold("Wait for better exit price".to_string())
        };

        ExitRecommendation {
            current_price: position.current_price,
            entry_price: entry,
            ideal_exit: strategy.ideal_exit_price,
            stop_loss: stop_loss_price,
            recommendation,
            expected_return: strategy.expected_return_pct,
            distance_to_ideal: ((strategy.ideal_exit_price - position.current_price)
                / position.current_price)
                * 100.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RiskTolerance {
    Conservative,
    Moderate,
    Aggressive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecommendationType {
    ExitNow(String),
    PartialExit(String),
    Hold(String),
    StopOut(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExitRecommendation {
    pub current_price: f64,
    pub entry_price: f64,
    pub ideal_exit: f64,
    pub stop_loss: f64,
    pub recommendation: RecommendationType,
    pub expected_return: f64,
    pub distance_to_ideal: f64,
}

/// DCA Rules for pyramid entries
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DCARules {
    pub max_entries: u32,
    pub entries: Vec<DCAEntry>,
    pub min_confluence_for_add: f64,
    pub min_confluence_for_3rd_add: f64,
    pub support_level: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DCAEntry {
    pub entry_number: u32,
    pub price_drop_pct: f64,     // e.g., 5% drop from previous
    pub position_size_pct: f64,   // 25% of capital
    pub leverage: f64,
    pub confluence_requirement: f64,  // 0.75 minimum
}

impl DCARules {
    /// Create default DCA rules for a position
    /// NOTE: Uses FIXED leverage (10x) for all entries - most exchanges don't support
    /// changing leverage on open positions. Risk is managed through capital staging,
    /// not leverage reduction.
    pub fn default() -> Self {
        Self {
            max_entries: 4,
            entries: vec![
                DCAEntry {
                    entry_number: 1,
                    price_drop_pct: 0.0,
                    position_size_pct: 0.25,
                    leverage: 10.0,  // Entry 1: High conviction, initial signal
                    confluence_requirement: 0.75,
                },
                DCAEntry {
                    entry_number: 2,
                    price_drop_pct: 5.0,
                    position_size_pct: 0.25,
                    leverage: 10.0,  // Entry 2: SAME leverage (practical approach)
                    confluence_requirement: 0.75,  // Must maintain confluence
                },
                DCAEntry {
                    entry_number: 3,
                    price_drop_pct: 10.0,
                    position_size_pct: 0.25,
                    leverage: 10.0,  // Entry 3: SAME leverage, but higher confluence requirement
                    confluence_requirement: 0.80,  // Higher bar for deeper dip
                },
                DCAEntry {
                    entry_number: 4,
                    price_drop_pct: 15.0,
                    position_size_pct: 0.25,
                    leverage: 10.0,  // Entry 4: SAME leverage, but extreme conviction required
                    confluence_requirement: 0.85,  // Very high bar for extreme dip
                },
            ],
            min_confluence_for_add: 0.75,
            min_confluence_for_3rd_add: 0.80,
            support_level: 0.0,  // Set by user
        }
    }

    /// Check if next DCA entry is valid
    pub fn should_add_entry(
        &self,
        current_entry_count: u32,
        current_price: f64,
        previous_entry_price: f64,
        confluence: f64,
        support_held: bool,
    ) -> bool {
        if current_entry_count >= self.max_entries {
            return false; // Max entries reached
        }

        let entry_rule = self.entries.get(current_entry_count as usize);
        if entry_rule.is_none() {
            return false;
        }

        let rule = entry_rule.unwrap();

        // Check price drop
        let price_drop = ((previous_entry_price - current_price) / previous_entry_price) * 100.0;
        if price_drop < rule.price_drop_pct - 1.0 {
            return false; // Not enough drop yet
        }

        // Check confluence
        if confluence < rule.confluence_requirement {
            return false; // Not enough confidence
        }

        // Check support
        if !support_held && current_entry_count > 1 {
            return false; // Support broken
        }

        true
    }

    /// Get next entry rules
    pub fn get_next_entry(&self, current_count: u32) -> Option<&DCAEntry> {
        self.entries.get(current_count as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aggregate_position() {
        let mut pos = AggregatePosition::new("SOL".to_string());

        let entry1 = PositionEntry {
            entry_number: 1,
            entry_price: 100.0,
            entry_time: 0,
            quantity: 1.0,
            position_size_pct: 0.25,
            leverage: 10.0,
            confidence_at_entry: 0.80,
            confluence_signals: 7,
        };

        pos.add_entry(entry1);
        assert_eq!(pos.total_quantity, 1.0);
        assert_eq!(pos.average_entry_price, 100.0);

        let entry2 = PositionEntry {
            entry_number: 2,
            entry_price: 95.0,
            entry_time: 1,
            quantity: 1.1,
            position_size_pct: 0.25,
            leverage: 8.0,
            confidence_at_entry: 0.78,
            confluence_signals: 7,
        };

        pos.add_entry(entry2);
        assert_eq!(pos.total_quantity, 2.1);
        assert!(pos.average_entry_price < 100.0); // Should be lower

        pos.update_price(102.0);
        assert!(pos.unrealized_pnl > 0.0); // In profit
    }

    #[test]
    fn test_exit_strategy_bullish_day() {
        let mut pos = AggregatePosition::new("SOL".to_string());
        pos.add_entry(PositionEntry {
            entry_number: 1,
            entry_price: 100.0,
            entry_time: 0,
            quantity: 1.0,
            position_size_pct: 0.25,
            leverage: 10.0,
            confidence_at_entry: 0.80,
            confluence_signals: 7,
        });
        pos.update_price(102.0);

        let daily = DailyPriceContext {
            date: "2026-02-22".to_string(),
            open: 100.0,
            high: 105.0, // ATH at $105
            low: 99.0,
            close: 102.0,
            volume: 1000000.0,
        };

        let strategy = ExitCalculator::calculate_exit_strategy(&pos, &daily, 98.0);

        // Should target ATH area
        assert!(strategy.ideal_exit_price > 105.0);
        assert!(strategy.expected_return_pct > 0.0);
    }
}
