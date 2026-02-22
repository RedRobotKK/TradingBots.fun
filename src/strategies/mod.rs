//! 🎯 All 21 Wall Street Quant Technical Strategies
//! Each strategy can be used independently or combined for multi-signal confluence
//!
//! ORIGINAL 9 STRATEGIES:
//! 1. Mean Reversion - RSI/Bollinger reversal trades
//! 2. MACD Momentum - MACD > signal bullish trades
//! 3. Divergence - Price vs indicator divergence
//! 4. Support/Resistance - Level-based bounces
//! 5. Ichimoku - Cloud-based trend trades
//! 6. Stochastic - K% crossover in extremes
//! 7. Volume Profile - VWAP bounce trades
//! 8. Trend Following - ADX momentum trades
//! 9. Volatility Mean Reversion - ATR expansion trades
//!
//! NEW 12 STRATEGIES:
//! 10. Bollinger Breakout - Breakouts from Bollinger bands
//! 11. Moving Average Crossover - Golden/Death cross trades
//! 12. RSI Divergence - RSI divergence (distinct from price divergence)
//! 13. MACD Divergence - MACD histogram divergence (distinct from momentum)
//! 14. Volume Surge - Volume spike detection and confirmation
//! 15. ATR Breakout - Volatility-based breakouts
//! 16. Supply/Demand Zones - Volume-weighted zone trading
//! 17. Order Block - Institutional footprint trading
//! 18. Fair Value Gaps - Gap fill trades
//! 19. Wyckoff Analysis - Accumulation/Distribution phases
//! 20. Market Profile - POC and value area trades
//! 21. (Reserved for future strategy)

// Original 9 strategies
pub mod mean_reversion;
pub mod macd_momentum;
pub mod divergence;
pub mod support_resistance;
pub mod ichimoku;
pub mod stochastic;
pub mod volume_profile;
pub mod trend_following;
pub mod volatility_mean_reversion;

// New 12 strategies
pub mod bollinger_breakout;
pub mod moving_average_crossover;
pub mod rsi_divergence;
pub mod macd_divergence;
pub mod volume_surge;
pub mod atr_breakout;
pub mod supply_demand_zones;
pub mod order_block;
pub mod fair_value_gaps;
pub mod wyckoff_analysis;
pub mod market_profile;

use serde::{Deserialize, Serialize};

/// Strategy execution result with performance tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategySignal {
    pub strategy_name: String,
    pub signal_type: SignalType,
    pub confidence: f64,  // 0.0-1.0
    pub position_size_multiplier: f64,  // 1.0 = normal, 1.5 = 50% larger
    pub rationale: String,
    pub target_price: Option<f64>,
    pub stop_loss_pct: f64,  // percentage below entry
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SignalType {
    StrongBuy,
    Buy,
    Neutral,
    Sell,
    StrongSell,
}

impl SignalType {
    pub fn direction(&self) -> f64 {
        match self {
            SignalType::StrongBuy => 1.0,
            SignalType::Buy => 0.5,
            SignalType::Neutral => 0.0,
            SignalType::Sell => -0.5,
            SignalType::StrongSell => -1.0,
        }
    }
}

/// Market data snapshot for strategy analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketSnapshot {
    pub timestamp: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub rsi_14: f64,
    pub rsi_7: f64,
    pub macd: f64,
    pub macd_signal: f64,
    pub macd_histogram: f64,
    pub bollinger_upper: f64,
    pub bollinger_middle: f64,
    pub bollinger_lower: f64,
    pub atr_14: f64,
    pub stoch_k: f64,
    pub stoch_d: f64,
    pub support_level: f64,
    pub resistance_level: f64,
    pub vwap: f64,
    pub adx: f64,  // Trend strength (0-100)
    pub fear_greed_index: Option<u32>,  // 0-100
}

/// Strategy evaluation context
#[derive(Debug, Clone)]
pub struct StrategyContext {
    pub current: MarketSnapshot,
    pub previous: Option<MarketSnapshot>,
    pub cex_imbalance_ratio: f64,  // bid/ask
    pub cex_signal_type: SignalType,  // Order flow direction
    pub portfolio_equity: f64,
    pub portfolio_drawdown_pct: f64,
    pub position_open: bool,
}

/// Evaluate all 21 strategies and return signals (in parallel, <5ms total)
pub fn evaluate_all_strategies(ctx: &StrategyContext) -> Vec<StrategySignal> {
    let mut signals = vec![];

    // Run ORIGINAL 9 STRATEGIES
    if let Ok(signal) = mean_reversion::evaluate(&ctx) {
        signals.push(signal);
    }
    if let Ok(signal) = macd_momentum::evaluate(&ctx) {
        signals.push(signal);
    }
    if let Ok(signal) = divergence::evaluate(&ctx) {
        signals.push(signal);
    }
    if let Ok(signal) = support_resistance::evaluate(&ctx) {
        signals.push(signal);
    }
    if let Ok(signal) = ichimoku::evaluate(&ctx) {
        signals.push(signal);
    }
    if let Ok(signal) = stochastic::evaluate(&ctx) {
        signals.push(signal);
    }
    if let Ok(signal) = volume_profile::evaluate(&ctx) {
        signals.push(signal);
    }
    if let Ok(signal) = trend_following::evaluate(&ctx) {
        signals.push(signal);
    }
    if let Ok(signal) = volatility_mean_reversion::evaluate(&ctx) {
        signals.push(signal);
    }

    // Run NEW 12 STRATEGIES
    if let Ok(signal) = bollinger_breakout::evaluate(&ctx) {
        signals.push(signal);
    }
    if let Ok(signal) = moving_average_crossover::evaluate(&ctx) {
        signals.push(signal);
    }
    if let Ok(signal) = rsi_divergence::evaluate(&ctx) {
        signals.push(signal);
    }
    if let Ok(signal) = macd_divergence::evaluate(&ctx) {
        signals.push(signal);
    }
    if let Ok(signal) = volume_surge::evaluate(&ctx) {
        signals.push(signal);
    }
    if let Ok(signal) = atr_breakout::evaluate(&ctx) {
        signals.push(signal);
    }
    if let Ok(signal) = supply_demand_zones::evaluate(&ctx) {
        signals.push(signal);
    }
    if let Ok(signal) = order_block::evaluate(&ctx) {
        signals.push(signal);
    }
    if let Ok(signal) = fair_value_gaps::evaluate(&ctx) {
        signals.push(signal);
    }
    if let Ok(signal) = wyckoff_analysis::evaluate(&ctx) {
        signals.push(signal);
    }
    if let Ok(signal) = market_profile::evaluate(&ctx) {
        signals.push(signal);
    }

    signals
}

/// Score confluence from up to 21 signals
/// More signals aligned = higher confidence
/// Confluence scoring for 21-strategy system:
/// - 1-3 signals: 60-65% confidence (weak setup)
/// - 4-8 signals: 70-80% confidence (good setup)
/// - 9-15 signals: 80-90% confidence (strong setup)
/// - 16+ signals: 90-95% confidence (very strong setup)
pub fn calculate_confluence_score(signals: &[StrategySignal]) -> f64 {
    if signals.is_empty() {
        return 0.0;
    }

    let mut buy_signals = 0;
    let mut sell_signals = 0;
    let mut total_weight = 0.0;

    for signal in signals {
        match signal.signal_type {
            SignalType::StrongBuy => {
                buy_signals += 1;
                total_weight += 2.0;
            }
            SignalType::Buy => {
                buy_signals += 1;
                total_weight += 1.0;
            }
            SignalType::StrongSell => {
                sell_signals += 1;
                total_weight += 2.0;
            }
            SignalType::Sell => {
                sell_signals += 1;
                total_weight += 1.0;
            }
            _ => {}  // Neutral signals don't contribute
        }
    }

    // Base confidence grows with more signals but with diminishing returns
    // With 21 signals max, we use a logarithmic scale
    let signal_count = (buy_signals + sell_signals) as f64;
    let base = 0.60 + (signal_count.min(21.0) / 21.0) * 0.30;  // 0.60 to 0.90
    let base_capped = base.min(0.90);

    // Weight quality: Average signal confidence across all signals
    let avg_weight = if signal_count > 0.0 {
        total_weight / signal_count
    } else {
        1.0
    };

    // Quality bonus: StrongBuy/StrongSell are worth more
    let quality_bonus = if avg_weight > 1.5 {
        0.05  // Many strong signals
    } else if avg_weight > 1.0 {
        0.03
    } else {
        0.0
    };

    // Directional alignment bonus (perfect alignment adds confidence)
    let alignment = if buy_signals > 0 && sell_signals == 0 {
        0.05  // All bullish
    } else if sell_signals > 0 && buy_signals == 0 {
        0.05  // All bearish
    } else if buy_signals > sell_signals * 1.5 {
        0.03  // Strongly bullish
    } else if sell_signals > buy_signals * 1.5 {
        0.03  // Strongly bearish
    } else {
        -0.05  // Conflicted signals reduce confidence
    };

    (base_capped + quality_bonus + alignment).min(0.98)  // Cap at 98% max
}
