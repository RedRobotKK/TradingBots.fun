//! 📊 Dynamic Position Sizing: Risk/Reward Based on Support/Resistance
//! Calculates optimal position size based on:
//! - Support level (HARD STOP)
//! - Current price
//! - Expected move (technical setup)
//! - Acceptable risk per trade

use serde::{Deserialize, Serialize};

/// Support and Resistance Analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupportResistance {
    pub support_level: f64,      // Absolute floor - do NOT go below
    pub resistance_level: f64,    // Absolute ceiling
    pub current_price: f64,
    pub distance_to_support: f64, // How much runway we have
    pub distance_to_resistance: f64,
}

impl SupportResistance {
    pub fn new(support: f64, resistance: f64, current: f64) -> Self {
        Self {
            support_level: support,
            resistance_level: resistance,
            current_price: current,
            distance_to_support: (current - support).abs(),
            distance_to_resistance: (resistance - current).abs(),
        }
    }

    /// Is current price in a reasonable risk zone relative to support?
    pub fn is_safe_zone(&self, min_safety_margin_pct: f64) -> bool {
        let support_distance_pct = (self.distance_to_support / self.current_price) * 100.0;
        support_distance_pct >= min_safety_margin_pct
    }

    /// Get "pain" - maximum loss if support breaks
    pub fn max_loss_if_support_breaks(&self, position_size: f64) -> f64 {
        position_size * self.distance_to_support
    }
}

/// Technical Entry Setup (RSI, MACD, Bollinger)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TechnicalSetup {
    pub rsi: f64,                    // Current RSI
    pub rsi_oversold: f64,           // Threshold: < 30
    pub rsi_overbought: f64,         // Threshold: > 70
    pub macd_above_signal: bool,     // MACD above signal line
    pub price_vs_bollinger: f64,     // -1.0 (below lower), 0 (middle), 1.0 (above upper)
    pub bollinger_compression: f64,  // 0-1: how tight bands are
}

impl TechnicalSetup {
    /// Score technical setup from 0-1
    pub fn confidence_score(&self) -> f64 {
        let mut score: f64 = 0.5; // Base

        // RSI extremes are strong signals
        if self.rsi < self.rsi_oversold {
            score += 0.20;
        }
        if self.rsi > self.rsi_overbought {
            score += 0.20;
        }

        // MACD momentum
        if self.macd_above_signal {
            score += 0.15;
        }

        // Price at Bollinger extremes
        if (self.price_vs_bollinger - 1.0).abs() < 0.1 {
            // At lower Bollinger = oversold
            score += 0.15;
        }
        if (self.price_vs_bollinger + 1.0).abs() < 0.1 {
            // At upper Bollinger = overbought
            score += 0.15;
        }

        score.min(1.0)
    }

    /// Expected move based on technical setup
    pub fn expected_move_pct(&self) -> f64 {
        let confidence = self.confidence_score();

        // Higher confidence = expect bigger move
        if confidence > 0.85 {
            3.0 // 3% expected move
        } else if confidence > 0.75 {
            2.0 // 2% expected move
        } else if confidence > 0.65 {
            1.5 // 1.5% expected move
        } else {
            0.5 // 0.5% expected move (weak setup)
        }
    }
}

/// Dynamic position sizing based on pain vs reward
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicPositionSize {
    pub position_size_pct: f64,      // % of capital to deploy
    pub position_size_dollars: f64,  // Dollar amount
    pub expected_profit: f64,        // Expected reward
    pub max_loss: f64,               // If support breaks
    pub risk_reward_ratio: f64,      // reward / risk
    pub is_viable: bool,             // Risk/reward favorable?
    pub rationale: String,
}

pub struct DynamicSizer;

impl DynamicSizer {
    /// Calculate position size based on pain (distance to support) vs reward (expected move)
    pub fn calculate_position_size(
        capital: f64,
        support_resistance: &SupportResistance,
        technical: &TechnicalSetup,
        max_risk_per_trade_pct: f64, // e.g., 0.05 = 5% of capital
    ) -> DynamicPositionSize {
        let current = support_resistance.current_price;
        let support = support_resistance.support_level;
        let distance_to_support = support_resistance.distance_to_support;

        // Step 1: Calculate maximum acceptable loss
        let max_acceptable_loss = capital * max_risk_per_trade_pct;

        // Step 2: Calculate expected move
        let expected_move_pct = technical.expected_move_pct();
        let _expected_move_dollars = current * (expected_move_pct / 100.0);

        // Step 3: Determine position size based on "pain" (support distance)
        // If support is very close, position must be smaller
        // If support is far, position can be larger
        //
        // Position size = max_loss / distance_to_support
        // This ensures: IF support breaks, we lose at most max_acceptable_loss
        let position_size_dollars = max_acceptable_loss / (distance_to_support / current);

        // Step 4: Cap position size to reasonable % of capital
        let position_size_pct = (position_size_dollars / capital).min(0.25); // Max 25% per trade
        let final_position_size = capital * position_size_pct;

        // Step 5: Calculate expected profit and risk/reward
        let expected_profit = final_position_size * (expected_move_pct / 100.0);
        let max_loss = final_position_size * (distance_to_support / current);
        let risk_reward_ratio = if max_loss > 0.0 {
            expected_profit / max_loss
        } else {
            0.0
        };

        // Step 6: Determine viability
        let is_viable = risk_reward_ratio >= 1.0 // At minimum 1:1
            && expected_move_pct >= 0.5           // Need meaningful expected move
            && position_size_pct > 0.01;          // Must be meaningful position

        let rationale = format!(
            "Support: ${:.2}, Current: ${:.2}, Runway: ${:.2} ({:.1}%). \
             Technical confidence: {:.0}% (RSI: {:.0}, MACD: {}). \
             Expected move: {:.2}%, Max loss if support breaks: ${:.2}, \
             Expected profit: ${:.2}, Risk/Reward: {:.2}x {}",
            support,
            current,
            distance_to_support,
            (distance_to_support / current) * 100.0,
            technical.confidence_score() * 100.0,
            technical.rsi,
            if technical.macd_above_signal { "bullish" } else { "bearish" },
            expected_move_pct,
            max_loss,
            expected_profit,
            risk_reward_ratio,
            if is_viable { "✓ VIABLE" } else { "✗ NOT VIABLE" }
        );

        DynamicPositionSize {
            position_size_pct,
            position_size_dollars: final_position_size,
            expected_profit,
            max_loss,
            risk_reward_ratio,
            is_viable,
            rationale,
        }
    }

    /// For DCA entries, calculate position sizing that scales with support distance
    pub fn calculate_dca_entries(
        capital: f64,
        support_resistance: &SupportResistance,
        technical: &TechnicalSetup,
        max_risk_per_trade_pct: f64,
    ) -> Vec<DCAEntrySize> {
        let mut entries = vec![];

        // Entry 1: Largest position (most confident)
        let entry1 = Self::calculate_position_size(
            capital,
            support_resistance,
            technical,
            max_risk_per_trade_pct,
        );

        if entry1.is_viable {
            entries.push(DCAEntrySize {
                entry_number: 1,
                price_drop_pct: 0.0,
                position_size_pct: entry1.position_size_pct,
                position_size_dollars: entry1.position_size_dollars,
                confluence_requirement: 0.75,
                rationale: entry1.rationale.clone(),
            });

            // Entry 2: At 5% drop, reduce position by 20%
            let entry2_size = entry1.position_size_pct * 0.8;
            if entry2_size > 0.01 {
                entries.push(DCAEntrySize {
                    entry_number: 2,
                    price_drop_pct: 5.0,
                    position_size_pct: entry2_size,
                    position_size_dollars: capital * entry2_size,
                    confluence_requirement: 0.75,
                    rationale: format!("Entry 2: Scaled position ({:.0}% of Entry 1)", 80.0),
                });

                // Entry 3: At 10% drop, reduce further
                let entry3_size = entry1.position_size_pct * 0.6;
                if entry3_size > 0.01 {
                    entries.push(DCAEntrySize {
                        entry_number: 3,
                        price_drop_pct: 10.0,
                        position_size_pct: entry3_size,
                        position_size_dollars: capital * entry3_size,
                        confluence_requirement: 0.80,
                        rationale: format!("Entry 3: Scaled position ({:.0}% of Entry 1)", 60.0),
                    });

                    // Entry 4: At 15% drop, minimum position
                    let entry4_size = entry1.position_size_pct * 0.4;
                    if entry4_size > 0.01 {
                        entries.push(DCAEntrySize {
                            entry_number: 4,
                            price_drop_pct: 15.0,
                            position_size_pct: entry4_size,
                            position_size_dollars: capital * entry4_size,
                            confluence_requirement: 0.85,
                            rationale: format!("Entry 4: Scaled position ({:.0}% of Entry 1)", 40.0),
                        });
                    }
                }
            }
        }

        entries
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DCAEntrySize {
    pub entry_number: u32,
    pub price_drop_pct: f64,
    pub position_size_pct: f64,
    pub position_size_dollars: f64,
    pub confluence_requirement: f64,
    pub rationale: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sol_example() {
        // SOL example: $82 current, $60 support, $85 resistance
        let sr = SupportResistance::new(60.0, 85.0, 82.0);
        let tech = TechnicalSetup {
            rsi: 28.0,                    // Oversold
            rsi_oversold: 30.0,
            rsi_overbought: 70.0,
            macd_above_signal: true,      // Bullish
            price_vs_bollinger: -0.8,     // Near lower band
            bollinger_compression: 0.3,
        };

        let position = DynamicSizer::calculate_position_size(1000.0, &sr, &tech, 0.05);

        println!("SOL Position Sizing:");
        println!("  Position size: {:.1}% (${:.2})", position.position_size_pct * 100.0, position.position_size_dollars);
        println!("  Expected profit: ${:.2}", position.expected_profit);
        println!("  Max loss if support breaks: ${:.2}", position.max_loss);
        println!("  Risk/Reward: {:.2}x", position.risk_reward_ratio);
        println!("  Viable: {}", position.is_viable);

        assert!(position.is_viable);
        assert!(position.position_size_pct > 0.0);
    }

    #[test]
    fn test_dca_scaling() {
        let sr = SupportResistance::new(60.0, 85.0, 82.0);
        let tech = TechnicalSetup {
            rsi: 28.0,
            rsi_oversold: 30.0,
            rsi_overbought: 70.0,
            macd_above_signal: true,
            price_vs_bollinger: -0.8,
            bollinger_compression: 0.3,
        };

        let entries = DynamicSizer::calculate_dca_entries(1000.0, &sr, &tech, 0.05);

        println!("\nDCA Entry Scaling:");
        for entry in &entries {
            println!(
                "  Entry {}: {:.1}% (${:.2}) @ {}% drop, confluence: {:.2}",
                entry.entry_number,
                entry.position_size_pct * 100.0,
                entry.position_size_dollars,
                entry.price_drop_pct,
                entry.confluence_requirement
            );
        }

        assert!(entries.len() > 0);
        // Each entry should be smaller than the previous
        if entries.len() > 1 {
            assert!(entries[0].position_size_pct > entries[1].position_size_pct);
        }
    }
}
