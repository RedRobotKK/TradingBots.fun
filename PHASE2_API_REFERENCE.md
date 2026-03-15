# Phase 2 API Reference

## Hyperliquid Protocol Client API

### Initialization
```rust
use tradingbots_fun::HyperliquidClient;

let client = HyperliquidClient::new(
    "account-123".to_string(),
    "api-key-secret".to_string()
);
```

### Market Data Operations

#### Get Market Snapshot
```rust
let market_data: MarketData = client.get_market_data("SOLUSDT").await?;
println!("Bid: {}, Ask: {}", market_data.bid, market_data.ask);
println!("Spread: {:.4}%", market_data.spread_percentage());
```

#### Get Order Book
```rust
let order_book: OrderBook = client.get_order_book("SOLUSDT", 20).await?;
if let Some(best_bid) = order_book.best_bid() {
    println!("Best bid: {}", best_bid);
}
println!("Bid volume (5 levels): {}", order_book.bid_volume(5));
```

### Order Management

#### Place Limit Order
```rust
use tradingbots_fun::models::market::{LimitOrder, OrderSide};

let order = LimitOrder {
    symbol: "SOLUSDT".to_string(),
    side: OrderSide::Buy,
    price: 100.50,
    size: 10.5,
    leverage: 2.0,
    post_only: true,
};

let response = client.place_limit_order(&order).await?;
println!("Order ID: {}", response.order_id);
println!("Filled: {} at {}", response.filled_size, response.average_price);
```

#### Place Market Order
```rust
use tradingbots_fun::models::market::MarketOrder;

let order = MarketOrder {
    symbol: "BTCUSDT".to_string(),
    side: OrderSide::Sell,
    size: 0.5,
    leverage: 1.0,
};

let response = client.place_market_order(&order).await?;
```

#### Cancel Order
```rust
client.cancel_order("order-id-123", "SOLUSDT").await?;
```

### Position Management

#### Get All Positions
```rust
let positions: Vec<Position> = client.get_positions().await?;
for pos in positions {
    println!("{}: {} {} @ {}",
        pos.symbol,
        pos.size,
        if pos.side.is_buy() { "LONG" } else { "SHORT" },
        pos.entry_price
    );
    println!("  PnL: ${}", pos.unrealized_pnl);
    println!("  Time to liquidation: {:?}ms", pos.time_to_liquidation());
}
```

#### Get Specific Position
```rust
if let Some(position) = client.get_position("SOLUSDT").await? {
    println!("Size: {}", position.size);
    println!("Margin ratio: {:.2}%", position.margin_ratio() * 100.0);
} else {
    println!("No position for SOLUSDT");
}
```

### Account Information

#### Get Account Info
```rust
let account: AccountInfo = client.get_account_info().await?;
println!("Total equity: ${}", account.total_equity);
println!("Free margin: ${}", account.free_margin);
println!("Can trade: {}", account.can_trade());
println!("Liquidation risk: {:.2}%", account.liquidation_risk() * 100.0);
```

#### Get Fill History
```rust
let fills: Vec<Fill> = client.get_fills(100).await?;
for fill in fills {
    println!("{}: {} @ {}, fee: {}",
        fill.symbol,
        fill.size,
        fill.price,
        fill.fee
    );
}
```

### Risk Monitoring

#### Check Liquidation Risk
```rust
let is_risky = client.check_liquidation_risk("SOLUSDT", 0.10).await?;
if is_risky {
    eprintln!("Position is within 10% of liquidation!");
    // Take action: reduce position, add margin, etc.
}
```

#### Cache Management
```rust
// Clear all caches to force fresh API calls
client.clear_caches().await;
```

---

## Order Execution Engine API

### Initialization

#### Create Repository
```rust
use tradingbots_fun::ExecutionRepository;

let repository = ExecutionRepository::new();
```

#### Create Engine
```rust
use tradingbots_fun::OrderExecutionEngine;

let engine = OrderExecutionEngine::new(repository);
```

### Routing Strategy Configuration

#### Set Default Strategy
```rust
use tradingbots_fun::RoutingStrategy;

// Available strategies:
// - RoutingStrategy::BestPrice
// - RoutingStrategy::MinSlippage
// - RoutingStrategy::SmartRouting
// - RoutingStrategy::SplitExecution

repository.set_default_strategy(RoutingStrategy::SmartRouting).await;
```

#### Get Default Strategy
```rust
let strategy = repository.get_default_strategy().await;
println!("Current routing strategy: {:?}", strategy);
```

### Protocol Availability

```rust
use tradingbots_fun::ExecutionProtocol;

// Mark a protocol as unavailable
engine.set_protocol_available(ExecutionProtocol::Drift, false);

// Mark as available again
engine.set_protocol_available(ExecutionProtocol::Hyperliquid, true);
```

### Order Execution Planning

#### Plan Execution with Analysis
```rust
use tradingbots_fun::{ExecutionVenue, ExecutionProtocol};

// Prepare execution venues
let drift_venue = ExecutionVenue {
    protocol: ExecutionProtocol::Drift,
    market_data: drift_market_data,
    order_book: Some(drift_order_book),
    available: true,
    latency_ms: 45,
};

let hyperliquid_venue = ExecutionVenue {
    protocol: ExecutionProtocol::Hyperliquid,
    market_data: hyperliquid_market_data,
    order_book: None,
    available: true,
    latency_ms: 55,
};

let venues = vec![drift_venue, hyperliquid_venue];

// Plan execution with explicit strategy
let plan = engine.plan_execution(
    Order::Market(market_order),
    venues,
    Some(RoutingStrategy::MinSlippage)
).await?;

// Analyze the plan
println!("Selected protocol: {:?}", plan.selected_protocol);
println!("Estimated slippage: {:.4}% (${:.2})",
    plan.slippage_estimate.slippage_percentage,
    plan.slippage_estimate.estimated_slippage
);
println!("Market impact: {:.4}%", plan.slippage_estimate.market_impact);
println!("Execution price: ${:.2}", plan.slippage_estimate.execution_price);

// Check alternative venues
for alt in &plan.alternative_venues {
    println!("  Alternative {:?}: {:.4}% slippage",
        alt.protocol,
        alt.slippage_percentage
    );
}
```

### Order Execution

#### Execute Planned Order
```rust
let result = engine.execute(plan).await?;

// Analyze execution
println!("Order ID: {}", result.result.order_id);
println!("Status: {:?}", result.result.status);
println!("Filled: {} @ {} average",
    result.result.filled_size,
    result.result.average_price
);
println!("Total fees: ${}", result.total_fees);
println!("Actual slippage: ${:.4}", result.actual_slippage);
println!("Execution time: {}ms", result.execution_time_ms);

// Review fills
for fill in result.fills {
    println!("  {} @ {} (fee: ${})", fill.size, fill.price, fill.fee);
}
```

### Slippage Estimation

#### Estimate Individual Venue Slippage
```rust
let order = Order::Market(MarketOrder {
    symbol: "SOLUSDT".to_string(),
    side: OrderSide::Buy,
    size: 100.0,
    leverage: 1.0,
});

let slippage = engine.estimate_slippage(&order, &venue)?;

println!("Protocol: {:?}", slippage.protocol);
println!("Estimated slippage: {:.4}% (${:.2})",
    slippage.slippage_percentage,
    slippage.estimated_slippage
);
println!("Execution price: ${:.2}", slippage.execution_price);
println!("Market impact: {:.4}%", slippage.market_impact);
println!("Recommended: {}", slippage.recommended);
```

### Partial Fill Handling

#### Process Partial Fills
```rust
// Original order size: 100.0
// Filled size: 60.0
// Need to place remaining order

let remaining = engine.handle_partial_fill(&order, 60.0)?;

if let Some(remaining_order) = remaining {
    // Remaining size: 40.0
    match remaining_order {
        Order::Market(mo) => {
            println!("Placing remaining market order for {} units", mo.size);
            let result = client.place_market_order(&mo).await?;
        }
        Order::Limit(lo) => {
            println!("Placing remaining limit order for {} units @ {}",
                lo.size,
                lo.price
            );
            let result = client.place_limit_order(&lo).await?;
        }
    }
} else {
    println!("Order completely filled!");
}
```

### Execution History Management

#### Retrieve Execution History
```rust
// Get last 10 executions
let history = repository.get_execution_history(10).await;

for execution in history {
    println!("Executed on {:?}:", execution.protocol);
    println!("  Order ID: {}", execution.result.order_id);
    println!("  Status: {:?}", execution.result.status);
    println!("  Avg price: ${}", execution.result.average_price);
    println!("  Slippage: ${:.4}", execution.actual_slippage);
}
```

#### Filter History by Symbol
```rust
let symbol_history = repository.get_symbol_history("SOLUSDT", 20).await;

for execution in symbol_history {
    println!("{} units @ ${} ({}ms)",
        execution.result.filled_size,
        execution.result.average_price,
        execution.execution_time_ms
    );
}
```

#### Manage Execution Plans
```rust
// Store a plan for later reference
let plan_id = "plan-2025-02-21-001".to_string();
repository.store_plan(plan_id.clone(), plan).await;

// Retrieve the plan
if let Some(retrieved_plan) = repository.get_plan(&plan_id).await {
    println!("Retrieved plan: {:?}", retrieved_plan.routing_strategy);
}

// Remove when done
repository.remove_plan(&plan_id).await;
```

#### Clear History
```rust
// Clear all execution history (useful for testing)
repository.clear_history().await;
```

---

## Complete Example: Market Making Bot

```rust
use tradingbots_fun::{
    HyperliquidClient, OrderExecutionEngine, ExecutionRepository,
    RoutingStrategy, ExecutionVenue, ExecutionProtocol,
    models::market::{MarketOrder, OrderSide, Order},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize
    let client = HyperliquidClient::new("account-1".to_string(), "key".to_string());
    let repo = ExecutionRepository::new();
    let mut engine = OrderExecutionEngine::new(repo);

    repo.set_default_strategy(RoutingStrategy::MinSlippage).await;

    // Market making loop
    loop {
        // Fetch market data from both venues
        let market = client.get_market_data("SOLUSDT").await?;
        let order_book = client.get_order_book("SOLUSDT", 20).await?;

        // Check if we have open positions
        if let Some(position) = client.get_position("SOLUSDT").await? {
            if position.side.is_buy() && market.bid > position.entry_price * 1.01 {
                // Take profit: sell 50%
                let sell_order = MarketOrder {
                    symbol: "SOLUSDT".to_string(),
                    side: OrderSide::Sell,
                    size: position.size * 0.5,
                    leverage: 1.0,
                };

                // Create execution venues
                let venues = vec![/* drift_venue, hyperliquid_venue */];

                // Execute with minimum slippage routing
                let plan = engine.plan_execution(
                    Order::Market(sell_order),
                    venues,
                    Some(RoutingStrategy::MinSlippage)
                ).await?;

                let result = engine.execute(plan).await?;
                println!("Sold: {} @ {}", result.result.filled_size, result.result.average_price);

                // Handle partial fills
                if let Some(remaining) = engine.handle_partial_fill(
                    &Order::Market(sell_order),
                    result.result.filled_size
                )? {
                    // Place remaining order
                }
            }
        }

        // Check account health
        let account = client.get_account_info().await?;
        if !account.can_trade() {
            eprintln!("Account health too low!");
            break;
        }

        // Check liquidation risk on positions
        if client.check_liquidation_risk("SOLUSDT", 0.10).await? {
            eprintln!("Liquidation risk! Closing positions...");
            // Implement emergency close logic
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    Ok(())
}
```

---

## Error Handling Patterns

### Comprehensive Error Handling
```rust
match client.get_market_data("SOLUSDT").await {
    Ok(market) => {
        println!("Market data: bid={}, ask={}", market.bid, market.ask);
    }
    Err(Error::ApiRequestFailed(msg)) => {
        eprintln!("API error: {}", msg);
        // Retry logic
    }
    Err(e) => {
        eprintln!("Unexpected error: {}", e);
    }
}
```

### Using Result Type Alias
```rust
async fn execute_with_fallback() -> Result<DetailedExecutionResult> {
    // Try primary venue
    match engine.execute(plan).await {
        Ok(result) => Ok(result),
        Err(Error::ApiRequestFailed(_)) => {
            // Fallback to secondary venue
            engine.execute(fallback_plan).await
        }
        Err(e) => Err(e),
    }
}
```

---

## Performance Tips

1. **Reuse Clients**: Create once, use many times
   ```rust
   let client = Arc::new(HyperliquidClient::new(id, key));
   ```

2. **Cache Market Data**: Avoid repeated calls
   ```rust
   let market = client.get_market_data("SOLUSDT").await?; // Cached
   ```

3. **Batch Operations**: Process fills in bulk
   ```rust
   let fills = client.get_fills(1000).await?;
   ```

4. **Use Appropriate Strategies**:
   - `BestPrice`: When price matters most
   - `MinSlippage`: For large orders
   - `SmartRouting`: For balanced execution

---

## Type Safety Guarantees

All code uses Rust's type system to prevent:
- Invalid order states
- Missing error handling
- Memory unsafety
- Data races
- Null pointer dereferences

Example: Cannot place an order without all required fields
```rust
// COMPILE ERROR: missing required fields
let bad_order = MarketOrder {
    symbol: "SOLUSDT".to_string(),
    // Missing: side, size, leverage
};
```
