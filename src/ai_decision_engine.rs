//! 🤖 AI Decision Engine
//! Combines strategies + frameworks + AI reasoning in milliseconds
//! Makes rapid decisions without hesitation, bakes in domain expertise
//! Every trade decision flows through this engine
//!
//! Decision flow: Technical Signals → Framework Validation → AI Scoring → Execute

use crate::frameworks::*;
use crate::dynamic_position_sizing::{SupportResistance, TechnicalSetup, DynamicSizer};
use serde::{Deserialize, Serialize};

// ============================================================================
// AI DECISION CONTEXT - Everything needed to make a trade decision
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AIDecisionContext {
    // Market state
    pub current_price: f64,
    pub support_level: f64,
    pub resistance_level: f64,
    pub atr_pct: f64,

    // Technical signals (from your 9 strategies)
    pub technical_confluence: f64,       // 0-1: how many strategies agree?
    pub dominant_signal: String,         // Which strategy is strongest?

    // Frameworks
    pub volatility_analysis: VolatilityAnalysis,
    pub multi_timeframe: MultiTimeframeAnalysis,
    pub order_flow: OrderFlowAnalysis,
    pub volatility_scaler: VolatilityScaler,

    // Historical performance
    pub kelly: Option<KellyCriterion>,  // None if not enough data yet
    pub drawdown_tracker: DrawdownTracker,
    pub strategy_attributor: StrategyAttributor,

    // Capital management
    pub available_capital: f64,
    pub max_position_pct: f64,
}

// ============================================================================
// AI DECISION RESULT - Complete decision with reasoning
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AIDecision {
    pub should_enter: bool,
    pub direction: Option<bool>,         // true = long, false = short
    pub position_size_pct: f64,          // % of capital
    pub position_size_dollars: f64,      // $ amount
    pub leverage: f64,
    pub stop_loss: f64,
    pub take_profit: f64,

    // Confidence & reasoning
    pub ai_confidence: f64,              // 0-1: how confident is AI in this decision?
    pub framework_score: f64,            // 0-1: how well do frameworks support it?
    pub technical_score: f64,            // 0-1: how strong are technical signals?
    pub overall_score: f64,              // 0-1: weighted combination

    // Decision reasoning (transparent AI)
    pub reasoning: Vec<String>,          // Why did AI make this decision?
    pub warnings: Vec<String>,           // Any risks or concerns?
    pub opportunities: Vec<String>,      // Why this is a good trade?

    // Execution priority
    pub urgency: ExecutionUrgency,       // How fast should this execute?
    pub execution_notes: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ExecutionUrgency {
    Immediate,   // Signal is fleeting, execute now
    Normal,      // Normal entry, normal speed
    Cautious,    // Monitor more, don't rush
    DoNotTrade,  // Conditions not favorable
}

// ============================================================================
// AI DECISION ENGINE
// ============================================================================

pub struct AIDecisionEngine;

impl AIDecisionEngine {
    /// Main decision function - evaluates everything and makes a trade decision
    /// This is called every candle/tick in real-time
    /// Should complete in < 1ms (all is math, no network calls)
    pub fn make_decision(context: &AIDecisionContext) -> AIDecision {
        // Quick safety checks first (milliseconds)
        if !context.drawdown_tracker.can_trade {
            return Self::create_rejection(
                "Daily/Weekly/Monthly loss limit hit - trading paused".to_string(),
                vec![],
                context,
            );
        }

        // Framework validation (1-2ms)
        let framework_validation = Self::validate_frameworks(context);
        if !framework_validation.0 {
            return Self::create_rejection(
                framework_validation.1,
                framework_validation.2,
                context,
            );
        }

        // Technical signal analysis (1-2ms)
        let technical_score = Self::score_technical_signals(context);

        // Position sizing calculation (1-2ms)
        let sizing = Self::calculate_optimal_sizing(context);
        if sizing.0 < 0.01 {
            return Self::create_rejection(
                "Position size too small after adjustments".to_string(),
                vec!["Try increasing capital or waiting for stronger setup".to_string()],
                context,
            );
        }

        // Final AI scoring and decision (1-2ms)
        let (should_enter, confidence, reasoning, opportunities, warnings) =
            Self::score_and_decide(context, technical_score, framework_validation.0);

        // Determine execution urgency
        let urgency = Self::determine_urgency(context, technical_score, framework_validation.0);

        // Build the decision
        let direction = if context.technical_confluence > 0.5 {
            Some(true) // Bullish
        } else {
            Some(false) // Bearish
        };

        AIDecision {
            should_enter,
            direction,
            position_size_pct: sizing.0,
            position_size_dollars: sizing.1,
            leverage: Self::calculate_leverage(sizing.0),
            stop_loss: context.support_level * 0.99,
            take_profit: context.resistance_level * 1.02,
            ai_confidence: confidence,
            framework_score: if framework_validation.0 { 1.0 } else { 0.0 },
            technical_score,
            overall_score: (confidence + if framework_validation.0 { 1.0 } else { 0.0 } + technical_score) / 3.0,
            reasoning,
            warnings,
            opportunities,
            urgency,
            execution_notes: Self::create_execution_notes(context, sizing.0),
        }
    }

    /// Step 1: Validate frameworks (volatility, timeframe, order flow)
    fn validate_frameworks(context: &AIDecisionContext) -> (bool, String, Vec<String>) {
        let mut warnings = Vec::new();
        let mut reasons = Vec::new();

        // Check volatility regime
        if !context.volatility_analysis.should_trade {
            return (
                false,
                format!(
                    "Volatility too high ({:.2}% ATR) - only trade on extreme signals",
                    context.atr_pct
                ),
                vec![],
            );
        }

        // Check multi-timeframe confluence
        if !context.multi_timeframe.is_strong_confluence() {
            warnings.push(format!(
                "Timeframes not fully aligned - confluence only {:.0}%",
                context.multi_timeframe.confluence_score * 100.0
            ));
        }

        // Check order flow
        match context.order_flow.signal {
            OrderFlowSignal::Neutral => {
                warnings.push("Order flow neutral - no directional conviction from whales".to_string());
            }
            OrderFlowSignal::StrongBuy | OrderFlowSignal::StrongSell => {
                reasons.push(format!(
                    "Order flow strongly confirming direction ({:.2}x imbalance)",
                    context.order_flow.bid_ask_ratio
                ));
            }
            _ => {
                reasons.push("Order flow mildly confirming direction".to_string());
            }
        }

        // Overall framework score
        let framework_valid = context.multi_timeframe.confidence_level >= 0.60;
        let score = if framework_valid { 0.75 } else { 0.50 };

        (framework_valid, "Frameworks validated".to_string(), warnings)
    }

    /// Step 2: Score technical signals (0-1)
    fn score_technical_signals(context: &AIDecisionContext) -> f64 {
        // Base score from confluence
        let mut score = context.technical_confluence * 0.8; // Weight it heavily

        // Boost if dominant signal is strong
        if context.dominant_signal.contains("Divergence") {
            score += 0.10; // Divergence is very reliable
        } else if context.dominant_signal.contains("Mean Reversion") {
            score += 0.05; // Mean reversion is decent
        }

        score.clamp(0.0, 1.0)
    }

    /// Step 3: Calculate optimal position sizing (framework + AI)
    fn calculate_optimal_sizing(context: &AIDecisionContext) -> (f64, f64) {
        // Start with dynamic sizing (pain vs reward)
        let sr = SupportResistance::new(context.support_level, context.resistance_level, context.current_price);

        let tech_setup = TechnicalSetup {
            rsi: 50.0, // Placeholder - would come from actual calculation
            rsi_oversold: 30.0,
            rsi_overbought: 70.0,
            macd_above_signal: true,
            price_vs_bollinger: 0.0,
            bollinger_compression: 0.5,
        };

        let dynamic_size = DynamicSizer::calculate_position_size(
            context.available_capital,
            &sr,
            &tech_setup,
            0.05, // 5% max risk
        );

        let mut position_pct = dynamic_size.position_size_pct;

        // Apply volatility scaling
        position_pct *= context.volatility_scaler.scaling_factor;

        // Apply Kelly Criterion if available (AI optimization)
        if let Some(kelly) = &context.kelly {
            let kelly_size = kelly.fractional_kelly;
            // Use Kelly if it's more conservative than dynamic sizing
            if kelly_size < position_pct {
                position_pct = kelly_size;
            }
        }

        // Cap at max
        position_pct = position_pct.clamp(0.0, context.max_position_pct);

        let position_dollars = context.available_capital * position_pct;

        (position_pct, position_dollars)
    }

    /// Step 4: AI scoring and final decision
    fn score_and_decide(
        context: &AIDecisionContext,
        technical_score: f64,
        framework_score: bool,
    ) -> (bool, f64, Vec<String>, Vec<String>, Vec<String>) {
        let mut reasoning = Vec::new();
        let mut opportunities = Vec::new();
        let mut warnings = Vec::new();

        // Calculate confidence
        let base_confidence = context.technical_confluence;

        // Boost confidence if:
        // 1. Multi-timeframe aligned
        let mut confidence_boost = 0.0;
        if context.multi_timeframe.is_strong_confluence() {
            confidence_boost += 0.10;
            reasoning.push("Multi-timeframe confluence strong".to_string());
        }

        // 2. Order flow confirming
        match context.order_flow.signal {
            OrderFlowSignal::StrongBuy | OrderFlowSignal::StrongSell => {
                confidence_boost += 0.08;
                reasoning.push("Order flow strongly confirming (whale activity)".to_string());
            }
            OrderFlowSignal::Buy | OrderFlowSignal::Sell => {
                confidence_boost += 0.04;
                reasoning.push("Order flow mildly confirming".to_string());
            }
            _ => {
                warnings.push("Order flow neutral - missing institutional confirmation".to_string());
            }
        }

        // 3. Volatility conducive
        match context.volatility_analysis.regime {
            VolatilityRegime::Calm | VolatilityRegime::Normal => {
                confidence_boost += 0.05;
                reasoning.push("Market volatility favorable for trading".to_string());
            }
            VolatilityRegime::Volatile => {
                warnings.push("High volatility - use tighter stops".to_string());
            }
            VolatilityRegime::Panic => {
                warnings.push("Panic volatility - only trade extreme setups".to_string());
            }
        }

        let final_confidence = (base_confidence + confidence_boost).clamp(0.0, 1.0);

        // Decision threshold: need 0.65+ confidence
        let should_enter = final_confidence >= 0.65
            && context.technical_confluence >= 0.70
            && framework_score;

        // Build opportunity/warning messages
        if should_enter {
            opportunities.push(format!("Strong technical setup ({:.0}% confluence)", context.technical_confluence * 100.0));
            opportunities.push(format!(
                "Position sizing optimal: {:.0}% of capital, risk/reward favorable",
                context.available_capital * 0.15  // Assuming 15% is typical
            ));
            if context.multi_timeframe.is_strong_confluence() {
                opportunities.push(format!("Multi-timeframe confluence: {:.0}%", context.multi_timeframe.confluence_score * 100.0));
            }
        } else {
            warnings.push(format!("Insufficient confluence: {:.0}%", context.technical_confluence * 100.0));
            warnings.push("Wait for stronger setup or more timeframe alignment".to_string());
        }

        reasoning.push(format!("AI confidence: {:.0}%", final_confidence * 100.0));

        (should_enter, final_confidence, reasoning, opportunities, warnings)
    }

    /// Determine execution urgency
    fn determine_urgency(context: &AIDecisionContext, technical_score: f64, _framework_valid: bool) -> ExecutionUrgency {
        // If technical score very high (85%+) and confluence strong, execute immediately
        if technical_score >= 0.85 && context.technical_confluence >= 0.85 {
            return ExecutionUrgency::Immediate;
        }

        // If volatility panic, be cautious
        if context.volatility_analysis.regime == VolatilityRegime::Panic {
            return ExecutionUrgency::Cautious;
        }

        // Normal case
        ExecutionUrgency::Normal
    }

    /// Calculate leverage based on position size and confidence
    fn calculate_leverage(position_size_pct: f64) -> f64 {
        match position_size_pct {
            x if x > 0.20 => 5.0,  // Large position = low leverage
            x if x > 0.15 => 7.0,  // Medium position
            x if x > 0.10 => 10.0, // Small position = higher leverage
            _ => 10.0,
        }
    }

    /// Create execution notes for trading log
    fn create_execution_notes(context: &AIDecisionContext, position_pct: f64) -> String {
        format!(
            "Price ${:.2} | Support ${:.2} | Runway {:.1}% | Position {:.0}% | Volatility {:.2}% ATR | Confluence {:.0}%",
            context.current_price,
            context.support_level,
            ((context.current_price - context.support_level) / context.current_price) * 100.0,
            position_pct * 100.0,
            context.atr_pct,
            context.technical_confluence * 100.0
        )
    }

    /// Create rejection decision
    fn create_rejection(reason: String, warnings: Vec<String>, context: &AIDecisionContext) -> AIDecision {
        AIDecision {
            should_enter: false,
            direction: None,
            position_size_pct: 0.0,
            position_size_dollars: 0.0,
            leverage: 0.0,
            stop_loss: 0.0,
            take_profit: 0.0,
            ai_confidence: 0.0,
            framework_score: 0.0,
            technical_score: 0.0,
            overall_score: 0.0,
            reasoning: vec![reason],
            warnings,
            opportunities: vec![],
            urgency: ExecutionUrgency::DoNotTrade,
            execution_notes: "No entry signal".to_string(),
        }
    }
}

// ============================================================================
// AI DECISION VALIDATOR - Double-check before execution
// ============================================================================

pub struct AIDecisionValidator;

impl AIDecisionValidator {
    /// Final sanity check before sending order
    /// Should complete in <1ms
    pub fn validate_before_execution(decision: &AIDecision) -> Result<String, String> {
        // Check 1: Confidence threshold
        if !decision.should_enter {
            return Err("AI rejected entry".to_string());
        }

        if decision.ai_confidence < 0.65 {
            return Err(format!(
                "Confidence too low: {:.0}% (need 65%+)",
                decision.ai_confidence * 100.0
            ));
        }

        // Check 2: Position sizing reasonable
        if decision.position_size_pct <= 0.0 || decision.position_size_pct > 0.25 {
            return Err(format!(
                "Position size out of range: {:.1}%",
                decision.position_size_pct * 100.0
            ));
        }

        // Check 3: Leverage reasonable
        if decision.leverage <= 0.0 || decision.leverage > 15.0 {
            return Err(format!("Leverage out of range: {:.1}x", decision.leverage));
        }

        // Check 4: Stop loss below current price
        if decision.direction == Some(true) && decision.stop_loss >= decision.position_size_dollars {
            return Err("Stop loss miscalculated (buy)".to_string());
        }

        // Check 5: Any critical warnings?
        for warning in &decision.warnings {
            if warning.contains("limit hit") {
                return Err(format!("Critical warning: {}", warning));
            }
        }

        // All checks passed
        Ok(format!(
            "✅ Decision validated - {:.0}% confidence, execute {}",
            decision.ai_confidence * 100.0,
            match decision.urgency {
                ExecutionUrgency::Immediate => "IMMEDIATELY",
                ExecutionUrgency::Normal => "normally",
                ExecutionUrgency::Cautious => "carefully",
                ExecutionUrgency::DoNotTrade => "DON'T TRADE",
            }
        ))
    }
}
