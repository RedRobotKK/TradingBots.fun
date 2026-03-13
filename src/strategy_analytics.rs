//! 🎯 Strategy Analytics & Viability Analysis for Crypto Trading
//! Determines which technical strategies actually work in cryptocurrency markets
//! Provides data-driven recommendations on strategy usage
//!
//! Analysis includes:
//! - Strategy profitability scoring
//! - Crypto-specific vs. traditional stock strategy effectiveness
//! - Market volatility impact on strategy performance
//! - Risk-adjusted returns (Sharpe, Sortino, Calmar ratios)
//! - Strategy clustering (grouping similar performing strategies)

use crate::strategy_attribution::{StrategyMetrics, MarketRegime, AttributedTrade};
use serde::{Deserialize, Serialize};

/// Strategy viability score (0-100)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyViability {
    pub name: String,
    pub viability_score: f64,        // 0-100, where 80+ = highly viable
    pub profitability_rating: String, // "Excellent" / "Good" / "Fair" / "Poor" / "Remove"
    pub minimum_trades_validated: bool, // At least 30 trades for statistical significance
    pub win_rate: f64,
    pub profit_factor: f64,
    pub sharpe_ratio: f64,
    pub monthly_return_estimate: f64, // Expected % return per month
    pub recommended_action: String,    // "Increase Weight" / "Use as-is" / "Monitor" / "Reduce Weight" / "Remove"
    pub risk_level: String,            // "Low" / "Medium" / "High"
    pub best_in_regimes: Vec<MarketRegime>,
    pub worst_in_regimes: Vec<MarketRegime>,
}

/// Crypto-specific strategy metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CryptoStrategyProfile {
    pub name: String,
    pub works_in_high_volatility: bool,  // >5% daily moves
    pub works_in_low_volatility: bool,   // <2% daily moves
    pub works_in_trending: bool,         // Strong directional moves
    pub works_in_ranging: bool,          // Sideways/consolidation
    pub signal_frequency: String,        // "Very Frequent" / "Frequent" / "Occasional" / "Rare"
    pub false_signal_rate: f64,          // % of signals that are losing trades
    pub avg_trade_duration: String,      // "Scalp" / "Short-term" / "Medium-term" / "Long-term"
    pub suitable_for_crypto: bool,       // Overall recommendation
    pub typical_win_size_pct: f64,       // Average % win per trade
    pub typical_loss_size_pct: f64,      // Average % loss per trade
}

/// Strategy comparison matrix
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyComparison {
    pub strategy_a: String,
    pub strategy_b: String,
    pub winner: String,                // Which strategy is better
    pub win_rate_difference: f64,       // How much better (%)
    pub profit_factor_difference: f64,  // Comparison ratio
    pub monthly_return_difference: f64, // % difference
    pub recommendation: String,         // Which to prefer
}

/// Market-specific strategy performance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketSpecificPerformance {
    pub strategy_name: String,
    pub market_regime: MarketRegime,
    pub effectiveness_score: f64,       // 0-100
    pub win_rate_in_regime: f64,
    pub trades_in_regime: u32,
    pub avg_pnl_in_regime: f64,
    pub recommendation: String,         // "Use heavily" / "Use cautiously" / "Avoid" / "Not enough data"
}

/// Main analytics engine
pub struct StrategyAnalytics;

impl StrategyAnalytics {
    /// Calculate viability score for a strategy
    pub fn calculate_viability(metrics: &StrategyMetrics) -> StrategyViability {
        let has_enough_data = metrics.total_signals >= 30;

        // Base score from win rate (40 points max)
        let win_rate_score = if metrics.win_rate >= 0.65 {
            40.0
        } else if metrics.win_rate >= 0.55 {
            30.0
        } else if metrics.win_rate >= 0.45 {
            15.0
        } else {
            0.0
        };

        // Profit factor score (30 points max)
        let pf_score = if metrics.profit_factor >= 3.0 {
            30.0
        } else if metrics.profit_factor >= 2.0 {
            25.0
        } else if metrics.profit_factor >= 1.5 {
            20.0
        } else if metrics.profit_factor >= 1.0 {
            10.0
        } else {
            0.0
        };

        // Sharpe ratio score (20 points max) - risk-adjusted returns
        let sharpe_score = if metrics.sharpe_ratio >= 2.0 {
            20.0
        } else if metrics.sharpe_ratio >= 1.0 {
            15.0
        } else if metrics.sharpe_ratio >= 0.5 {
            10.0
        } else if metrics.sharpe_ratio > 0.0 {
            5.0
        } else {
            0.0
        };

        // Data quality score (10 points max)
        let data_score = if has_enough_data {
            10.0
        } else if metrics.total_signals >= 20 {
            5.0
        } else {
            0.0
        };

        let total_score = win_rate_score + pf_score + sharpe_score + data_score;

        // Estimate monthly return
        let monthly_return = if metrics.avg_win_usd > 0.0 && metrics.avg_loss_usd > 0.0 {
            (metrics.win_rate * metrics.avg_win_usd - (1.0 - metrics.win_rate) * metrics.avg_loss_usd)
                * 20.0  // Approximate 20 trading days per month
        } else {
            0.0
        };

        let profitability_rating = if total_score >= 80.0 {
            "Excellent".to_string()
        } else if total_score >= 60.0 {
            "Good".to_string()
        } else if total_score >= 40.0 {
            "Fair".to_string()
        } else if total_score >= 20.0 {
            "Poor".to_string()
        } else {
            "Remove".to_string()
        };

        let recommended_action = if total_score >= 85.0 && metrics.profit_factor > 2.5 {
            "Increase Weight".to_string()
        } else if total_score >= 70.0 {
            "Use as-is".to_string()
        } else if total_score >= 50.0 {
            "Monitor".to_string()
        } else if total_score >= 30.0 {
            "Reduce Weight".to_string()
        } else {
            "Remove".to_string()
        };

        let risk_level = match metrics.sharpe_ratio {
            s if s > 1.5 => "Low".to_string(),
            s if s > 0.75 => "Medium".to_string(),
            _ => "High".to_string(),
        };

        StrategyViability {
            name: metrics.name.clone(),
            viability_score: total_score,
            profitability_rating,
            minimum_trades_validated: has_enough_data,
            win_rate: metrics.win_rate,
            profit_factor: metrics.profit_factor,
            sharpe_ratio: metrics.sharpe_ratio,
            monthly_return_estimate: monthly_return,
            recommended_action,
            risk_level,
            best_in_regimes: vec![],  // Will be populated by regime analysis
            worst_in_regimes: vec![],
        }
    }

    /// Analyze strategy for crypto-specific characteristics
    pub fn analyze_crypto_suitability(metrics: &StrategyMetrics) -> CryptoStrategyProfile {
        let false_signal_rate =
            if metrics.total_signals > 0 {
                (metrics.losing_trades as f64 / metrics.total_signals as f64) * 100.0
            } else {
                0.0
            };

        let signal_frequency = match metrics.total_signals {
            n if n > 100 => "Very Frequent".to_string(),
            n if n > 50 => "Frequent".to_string(),
            n if n > 20 => "Occasional".to_string(),
            _ => "Rare".to_string(),
        };

        let avg_trade_duration = match metrics.avg_duration_minutes {
            m if m < 5 => "Scalp".to_string(),
            m if m < 60 => "Short-term".to_string(),
            m if m < 240 => "Medium-term".to_string(),
            _ => "Long-term".to_string(),
        };

        let typical_win_size_pct = if metrics.avg_win_usd > 0.0 {
            (metrics.avg_win_usd / 100.0) * 2.0  // Rough estimate
        } else {
            0.0
        };

        let typical_loss_size_pct = if metrics.avg_loss_usd > 0.0 {
            (metrics.avg_loss_usd / 100.0) * 2.0
        } else {
            0.0
        };

        // Crypto suitability: High volatility, trending moves, quick exits
        let suitable_for_crypto = metrics.win_rate > 0.50
            && metrics.profit_factor > 1.3
            && false_signal_rate < 60.0
            && !matches!(signal_frequency.as_str(), "Rare");

        CryptoStrategyProfile {
            name: metrics.name.clone(),
            works_in_high_volatility: metrics.sharpe_ratio > 0.5,  // Handles volatility
            works_in_low_volatility: metrics.win_rate > 0.55,      // Stable performance
            works_in_trending: metrics.avg_duration_minutes < 120, // Quick exits
            works_in_ranging: metrics.win_rate > 0.58,
            signal_frequency,
            false_signal_rate,
            avg_trade_duration,
            suitable_for_crypto,
            typical_win_size_pct,
            typical_loss_size_pct,
        }
    }

    /// Compare two strategies
    pub fn compare_strategies(
        metrics_a: &StrategyMetrics,
        metrics_b: &StrategyMetrics,
    ) -> StrategyComparison {
        let winner = if metrics_a.profit_factor > metrics_b.profit_factor {
            metrics_a.name.clone()
        } else {
            metrics_b.name.clone()
        };

        let win_rate_diff = (metrics_a.win_rate - metrics_b.win_rate) * 100.0;
        let pf_diff = metrics_a.profit_factor - metrics_b.profit_factor;
        let monthly_return_diff = (metrics_a.total_pnl_usd - metrics_b.total_pnl_usd) / 100.0;

        let recommendation = if metrics_a.profit_factor > metrics_b.profit_factor * 1.2 {
            format!("Use {} preferentially", metrics_a.name)
        } else if metrics_b.profit_factor > metrics_a.profit_factor * 1.2 {
            format!("Use {} preferentially", metrics_b.name)
        } else {
            "Both have merit - use in combination".to_string()
        };

        StrategyComparison {
            strategy_a: metrics_a.name.clone(),
            strategy_b: metrics_b.name.clone(),
            winner,
            win_rate_difference: win_rate_diff.abs(),
            profit_factor_difference: pf_diff.abs(),
            monthly_return_difference: monthly_return_diff,
            recommendation,
        }
    }

    /// Analyze strategy performance in specific market regime
    pub fn analyze_regime_performance(
        strategy_name: &str,
        metrics: &StrategyMetrics,
        regime: MarketRegime,
        regime_trades: &[AttributedTrade],
    ) -> MarketSpecificPerformance {
        if regime_trades.is_empty() {
            return MarketSpecificPerformance {
                strategy_name: strategy_name.to_string(),
                market_regime: regime,
                effectiveness_score: 0.0,
                win_rate_in_regime: 0.0,
                trades_in_regime: 0,
                avg_pnl_in_regime: 0.0,
                recommendation: "Not enough data".to_string(),
            };
        }

        let trades_in_regime = regime_trades.len() as u32;
        let wins = regime_trades.iter().filter(|t| t.is_win).count() as f64;
        let win_rate = wins / trades_in_regime as f64;
        let avg_pnl = regime_trades.iter().map(|t| t.pnl_usd).sum::<f64>() / trades_in_regime as f64;

        let effectiveness_score = (win_rate * 50.0) + (avg_pnl.abs() / 10.0).min(50.0);

        let recommendation = if effectiveness_score > 70.0 {
            "Use heavily".to_string()
        } else if effectiveness_score > 50.0 {
            "Use cautiously".to_string()
        } else if effectiveness_score > 30.0 {
            "Avoid".to_string()
        } else {
            "Not enough data".to_string()
        };

        MarketSpecificPerformance {
            strategy_name: strategy_name.to_string(),
            market_regime: regime,
            effectiveness_score,
            win_rate_in_regime: win_rate,
            trades_in_regime,
            avg_pnl_in_regime: avg_pnl,
            recommendation,
        }
    }

    /// Generate viability report
    pub fn generate_viability_report(strategies: Vec<&StrategyMetrics>) -> String {
        let mut report = String::from(
            "📊 STRATEGY VIABILITY REPORT FOR CRYPTO\n\
             ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n\n"
        );

        // Sort by viability score
        let mut viabilities: Vec<_> = strategies
            .iter()
            .map(|m| Self::calculate_viability(m))
            .collect();
        viabilities.sort_by(|a, b| b.viability_score.partial_cmp(&a.viability_score).unwrap());

        // Print report sections
        report.push_str("🟢 HIGHLY VIABLE (Score 80+):\n");
        for v in viabilities.iter().filter(|v| v.viability_score >= 80.0) {
            report.push_str(&format!(
                "  ✓ {} | Score: {:.0} | WR: {:.0}% | PF: {:.2}x | Action: {}\n",
                v.name, v.viability_score, v.win_rate * 100.0, v.profit_factor, v.recommended_action
            ));
        }

        report.push_str("\n🟡 MODERATE (Score 60-79):\n");
        for v in viabilities
            .iter()
            .filter(|v| v.viability_score >= 60.0 && v.viability_score < 80.0)
        {
            report.push_str(&format!(
                "  ◐ {} | Score: {:.0} | WR: {:.0}% | PF: {:.2}x | Action: {}\n",
                v.name, v.viability_score, v.win_rate * 100.0, v.profit_factor, v.recommended_action
            ));
        }

        report.push_str("\n🔴 LOW VIABILITY (Score <60):\n");
        for v in viabilities.iter().filter(|v| v.viability_score < 60.0) {
            report.push_str(&format!(
                "  ✗ {} | Score: {:.0} | WR: {:.0}% | PF: {:.2}x | Action: {}\n",
                v.name, v.viability_score, v.win_rate * 100.0, v.profit_factor, v.recommended_action
            ));
        }

        report.push_str("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
        report.push_str(&format!(
            "Total Strategies Analyzed: {}\n\
             Viable (80+): {}\n\
             Monitor (60-79): {}\n\
             Remove (<60): {}",
            viabilities.len(),
            viabilities.iter().filter(|v| v.viability_score >= 80.0).count(),
            viabilities.iter().filter(|v| v.viability_score >= 60.0 && v.viability_score < 80.0).count(),
            viabilities.iter().filter(|v| v.viability_score < 60.0).count()
        ));

        report
    }

    /// Get crypto-specific recommendations
    pub fn crypto_recommendations(strategies: Vec<&StrategyMetrics>) -> String {
        let mut report = String::from(
            "🚀 CRYPTO-SPECIFIC STRATEGY RECOMMENDATIONS\n\
             ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n\n"
        );

        let profiles: Vec<_> = strategies
            .iter()
            .map(|m| Self::analyze_crypto_suitability(m))
            .collect();

        report.push_str("✓ RECOMMENDED FOR CRYPTO:\n");
        for p in profiles.iter().filter(|p| p.suitable_for_crypto) {
            report.push_str(&format!(
                "  • {} | High Vol: {} | Trending: {} | False Signals: {:.0}%\n",
                p.name,
                p.works_in_high_volatility,
                p.works_in_trending,
                p.false_signal_rate
            ));
        }

        report.push_str("\n✗ NOT RECOMMENDED:\n");
        for p in profiles.iter().filter(|p| !p.suitable_for_crypto) {
            report.push_str(&format!(
                "  • {} | Reason: High false signals ({:.0}%) or low win rate\n",
                p.name, p.false_signal_rate
            ));
        }

        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_viability_scoring() {
        let metrics = StrategyMetrics {
            name: "Test Strategy".to_string(),
            total_signals: 50,
            winning_trades: 35,
            losing_trades: 15,
            win_rate: 0.70,
            profit_factor: 2.5,
            avg_win_usd: 100.0,
            avg_loss_usd: 40.0,
            avg_duration_minutes: 30,
            sharpe_ratio: 1.2,
            total_pnl_usd: 2500.0,
            max_consecutive_wins: 8,
            max_consecutive_losses: 3,
        };

        let viability = StrategyAnalytics::calculate_viability(&metrics);
        println!("{:?}", viability);
        assert!(viability.viability_score > 70.0);
        assert_eq!(viability.profitability_rating, "Good");
    }
}
