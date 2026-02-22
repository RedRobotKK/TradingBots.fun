//! 🚀 DCA + Scoring System Integration
//!
//! Connects the new scoring system to the existing DCA/Pyramiding framework.
//! Determines when to add pyramid entries based on:
//! - Price movement (5%, 10%, 15% drops)
//! - Current signal score (confluence requirement)
//! - Support levels
//! - Capital efficiency

use crate::position_manager::{AggregatePosition, DCARules, PositionEntry, PositionStatus};
use crate::scoring_system::{StrategyScore, PortfolioScore, PortfolioAction};

/// Determines if pyramid entry should be added based on scoring + DCA rules
#[derive(Debug, Clone)]
pub struct DCAPyramidDecision {
    pub should_add_entry: bool,
    pub entry_number: u32,
    pub position_size_pct: f64,
    pub leverage: f64,
    pub confidence: f64,
    pub rationale: String,
}

/// Evaluates DCA entry eligibility
pub fn evaluate_dca_entry(
    position: &AggregatePosition,
    dca_rules: &DCARules,
    portfolio_score: &PortfolioScore,
    current_price: f64,
    previous_entry_price: f64,
    support_level: f64,
) -> DCAPyramidDecision {
    let current_entry_count = position.entries.len() as u32;
    let support_held = current_price > support_level;

    // Get the confluence from portfolio score
    let confluence = (portfolio_score.portfolio_composite / 100.0).min(1.0);

    // Check if DCA rules allow entry
    let rules_allow = dca_rules.should_add_entry(
        current_entry_count,
        current_price,
        previous_entry_price,
        confluence,
        support_held,
    );

    if !rules_allow {
        return DCAPyramidDecision {
            should_add_entry: false,
            entry_number: current_entry_count + 1,
            position_size_pct: 0.0,
            leverage: 0.0,
            confidence: 0.0,
            rationale: "DCA rules not met for next entry".to_string(),
        };
    }

    // DCA rules allow entry - check portfolio action
    let portfolio_action_allows = matches!(
        portfolio_score.overall_action,
        PortfolioAction::FullAggressive | PortfolioAction::Balanced
    );

    if !portfolio_action_allows {
        return DCAPyramidDecision {
            should_add_entry: false,
            entry_number: current_entry_count + 1,
            position_size_pct: 0.0,
            leverage: 0.0,
            confidence: 0.0,
            rationale: format!(
                "Portfolio action {:?} doesn't support pyramiding",
                portfolio_score.overall_action
            ),
        };
    }

    // Get entry rules
    let next_entry_rules = dca_rules.get_next_entry(current_entry_count);
    if next_entry_rules.is_none() {
        return DCAPyramidDecision {
            should_add_entry: false,
            entry_number: current_entry_count + 1,
            position_size_pct: 0.0,
            leverage: 0.0,
            confidence: 0.0,
            rationale: "Max pyramid entries reached".to_string(),
        };
    }

    let entry_rules = next_entry_rules.unwrap();

    // Calculate price drop
    let price_drop_pct = ((previous_entry_price - current_price) / previous_entry_price) * 100.0;

    DCAPyramidDecision {
        should_add_entry: true,
        entry_number: current_entry_count + 1,
        position_size_pct: entry_rules.position_size_pct,
        leverage: entry_rules.leverage,
        confidence: (confluence * 100.0).min(100.0),
        rationale: format!(
            "Entry {}: Price dropped {:.1}% from {:.2}. \
             Portfolio score: {:.0}/100. Confluence: {:.1}%. \
             Support held: {}. Ready for pyramid entry.",
            current_entry_count + 1,
            price_drop_pct,
            previous_entry_price,
            portfolio_score.portfolio_composite,
            confluence * 100.0,
            support_held
        ),
    }
}

/// Create a position entry from DCA decision and scoring
pub fn create_pyramid_entry(
    decision: &DCAPyramidDecision,
    entry_price: f64,
    account_size: f64,
    timestamp: i64,
    portfolio_score: &PortfolioScore,
) -> PositionEntry {
    let position_value = account_size * decision.position_size_pct;

    PositionEntry {
        entry_number: decision.entry_number,
        entry_price,
        entry_time: timestamp,
        quantity: position_value / entry_price,
        position_size_pct: decision.position_size_pct,
        leverage: decision.leverage,
        confidence_at_entry: decision.confidence / 100.0,
        confluence_signals: portfolio_score.strategy_scores.len(),
    }
}

/// Capital staging: How much capital to deploy at each entry
#[derive(Debug, Clone)]
pub struct CapitalStaging {
    pub entry_1_pct: f64,  // 25% of total
    pub entry_2_pct: f64,  // +25% when 5% dip
    pub entry_3_pct: f64,  // +25% when 10% dip
    pub entry_4_pct: f64,  // +25% when 15% dip
}

impl CapitalStaging {
    pub fn default_dca() -> Self {
        CapitalStaging {
            entry_1_pct: 0.25,
            entry_2_pct: 0.25,
            entry_3_pct: 0.25,
            entry_4_pct: 0.25,
        }
    }

    /// Conservative: Start small, add more only if high conviction
    pub fn conservative() -> Self {
        CapitalStaging {
            entry_1_pct: 0.15,
            entry_2_pct: 0.20,
            entry_3_pct: 0.30,
            entry_4_pct: 0.35,
        }
    }

    /// Aggressive: Load early, smaller additions
    pub fn aggressive() -> Self {
        CapitalStaging {
            entry_1_pct: 0.40,
            entry_2_pct: 0.25,
            entry_3_pct: 0.20,
            entry_4_pct: 0.15,
        }
    }

    pub fn total_deployed(&self) -> f64 {
        self.entry_1_pct + self.entry_2_pct + self.entry_3_pct + self.entry_4_pct
    }
}

/// Pyramid strategy recommendation based on market regime
#[derive(Debug, Clone)]
pub struct PyramidStrategy {
    pub strategy_name: String,
    pub capital_staging: CapitalStaging,
    pub max_entries: u32,
    pub rationale: String,
}

pub fn get_pyramid_strategy_for_regime(
    regime: &str,
    portfolio_score: f64,
) -> PyramidStrategy {
    match (regime, portfolio_score) {
        // Trend regime + strong signals = aggressive pyramiding
        ("trend", s) if s > 75.0 => PyramidStrategy {
            strategy_name: "Aggressive Trend Pyramiding".to_string(),
            capital_staging: CapitalStaging::aggressive(),
            max_entries: 4,
            rationale: "Strong trend with high conviction signals. Load early, let trend run."
                .to_string(),
        },

        // Trend regime + weak signals = conservative
        ("trend", s) if s > 50.0 => PyramidStrategy {
            strategy_name: "Conservative Trend Pyramiding".to_string(),
            capital_staging: CapitalStaging::default_dca(),
            max_entries: 4,
            rationale: "Trend setup but moderate confidence. Equal staging across entries."
                .to_string(),
        },

        // Mean revert regime = equal spacing
        ("mean_revert", s) if s > 60.0 => PyramidStrategy {
            strategy_name: "Mean Reversion Pyramiding".to_string(),
            capital_staging: CapitalStaging::default_dca(),
            max_entries: 4,
            rationale: "Mean revert setup. Equal capital at each dip level."
                .to_string(),
        },

        // Breakout regime + strong = aggressive
        ("breakout", s) if s > 70.0 => PyramidStrategy {
            strategy_name: "Breakout Pyramiding".to_string(),
            capital_staging: CapitalStaging::aggressive(),
            max_entries: 3,  // Fewer entries, faster moves
            rationale: "Breakout with strong signals. Quick entries, less pyramiding."
                .to_string(),
        },

        // Crisis regime = conservative, fewer entries
        ("crisis", _) => PyramidStrategy {
            strategy_name: "Crisis Mode - Minimal Pyramiding".to_string(),
            capital_staging: CapitalStaging::conservative(),
            max_entries: 2,
            rationale: "Crisis conditions. Only enter if extreme conviction on dip."
                .to_string(),
        },

        // Default
        (_, _) => PyramidStrategy {
            strategy_name: "Balanced Pyramiding".to_string(),
            capital_staging: CapitalStaging::default_dca(),
            max_entries: 4,
            rationale: "Standard DCA pyramiding across 4 entries".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dca_pyramid_decision() {
        // Create a mock position with 1 entry
        let mut position = AggregatePosition::new("BTC".to_string());

        // Add first entry
        let entry1 = PositionEntry {
            entry_number: 1,
            entry_price: 50000.0,
            entry_time: 0,
            quantity: 0.1,
            position_size_pct: 0.25,
            leverage: 10.0,
            confidence_at_entry: 0.85,
            confluence_signals: 20,
        };
        position.add_entry(entry1);

        // Position after 5% drop (45000)
        let dca_rules = DCARules::default();
        let mock_portfolio_score = PortfolioScore {
            strategy_scores: vec![],
            portfolio_composite: 80.0,
            total_capital_used_pct: 0.25,
            portfolio_sharpe: 1.9,
            signal_correlation: 0.3,
            diversification_ratio: 1.25,
            overall_action: PortfolioAction::Balanced,
            recommendation: "Test".to_string(),
        };

        let decision = evaluate_dca_entry(
            &position,
            &dca_rules,
            &mock_portfolio_score,
            45000.0,  // 10% drop
            50000.0,  // previous entry
            48000.0,  // support
        );

        assert!(decision.should_add_entry);
        assert_eq!(decision.entry_number, 2);
    }

    #[test]
    fn test_capital_staging() {
        let default = CapitalStaging::default_dca();
        assert_eq!(default.total_deployed(), 1.0);

        let conservative = CapitalStaging::conservative();
        assert_eq!(conservative.total_deployed(), 1.0);

        let aggressive = CapitalStaging::aggressive();
        assert_eq!(aggressive.total_deployed(), 1.0);
    }

    #[test]
    fn test_pyramid_strategy_for_regime() {
        let aggressive_strategy = get_pyramid_strategy_for_regime("trend", 85.0);
        assert_eq!(aggressive_strategy.strategy_name, "Aggressive Trend Pyramiding");
        assert_eq!(aggressive_strategy.max_entries, 4);

        let crisis_strategy = get_pyramid_strategy_for_regime("crisis", 50.0);
        assert_eq!(crisis_strategy.max_entries, 2);
    }
}
