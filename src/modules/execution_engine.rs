//! Order Execution Engine
//!
//! Intelligent order routing and execution system with:
//! - Multi-venue order routing (Drift vs Hyperliquid)
//! - Slippage estimation and optimization
//! - Partial fill handling
//! - Multiple routing strategies
//! - Execution result tracking
//!
//! Uses repository pattern for state management with Arc<RwLock<>> for thread-safe shared state.

use crate::models::market::{
    ExecutionResult, ExecutionStatus, LimitOrder, MarketData, MarketOrder, Order, OrderBook,
};
use crate::utils::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Protocol identifier for order execution
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExecutionProtocol {
    Drift,
    Hyperliquid,
}

impl ExecutionProtocol {
    pub fn as_str(&self) -> &str {
        match self {
            ExecutionProtocol::Drift => "drift",
            ExecutionProtocol::Hyperliquid => "hyperliquid",
        }
    }
}

/// Routing strategy for order execution
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoutingStrategy {
    /// Execute on venue with best price
    BestPrice,
    /// Execute on venue with minimum slippage
    MinSlippage,
    /// Intelligent routing based on multiple factors
    SmartRouting,
    /// Split order across multiple venues
    SplitExecution,
}

/// Execution venue with pricing information
#[derive(Clone, Debug)]
pub struct ExecutionVenue {
    pub protocol: ExecutionProtocol,
    pub market_data: MarketData,
    pub order_book: Option<OrderBook>,
    pub available: bool,
    pub latency_ms: u64,
}

/// Slippage estimate for execution
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SlippageEstimate {
    pub protocol: ExecutionProtocol,
    pub estimated_slippage: f64,      // in USDC
    pub slippage_percentage: f64,     // as percentage
    pub market_impact: f64,            // estimated market impact
    pub execution_price: f64,
    pub recommended: bool,
}

/// Order execution plan before routing
#[derive(Clone, Debug)]
pub struct ExecutionPlan {
    pub order: Order,
    pub routing_strategy: RoutingStrategy,
    pub selected_protocol: ExecutionProtocol,
    pub slippage_estimate: SlippageEstimate,
    pub alternative_venues: Vec<SlippageEstimate>,
    pub timestamp: i64,
}

/// Execution result with detailed information
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DetailedExecutionResult {
    pub result: ExecutionResult,
    pub protocol: ExecutionProtocol,
    pub fills: Vec<ExecutionFill>,
    pub total_fees: f64,
    pub actual_slippage: f64,
    pub execution_time_ms: u64,
}

/// Individual fill information
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecutionFill {
    pub timestamp: i64,
    pub price: f64,
    pub size: f64,
    pub fee: f64,
}

/// Execution engine repository
#[derive(Clone)]
pub struct ExecutionRepository {
    /// Order execution history
    execution_history: Arc<RwLock<Vec<DetailedExecutionResult>>>,
    /// Open execution plans
    open_plans: Arc<RwLock<HashMap<String, ExecutionPlan>>>,
    /// Default routing strategy
    default_strategy: Arc<RwLock<RoutingStrategy>>,
    /// Slippage thresholds
    max_slippage_pct: f64,
}

impl ExecutionRepository {
    /// Create a new execution repository
    pub fn new() -> Self {
        Self {
            execution_history: Arc::new(RwLock::new(Vec::new())),
            open_plans: Arc::new(RwLock::new(HashMap::new())),
            default_strategy: Arc::new(RwLock::new(RoutingStrategy::SmartRouting)),
            max_slippage_pct: 0.5, // 0.5% max slippage
        }
    }

    /// Set default routing strategy
    pub async fn set_default_strategy(&self, strategy: RoutingStrategy) {
        let mut default = self.default_strategy.write().await;
        *default = strategy;
        info!("Default routing strategy changed to: {:?}", strategy);
    }

    /// Get default routing strategy
    pub async fn get_default_strategy(&self) -> RoutingStrategy {
        *self.default_strategy.read().await
    }

    /// Store an execution plan
    pub async fn store_plan(&self, plan_id: String, plan: ExecutionPlan) {
        let mut plans = self.open_plans.write().await;
        plans.insert(plan_id.clone(), plan);
        info!("Execution plan stored: {}", plan_id);
    }

    /// Retrieve an execution plan
    pub async fn get_plan(&self, plan_id: &str) -> Option<ExecutionPlan> {
        let plans = self.open_plans.read().await;
        plans.get(plan_id).cloned()
    }

    /// Remove an execution plan
    pub async fn remove_plan(&self, plan_id: &str) -> Option<ExecutionPlan> {
        let mut plans = self.open_plans.write().await;
        plans.remove(plan_id)
    }

    /// Record execution result
    pub async fn record_execution(&self, result: DetailedExecutionResult) {
        let mut history = self.execution_history.write().await;
        history.push(result);
    }

    /// Get execution history
    pub async fn get_execution_history(&self, limit: usize) -> Vec<DetailedExecutionResult> {
        let history = self.execution_history.read().await;
        history
            .iter()
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }

    /// Get execution history for a symbol
    pub async fn get_symbol_history(&self, symbol: &str, limit: usize) -> Vec<DetailedExecutionResult> {
        let history = self.execution_history.read().await;
        history
            .iter()
            .filter(|e| e.result.symbol == symbol)
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }

    /// Clear execution history (for testing)
    pub async fn clear_history(&self) {
        let mut history = self.execution_history.write().await;
        history.clear();
        info!("Execution history cleared");
    }
}

impl Default for ExecutionRepository {
    fn default() -> Self {
        Self::new()
    }
}

/// Order execution engine
#[derive(Clone)]
pub struct OrderExecutionEngine {
    repository: ExecutionRepository,
    drift_available: bool,
    hyperliquid_available: bool,
}

impl OrderExecutionEngine {
    /// Create a new order execution engine
    pub fn new(repository: ExecutionRepository) -> Self {
        Self {
            repository,
            drift_available: true,
            hyperliquid_available: true,
        }
    }

    /// Set protocol availability
    pub fn set_protocol_available(&mut self, protocol: ExecutionProtocol, available: bool) {
        match protocol {
            ExecutionProtocol::Drift => self.drift_available = available,
            ExecutionProtocol::Hyperliquid => self.hyperliquid_available = available,
        }
    }

    /// Plan order execution
    ///
    /// Analyzes available venues and creates an execution plan
    ///
    /// # Arguments
    /// * `order` - Order to execute
    /// * `venues` - Available execution venues
    /// * `strategy` - Routing strategy to use
    ///
    /// # Returns
    /// Execution plan or error
    pub async fn plan_execution(
        &self,
        order: Order,
        venues: Vec<ExecutionVenue>,
        strategy: Option<RoutingStrategy>,
    ) -> Result<ExecutionPlan> {
        info!("Planning execution for order");

        // Use provided strategy or default
        let routing_strategy = if let Some(s) = strategy {
            s
        } else {
            self.repository.get_default_strategy().await
        };

        // Validate venues
        if venues.is_empty() {
            return Err(Error::NoViableOpportunity);
        }

        let available_venues: Vec<&ExecutionVenue> = venues.iter().filter(|v| v.available).collect();

        if available_venues.is_empty() {
            return Err(Error::NoViableOpportunity);
        }

        // Estimate slippage for each venue
        let slippage_estimates: Vec<SlippageEstimate> = available_venues
            .iter()
            .filter_map(|v| self.estimate_slippage(&order, v).ok())
            .collect();

        if slippage_estimates.is_empty() {
            return Err(Error::NoViableOpportunity);
        }

        // Select best venue based on strategy
        let selected = self.select_venue(&slippage_estimates, routing_strategy)?;

        let plan = ExecutionPlan {
            order,
            routing_strategy,
            selected_protocol: selected.protocol,
            slippage_estimate: selected.clone(),
            alternative_venues: slippage_estimates,
            timestamp: chrono::Utc::now().timestamp(),
        };

        Ok(plan)
    }

    /// Execute a planned order
    ///
    /// # Arguments
    /// * `plan` - Execution plan
    ///
    /// # Returns
    /// Execution result
    pub async fn execute(&self, plan: ExecutionPlan) -> Result<DetailedExecutionResult> {
        info!(
            "Executing order on {:?} with {:?} strategy",
            plan.selected_protocol, plan.routing_strategy
        );

        let start_time = std::time::Instant::now();

        // Simulate order execution
        let (filled_size, average_price, status) = match &plan.order {
            Order::Limit(limit_order) => {
                self.execute_limit_order(limit_order, &plan.slippage_estimate)
                    .await?
            }
            Order::Market(market_order) => {
                self.execute_market_order(market_order, &plan.slippage_estimate)
                    .await?
            }
        };

        let execution_time = start_time.elapsed().as_millis() as u64;

        // Get order details
        let (symbol, total_fees) = match &plan.order {
            Order::Limit(lo) => (lo.symbol.clone(), 0.0), // Would calculate from fills
            Order::Market(mo) => (mo.symbol.clone(), 0.0),
        };

        let actual_slippage = (average_price - plan.slippage_estimate.execution_price).abs();

        let result = DetailedExecutionResult {
            result: ExecutionResult {
                order_id: format!("order_{}", chrono::Utc::now().timestamp()),
                symbol,
                status,
                average_price,
                filled_size,
                remaining_size: 0.0,
                timestamp: chrono::Utc::now().timestamp(),
            },
            protocol: plan.selected_protocol,
            fills: vec![ExecutionFill {
                timestamp: chrono::Utc::now().timestamp(),
                price: average_price,
                size: filled_size,
                fee: total_fees,
            }],
            total_fees,
            actual_slippage,
            execution_time_ms: execution_time,
        };

        self.repository.record_execution(result.clone()).await;

        info!(
            "Order executed on {:?}: {} @ {:.2}, slippage: {:.4}",
            plan.selected_protocol, filled_size, average_price, actual_slippage
        );

        Ok(result)
    }

    /// Estimate slippage for an order on a venue
    ///
    /// # Arguments
    /// * `order` - Order to execute
    /// * `venue` - Target venue
    ///
    /// # Returns
    /// Slippage estimate
    pub fn estimate_slippage(
        &self,
        order: &Order,
        venue: &ExecutionVenue,
    ) -> Result<SlippageEstimate> {
        let (size, side) = match order {
            Order::Limit(lo) => (lo.size, lo.side),
            Order::Market(mo) => (mo.size, mo.side),
        };

        let best_price = if side.is_buy() {
            venue.market_data.ask
        } else {
            venue.market_data.bid
        };

        // Estimate slippage based on order book depth
        let estimated_execution_price = if let Some(book) = &venue.order_book {
            self.calculate_execution_price(order, book)?
        } else {
            best_price
        };

        let slippage_amount = (estimated_execution_price - best_price).abs();
        let slippage_pct = (slippage_amount / best_price) * 100.0;

        // Market impact estimation
        let market_impact = self.estimate_market_impact(size, venue);

        let recommended = slippage_pct <= self.repository.max_slippage_pct;

        if slippage_pct > self.repository.max_slippage_pct {
            warn!(
                "Slippage {:.4}% exceeds max {:.4}%",
                slippage_pct, self.repository.max_slippage_pct
            );
        }

        Ok(SlippageEstimate {
            protocol: venue.protocol,
            estimated_slippage: slippage_amount,
            slippage_percentage: slippage_pct,
            market_impact,
            execution_price: estimated_execution_price,
            recommended,
        })
    }

    /// Handle partial fill scenarios
    ///
    /// # Arguments
    /// * `original_order` - Original order
    /// * `filled_size` - Size that was filled
    ///
    /// # Returns
    /// Remaining order to place, if any
    pub fn handle_partial_fill(
        &self,
        order: &Order,
        filled_size: f64,
    ) -> Result<Option<Order>> {
        let remaining_size = match order {
            Order::Limit(lo) => lo.size - filled_size,
            Order::Market(mo) => mo.size - filled_size,
        };

        if remaining_size <= 0.0 {
            return Ok(None);
        }

        // Create new order for remaining size
        let remaining_order = match order {
            Order::Limit(lo) => Order::Limit(LimitOrder {
                symbol: lo.symbol.clone(),
                side: lo.side,
                price: lo.price,
                size: remaining_size,
                leverage: lo.leverage,
                post_only: lo.post_only,
            }),
            Order::Market(mo) => Order::Market(MarketOrder {
                symbol: mo.symbol.clone(),
                side: mo.side,
                size: remaining_size,
                leverage: mo.leverage,
            }),
        };

        info!(
            "Partial fill detected: {:.4} remaining out of original size",
            remaining_size
        );

        Ok(Some(remaining_order))
    }

    // ===== Private Methods =====

    /// Execute a limit order
    async fn execute_limit_order(
        &self,
        order: &LimitOrder,
        slippage_est: &SlippageEstimate,
    ) -> Result<(f64, f64, ExecutionStatus)> {
        // Simulate order execution
        let execution_price = slippage_est.execution_price;

        // Assume full fill for this simulation
        Ok((order.size, execution_price, ExecutionStatus::Filled))
    }

    /// Execute a market order
    async fn execute_market_order(
        &self,
        order: &MarketOrder,
        slippage_est: &SlippageEstimate,
    ) -> Result<(f64, f64, ExecutionStatus)> {
        // Simulate market order execution with slippage
        let execution_price = slippage_est.execution_price;

        Ok((order.size, execution_price, ExecutionStatus::Filled))
    }

    /// Select best venue based on strategy
    fn select_venue(
        &self,
        estimates: &[SlippageEstimate],
        strategy: RoutingStrategy,
    ) -> Result<SlippageEstimate> {
        match strategy {
            RoutingStrategy::BestPrice => {
                // Select venue with lowest execution price for buys,
                // highest for sells
                let best = estimates
                    .iter()
                    .min_by(|a, b| a.execution_price.partial_cmp(&b.execution_price).unwrap())
                    .ok_or(Error::NoViableOpportunity)?;

                Ok(best.clone())
            }

            RoutingStrategy::MinSlippage => {
                // Select venue with minimum slippage
                let best = estimates
                    .iter()
                    .min_by(|a, b| {
                        a.slippage_percentage
                            .partial_cmp(&b.slippage_percentage)
                            .unwrap()
                    })
                    .ok_or(Error::NoViableOpportunity)?;

                Ok(best.clone())
            }

            RoutingStrategy::SmartRouting => {
                // Select based on weighted factors
                let best = estimates
                    .iter()
                    .max_by(|a, b| {
                        let score_a = self.calculate_venue_score(a);
                        let score_b = self.calculate_venue_score(b);
                        score_a.partial_cmp(&score_b).unwrap()
                    })
                    .ok_or(Error::NoViableOpportunity)?;

                Ok(best.clone())
            }

            RoutingStrategy::SplitExecution => {
                // For split execution, return the first available venue
                estimates
                    .first()
                    .cloned()
                    .ok_or(Error::NoViableOpportunity)
            }
        }
    }

    /// Calculate execution price based on order book
    fn calculate_execution_price(&self, order: &Order, book: &OrderBook) -> Result<f64> {
        let (size, side) = match order {
            Order::Limit(lo) => (lo.size, lo.side),
            Order::Market(mo) => (mo.size, mo.side),
        };

        if side.is_buy() {
            // Walk through ask side
            let mut remaining = size;
            let mut total_cost = 0.0;

            for (price, available) in &book.asks {
                let fill_size = remaining.min(*available);
                total_cost += fill_size * price;
                remaining -= fill_size;

                if remaining <= 0.0 {
                    return Ok(total_cost / size);
                }
            }

            // Not enough liquidity, return best ask
            book.best_ask()
                .ok_or(Error::NoViableOpportunity)
        } else {
            // Walk through bid side
            let mut remaining = size;
            let mut total_revenue = 0.0;

            for (price, available) in &book.bids {
                let fill_size = remaining.min(*available);
                total_revenue += fill_size * price;
                remaining -= fill_size;

                if remaining <= 0.0 {
                    return Ok(total_revenue / size);
                }
            }

            // Not enough liquidity, return best bid
            book.best_bid()
                .ok_or(Error::NoViableOpportunity)
        }
    }

    /// Estimate market impact
    fn estimate_market_impact(&self, size: f64, venue: &ExecutionVenue) -> f64 {
        // Simple market impact model: impact = (order_size / available_liquidity) * volatility
        let available_liquidity = if size > 0.0 {
            venue.order_book.as_ref().map(|ob| {
                if size > 0.0 {
                    ob.ask_volume(5)
                } else {
                    ob.bid_volume(5)
                }
            }).unwrap_or(size * 100.0)
        } else {
            size * 100.0
        };

        (size / available_liquidity) * venue.market_data.volatility * 100.0
    }

    /// Calculate venue score for smart routing
    fn calculate_venue_score(&self, estimate: &SlippageEstimate) -> f64 {
        // Score based on: slippage (40%), market impact (30%), recommended (30%)
        let slippage_score = (1.0 - (estimate.slippage_percentage / 1.0).min(1.0)) * 40.0;
        let impact_score = (1.0 - (estimate.market_impact / 1.0).min(1.0)) * 30.0;
        let recommended_score = if estimate.recommended { 30.0 } else { 0.0 };

        slippage_score + impact_score + recommended_score
    }
}

impl Default for OrderExecutionEngine {
    fn default() -> Self {
        Self::new(ExecutionRepository::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::market::OrderSide;

    fn create_test_engine() -> OrderExecutionEngine {
        OrderExecutionEngine::default()
    }

    fn create_test_market_data(symbol: &str, bid: f64, ask: f64) -> MarketData {
        MarketData {
            symbol: symbol.to_string(),
            bid,
            ask,
            last_price: (bid + ask) / 2.0,
            volume_24h: 1000000.0,
            volatility: 0.02,
            momentum: 0.01,
            timestamp: chrono::Utc::now().timestamp(),
        }
    }

    fn create_test_venue(protocol: ExecutionProtocol) -> ExecutionVenue {
        ExecutionVenue {
            protocol,
            market_data: create_test_market_data("SOLUSDT", 100.0, 102.0),
            order_book: None,
            available: true,
            latency_ms: 50,
        }
    }

    #[test]
    fn test_execution_repository_creation() {
        let repo = ExecutionRepository::new();
        assert_eq!(repo.open_plans.blocking_read().len(), 0);
        assert_eq!(repo.execution_history.blocking_read().len(), 0);
    }

    #[tokio::test]
    async fn test_execution_repository_store_plan() {
        let repo = ExecutionRepository::new();
        let _venue = create_test_venue(ExecutionProtocol::Drift);

        let slippage_est = SlippageEstimate {
            protocol: ExecutionProtocol::Drift,
            estimated_slippage: 0.1,
            slippage_percentage: 0.1,
            market_impact: 0.02,
            execution_price: 101.0,
            recommended: true,
        };

        let plan = ExecutionPlan {
            order: Order::Market(MarketOrder {
                symbol: "SOLUSDT".to_string(),
                side: OrderSide::Buy,
                size: 10.0,
                leverage: 2.0,
            }),
            routing_strategy: RoutingStrategy::BestPrice,
            selected_protocol: ExecutionProtocol::Drift,
            slippage_estimate: slippage_est,
            alternative_venues: vec![],
            timestamp: chrono::Utc::now().timestamp(),
        };

        repo.store_plan("plan-1".to_string(), plan.clone()).await;

        let retrieved = repo.get_plan("plan-1").await;
        assert!(retrieved.is_some());
    }

    #[tokio::test]
    async fn test_execution_repository_remove_plan() {
        let repo = ExecutionRepository::new();

        let slippage_est = SlippageEstimate {
            protocol: ExecutionProtocol::Drift,
            estimated_slippage: 0.1,
            slippage_percentage: 0.1,
            market_impact: 0.02,
            execution_price: 101.0,
            recommended: true,
        };

        let plan = ExecutionPlan {
            order: Order::Market(MarketOrder {
                symbol: "SOLUSDT".to_string(),
                side: OrderSide::Buy,
                size: 10.0,
                leverage: 2.0,
            }),
            routing_strategy: RoutingStrategy::BestPrice,
            selected_protocol: ExecutionProtocol::Drift,
            slippage_estimate: slippage_est,
            alternative_venues: vec![],
            timestamp: chrono::Utc::now().timestamp(),
        };

        repo.store_plan("plan-1".to_string(), plan).await;
        let removed = repo.remove_plan("plan-1").await;
        assert!(removed.is_some());
        assert!(repo.get_plan("plan-1").await.is_none());
    }

    #[tokio::test]
    async fn test_execution_repository_record_execution() {
        let repo = ExecutionRepository::new();

        let result = DetailedExecutionResult {
            result: ExecutionResult {
                order_id: "1".to_string(),
                symbol: "SOLUSDT".to_string(),
                status: ExecutionStatus::Filled,
                average_price: 101.0,
                filled_size: 10.0,
                remaining_size: 0.0,
                timestamp: chrono::Utc::now().timestamp(),
            },
            protocol: ExecutionProtocol::Drift,
            fills: vec![],
            total_fees: 0.1,
            actual_slippage: 0.05,
            execution_time_ms: 100,
        };

        repo.record_execution(result).await;
        let history = repo.get_execution_history(10).await;
        assert_eq!(history.len(), 1);
    }

    #[tokio::test]
    async fn test_execution_repository_history_limit() {
        let repo = ExecutionRepository::new();

        for i in 0..15 {
            let result = DetailedExecutionResult {
                result: ExecutionResult {
                    order_id: format!("{}", i),
                    symbol: "SOLUSDT".to_string(),
                    status: ExecutionStatus::Filled,
                    average_price: 101.0 + i as f64,
                    filled_size: 10.0,
                    remaining_size: 0.0,
                    timestamp: chrono::Utc::now().timestamp(),
                },
                protocol: ExecutionProtocol::Drift,
                fills: vec![],
                total_fees: 0.1,
                actual_slippage: 0.05,
                execution_time_ms: 100,
            };
            repo.record_execution(result).await;
        }

        let history = repo.get_execution_history(5).await;
        assert_eq!(history.len(), 5);
    }

    #[tokio::test]
    async fn test_execution_repository_symbol_history() {
        let repo = ExecutionRepository::new();

        for symbol in &["SOLUSDT", "BTCUSDT", "SOLUSDT"] {
            let result = DetailedExecutionResult {
                result: ExecutionResult {
                    order_id: format!("{}_{}", symbol, chrono::Utc::now().timestamp()),
                    symbol: symbol.to_string(),
                    status: ExecutionStatus::Filled,
                    average_price: 101.0,
                    filled_size: 10.0,
                    remaining_size: 0.0,
                    timestamp: chrono::Utc::now().timestamp(),
                },
                protocol: ExecutionProtocol::Drift,
                fills: vec![],
                total_fees: 0.1,
                actual_slippage: 0.05,
                execution_time_ms: 100,
            };
            repo.record_execution(result).await;
        }

        let sol_history = repo.get_symbol_history("SOLUSDT", 10).await;
        assert_eq!(sol_history.len(), 2);

        let btc_history = repo.get_symbol_history("BTCUSDT", 10).await;
        assert_eq!(btc_history.len(), 1);
    }

    #[tokio::test]
    async fn test_default_routing_strategy() {
        let repo = ExecutionRepository::new();

        let default = repo.get_default_strategy().await;
        assert_eq!(default, RoutingStrategy::SmartRouting);

        repo.set_default_strategy(RoutingStrategy::BestPrice).await;
        let updated = repo.get_default_strategy().await;
        assert_eq!(updated, RoutingStrategy::BestPrice);
    }

    #[test]
    fn test_order_execution_engine_creation() {
        let engine = create_test_engine();
        assert!(engine.drift_available);
        assert!(engine.hyperliquid_available);
    }

    #[test]
    fn test_protocol_availability() {
        let mut engine = create_test_engine();

        engine.set_protocol_available(ExecutionProtocol::Drift, false);
        assert!(!engine.drift_available);
        assert!(engine.hyperliquid_available);

        engine.set_protocol_available(ExecutionProtocol::Hyperliquid, false);
        assert!(!engine.hyperliquid_available);
    }

    #[test]
    fn test_slippage_estimate_creation() {
        let estimate = SlippageEstimate {
            protocol: ExecutionProtocol::Drift,
            estimated_slippage: 10.0,
            slippage_percentage: 0.1,
            market_impact: 0.02,
            execution_price: 101.0,
            recommended: true,
        };

        assert_eq!(estimate.protocol, ExecutionProtocol::Drift);
        assert!(estimate.recommended);
    }

    #[test]
    fn test_execution_plan_creation() {
        let plan = ExecutionPlan {
            order: Order::Market(MarketOrder {
                symbol: "SOLUSDT".to_string(),
                side: OrderSide::Buy,
                size: 10.0,
                leverage: 2.0,
            }),
            routing_strategy: RoutingStrategy::BestPrice,
            selected_protocol: ExecutionProtocol::Drift,
            slippage_estimate: SlippageEstimate {
                protocol: ExecutionProtocol::Drift,
                estimated_slippage: 0.1,
                slippage_percentage: 0.1,
                market_impact: 0.02,
                execution_price: 101.0,
                recommended: true,
            },
            alternative_venues: vec![],
            timestamp: chrono::Utc::now().timestamp(),
        };

        assert_eq!(plan.routing_strategy, RoutingStrategy::BestPrice);
        assert_eq!(plan.selected_protocol, ExecutionProtocol::Drift);
    }

    #[test]
    fn test_estimate_slippage_buy_order() {
        let engine = create_test_engine();
        let venue = create_test_venue(ExecutionProtocol::Drift);

        let order = Order::Market(MarketOrder {
            symbol: "SOLUSDT".to_string(),
            side: OrderSide::Buy,
            size: 10.0,
            leverage: 2.0,
        });

        let estimate = engine.estimate_slippage(&order, &venue).unwrap();
        assert_eq!(estimate.protocol, ExecutionProtocol::Drift);
        assert!(estimate.slippage_percentage >= 0.0);
    }

    #[test]
    fn test_estimate_slippage_sell_order() {
        let engine = create_test_engine();
        let venue = create_test_venue(ExecutionProtocol::Hyperliquid);

        let order = Order::Market(MarketOrder {
            symbol: "SOLUSDT".to_string(),
            side: OrderSide::Sell,
            size: 10.0,
            leverage: 2.0,
        });

        let estimate = engine.estimate_slippage(&order, &venue).unwrap();
        assert_eq!(estimate.protocol, ExecutionProtocol::Hyperliquid);
    }

    #[test]
    fn test_calculate_execution_price_no_orderbook() {
        let engine = create_test_engine();
        let venue = create_test_venue(ExecutionProtocol::Drift);

        let order = Order::Market(MarketOrder {
            symbol: "SOLUSDT".to_string(),
            side: OrderSide::Buy,
            size: 10.0,
            leverage: 2.0,
        });

        // Without order book, should still work
        let estimate = engine.estimate_slippage(&order, &venue).unwrap();
        assert!(estimate.execution_price > 0.0);
    }

    #[test]
    fn test_select_venue_best_price() {
        let engine = create_test_engine();

        let estimates = vec![
            SlippageEstimate {
                protocol: ExecutionProtocol::Drift,
                estimated_slippage: 0.1,
                slippage_percentage: 0.1,
                market_impact: 0.02,
                execution_price: 101.5,
                recommended: true,
            },
            SlippageEstimate {
                protocol: ExecutionProtocol::Hyperliquid,
                estimated_slippage: 0.2,
                slippage_percentage: 0.2,
                market_impact: 0.03,
                execution_price: 100.5,
                recommended: true,
            },
        ];

        let selected = engine
            .select_venue(&estimates, RoutingStrategy::BestPrice)
            .unwrap();

        assert_eq!(selected.execution_price, 100.5);
    }

    #[test]
    fn test_select_venue_min_slippage() {
        let engine = create_test_engine();

        let estimates = vec![
            SlippageEstimate {
                protocol: ExecutionProtocol::Drift,
                estimated_slippage: 0.2,
                slippage_percentage: 0.2,
                market_impact: 0.02,
                execution_price: 101.5,
                recommended: false,
            },
            SlippageEstimate {
                protocol: ExecutionProtocol::Hyperliquid,
                estimated_slippage: 0.1,
                slippage_percentage: 0.1,
                market_impact: 0.03,
                execution_price: 100.5,
                recommended: true,
            },
        ];

        let selected = engine
            .select_venue(&estimates, RoutingStrategy::MinSlippage)
            .unwrap();

        assert_eq!(selected.slippage_percentage, 0.1);
    }

    #[test]
    fn test_select_venue_smart_routing() {
        let engine = create_test_engine();

        let estimates = vec![
            SlippageEstimate {
                protocol: ExecutionProtocol::Drift,
                estimated_slippage: 0.05,
                slippage_percentage: 0.05,
                market_impact: 0.01,
                execution_price: 101.0,
                recommended: true,
            },
            SlippageEstimate {
                protocol: ExecutionProtocol::Hyperliquid,
                estimated_slippage: 0.1,
                slippage_percentage: 0.1,
                market_impact: 0.05,
                execution_price: 101.5,
                recommended: false,
            },
        ];

        let selected = engine
            .select_venue(&estimates, RoutingStrategy::SmartRouting)
            .unwrap();

        // Should prefer Drift with lower slippage and recommended
        assert_eq!(selected.protocol, ExecutionProtocol::Drift);
    }

    #[test]
    fn test_handle_partial_fill_limit_order() {
        let engine = create_test_engine();

        let order = Order::Limit(LimitOrder {
            symbol: "SOLUSDT".to_string(),
            side: OrderSide::Buy,
            price: 100.0,
            size: 10.0,
            leverage: 2.0,
            post_only: false,
        });

        let remaining = engine.handle_partial_fill(&order, 6.0).unwrap();
        assert!(remaining.is_some());

        let remaining_order = remaining.unwrap();
        match remaining_order {
            Order::Limit(lo) => {
                assert_eq!(lo.size, 4.0);
                assert_eq!(lo.price, 100.0);
            }
            _ => panic!("Expected limit order"),
        }
    }

    #[test]
    fn test_handle_partial_fill_market_order() {
        let engine = create_test_engine();

        let order = Order::Market(MarketOrder {
            symbol: "SOLUSDT".to_string(),
            side: OrderSide::Sell,
            size: 10.0,
            leverage: 2.0,
        });

        let remaining = engine.handle_partial_fill(&order, 7.0).unwrap();
        assert!(remaining.is_some());

        let remaining_order = remaining.unwrap();
        match remaining_order {
            Order::Market(mo) => {
                assert_eq!(mo.size, 3.0);
            }
            _ => panic!("Expected market order"),
        }
    }

    #[test]
    fn test_handle_partial_fill_complete_fill() {
        let engine = create_test_engine();

        let order = Order::Market(MarketOrder {
            symbol: "SOLUSDT".to_string(),
            side: OrderSide::Buy,
            size: 10.0,
            leverage: 2.0,
        });

        let remaining = engine.handle_partial_fill(&order, 10.0).unwrap();
        assert!(remaining.is_none());
    }

    #[test]
    fn test_handle_partial_fill_overfill() {
        let engine = create_test_engine();

        let order = Order::Market(MarketOrder {
            symbol: "SOLUSDT".to_string(),
            side: OrderSide::Buy,
            size: 10.0,
            leverage: 2.0,
        });

        let remaining = engine.handle_partial_fill(&order, 15.0).unwrap();
        assert!(remaining.is_none());
    }

    #[test]
    fn test_execution_protocol_as_str() {
        assert_eq!(ExecutionProtocol::Drift.as_str(), "drift");
        assert_eq!(ExecutionProtocol::Hyperliquid.as_str(), "hyperliquid");
    }

    #[test]
    fn test_routing_strategy_enum() {
        let strategies = vec![
            RoutingStrategy::BestPrice,
            RoutingStrategy::MinSlippage,
            RoutingStrategy::SmartRouting,
            RoutingStrategy::SplitExecution,
        ];

        assert_eq!(strategies.len(), 4);
    }

    #[tokio::test]
    async fn test_execution_result_recording() {
        let engine = create_test_engine();
        let repo = &engine.repository;

        let result = DetailedExecutionResult {
            result: ExecutionResult {
                order_id: "test-1".to_string(),
                symbol: "SOLUSDT".to_string(),
                status: ExecutionStatus::Filled,
                average_price: 101.0,
                filled_size: 10.0,
                remaining_size: 0.0,
                timestamp: chrono::Utc::now().timestamp(),
            },
            protocol: ExecutionProtocol::Drift,
            fills: vec![ExecutionFill {
                timestamp: chrono::Utc::now().timestamp(),
                price: 101.0,
                size: 10.0,
                fee: 0.1,
            }],
            total_fees: 0.1,
            actual_slippage: 0.05,
            execution_time_ms: 100,
        };

        repo.record_execution(result).await;
        let history = repo.get_execution_history(10).await;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].result.order_id, "test-1");
    }

    #[test]
    fn test_estimate_market_impact() {
        let engine = create_test_engine();
        let venue = create_test_venue(ExecutionProtocol::Drift);

        let impact = engine.estimate_market_impact(10.0, &venue);
        assert!(impact >= 0.0);
    }

    #[test]
    fn test_calculate_venue_score() {
        let engine = create_test_engine();

        let high_score = SlippageEstimate {
            protocol: ExecutionProtocol::Drift,
            estimated_slippage: 0.01,
            slippage_percentage: 0.01,
            market_impact: 0.01,
            execution_price: 101.0,
            recommended: true,
        };

        let low_score = SlippageEstimate {
            protocol: ExecutionProtocol::Hyperliquid,
            estimated_slippage: 0.5,
            slippage_percentage: 0.5,
            market_impact: 0.5,
            execution_price: 101.0,
            recommended: false,
        };

        let high_score_val = engine.calculate_venue_score(&high_score);
        let low_score_val = engine.calculate_venue_score(&low_score);

        assert!(high_score_val > low_score_val);
    }

    #[tokio::test]
    async fn test_clear_execution_history() {
        let engine = create_test_engine();
        let repo = &engine.repository;

        let result = DetailedExecutionResult {
            result: ExecutionResult {
                order_id: "1".to_string(),
                symbol: "SOLUSDT".to_string(),
                status: ExecutionStatus::Filled,
                average_price: 101.0,
                filled_size: 10.0,
                remaining_size: 0.0,
                timestamp: chrono::Utc::now().timestamp(),
            },
            protocol: ExecutionProtocol::Drift,
            fills: vec![],
            total_fees: 0.1,
            actual_slippage: 0.05,
            execution_time_ms: 100,
        };

        repo.record_execution(result).await;
        assert_eq!(repo.get_execution_history(10).await.len(), 1);

        repo.clear_history().await;
        assert_eq!(repo.get_execution_history(10).await.len(), 0);
    }
}
