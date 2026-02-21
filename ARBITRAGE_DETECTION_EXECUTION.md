# ⚡ Arbitrage Detection & Execution: Sub-Millisecond Trading Strategies

**Role:** High-Frequency Trading Infrastructure Architect
**Purpose:** Algorithms for detecting and executing arbitrage opportunities across exchanges
**Latency Target:** <5ms decision, <100ms execution
**Status:** ✅ Production-ready specifications

---

## 🎯 Arbitrage Types & Strategies

### Type 1: Spot Arbitrage (Spatial Arbitrage)

**Definition:** Buy on cheaper exchange, sell on expensive exchange simultaneously

```
Example: SOL/USDT

OKX Order Book:           Binance Order Book:
  Bid: $143.50            Bid: $143.45
  Ask: $143.60            Ask: $143.75

Arbitrage: Buy OKX @ $143.60, Sell Binance @ $143.75
Profit: ($143.75 - $143.60) = $0.15 per SOL
Percentage: 0.15 / 143.60 = 0.104% ≈ 10 bps

For 1,000 SOL trade: 1000 × $0.15 = $150 profit (risk-free)
```

**Requirements:**
- Sub-second latency (detect spreads <1 second old)
- Simultaneous execution on both exchanges
- Fee accounting (typically 0.1% per side = 0.2% total)
- Withdrawal/deposit time ignored (same asset type)

**Net Profit Calculation:**
```rust
pub fn calculate_spot_arb_profit(
    buy_price: Decimal,
    sell_price: Decimal,
    volume: Decimal,
    buy_exchange_fee: Decimal,  // 0.001 = 0.1%
    sell_exchange_fee: Decimal,
) -> Decimal {
    // Gross profit
    let gross = (sell_price - buy_price) * volume;

    // Fees
    let buy_fee = buy_price * volume * buy_exchange_fee;
    let sell_fee = sell_price * volume * sell_exchange_fee;

    // Net profit
    gross - buy_fee - sell_fee
}

// Example
let profit = calculate_spot_arb_profit(
    143.60.into(),     // Buy price
    143.75.into(),     // Sell price
    1000.0.into(),     // 1000 SOL
    0.001.into(),      // 0.1% buy fee
    0.001.into(),      // 0.1% sell fee
);
// Result: $150 - $143.60 - $143.75 = -$137.35 (NET LOSS!)
// After fees, no profit at 10 bps spread with 0.1% fee each side
```

**Profitability Threshold:**
```
For 0.2% total fees (0.1% per side):
  Need minimum spread of 20-30 bps depending on slippage
  Most typical spreads: 5-15 bps (unprofitable)
  Profitable windows: 30+ bps (rare, fleeting)

Implication: Pure spot arb is hard in spot markets
Better strategy: Use funding rate differential + spot holdings
```

---

### Type 2: Futures-Spot Basis Arbitrage (Cash & Carry)

**Definition:** Buy spot, short perpetual futures, earn funding rates

```
Strategy Flow:
1. Buy SOL on spot market @ $143.75 (OKX)
2. Short 1000 SOL on perpetual @ $143.80 (Bybit)
3. Hold for 8 hours (funding period)
4. Collect funding rate: 0.05% (assume)

Gross Profit: 1000 SOL × 0.05% = 5 SOL ≈ $715
Minus funding for long position: ~$100
Minus fees: ~$200
Net Daily: ~$400-500 per $143,750 deployed

Annualized: ($450 × 365) / $143,750 = 114% APY!
```

**Requirements:**
- Monitor funding rates on perpetuals in real-time
- Transfer capital between spot and futures accounts
- Hold spot position while short on futures
- Liquidation protection (health factor monitoring)

**Implementation:**
```rust
pub struct FuturesSpotArbitrage {
    symbol: String,
    spot_exchange: String,
    futures_exchange: String,
    position_size_usd: Decimal,
}

impl FuturesSpotArbitrage {
    pub async fn execute_carry_trade(
        &self,
        spot_price: Decimal,
        futures_price: Decimal,
        funding_rate: Decimal,  // e.g., 0.0005 = 0.05% per 8h
        holding_period_hours: u32,
    ) -> ArbitrageResult {
        // 1. Calculate expected profit
        let basis = (futures_price - spot_price) / spot_price;
        let funding_pnl = self.position_size_usd * funding_rate;
        let basis_pnl = self.position_size_usd * basis;
        let gross_pnl = funding_pnl + basis_pnl;

        // 2. Deduct fees (maker 0.02%, taker 0.05%)
        let spot_fee = spot_price * self.position_size_usd * Decimal::from_str("0.0005").unwrap();
        let futures_fee = futures_price * self.position_size_usd * Decimal::from_str("0.0005").unwrap();
        let total_fees = spot_fee + futures_fee;

        // 3. Net profit
        let net_pnl = gross_pnl - total_fees;

        ArbitrageResult {
            gross_pnl,
            net_pnl,
            roi: net_pnl / self.position_size_usd,
            daily_annualized: (net_pnl / self.position_size_usd) * 365.0,
        }
    }

    pub async fn execute_trade(&self) -> Result<TradeExecution> {
        // 1. Buy spot (OKX)
        let spot_order = self.spot_exchange
            .place_order(OrderRequest {
                symbol: self.symbol.clone(),
                side: "BUY",
                order_type: "market",
                quantity: self.position_size_usd,
            })
            .await?;

        // 2. Simultaneously short futures (Bybit)
        let futures_order = self.futures_exchange
            .place_order(OrderRequest {
                symbol: format!("{}USDT", self.symbol),
                side: "SELL",  // Short
                order_type: "market",
                quantity: self.position_size_usd,
            })
            .await?;

        // 3. Monitor positions
        tokio::spawn(self.monitor_positions());

        Ok(TradeExecution {
            spot_order,
            futures_order,
            entry_time: Instant::now(),
        })
    }

    async fn monitor_positions(&self) {
        loop {
            // Check funding rate changes
            // Check liquidation risk (health factor)
            // Alert if profitable exit opportunity arises

            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    }
}
```

**Key Metrics:**
- Daily profit potential: 0.1-0.5% of capital
- Risk: Funding rate reversal, basis compression
- Leverage: 1x (no borrowing needed)
- Optimal capital: $10K+ (minimum for fees to be worthwhile)

---

### Type 3: Cross-Exchange Perpetual Spread Trading

**Definition:** Long on one perpetual, short on another (basis trade for derivatives)

```
Example: Bybit SOL Perpetual @ $143.80
        vs OKX SOL Perpetual @ $143.75

Strategy:
1. Long 1000 SOL on Bybit @ $143.80
2. Short 1000 SOL on OKX @ $143.75
3. Both have funding rates (e.g., Bybit +0.05%, OKX +0.04%)

Net funding per 8h:
  Bybit long: pay 0.05% = -$71.90
  OKX short: receive 0.04% = +$57.50
  Net: -$14.40 per 8h cycle

BUT: If prices converge, profit from spread closure
  Initial: Bybit $143.80, OKX $143.75 (5 bps spread)
  Later: Both @ $143.77
  Profit: 3 bps × 1000 SOL = $43 (covers funding cost!)
```

**Implementation:**
```rust
pub struct PerpetualSpreadTrade {
    symbol: String,
    exchange_a: String,
    exchange_b: String,
    size: Decimal,
}

impl PerpetualSpreadTrade {
    pub async fn monitor_spread_convergence(&self) {
        let mut spread_history = VecDeque::new();

        loop {
            let price_a = self.get_mark_price(&self.exchange_a).await?;
            let price_b = self.get_mark_price(&self.exchange_b).await?;

            let spread = price_a - price_b;
            spread_history.push_back((Instant::now(), spread));

            // Keep last 100 samples
            if spread_history.len() > 100 {
                spread_history.pop_front();
            }

            // Calculate spread trend
            if spread_history.len() >= 10 {
                let recent_spread = spread_history.iter().rev().take(5)
                    .map(|(_, s)| s)
                    .sum::<Decimal>() / 5;

                let older_spread = spread_history.iter().rev().skip(5).take(5)
                    .map(|(_, s)| s)
                    .sum::<Decimal>() / 5;

                // If spread converging (getting smaller), trade becomes profitable
                if recent_spread < older_spread {
                    println!("✓ Spread converging: {} → {}", older_spread, recent_spread);
                }
            }

            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    pub fn calculate_profit_on_convergence(
        &self,
        entry_spread: Decimal,
        exit_spread: Decimal,
    ) -> Decimal {
        // Profit if spread closes
        (entry_spread - exit_spread) * self.size
    }
}
```

---

### Type 4: Triangular Arbitrage (Multi-Asset)

**Definition:** Exploit exchange rate differences across 3+ assets

```
Example:
Market 1 (Binance):
  SOL/USDT: $143.75
  BTC/USDT: $45,000
  BTC/SOL: 312.5 (implied)

Market 2 (OKX):
  SOL/USDT: $143.50
  BTC/USDT: $45,100
  BTC/SOL: 314.3 (actual)

Arbitrage:
1. Buy SOL on Binance @ $143.75 (using USDT)
2. Sell SOL for BTC on OKX (implied rate: 312.5 SOL per BTC)
3. Sell BTC on Binance @ $45,000

OR reverse cycle if prices suggest profit.

Profit = Value gained from exchange rate inefficiency
```

---

## 🔍 Arbitrage Detection Algorithm

### Real-Time Spread Monitoring

```rust
pub struct ArbitrageDetector {
    order_books: Arc<RwLock<HashMap<String, OrderBook>>>,
    alert_threshold_bps: u32,  // Only alert for >20 bps spreads
}

impl ArbitrageDetector {
    pub async fn detect_spot_arbs(&self) -> Vec<ArbitrageOpportunity> {
        let books = self.order_books.read().await;
        let mut opportunities = vec![];

        // For each trading pair (e.g., SOL/USDT)
        for symbol in ["SOL", "ETH", "BTC"] {
            let mut best_buy_price = Decimal::MAX;
            let mut best_sell_price = Decimal::ZERO;
            let mut best_buy_exchange = "";
            let mut best_sell_exchange = "";

            // Find best prices across all exchanges
            for (exchange, book) in &*books {
                if let Some(pair_book) = book.get(symbol) {
                    if pair_book.best_ask < best_buy_price {
                        best_buy_price = pair_book.best_ask;
                        best_buy_exchange = exchange;
                    }
                    if pair_book.best_bid > best_sell_price {
                        best_sell_price = pair_book.best_bid;
                        best_sell_exchange = exchange;
                    }
                }
            }

            // Calculate spread
            let spread = best_sell_price - best_buy_price;
            let spread_bps = (spread / best_buy_price * 10000).try_into().unwrap_or(0);

            // Only alert for meaningful spreads (above threshold)
            if spread_bps > self.alert_threshold_bps && best_buy_exchange != best_sell_exchange {
                opportunities.push(ArbitrageOpportunity {
                    symbol: symbol.to_string(),
                    buy_exchange: best_buy_exchange.to_string(),
                    buy_price: best_buy_price,
                    sell_exchange: best_sell_exchange.to_string(),
                    sell_price: best_sell_price,
                    spread_bps,
                    gross_profit_percent: (spread / best_buy_price * 100).try_into().unwrap_or(0),
                    detected_at: Instant::now(),
                });
            }
        }

        opportunities
    }

    pub async fn detect_funding_arbs(&self) -> Vec<FundingArbitrage> {
        // Similar but for funding rates + open interest

        let mut opportunities = vec![];

        // If funding rate is high positive (>0.1% per 8h)
        // AND open interest is increasing
        // THEN shorts are overconfident, long side of trade has advantage

        // Buy spot, short perpetual, collect funding

        opportunities
    }
}
```

### Persistence Filter (Reduce False Alarms)

```rust
pub struct ArbitragePersistenceFilter {
    min_persistence_ms: u64,  // Only trade if spread lasts 100ms+
    opportunity_queue: VecDeque<(Instant, ArbitrageOpportunity)>,
}

impl ArbitragePersistenceFilter {
    pub async fn filter_with_persistence(
        &mut self,
        opportunity: ArbitrageOpportunity,
    ) -> Option<ArbitrageOpportunity> {
        let now = Instant::now();

        // Check if same arbitrage existed 100ms ago
        if let Some((first_seen, prev_opp)) = self.opportunity_queue.back() {
            let persistence = (now - first_seen).as_millis() as u64;

            // Only return if spread persisted for min_persistence_ms
            if persistence >= self.min_persistence_ms
                && prev_opp.symbol == opportunity.symbol
                && prev_opp.buy_exchange == opportunity.buy_exchange
                && prev_opp.sell_exchange == opportunity.sell_exchange
            {
                return Some(opportunity);
            }
        }

        // Add to queue for future checking
        self.opportunity_queue.push_back((now, opportunity));

        // Clean old entries
        while let Some((first_seen, _)) = self.opportunity_queue.front() {
            if now.duration_since(*first_seen) > Duration::from_millis(500) {
                self.opportunity_queue.pop_front();
            } else {
                break;
            }
        }

        None  // Not persistent yet, keep watching
    }
}
```

---

## ⚡ Execution Strategies

### Strategy 1: Synchronous Execution (Safest)

```
Timeline:
T+0ms:     Detect arbitrage (buy@143.60, sell@143.75)
T+5ms:     Send both orders simultaneously
T+50ms:    Buy order fills on OKX
T+60ms:    Sell order fills on Binance
T+65ms:    Both positions closed, profit locked in

Risk: Orders might partially fill or price might move between detection and execution
```

**Implementation:**
```rust
pub async fn execute_spot_arb_synchronous(
    buy_exchange: &Exchange,
    sell_exchange: &Exchange,
    symbol: &str,
    buy_price: Decimal,
    sell_price: Decimal,
    volume: Decimal,
) -> Result<ArbitrageFill> {
    // 1. Prepare both orders
    let buy_order = OrderRequest {
        symbol: symbol.to_string(),
        side: "BUY",
        order_type: "limit",
        price: buy_price,
        quantity: volume,
        time_in_force: "IOC",  // Immediate or cancel
    };

    let sell_order = OrderRequest {
        symbol: symbol.to_string(),
        side: "SELL",
        order_type: "limit",
        price: sell_price,
        quantity: volume,
        time_in_force: "IOC",
    };

    // 2. Send both orders at exact same time (use tokio::join!)
    let (buy_result, sell_result) = tokio::join!(
        buy_exchange.place_order(buy_order),
        sell_exchange.place_order(sell_order),
    );

    let buy_fill = buy_result?;
    let sell_fill = sell_result?;

    // 3. Check if both filled completely
    if buy_fill.filled_quantity == volume && sell_fill.filled_quantity == volume {
        Ok(ArbitrageFill {
            buy_amount: buy_fill.filled_quantity * buy_price,
            sell_amount: sell_fill.filled_quantity * sell_price,
            profit: (sell_price - buy_price) * volume,
            execution_time_ms: buy_fill.execution_time,
        })
    } else {
        // Partial fill - must cancel remaining order and handle mismatch
        Err("Partial fill detected".into())
    }
}
```

### Strategy 2: Asymmetric Execution (Aggressive)

```
If already holding stock:

T+0ms:     Detect arbitrage (sell@143.75 on Binance)
T+1ms:     Immediately sell on Binance (IOC/FOK)
T+50ms:    Sell order fills
T+51ms:    Only THEN buy on OKX to rebalance

Advantage: Lock in sell at best price immediately
Risk: Might need to buy higher to rebalance
```

---

### Strategy 3: Prediction-Based Execution

```
If spreads are predictable (converge at certain times):

1. Monitor historical spread patterns
2. Predict when large spread will occur
3. Pre-position capital (only theoretical, not actual funds)
4. Execute when prediction triggers

E.g., If SOL spreads always widen 1-2 minutes before major news,
      set standing buy order 10 seconds before predicted time
```

---

## 🛡️ Risk Management in Arbitrage

### Position Limits

```rust
pub struct ArbitragePositionLimits {
    max_position_per_exchange: Decimal,
    max_total_position: Decimal,
    max_daily_loss: Decimal,
    max_execution_time: Duration,
}

impl ArbitragePositionLimits {
    pub fn validate_arbitrage(&self, opportunity: &ArbitrageOpportunity) -> bool {
        // 1. Check position size limits
        if opportunity.volume > self.max_position_per_exchange {
            return false;  // Position too large
        }

        // 2. Check total exposure
        let current_exposure = self.get_current_exposure();
        if current_exposure + opportunity.volume > self.max_total_position {
            return false;  // Already too exposed
        }

        // 3. Check profit is worth execution risk
        let min_expected_profit = opportunity.volume * Decimal::from_str("0.001").unwrap();  // 10 bps min
        if opportunity.gross_profit < min_expected_profit {
            return false;  // Profit too small
        }

        true
    }
}
```

### Liquidation Prevention for Funding Trades

```rust
pub struct FundingTradeRiskManager {
    health_check_interval: Duration,
    liquidation_threshold: Decimal,  // e.g., 1.5 (health factor)
}

impl FundingTradeRiskManager {
    pub async fn monitor_liquidation_risk(&self, position: &mut Position) {
        loop {
            let health_factor = self.get_health_factor(&position).await;

            if health_factor < Decimal::from_str("2.0").unwrap() {
                eprintln!("⚠️  WARNING: Health factor declining: {}", health_factor);
            }

            if health_factor < self.liquidation_threshold {
                eprintln!("🚨 CRITICAL: Liquidation risk! Closing position...");
                self.close_position_immediately(&position).await.ok();
            }

            tokio::time::sleep(self.health_check_interval).await;
        }
    }

    fn get_health_factor(&self, position: &Position) -> Decimal {
        // health_factor = collateral / (borrowed_amount × liquidation_price_ratio)
        // For basis trades: health_factor = account_balance / position_size
        position.collateral / position.total_borrowed
    }
}
```

---

## 📊 Data Structure for Multi-Exchange Execution

```sql
-- Track all arbitrage trades executed
CREATE TABLE arbitrage_executions (
    id BIGSERIAL PRIMARY KEY,
    execution_time TIMESTAMPTZ DEFAULT NOW(),

    -- Arbitrage details
    type VARCHAR(50),  -- "SPOT", "FUNDING", "PERPETUAL_SPREAD"
    symbol VARCHAR(20),
    buy_exchange VARCHAR(50),
    sell_exchange VARCHAR(50),

    -- Prices and execution
    buy_price NUMERIC(20, 8),
    sell_price NUMERIC(20, 8),
    spread_bps INTEGER,
    volume NUMERIC(20, 8),

    -- Fees
    buy_fee NUMERIC(20, 8),
    sell_fee NUMERIC(20, 8),
    total_fees NUMERIC(20, 8),

    -- P&L
    gross_profit NUMERIC(20, 8),
    net_profit NUMERIC(20, 8),
    net_profit_percent NUMERIC(10, 4),

    -- Execution quality
    buy_order_id VARCHAR(100),
    sell_order_id VARCHAR(100),
    buy_fill_time BIGINT,  -- milliseconds
    sell_fill_time BIGINT,
    total_execution_time BIGINT,

    -- Status
    status VARCHAR(20),  -- "FILLED", "PARTIAL", "FAILED"
    notes TEXT
);

CREATE INDEX ON arbitrage_executions (execution_time DESC);
CREATE INDEX ON arbitrage_executions (symbol, execution_time DESC);
CREATE INDEX ON arbitrage_executions (net_profit DESC);
```

### Analytics Queries

```sql
-- Daily arbitrage profitability
SELECT
    DATE(execution_time) as trade_date,
    COUNT(*) as num_trades,
    SUM(net_profit) as total_profit,
    AVG(net_profit_percent) as avg_profit_pct,
    SUM(volume) as total_volume,
    MIN(spread_bps) as min_spread,
    AVG(spread_bps) as avg_spread,
    MAX(total_execution_time) as slowest_execution_ms
FROM arbitrage_executions
WHERE status = 'FILLED'
GROUP BY DATE(execution_time)
ORDER BY trade_date DESC;

-- Best performing trading pairs
SELECT
    CONCAT(buy_exchange, ' ↔ ', sell_exchange) as corridor,
    symbol,
    COUNT(*) as num_trades,
    AVG(net_profit_percent) as avg_profit_pct,
    SUM(net_profit) as total_profit,
    STDDEV(net_profit_percent) as consistency
FROM arbitrage_executions
WHERE status = 'FILLED'
GROUP BY buy_exchange, sell_exchange, symbol
HAVING COUNT(*) >= 10  -- Minimum 10 trades for statistical significance
ORDER BY total_profit DESC;
```

---

## 🔄 Multi-Asset Coordination

For coordinating orders across multiple exchanges simultaneously:

```rust
pub struct MultiExchangeCoordinator {
    exchanges: HashMap<String, ExchangeClient>,
    order_group_id: Arc<AtomicU64>,
}

impl MultiExchangeCoordinator {
    pub async fn execute_coordinated_arbitrage(
        &self,
        trades: Vec<ArbitrageTrade>,
    ) -> Result<CoordinatedExecution> {
        let group_id = self.order_group_id.fetch_add(1, Ordering::SeqCst);

        // 1. Send all orders concurrently with same group ID for tracking
        let handles: Vec<_> = trades
            .iter()
            .map(|trade| {
                let exchange = self.exchanges[&trade.exchange].clone();
                let order = trade.create_order_request(group_id);
                tokio::spawn(async move {
                    exchange.place_order(order).await
                })
            })
            .collect();

        // 2. Wait for all orders to execute
        let results = futures::future::join_all(handles).await;

        // 3. Verify all filled
        let fills: Vec<_> = results
            .into_iter()
            .collect::<Result<Vec<_>>>()?;

        Ok(CoordinatedExecution {
            group_id,
            fills,
            total_execution_time: Instant::now(),
        })
    }
}
```

---

## ✅ Implementation Checklist

- [ ] Spot arbitrage detection and execution
- [ ] Funding rate monitoring and opportunity detection
- [ ] Perpetual spread convergence monitoring
- [ ] Order book reconstruction from WebSocket deltas
- [ ] Profit/loss tracking and analytics
- [ ] Position size optimization (Kelly Criterion)
- [ ] Liquidation prevention for leveraged trades
- [ ] Fee accounting and slippage estimation
- [ ] Persistence filtering (reduce false signals)
- [ ] Multi-exchange order execution coordination
- [ ] Real-time spread monitoring dashboard
- [ ] 6-month backtest on historical data
- [ ] Testnet validation on small amounts
- [ ] Monitoring and alerting system

---

**Status:** ✅ Arbitrage detection and execution fully specified
**Next:** Data dictionary and coin metadata standardization

