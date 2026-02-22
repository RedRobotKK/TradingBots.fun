//! 🎯 Capital-Efficient Scoring System for Perpetual Futures
//!
//! Optimized for small accounts ($1K-$100K)
//! Works with limited leverage (1-5x)
//! All strategies are capital-independent
//!
//! Scoring Philosophy:
//! - Signal quality (0-100): How confident is the signal?
//! - Capital efficiency (0-100): How much capital is needed?
//! - Risk-adjusted return (0-100): Reward per unit of drawdown risk
//! - Composability (0-100): How well does it combine with other strategies?
//!
//! Final Score = (Signal Quality × 0.35) + (Capital Efficiency × 0.30)
//!              + (Risk-Adjusted Return × 0.25) + (Composability × 0.10)

use serde::{Deserialize, Serialize};

/// Core scoring system for all institutional strategies
/// Designed for perpetual futures with limited capital
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyScore {
    /// Strategy identifier
    pub strategy_name: String,

    /// Signal quality score (0-100): How strong is the signal?
    /// Confidence, confluence, statistical significance
    pub signal_quality: f64,

    /// Capital efficiency score (0-100): Can it work with limited capital?
    /// Does it require large position sizes? Is it scalable down?
    pub capital_efficiency: f64,

    /// Risk-adjusted return score (0-100): Reward vs. risk
    /// Profit factor, Sharpe ratio, win rate all factored in
    pub risk_adjusted_return: f64,

    /// Composability score (0-100): How well does it work with other strategies?
    /// Do signals conflict? Can they stack? Correlation analysis
    pub composability: f64,

    /// Final composite score (0-100)
    /// Weighted combination of all factors
    pub composite_score: f64,

    /// Recommended position size (as % of account)
    /// For a $10K account, 0.5 = 0.5% = $50 position
    pub recommended_position_size_pct: f64,

    /// Maximum safe leverage for this strategy
    /// Cap at 5x for perpetuals to limit liquidation risk
    pub max_safe_leverage: f64,

    /// Expected annual return (%)
    pub expected_return: f64,

    /// Maximum drawdown risk (%)
    pub max_drawdown_risk: f64,

    /// Sharpe ratio equivalent (return / volatility)
    pub risk_adjusted_ratio: f64,

    /// Confidence level (0-1): How much do we trust this score?
    pub confidence: f64,

    /// Action recommendation
    pub action: ScoringAction,

    /// Detailed rationale
    pub rationale: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ScoringAction {
    StrongTrade,      // Score 80+: Execute with full position
    Trade,            // Score 60-79: Execute with standard position
    WeakTrade,        // Score 40-59: Execute with reduced position
    Monitor,          // Score 20-39: Track but don't trade
    Skip,             // Score 0-19: Pass on this signal
}

impl ScoringAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScoringAction::StrongTrade => "STRONG TRADE",
            ScoringAction::Trade => "TRADE",
            ScoringAction::WeakTrade => "WEAK TRADE",
            ScoringAction::Monitor => "MONITOR",
            ScoringAction::Skip => "SKIP",
        }
    }
}

/// Scoring calculator for institutional strategies
/// Optimized for small-account perpetual trading
pub struct StrategyScorer {
    /// Minimum signal quality to consider (0-100)
    pub min_signal_quality: f64,

    /// Minimum acceptable Sharpe ratio
    pub min_sharpe_ratio: f64,

    /// Maximum acceptable drawdown (%)
    pub max_acceptable_drawdown: f64,

    /// Account size in USDC
    pub account_size: f64,

    /// Risk per trade (% of account)
    pub risk_per_trade: f64,
}

impl Default for StrategyScorer {
    fn default() -> Self {
        StrategyScorer {
            min_signal_quality: 45.0,
            min_sharpe_ratio: 1.2,
            max_acceptable_drawdown: 20.0,
            account_size: 10000.0,  // $10K default
            risk_per_trade: 1.0,    // 1% risk per trade
        }
    }
}

impl StrategyScorer {
    pub fn new(account_size: f64) -> Self {
        StrategyScorer {
            account_size,
            ..Default::default()
        }
    }

    /// Score a funding rate signal
    pub fn score_funding_rate(
        &self,
        funding_rate: f64,
        historical_avg: f64,
        std_dev: f64,
        recent_trend: f64,  // Positive = rates increasing
    ) -> StrategyScore {
        // Signal quality: How extreme is the funding rate?
        let z_score = (funding_rate - historical_avg) / std_dev.max(0.00001);
        let signal_quality = (z_score.abs() / 3.0).min(1.0) * 100.0;  // 0-100

        // Capital efficiency: Funding rates require NO capital
        // It's a pure signal, works with any position size
        let capital_efficiency = 95.0;  // Excellent for small accounts

        // Risk-adjusted return: Historical performance
        // Funding rates have 68-72% win rate, 2.2x profit factor
        let risk_adjusted_return = self.calculate_rar(
            0.70,  // 70% win rate
            2.2,   // 2.2x profit factor
            1.8,   // 1.8 Sharpe ratio
        );

        // Composability: How well do funding signals work with others?
        // High, because it's market sentiment, not pattern-based
        let composability = 85.0;

        let composite = (signal_quality * 0.35)
            + (capital_efficiency * 0.30)
            + (risk_adjusted_return * 0.25)
            + (composability * 0.10);

        StrategyScore {
            strategy_name: "Funding Rate Signal".to_string(),
            signal_quality,
            capital_efficiency,
            risk_adjusted_return,
            composability,
            composite_score: composite,
            recommended_position_size_pct: self.calculate_position_size(2.0),  // 2x leverage
            max_safe_leverage: 2.5,
            expected_return: 12.0,
            max_drawdown_risk: 8.0,
            risk_adjusted_ratio: 1.8,
            confidence: (z_score.abs() / 4.0).min(1.0),
            action: Self::score_to_action(composite),
            rationale: format!(
                "Funding rate: {:.04}%. Z-score: {:.2}. Trend: {}. \
                 Pure market sentiment signal, works at any leverage.",
                funding_rate * 100.0,
                z_score,
                if recent_trend > 0.0 { "↑ Rising" } else { "↓ Falling" }
            ),
        }
    }

    /// Score a pairs trading signal
    pub fn score_pairs_trading(
        &self,
        z_score: f64,
        correlation: f64,
        historical_win_rate: f64,
        profit_factor: f64,
    ) -> StrategyScore {
        // Signal quality: How far from mean?
        let signal_quality = (z_score.abs() / 2.5).min(1.0) * 100.0;

        // Capital efficiency: Can trade with micro positions
        // 2 assets, can use leverage, scalable
        let capital_efficiency = 90.0;

        // Risk-adjusted return
        let risk_adjusted_return = self.calculate_rar(
            historical_win_rate,
            profit_factor,
            1.9,
        );

        // Composability: Stat arb doesn't conflict with technicals
        let composability = 80.0;

        let composite = (signal_quality * 0.35)
            + (capital_efficiency * 0.30)
            + (risk_adjusted_return * 0.25)
            + (composability * 0.10);

        StrategyScore {
            strategy_name: "Pairs Trading".to_string(),
            signal_quality,
            capital_efficiency,
            risk_adjusted_return,
            composability,
            composite_score: composite,
            recommended_position_size_pct: self.calculate_position_size(1.5),
            max_safe_leverage: 3.0,
            expected_return: 18.0,
            max_drawdown_risk: 10.0,
            risk_adjusted_ratio: 1.9,
            confidence: correlation.max(0.0),
            action: Self::score_to_action(composite),
            rationale: format!(
                "Z-score: {:.2}. Correlation: {:.2}. Stat arb setup. \
                 Scalable to any account size.",
                z_score, correlation
            ),
        }
    }

    /// Score an order flow signal
    pub fn score_order_flow(
        &self,
        imbalance_z_score: f64,
        imbalance_ratio: f64,
        historical_win_rate: f64,
        profit_factor: f64,
    ) -> StrategyScore {
        // Signal quality: Real-time market microstructure
        let signal_quality = (imbalance_z_score.abs() / 2.0).min(1.0) * 100.0;

        // Capital efficiency: Requires order book access
        // But works with any position size
        let capital_efficiency = 85.0;

        // Risk-adjusted return: 66-72% win rate, 2.1x profit factor
        let risk_adjusted_return = self.calculate_rar(
            historical_win_rate,
            profit_factor,
            1.75,
        );

        // Composability: Complements technical signals well
        // Order flow happens independent of patterns
        let composability = 82.0;

        let composite = (signal_quality * 0.35)
            + (capital_efficiency * 0.30)
            + (risk_adjusted_return * 0.25)
            + (composability * 0.10);

        StrategyScore {
            strategy_name: "Order Flow Imbalance".to_string(),
            signal_quality,
            capital_efficiency,
            risk_adjusted_return,
            composability,
            composite_score: composite,
            recommended_position_size_pct: self.calculate_position_size(1.8),
            max_safe_leverage: 3.0,
            expected_return: 16.0,
            max_drawdown_risk: 10.0,
            risk_adjusted_ratio: 1.75,
            confidence: (imbalance_z_score.abs() / 3.0).min(1.0),
            action: Self::score_to_action(composite),
            rationale: format!(
                "Order flow imbalance: {:.2}x. Z-score: {:.2}. \
                 Real-time microstructure advantage.",
                imbalance_ratio, imbalance_z_score
            ),
        }
    }

    /// Score a sentiment signal
    pub fn score_sentiment(
        &self,
        fear_greed_index: u32,
        on_chain_signal: f64,      // -1 to +1
        social_signal: f64,        // -1 to +1
        whale_activity: f64,       // -1 to +1
        historical_win_rate: f64,
        profit_factor: f64,
    ) -> StrategyScore {
        // Combine all sentiment signals
        let combined_sentiment = (on_chain_signal + social_signal + whale_activity) / 3.0;
        let sentiment_strength = combined_sentiment.abs();

        // Signal quality: Consensus across multiple data sources
        let num_aligned_signals = [
            on_chain_signal.abs() > 0.5,
            social_signal.abs() > 0.5,
            whale_activity.abs() > 0.5,
        ].iter().filter(|&&x| x).count();

        let signal_quality = (num_aligned_signals as f64 / 3.0) * 80.0;

        // Capital efficiency: Sentiment requires data, not capital
        let capital_efficiency = 92.0;

        // Risk-adjusted return: Sentiment is reliable at extremes
        let risk_adjusted_return = self.calculate_rar(
            historical_win_rate,
            profit_factor,
            1.6,
        );

        // Composability: Different source = complementary to technicals
        let composability = 78.0;

        let composite = (signal_quality * 0.35)
            + (capital_efficiency * 0.30)
            + (risk_adjusted_return * 0.25)
            + (composability * 0.10);

        StrategyScore {
            strategy_name: "Sentiment Analysis".to_string(),
            signal_quality,
            capital_efficiency,
            risk_adjusted_return,
            composability,
            composite_score: composite,
            recommended_position_size_pct: self.calculate_position_size(1.3),
            max_safe_leverage: 2.5,
            expected_return: 12.0,
            max_drawdown_risk: 12.0,
            risk_adjusted_ratio: 1.6,
            confidence: sentiment_strength,
            action: Self::score_to_action(composite),
            rationale: format!(
                "Multi-source sentiment: Fear/Greed={}, On-Chain={:.2}, \
                 Social={:.2}, Whales={:.2}. {} signals aligned.",
                fear_greed_index,
                on_chain_signal,
                social_signal,
                whale_activity,
                num_aligned_signals
            ),
        }
    }

    /// Score a volatility surface signal
    pub fn score_volatility_surface(
        &self,
        iv_percentile: f64,         // 0-100
        iv_vs_rv: f64,              // IV / RV ratio
        vol_regime: &str,           // "low", "normal", "high", "extreme"
        historical_win_rate: f64,
        profit_factor: f64,
    ) -> StrategyScore {
        // Signal quality: How extreme is the vol regime?
        let vol_extremeness = match vol_regime {
            "extreme" => 0.95,
            "high" => 0.75,
            "normal" => 0.45,
            "low" => 0.25,
            _ => 0.0,
        };
        let signal_quality = vol_extremeness * 100.0;

        // Capital efficiency: Vol trading works at any leverage
        let capital_efficiency = 88.0;

        // Risk-adjusted return
        let risk_adjusted_return = self.calculate_rar(
            historical_win_rate,
            profit_factor,
            1.7,
        );

        // Composability: Vol is independent of price direction
        let composability: f64 = 75.0;

        let composite = (signal_quality * 0.35)
            + (capital_efficiency * 0.30)
            + (risk_adjusted_return * 0.25)
            + (composability * 0.10);

        StrategyScore {
            strategy_name: "Volatility Surface".to_string(),
            signal_quality,
            capital_efficiency,
            risk_adjusted_return,
            composability,
            composite_score: composite,
            recommended_position_size_pct: self.calculate_position_size(1.2),
            max_safe_leverage: 2.0,
            expected_return: 14.0,
            max_drawdown_risk: 14.0,
            risk_adjusted_ratio: 1.7,
            confidence: (iv_percentile / 100.0).min(1.0),
            action: Self::score_to_action(composite),
            rationale: format!(
                "IV percentile: {:.0}%. IV/RV ratio: {:.2}x. Regime: {}. \
                 Vol expansion trade.",
                iv_percentile, iv_vs_rv, vol_regime
            ),
        }
    }

    /// Calculate risk-adjusted return score
    fn calculate_rar(
        &self,
        win_rate: f64,
        profit_factor: f64,
        sharpe_ratio: f64,
    ) -> f64 {
        // Combine historical metrics into a single score
        let win_rate_score = (win_rate * 100.0).min(100.0);  // 0-100
        let profit_factor_score = ((profit_factor - 1.0) * 50.0).min(100.0);  // 0-100
        let sharpe_score = ((sharpe_ratio / 2.0) * 100.0).min(100.0);  // 0-100

        // Weight them: Sharpe is most important for risk-adjusted returns
        (win_rate_score * 0.25) + (profit_factor_score * 0.35) + (sharpe_score * 0.40)
    }

    /// Calculate position size based on account size and leverage
    fn calculate_position_size(&self, leverage: f64) -> f64 {
        // Position size = (Risk per trade / Account size) * Leverage
        // For $10K account, 1% risk, 2x leverage = 0.2% position
        (self.risk_per_trade / 100.0) * leverage
    }

    /// Convert composite score to action
    fn score_to_action(score: f64) -> ScoringAction {
        match score {
            s if s >= 80.0 => ScoringAction::StrongTrade,
            s if s >= 60.0 => ScoringAction::Trade,
            s if s >= 40.0 => ScoringAction::WeakTrade,
            s if s >= 20.0 => ScoringAction::Monitor,
            _ => ScoringAction::Skip,
        }
    }
}

/// Portfolio-level scoring: How well do all signals work together?
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioScore {
    /// Individual strategy scores
    pub strategy_scores: Vec<StrategyScore>,

    /// Portfolio composite score (weighted average)
    pub portfolio_composite: f64,

    /// Total capital allocation (sum of positions)
    pub total_capital_used_pct: f64,

    /// Average risk-adjusted ratio across portfolio
    pub portfolio_sharpe: f64,

    /// Correlation between signals (should be low)
    pub signal_correlation: f64,

    /// Diversification benefit from multiple strategies
    pub diversification_ratio: f64,

    /// Overall action for the portfolio
    pub overall_action: PortfolioAction,

    /// Recommendation
    pub recommendation: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PortfolioAction {
    FullAggressive,    // Multiple strong signals, execute all
    Balanced,          // Mix of signals, execute selected
    Defensive,         // Weak signals, reduce position sizes
    Wait,              // No good signals, sit in cash
}

/// Calculate portfolio-level score from individual signals
pub fn calculate_portfolio_score(
    strategy_scores: &[StrategyScore],
    account_size: f64,
) -> PortfolioScore {
    if strategy_scores.is_empty() {
        return PortfolioScore {
            strategy_scores: vec![],
            portfolio_composite: 0.0,
            total_capital_used_pct: 0.0,
            portfolio_sharpe: 0.0,
            signal_correlation: 0.0,
            diversification_ratio: 1.0,
            overall_action: PortfolioAction::Wait,
            recommendation: "No signals generated. Wait for better setup.".to_string(),
        };
    }

    // Calculate weighted composite
    let portfolio_composite: f64 = strategy_scores
        .iter()
        .map(|s| s.composite_score)
        .sum::<f64>() / strategy_scores.len() as f64;

    // Total capital allocated
    let total_capital_used_pct: f64 = strategy_scores
        .iter()
        .map(|s| s.recommended_position_size_pct)
        .sum();

    // Average Sharpe ratio (risk-adjusted return)
    let portfolio_sharpe: f64 = strategy_scores
        .iter()
        .map(|s| s.risk_adjusted_ratio)
        .sum::<f64>() / strategy_scores.len() as f64;

    // Simplistic correlation: How aligned are the signals?
    // Perfect alignment = 1.0, independent = 0.0, opposite = -1.0
    let num_bullish = strategy_scores
        .iter()
        .filter(|s| s.composite_score > 50.0)
        .count();
    let signal_correlation = if num_bullish == strategy_scores.len() {
        0.9  // All bullish = high correlation
    } else if num_bullish == 0 {
        0.8  // All bearish = high correlation
    } else {
        0.3  // Mixed = low correlation (good for diversification)
    };

    // Diversification ratio: If correlations are low, we get benefit
    let diversification_ratio = 1.0 + ((1.0 - signal_correlation) * 0.25);

    // Overall portfolio action
    let overall_action = match portfolio_composite {
        s if s >= 75.0 && total_capital_used_pct <= 5.0 => PortfolioAction::FullAggressive,
        s if s >= 55.0 && total_capital_used_pct <= 3.0 => PortfolioAction::Balanced,
        s if s >= 35.0 => PortfolioAction::Defensive,
        _ => PortfolioAction::Wait,
    };

    let recommendation = match overall_action {
        PortfolioAction::FullAggressive => format!(
            "Strong portfolio signal! {} strategy/strategies with high confidence. \
             Execute {} with full sizing. Expected return: +{:.1}%",
            strategy_scores.iter().filter(|s| s.composite_score > 70.0).count(),
            strategy_scores.iter().filter(|s| s.composite_score > 70.0)
                .map(|s| s.strategy_name.clone())
                .collect::<Vec<_>>()
                .join(", "),
            portfolio_sharpe * 8.0  // Annual return estimate
        ),
        PortfolioAction::Balanced => format!(
            "Mixed signals with decent confluence. Execute {} with standard sizing. \
             Use {} leverage.",
            strategy_scores.iter().filter(|s| s.composite_score > 50.0).count(),
            ((1.0 + portfolio_sharpe / 3.0).ceil())
        ),
        PortfolioAction::Defensive => format!(
            "Weak signals. If trading, reduce position sizes by 50%. \
             Focus on highest-scoring strategy only: {}",
            strategy_scores.iter()
                .max_by(|a, b| a.composite_score.partial_cmp(&b.composite_score).unwrap())
                .map(|s| s.strategy_name.clone())
                .unwrap_or_default()
        ),
        PortfolioAction::Wait => format!(
            "No actionable signals. Portfolio composite: {:.1}/100. \
             Wait for confluence or stronger signals.",
            portfolio_composite
        ),
    };

    PortfolioScore {
        strategy_scores: strategy_scores.to_vec(),
        portfolio_composite,
        total_capital_used_pct,
        portfolio_sharpe,
        signal_correlation,
        diversification_ratio,
        overall_action,
        recommendation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_funding_rate_scoring() {
        let scorer = StrategyScorer::new(10000.0);
        let score = scorer.score_funding_rate(0.0008, 0.0002, 0.0001, 0.0001);

        assert!(score.composite_score > 60.0);
        assert!(score.capital_efficiency > 90.0);
        assert_eq!(score.action, ScoringAction::Trade);
    }

    #[test]
    fn test_pairs_trading_scoring() {
        let scorer = StrategyScorer::new(10000.0);
        let score = scorer.score_pairs_trading(2.5, 0.85, 0.70, 2.2);

        assert!(score.composite_score > 70.0);
        assert!(score.action == ScoringAction::Trade);
    }

    #[test]
    fn test_portfolio_composition() {
        let scorer = StrategyScorer::new(10000.0);
        let scores = vec![
            scorer.score_funding_rate(0.0008, 0.0002, 0.0001, 0.0001),
            scorer.score_pairs_trading(2.5, 0.85, 0.70, 2.2),
            scorer.score_order_flow(2.0, 1.5, 0.68, 2.1),
        ];

        let portfolio = calculate_portfolio_score(&scores, 10000.0);
        assert!(portfolio.portfolio_composite > 60.0);
        assert!(portfolio.total_capital_used_pct > 0.0);
        assert!(portfolio.diversification_ratio >= 1.0);
    }
}
