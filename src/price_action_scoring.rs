//! 🎯 PRICE ACTION SCORING INTEGRATION
//!
//! Integrates institutional price action patterns with the capital-efficient scoring system
//! Allows patterns to be combined with technical strategies for confluence signals

use crate::price_action::{PriceActionPattern, PriceActionDetector, PatternType};
use crate::scoring_system::{StrategyScore, ScoringAction};
use serde::{Deserialize, Serialize};

/// Price action signal with scoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceActionScore {
    /// Pattern detected
    pub pattern: PriceActionPattern,

    /// Confidence score (0-100) from pattern analysis
    pub pattern_confidence: f64,

    /// Entry quality score (0-100): How clean is the entry?
    pub entry_quality: f64,

    /// Risk/Reward quality (0-100): How good is the RR ratio?
    pub rr_quality: f64,

    /// Confluence score (0-100): How many patterns align?
    pub confluence: f64,

    /// Overall institutional score (0-100)
    pub institutional_score: f64,

    /// Recommended action
    pub action: ScoringAction,

    /// Rationale
    pub rationale: String,
}

/// Scorer for price action patterns
pub struct PriceActionScorer;

impl PriceActionScorer {
    /// Score a single pattern
    pub fn score_pattern(
        pattern: &PriceActionPattern,
        detector: &PriceActionDetector,
    ) -> PriceActionScore {
        let pattern_confidence = pattern.confidence;

        // Entry quality: How clean is the entry?
        // Measured by how tight the stop is relative to entry
        let stop_distance = (pattern.stop_zone - pattern.entry_price).abs();
        let entry_quality = Self::calculate_entry_quality(&pattern);

        // RR quality: How good is the risk/reward?
        let rr_quality = Self::calculate_rr_quality(&pattern);

        // Confluence: Do other patterns align?
        let confluence = detector.get_confluence_score();

        // Overall score: Weighted combination
        let institutional_score = (pattern_confidence * 0.40)
            + (entry_quality * 0.25)
            + (rr_quality * 0.20)
            + (confluence * 0.15);

        let action = Self::score_to_action(institutional_score);

        let rationale = Self::generate_rationale(
            &pattern,
            institutional_score,
            entry_quality,
            rr_quality,
            confluence,
        );

        PriceActionScore {
            pattern: pattern.clone(),
            pattern_confidence,
            entry_quality,
            rr_quality,
            confluence,
            institutional_score: institutional_score.min(100.0),
            action,
            rationale,
        }
    }

    /// Calculate entry quality score
    fn calculate_entry_quality(pattern: &PriceActionPattern) -> f64 {
        let stop_distance = (pattern.stop_zone - pattern.entry_price).abs();
        let expected_move = pattern.expected_move;

        // Good entry: stop is tight relative to target
        let entry_ratio = stop_distance / expected_move;

        // Prefer tighter stops (0.15-0.30 is ideal)
        if entry_ratio < 0.20 {
            85.0 + ((0.20 - entry_ratio) / 0.20 * 15.0).min(15.0)
        } else if entry_ratio < 0.30 {
            75.0 + ((0.30 - entry_ratio) / 0.10 * 10.0)
        } else if entry_ratio < 0.50 {
            65.0 - ((entry_ratio - 0.30) / 0.20 * 15.0)
        } else {
            40.0
        }
    }

    /// Calculate RR quality score
    fn calculate_rr_quality(pattern: &PriceActionPattern) -> f64 {
        let rr = pattern.risk_reward_ratio;

        // Excellent: RR > 2.5
        // Good: RR 1.5-2.5
        // Acceptable: RR 1.0-1.5
        // Poor: RR < 1.0

        if rr >= 2.5 {
            90.0 + ((rr - 2.5) / 2.5 * 10.0).min(10.0)
        } else if rr >= 1.5 {
            80.0 + ((rr - 1.5) / 1.0 * 10.0)
        } else if rr >= 1.0 {
            70.0 - ((1.5 - rr) * 10.0)
        } else {
            50.0 - ((1.0 - rr) * 50.0).min(50.0)
        }
    }

    /// Convert score to action
    fn score_to_action(score: f64) -> ScoringAction {
        match score {
            s if s >= 80.0 => ScoringAction::StrongTrade,
            s if s >= 65.0 => ScoringAction::Trade,
            s if s >= 50.0 => ScoringAction::WeakTrade,
            s if s >= 30.0 => ScoringAction::Monitor,
            _ => ScoringAction::Skip,
        }
    }

    /// Generate detailed rationale
    fn generate_rationale(
        pattern: &PriceActionPattern,
        score: f64,
        entry_quality: f64,
        rr_quality: f64,
        confluence: f64,
    ) -> String {
        let mut rationale = format!(
            "{} | Conf: {:.0}% | Entry: {:.0}% | RR: {:.0}% | Confluence: {:.0}%",
            pattern.pattern_type.as_str(),
            pattern.confidence,
            entry_quality,
            rr_quality,
            confluence
        );

        // Add specific insights
        if rr_quality > 85.0 {
            rationale.push_str(" | Excellent Risk/Reward");
        }

        if entry_quality > 85.0 {
            rationale.push_str(" | Tight Entry");
        }

        if confluence > 70.0 {
            rationale.push_str(" | Multi-Pattern Confluence");
        }

        if pattern.risk_reward_ratio > 3.0 {
            rationale.push_str(" | 3:1+ Reward");
        }

        rationale
    }

    /// Score patterns for portfolio-level confluence
    pub fn score_pattern_set(
        patterns: &[PriceActionPattern],
        detector: &PriceActionDetector,
    ) -> Vec<PriceActionScore> {
        patterns
            .iter()
            .map(|p| Self::score_pattern(p, detector))
            .collect()
    }

    /// Get best pattern by score
    pub fn get_best_pattern(patterns: &[PriceActionPattern], detector: &PriceActionDetector) -> Option<PriceActionScore> {
        patterns
            .iter()
            .map(|p| Self::score_pattern(p, detector))
            .max_by(|a, b| a.institutional_score.partial_cmp(&b.institutional_score).unwrap())
    }

    /// Combine price action score with technical strategy score
    pub fn combine_scores(
        price_action_score: &PriceActionScore,
        technical_score: &StrategyScore,
    ) -> f64 {
        // 50/50 weight between institutional price action and technical analysis
        let combined = (price_action_score.institutional_score * 0.50)
            + (technical_score.composite_score * 0.50);

        // Bonus if both strongly agree
        let agreement_bonus = if price_action_score.action == technical_score.action {
            5.0
        } else {
            0.0
        };

        (combined + agreement_bonus).min(100.0)
    }
}

/// Confluence analysis for multiple pattern types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternConfluence {
    /// All patterns detected
    pub patterns: Vec<PriceActionPattern>,

    /// Confluent patterns (all agree on direction)
    pub confluent_patterns: Vec<PriceActionPattern>,

    /// Confluence strength (0-100)
    pub confluence_strength: f64,

    /// Direction agreement: 1.0 = all agree up, 0.0 = all agree down, 0.5 = conflicted
    pub direction_agreement: f64,

    /// Recommendation
    pub recommendation: String,
}

impl PatternConfluence {
    /// Analyze multiple patterns for confluence
    pub fn analyze(patterns: Vec<PriceActionPattern>) -> Self {
        if patterns.is_empty() {
            return Self {
                patterns: vec![],
                confluent_patterns: vec![],
                confluence_strength: 0.0,
                direction_agreement: 0.5,
                recommendation: "No patterns detected".to_string(),
            };
        }

        // Determine primary direction
        let bullish_patterns = patterns.iter().filter(|p| Self::is_bullish(p)).count();
        let bearish_patterns = patterns.len() - bullish_patterns;

        let direction_agreement = if patterns.len() > 0 {
            bullish_patterns as f64 / patterns.len() as f64
        } else {
            0.5
        };

        let is_mostly_bullish = direction_agreement > 0.6;
        let is_mostly_bearish = direction_agreement < 0.4;
        let is_conflicted = !is_mostly_bullish && !is_mostly_bearish;

        // Get confluent patterns (those that agree with majority)
        let confluent_patterns: Vec<PriceActionPattern> = patterns
            .iter()
            .filter(|p| {
                let is_bullish = Self::is_bullish(p);
                (is_mostly_bullish && is_bullish) || (is_mostly_bearish && !is_bullish)
            })
            .cloned()
            .collect();

        // Calculate confluence strength
        let confluence_strength = if is_conflicted {
            40.0
        } else {
            let pattern_count = patterns.len();
            50.0 + ((pattern_count as f64 / 5.0) * 30.0).min(50.0)
                - ((patterns.len() - confluent_patterns.len()) as f64 * 10.0)
        };

        let recommendation = if is_conflicted {
            "CONFLICTED: Patterns disagree. Wait for clarity.".to_string()
        } else if is_mostly_bullish {
            format!(
                "BULLISH CONFLUENCE: {} patterns agree. Score: {:.0}",
                confluent_patterns.len(),
                confluence_strength
            )
        } else {
            format!(
                "BEARISH CONFLUENCE: {} patterns agree. Score: {:.0}",
                confluent_patterns.len(),
                confluence_strength
            )
        };

        Self {
            patterns,
            confluent_patterns,
            confluence_strength: confluence_strength.min(100.0).max(0.0),
            direction_agreement,
            recommendation,
        }
    }

    /// Determine if pattern is bullish
    fn is_bullish(pattern: &PriceActionPattern) -> bool {
        // Pattern is bullish if targets are above entry
        pattern.targets.iter().any(|t| *t > pattern.entry_price)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entry_quality_calculation() {
        let quality = PriceActionScorer::calculate_entry_quality(&PriceActionPattern {
            pattern_type: PatternType::CompressionExpansion,
            confidence: 75.0,
            entry_price: 100.0,
            stop_zone: 98.0,
            targets: vec![110.0],
            formation_candle: 0,
            trigger_candle: Some(5),
            is_triggered: true,
            description: "Test".to_string(),
            risk_reward_ratio: 5.0,
            expected_move: 10.0,
        });

        assert!(quality > 80.0); // Tight stop should score well
    }

    #[test]
    fn test_confluence_analysis() {
        let patterns = vec![
            PriceActionPattern {
                pattern_type: PatternType::CompressionExpansion,
                confidence: 80.0,
                entry_price: 100.0,
                stop_zone: 98.0,
                targets: vec![110.0],
                formation_candle: 0,
                trigger_candle: Some(1),
                is_triggered: true,
                description: "Pattern 1".to_string(),
                risk_reward_ratio: 5.0,
                expected_move: 10.0,
            },
            PriceActionPattern {
                pattern_type: PatternType::LiquidityGrab,
                confidence: 75.0,
                entry_price: 100.0,
                stop_zone: 99.0,
                targets: vec![112.0],
                formation_candle: 0,
                trigger_candle: Some(2),
                is_triggered: true,
                description: "Pattern 2".to_string(),
                risk_reward_ratio: 12.0,
                expected_move: 12.0,
            },
        ];

        let confluence = PatternConfluence::analyze(patterns);
        assert!(confluence.confluence_strength > 50.0);
    }
}
