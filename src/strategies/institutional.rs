//! 🏛️ Institutional Trading Strategies for Perpetuals
//!
//! All 5 strategies optimized for small accounts (<$100K)
//! Capital-independent implementations
//! Works with 1-5x leverage on perpetual futures
//!
//! Strategies:
//! 1. Funding Rate Signal - Market sentiment from perpetual funding
//! 2. Pairs Trading - Statistical arbitrage using correlation
//! 3. Order Flow - Microstructure from order book imbalance
//! 4. Sentiment - Multi-source sentiment aggregation
//! 5. Volatility Surface - Implied vs realized vol mismatch

use crate::strategies::{StrategySignal, SignalType, StrategyContext};
use crate::scoring_system::{StrategyScore, StrategyScorer, ScoringAction};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for all institutional strategies
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstitutionalConfig {
    pub funding_rate: FundingRateConfig,
    pub pairs_trading: PairsTradingConfig,
    pub order_flow: OrderFlowConfig,
    pub sentiment: SentimentConfig,
    pub volatility_surface: VolatilityConfig,
}

impl Default for InstitutionalConfig {
    fn default() -> Self {
        InstitutionalConfig {
            funding_rate: FundingRateConfig::default(),
            pairs_trading: PairsTradingConfig::default(),
            order_flow: OrderFlowConfig::default(),
            sentiment: SentimentConfig::default(),
            volatility_surface: VolatilityConfig::default(),
        }
    }
}

// ============================================================================
// STRATEGY 1: FUNDING RATE SIGNALS
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingRateConfig {
    pub extreme_threshold: f64,      // 0.05% per hour
    pub warning_threshold: f64,      // 0.02% per hour
    pub lookback_periods: usize,     // 8 periods for moving average
    pub min_historical_data: usize,  // Need at least 24 periods
}

impl Default for FundingRateConfig {
    fn default() -> Self {
        FundingRateConfig {
            extreme_threshold: 0.0005,
            warning_threshold: 0.0002,
            lookback_periods: 8,
            min_historical_data: 24,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FundingRateData {
    pub current_rate: f64,
    pub historical_rates: Vec<f64>,
    pub timestamp: i64,
}

pub fn evaluate_funding_rate(
    data: &FundingRateData,
    config: &FundingRateConfig,
    account_size: f64,
) -> Result<(StrategySignal, StrategyScore), String> {
    if data.historical_rates.len() < config.min_historical_data {
        return Err("Insufficient historical funding rate data".to_string());
    }

    // Calculate statistics
    let avg = data.historical_rates.iter().sum::<f64>() / data.historical_rates.len() as f64;
    let variance = data
        .historical_rates
        .iter()
        .map(|r| (r - avg).powi(2))
        .sum::<f64>() / data.historical_rates.len() as f64;
    let std_dev = variance.sqrt();

    let z_score = (data.current_rate - avg) / std_dev.max(0.00001);

    // Trend: is funding moving up or down?
    let recent_avg = data.historical_rates[data.historical_rates.len() - config.lookback_periods..]
        .iter()
        .sum::<f64>() / config.lookback_periods as f64;
    let older_avg = data.historical_rates
        [data.historical_rates.len() - config.lookback_periods * 2..data.historical_rates.len() - config.lookback_periods]
        .iter()
        .sum::<f64>() / config.lookback_periods as f64;
    let trend = recent_avg - older_avg;

    // Scoring
    let scorer = StrategyScorer::new(account_size);
    let score = scorer.score_funding_rate(
        data.current_rate,
        avg,
        std_dev,
        trend,
    );

    // Signal generation
    let signal = if data.current_rate > config.extreme_threshold {
        StrategySignal {
            strategy_name: "Funding Rate - Extreme Long".to_string(),
            signal_type: SignalType::StrongSell,
            confidence: z_score.abs() / 3.0,
            position_size_multiplier: if score.action == ScoringAction::StrongTrade { 1.5 } else { 1.0 },
            rationale: format!(
                "Funding rate: {:.04}% (Z: {:.2}). Longs heavily funded. Short setup.",
                data.current_rate * 100.0, z_score
            ),
            target_price: None,
            stop_loss_pct: 4.0,
        }
    } else if data.current_rate < -config.extreme_threshold {
        StrategySignal {
            strategy_name: "Funding Rate - Extreme Short".to_string(),
            signal_type: SignalType::StrongBuy,
            confidence: z_score.abs() / 3.0,
            position_size_multiplier: if score.action == ScoringAction::StrongTrade { 1.5 } else { 1.0 },
            rationale: format!(
                "Funding rate: {:.04}% (Z: {:.2}). Shorts heavily paid. Long setup.",
                data.current_rate * 100.0, z_score
            ),
            target_price: None,
            stop_loss_pct: 4.0,
        }
    } else if data.current_rate > config.warning_threshold {
        StrategySignal {
            strategy_name: "Funding Rate - Elevated".to_string(),
            signal_type: SignalType::Sell,
            confidence: z_score.abs() / 4.0,
            position_size_multiplier: 1.0,
            rationale: format!(
                "Funding rate trending high: {:.04}%",
                data.current_rate * 100.0
            ),
            target_price: None,
            stop_loss_pct: 3.0,
        }
    } else {
        StrategySignal {
            strategy_name: "Funding Rate - Normal".to_string(),
            signal_type: SignalType::Neutral,
            confidence: 0.0,
            position_size_multiplier: 1.0,
            rationale: "Funding rate normal. No signal.".to_string(),
            target_price: None,
            stop_loss_pct: 2.0,
        }
    };

    Ok((signal, score))
}

// ============================================================================
// STRATEGY 2: PAIRS TRADING / STATISTICAL ARBITRAGE
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairsTradingConfig {
    pub correlation_threshold: f64,  // 0.70+ = good pair
    pub z_score_entry: f64,          // 2.0 = entry
    pub z_score_exit: f64,           // 0.5 = exit
    pub lookback: usize,             // 50 periods for correlation
}

impl Default for PairsTradingConfig {
    fn default() -> Self {
        PairsTradingConfig {
            correlation_threshold: 0.70,
            z_score_entry: 2.0,
            z_score_exit: 0.5,
            lookback: 50,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PairData {
    pub pair_name: String,
    pub asset_a_prices: Vec<f64>,
    pub asset_b_prices: Vec<f64>,
}

pub fn evaluate_pairs_trading(
    data: &PairData,
    config: &PairsTradingConfig,
    account_size: f64,
) -> Result<(StrategySignal, StrategyScore), String> {
    if data.asset_a_prices.len() < config.lookback || data.asset_b_prices.len() < config.lookback {
        return Err("Insufficient price data for pairs trading".to_string());
    }

    // Calculate correlation
    let a_slice = &data.asset_a_prices[data.asset_a_prices.len() - config.lookback..];
    let b_slice = &data.asset_b_prices[data.asset_b_prices.len() - config.lookback..];
    let correlation = calculate_correlation(a_slice, b_slice);

    if correlation < config.correlation_threshold {
        return Err(format!(
            "Correlation too low: {:.2} < {:.2}",
            correlation, config.correlation_threshold
        ));
    }

    // Calculate spread
    let current_spread = data.asset_a_prices[data.asset_a_prices.len() - 1]
        / data.asset_b_prices[data.asset_b_prices.len() - 1];

    // Mean and std dev of spread
    let spreads: Vec<f64> = data.asset_a_prices
        .windows(2)
        .zip(data.asset_b_prices.windows(2))
        .map(|(a, b)| a[1] / b[1])
        .collect();

    let mean = spreads.iter().sum::<f64>() / spreads.len() as f64;
    let variance = spreads
        .iter()
        .map(|s| (s - mean).powi(2))
        .sum::<f64>() / spreads.len() as f64;
    let std_dev = variance.sqrt();
    let z_score = (current_spread - mean) / std_dev.max(0.0001);

    // Scoring
    let scorer = StrategyScorer::new(account_size);
    let score = scorer.score_pairs_trading(z_score, correlation, 0.70, 2.2);

    // Signal generation
    let signal = if z_score > config.z_score_entry {
        StrategySignal {
            strategy_name: format!("Pairs Trading: {} SHORT", data.pair_name),
            signal_type: SignalType::Sell,
            confidence: (z_score / 3.0).min(0.95),
            position_size_multiplier: if z_score > config.z_score_entry * 1.5 { 1.3 } else { 1.0 },
            rationale: format!(
                "Z-score {:.2} (entry {}). {} overpriced. Target: mean {:.4}",
                z_score, config.z_score_entry, data.pair_name, mean
            ),
            target_price: Some(mean),
            stop_loss_pct: 3.0,
        }
    } else if z_score < -config.z_score_entry {
        StrategySignal {
            strategy_name: format!("Pairs Trading: {} LONG", data.pair_name),
            signal_type: SignalType::Buy,
            confidence: (-z_score / 3.0).min(0.95),
            position_size_multiplier: if z_score < -config.z_score_entry * 1.5 { 1.3 } else { 1.0 },
            rationale: format!(
                "Z-score {:.2} (entry {}). {} underpriced. Target: mean {:.4}",
                z_score, config.z_score_entry, data.pair_name, mean
            ),
            target_price: Some(mean),
            stop_loss_pct: 3.0,
        }
    } else {
        StrategySignal {
            strategy_name: format!("Pairs Trading: {}", data.pair_name),
            signal_type: SignalType::Neutral,
            confidence: 0.0,
            position_size_multiplier: 1.0,
            rationale: format!("Z-score {:.2}. Within normal range.", z_score),
            target_price: None,
            stop_loss_pct: 2.5,
        }
    };

    Ok((signal, score))
}

// ============================================================================
// STRATEGY 3: ORDER FLOW / MICROSTRUCTURE
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderFlowConfig {
    pub imbalance_threshold: f64,    // 1.3x = extreme
    pub min_book_depth: f64,         // Minimum order book size
    pub lookback_periods: usize,     // For calculating z-score
}

impl Default for OrderFlowConfig {
    fn default() -> Self {
        OrderFlowConfig {
            imbalance_threshold: 1.3,
            min_book_depth: 10000.0,
            lookback_periods: 100,
        }
    }
}

#[derive(Debug, Clone)]
pub struct OrderBookData {
    pub bid_volume: f64,
    pub ask_volume: f64,
    pub historical_imbalances: Vec<f64>,
    pub timestamp: i64,
}

pub fn evaluate_order_flow(
    data: &OrderBookData,
    config: &OrderFlowConfig,
    account_size: f64,
) -> Result<(StrategySignal, StrategyScore), String> {
    if data.bid_volume + data.ask_volume < config.min_book_depth {
        return Err("Order book too shallow".to_string());
    }

    let current_imbalance = data.bid_volume - data.ask_volume;
    let imbalance_ratio = data.bid_volume / data.ask_volume.max(0.001);

    // Z-score of imbalance
    let mean = data.historical_imbalances.iter().sum::<f64>()
        / data.historical_imbalances.len() as f64;
    let variance = data
        .historical_imbalances
        .iter()
        .map(|x| (x - mean).powi(2))
        .sum::<f64>() / data.historical_imbalances.len() as f64;
    let std_dev = variance.sqrt();
    let z_score = (current_imbalance - mean) / std_dev.max(0.001);

    // Scoring
    let scorer = StrategyScorer::new(account_size);
    let score = scorer.score_order_flow(z_score, imbalance_ratio, 0.68, 2.1);

    // Signal generation
    let signal = if z_score > 2.0 && imbalance_ratio > config.imbalance_threshold {
        StrategySignal {
            strategy_name: "Order Flow - Extreme Buyers".to_string(),
            signal_type: SignalType::StrongBuy,
            confidence: (z_score / 3.0).min(0.95),
            position_size_multiplier: 1.4,
            rationale: format!(
                "Order book heavily imbalanced. Bid/Ask ratio: {:.2}x (Z: {:.2}). Strong buying pressure.",
                imbalance_ratio, z_score
            ),
            target_price: None,
            stop_loss_pct: 3.0,
        }
    } else if z_score < -2.0 {
        StrategySignal {
            strategy_name: "Order Flow - Extreme Sellers".to_string(),
            signal_type: SignalType::StrongSell,
            confidence: (-z_score / 3.0).min(0.95),
            position_size_multiplier: 1.4,
            rationale: format!(
                "Order book heavily imbalanced. Bid/Ask ratio: {:.2}x (Z: {:.2}). Strong selling pressure.",
                imbalance_ratio, z_score
            ),
            target_price: None,
            stop_loss_pct: 3.0,
        }
    } else if z_score > 1.0 {
        StrategySignal {
            strategy_name: "Order Flow - Buyers Accumulating".to_string(),
            signal_type: SignalType::Buy,
            confidence: z_score / 3.0,
            position_size_multiplier: 1.0,
            rationale: format!(
                "Slight buyer advantage. Ratio: {:.2}x",
                imbalance_ratio
            ),
            target_price: None,
            stop_loss_pct: 2.5,
        }
    } else {
        StrategySignal {
            strategy_name: "Order Flow - Balanced".to_string(),
            signal_type: SignalType::Neutral,
            confidence: 0.0,
            position_size_multiplier: 1.0,
            rationale: "Order book balanced".to_string(),
            target_price: None,
            stop_loss_pct: 2.0,
        }
    };

    Ok((signal, score))
}

// ============================================================================
// STRATEGY 4: SENTIMENT ANALYSIS
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentimentConfig {
    pub fg_extreme_low: u32,         // 0-20 = extreme fear
    pub fg_extreme_high: u32,        // 80-100 = extreme greed
    pub on_chain_threshold: f64,     // 0.5+ = significant signal
    pub min_sources_aligned: usize,  // Need 2+ sources aligned
}

impl Default for SentimentConfig {
    fn default() -> Self {
        SentimentConfig {
            fg_extreme_low: 20,
            fg_extreme_high: 80,
            on_chain_threshold: 0.5,
            min_sources_aligned: 2,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SentimentData {
    pub fear_greed_index: u32,       // 0-100
    pub on_chain_signal: f64,        // -1 to +1 (whale activity, exchange flows)
    pub social_signal: f64,          // -1 to +1 (twitter sentiment)
    pub whale_activity: f64,         // -1 to +1 (large transfers)
    pub liquidation_pressure: f64,   // -1 to +1 (cascading liquidations)
}

pub fn evaluate_sentiment(
    data: &SentimentData,
    config: &SentimentConfig,
    account_size: f64,
) -> Result<(StrategySignal, StrategyScore), String> {
    // Count aligned signals
    let bullish_signals = [
        data.fear_greed_index < config.fg_extreme_low,  // Extreme fear
        data.on_chain_signal > config.on_chain_threshold,
        data.whale_activity > config.on_chain_threshold,
        data.liquidation_pressure < -config.on_chain_threshold,  // Shorts liquidating
    ].iter().filter(|&&x| x).count();

    let bearish_signals = [
        data.fear_greed_index > config.fg_extreme_high,  // Extreme greed
        data.on_chain_signal < -config.on_chain_threshold,
        data.social_signal < -config.on_chain_threshold,
        data.liquidation_pressure > config.on_chain_threshold,  // Longs liquidating
    ].iter().filter(|&&x| x).count();

    // Scoring
    let scorer = StrategyScorer::new(account_size);
    let score = scorer.score_sentiment(
        data.fear_greed_index,
        data.on_chain_signal,
        data.social_signal,
        data.whale_activity,
        0.65,  // Historical win rate
        2.0,   // Historical profit factor
    );

    // Signal generation
    let signal = if bullish_signals >= config.min_sources_aligned {
        StrategySignal {
            strategy_name: "Sentiment - Bullish Confluence".to_string(),
            signal_type: SignalType::Buy,
            confidence: (bullish_signals as f64 / 4.0).min(0.95),
            position_size_multiplier: if bullish_signals > 2 { 1.2 } else { 1.0 },
            rationale: format!(
                "Multi-source bullish: {} signals aligned. Fear/Greed={}, On-Chain={:.2}",
                bullish_signals, data.fear_greed_index, data.on_chain_signal
            ),
            target_price: None,
            stop_loss_pct: 3.0,
        }
    } else if bearish_signals >= config.min_sources_aligned {
        StrategySignal {
            strategy_name: "Sentiment - Bearish Confluence".to_string(),
            signal_type: SignalType::Sell,
            confidence: (bearish_signals as f64 / 4.0).min(0.95),
            position_size_multiplier: if bearish_signals > 2 { 1.2 } else { 1.0 },
            rationale: format!(
                "Multi-source bearish: {} signals aligned. Fear/Greed={}, On-Chain={:.2}",
                bearish_signals, data.fear_greed_index, data.on_chain_signal
            ),
            target_price: None,
            stop_loss_pct: 3.0,
        }
    } else {
        StrategySignal {
            strategy_name: "Sentiment - Neutral".to_string(),
            signal_type: SignalType::Neutral,
            confidence: 0.0,
            position_size_multiplier: 1.0,
            rationale: "Mixed sentiment signals. Insufficient confluence.".to_string(),
            target_price: None,
            stop_loss_pct: 2.0,
        }
    };

    Ok((signal, score))
}

// ============================================================================
// STRATEGY 5: VOLATILITY SURFACE
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolatilityConfig {
    pub iv_extreme_high: f64,        // 80th percentile
    pub iv_extreme_low: f64,         // 20th percentile
    pub iv_rv_divergence: f64,       // IV / RV ratio > 1.5
    pub lookback: usize,             // Periods for IV/RV history
}

impl Default for VolatilityConfig {
    fn default() -> Self {
        VolatilityConfig {
            iv_extreme_high: 80.0,
            iv_extreme_low: 20.0,
            iv_rv_divergence: 1.5,
            lookback: 60,
        }
    }
}

#[derive(Debug, Clone)]
pub struct VolatilityData {
    pub implied_vol: f64,            // From option prices or funding
    pub realized_vol_7d: f64,
    pub realized_vol_30d: f64,
    pub iv_percentile: f64,          // 0-100
    pub vol_regime: String,          // "low", "normal", "high", "extreme"
}

pub fn evaluate_volatility_surface(
    data: &VolatilityData,
    config: &VolatilityConfig,
    account_size: f64,
) -> Result<(StrategySignal, StrategyScore), String> {
    let iv_rv_ratio = data.implied_vol / data.realized_vol_7d.max(0.001);

    // Scoring
    let scorer = StrategyScorer::new(account_size);
    let score = scorer.score_volatility_surface(
        data.iv_percentile,
        iv_rv_ratio,
        &data.vol_regime,
        0.62,  // Historical win rate
        2.0,   // Historical profit factor
    );

    // Signal generation
    let signal = if data.iv_percentile > config.iv_extreme_high && iv_rv_ratio < 1.0 {
        // IV is extreme high but realized vol is even higher = trend acceleration
        StrategySignal {
            strategy_name: "Vol Surface - IV Underestimate".to_string(),
            signal_type: SignalType::StrongBuy,
            confidence: (data.iv_percentile / 100.0).min(0.95),
            position_size_multiplier: 1.3,
            rationale: format!(
                "IV extreme ({}th percentile) but RV higher (ratio {:.2}). Trend accelerating.",
                data.iv_percentile, iv_rv_ratio
            ),
            target_price: None,
            stop_loss_pct: 4.0,
        }
    } else if data.iv_percentile < config.iv_extreme_low && iv_rv_ratio > 1.2 {
        // IV extreme low but above realized = mean reversion expected
        StrategySignal {
            strategy_name: "Vol Surface - IV Overestimate".to_string(),
            signal_type: SignalType::Buy,
            confidence: ((100.0 - data.iv_percentile) / 100.0).min(0.90),
            position_size_multiplier: 1.2,
            rationale: format!(
                "IV low ({}th percentile) but above RV. Mean reversion setup.",
                data.iv_percentile
            ),
            target_price: None,
            stop_loss_pct: 3.0,
        }
    } else if data.vol_regime == "extreme" {
        StrategySignal {
            strategy_name: "Vol Surface - Extreme Regime".to_string(),
            signal_type: SignalType::Neutral,  // Extreme vol = uncertain direction
            confidence: 0.5,
            position_size_multiplier: 0.5,  // Reduce size in extreme vol
            rationale: "Extreme volatility regime. Reduce position sizing.".to_string(),
            target_price: None,
            stop_loss_pct: 5.0,
        }
    } else {
        StrategySignal {
            strategy_name: "Vol Surface - Normal".to_string(),
            signal_type: SignalType::Neutral,
            confidence: 0.0,
            position_size_multiplier: 1.0,
            rationale: format!(
                "Volatility normal. Regime: {}. IV/RV ratio: {:.2}",
                data.vol_regime, iv_rv_ratio
            ),
            target_price: None,
            stop_loss_pct: 2.5,
        }
    };

    Ok((signal, score))
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

fn calculate_correlation(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let mean_a = a.iter().sum::<f64>() / a.len() as f64;
    let mean_b = b.iter().sum::<f64>() / b.len() as f64;

    let mut numerator = 0.0;
    let mut sum_sq_a = 0.0;
    let mut sum_sq_b = 0.0;

    for (x, y) in a.iter().zip(b.iter()) {
        let diff_a = x - mean_a;
        let diff_b = y - mean_b;
        numerator += diff_a * diff_b;
        sum_sq_a += diff_a * diff_a;
        sum_sq_b += diff_b * diff_b;
    }

    let denominator = (sum_sq_a * sum_sq_b).sqrt();
    if denominator == 0.0 {
        0.0
    } else {
        numerator / denominator
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_funding_rate_extreme() {
        let data = FundingRateData {
            current_rate: 0.001,
            historical_rates: vec![0.0001; 30],
            timestamp: 0,
        };
        let config = FundingRateConfig::default();
        let (signal, score) = evaluate_funding_rate(&data, &config, 10000.0).unwrap();

        assert_eq!(signal.signal_type, SignalType::StrongSell);
        assert!(score.composite_score > 70.0);
    }

    #[test]
    fn test_pairs_trading_entry() {
        let data = PairData {
            pair_name: "SOL/BONK".to_string(),
            asset_a_prices: (0..100).map(|i| 100.0 + (i as f64 * 0.5)).collect(),
            asset_b_prices: (0..100).map(|i| 50.0 + (i as f64 * 0.2)).collect(),
        };
        let config = PairsTradingConfig::default();
        let (signal, _) = evaluate_pairs_trading(&data, &config, 10000.0).unwrap();

        assert!(signal.signal_type != SignalType::Neutral);
    }
}
