//! 💰 Fee-Aware Trading: Only trade when profit > fees
//! Prevents small profitable trades from becoming net losses

use serde::{Deserialize, Serialize};

/// Fee structure for different exchanges
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeStructure {
    /// Maker fee (limit orders) - typically 0.02-0.05%
    pub maker_fee_pct: f64,

    /// Taker fee (market orders) - typically 0.05-0.1%
    pub taker_fee_pct: f64,

    /// Platform/funding fee per day
    pub daily_funding_fee_pct: f64,

    /// Liquidation fee (if position liquidated)
    pub liquidation_fee_pct: f64,
}

impl FeeStructure {
    /// Hyperliquid fees (typical DEX rates)
    pub fn hyperliquid() -> Self {
        Self {
            maker_fee_pct: 0.02,    // 0.02% for limit orders
            taker_fee_pct: 0.05,    // 0.05% for market orders
            daily_funding_fee_pct: 0.01,  // ~0.01% daily (varies)
            liquidation_fee_pct: 0.05,
        }
    }

    /// Drift Protocol fees
    pub fn drift() -> Self {
        Self {
            maker_fee_pct: 0.02,
            taker_fee_pct: 0.05,
            daily_funding_fee_pct: 0.015,
            liquidation_fee_pct: 0.05,
        }
    }
}

/// Calculate total fees for a round-trip trade (entry + exit)
pub struct FeeCalculator {
    pub fees: FeeStructure,
}

impl FeeCalculator {
    pub fn new(fees: FeeStructure) -> Self {
        Self { fees }
    }

    /// Calculate total entry + exit fees (round-trip)
    /// Assumes market order entry (taker) and limit order exit (maker)
    pub fn calculate_round_trip_fees(
        &self,
        position_size: f64,
        entry_price: f64,
        exit_price: f64,
    ) -> RoundTripFees {
        let position_value = position_size * entry_price;

        // Entry: Market order = TAKER fee
        let entry_fee = position_value * (self.fees.taker_fee_pct / 100.0);

        // Exit: Limit order = MAKER fee (at higher price)
        let exit_value = position_size * exit_price;
        let exit_fee = exit_value * (self.fees.maker_fee_pct / 100.0);

        // Total fees
        let total_fees = entry_fee + exit_fee;

        // Expected profit (before fees)
        let gross_profit = exit_value - position_value;

        // Net profit (after fees)
        let net_profit = gross_profit - total_fees;

        // Fee percentage of the winning trade
        let fee_pct_of_profit = if gross_profit > 0.0 {
            (total_fees / gross_profit) * 100.0
        } else {
            0.0
        };

        RoundTripFees {
            entry_fee,
            exit_fee,
            total_fees,
            gross_profit,
            net_profit,
            fee_pct_of_profit,
            is_profitable: net_profit > 0.0,
        }
    }

    /// Calculate minimum move needed to break even on fees
    pub fn minimum_breakeven_move_pct(
        &self,
    ) -> f64 {
        // Entry fee (taker) + Exit fee (maker)
        // Total = taker + maker = entry cost + exit cost
        let round_trip_fee = self.fees.taker_fee_pct + self.fees.maker_fee_pct;

        // Need to move this % just to break even
        round_trip_fee
    }

    /// Calculate position size that makes sense for a profit target
    ///
    /// Example: If we expect +1% move but fees are 0.07%, position needs to be
    /// large enough that the absolute $ profit from 1% move > absolute $ fees
    pub fn calculate_minimum_position_size(
        &self,
        entry_price: f64,
        profit_target_pct: f64,
        available_capital: f64,
    ) -> MinimumPositionSize {
        let breakeven_fee_pct = self.minimum_breakeven_move_pct();

        // If our expected move is LESS than breakeven, position won't be profitable
        if profit_target_pct <= breakeven_fee_pct {
            return MinimumPositionSize {
                position_size: 0.0,
                is_viable: false,
                reason: format!(
                    "Expected move {:.2}% is less than breakeven fees {:.2}%. Trade not viable.",
                    profit_target_pct, breakeven_fee_pct
                ),
                required_capital: 0.0,
            };
        }

        // For a viable trade, we need:
        // position_size * entry_price * profit_target_pct/100 > position_size * entry_price * breakeven_fee_pct/100
        //
        // This is always true if profit_target_pct > breakeven_fee_pct
        // So any non-zero position size will work
        // But we want to find the minimum size where profit > $1 (not just theoretical)

        let minimum_profit_dollars = 0.50;  // Need at least $0.50 profit

        // position_size * entry_price * net_margin = minimum_profit
        // net_margin = profit_target_pct - breakeven_fees
        let net_margin = (profit_target_pct - breakeven_fee_pct) / 100.0;
        let required_position_value = minimum_profit_dollars / net_margin;
        let position_size = required_position_value / entry_price;

        // Check if we have enough capital
        if position_size > available_capital {
            return MinimumPositionSize {
                position_size: available_capital,
                is_viable: false,
                reason: format!(
                    "Trade viable but requires ${:.2} position (>available ${:.2})",
                    position_size, available_capital
                ),
                required_capital: position_size,
            };
        }

        MinimumPositionSize {
            position_size,
            is_viable: true,
            reason: format!(
                "Viable: {:.2}% move, {:.2}% fees, {:.2}% net profit margin",
                profit_target_pct, breakeven_fee_pct, net_margin * 100.0
            ),
            required_capital: position_size,
        }
    }

    /// Calculate what price move you need to make a specific profit
    pub fn required_move_for_target_profit(
        &self,
        position_size: f64,
        entry_price: f64,
        target_profit_dollars: f64,
    ) -> RequiredMove {
        let _position_value = position_size * entry_price;
        let fees = self.calculate_round_trip_fees(position_size, entry_price, entry_price);

        // We need: (exit_price - entry_price) * position_size - fees.total_fees >= target_profit
        // exit_price * position_size - position_value - fees >= target_profit
        // exit_price * position_size >= position_value + target_profit + fees
        // exit_price >= entry_price + (target_profit + fees) / position_size

        let exit_price = entry_price + ((target_profit_dollars + fees.total_fees) / position_size);
        let move_dollars = exit_price - entry_price;
        let move_pct = (move_dollars / entry_price) * 100.0;

        RequiredMove {
            required_exit_price: exit_price,
            required_move_dollars: move_dollars,
            required_move_pct: move_pct,
            including_fees: fees.total_fees,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoundTripFees {
    pub entry_fee: f64,
    pub exit_fee: f64,
    pub total_fees: f64,
    pub gross_profit: f64,
    pub net_profit: f64,
    pub fee_pct_of_profit: f64,
    pub is_profitable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinimumPositionSize {
    pub position_size: f64,
    pub is_viable: bool,
    pub reason: String,
    pub required_capital: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequiredMove {
    pub required_exit_price: f64,
    pub required_move_dollars: f64,
    pub required_move_pct: f64,
    pub including_fees: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hyperliquid_fees() {
        let fees = FeeStructure::hyperliquid();
        let calc = FeeCalculator::new(fees);

        // Trade: Buy 1 SOL at $100, sell at $102 (2% gain)
        let round_trip = calc.calculate_round_trip_fees(1.0, 100.0, 102.0);

        // Entry fee: 1 * 100 * 0.05% = $0.05
        // Exit fee: 1 * 102 * 0.02% = $0.0204
        // Total: $0.0704
        // Gross profit: $2.00
        // Net profit: $1.9296
        assert!(round_trip.is_profitable);
        assert!(round_trip.net_profit > round_trip.gross_profit - round_trip.total_fees);
    }

    #[test]
    fn test_small_trade_unprofitable() {
        let fees = FeeStructure::hyperliquid();
        let calc = FeeCalculator::new(fees);

        // Tiny trade: 0.01 SOL at $100, only expect 0.1% gain
        let round_trip = calc.calculate_round_trip_fees(0.01, 100.0, 100.1);

        // Entry fee: 0.01 * 100 * 0.05% = $0.005
        // Exit fee: 0.01 * 100.1 * 0.02% = $0.0002
        // Total: $0.0052
        // Gross profit: $0.01
        // Net profit: NEGATIVE (fees > profit)
        println!("Tiny trade: gross ${}, fees ${}, net ${}",
                 round_trip.gross_profit, round_trip.total_fees, round_trip.net_profit);
        assert!(!round_trip.is_profitable);
    }

    #[test]
    fn test_minimum_viable_position() {
        let fees = FeeStructure::hyperliquid();
        let calc = FeeCalculator::new(fees);

        // For a 1% expected move, what minimum position size?
        let min_pos = calc.calculate_minimum_position_size(100.0, 1.0, 1000.0);

        println!("For 1% move: minimum position = {}", min_pos.position_size);
        assert!(min_pos.is_viable);
        assert!(min_pos.position_size > 0.0);
    }
}
