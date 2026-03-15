# Week 1 Architecture: Core Trading System

## System Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                     TRADINGBOTS-HEDGEBOT MVP                       │
│              High-Speed DEX Trading System (Solana)             │
└─────────────────────────────────────────────────────────────────┘

DATA FLOW ARCHITECTURE:
═══════════════════════════════════════════════════════════════

[CEX Data Feeds]                  [Blockchain]
  ├─ Binance REST                  ├─ Drift (monitoring)
  ├─ Bybit WebSocket               └─ Hyperliquid (monitoring)
  └─ OKX WebSocket                        │
        │                                 │
        └────────────────┬────────────────┘
                         │
                    ┌────▼─────┐
                    │ Data      │
                    │ Pipeline  │ (Normalization, deduplication, caching)
                    └────┬─────┘
                         │
         ┌───────────────┼───────────────┐
         │               │               │
    ┌────▼────┐  ┌──────▼──────┐  ┌─────▼──────┐
    │Technical │  │   CEX Flow  │  │On-Chain    │
    │Signals   │  │ Detection   │  │Intelligence│
    └────┬────┘  └──────┬──────┘  └─────┬──────┘
         │               │               │
         └───────────────┼───────────────┘
                         │
                    ┌────▼──────────┐
                    │Confidence     │
                    │Aggregation    │
                    │Engine         │
                    └────┬──────────┘
                         │
                    ┌────▼──────────┐
                    │Risk Manager   │
                    │& Validator    │
                    └────┬──────────┘
                         │
                    ┌────▼──────────┐
                    │Decision Logic │
                    │& Sizing       │
                    └────┬──────────┘
                         │
                    ┌────▼──────────┐
                    │Execution      │
                    │(Hyperliquid)  │
                    └────┬──────────┘
                         │
         ┌───────────────┼───────────────┐
         │               │               │
    ┌────▼────┐  ┌──────▼──────┐  ┌─────▼──────┐
    │Monitoring│  │ Dashboard   │  │Supabase    │
    │& Alerts  │  │ Real-time   │  │ Logging    │
    └──────────┘  └─────────────┘  └────────────┘
```

---

## Week 1: Core System Components (Day-by-Day Breakdown)

### Day 1: Data Pipeline & CEX Monitoring

**Deliverables:**
- CEX data collector (Binance REST)
- Price cache manager
- Data normalization

**Key Files:**
```
src/
├── data/
│   ├── mod.rs                    # Data module entry
│   ├── cex_client.rs            # Binance/Bybit/OKX clients
│   ├── price_cache.rs           # Real-time price storage
│   ├── normalization.rs         # Symbol standardization
│   └── models.rs                # Data structures
├── config.rs                     # API keys, settings
└── main.rs                       # Entry point
```

**Core Data Structures:**
```rust
// Price data
pub struct PriceData {
    pub timestamp: i64,
    pub symbol: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

// CEX Order Book
pub struct OrderBook {
    pub symbol: String,
    pub timestamp: i64,
    pub bids: Vec<(f64, f64)>,     // (price, amount)
    pub asks: Vec<(f64, f64)>,
}

// Market State
pub struct MarketState {
    pub symbol: String,
    pub current_price: f64,
    pub bid: f64,
    pub ask: f64,
    pub volume_24h: f64,
    pub last_update: i64,
}
```

**Day 1 Goals:**
- [ ] Binance REST client (fetch prices every 500ms)
- [ ] Price normalization (handle symbol variations: SOL vs SOLANA)
- [ ] Price cache (efficient in-memory storage)
- [ ] Error handling (retry logic, timeout management)

**Expected Output:**
```
[14:32:15] CEX Data Pipeline Started
[14:32:16] ✓ Connected to Binance API
[14:32:17] SOL/USDT: $140.25 (bid: $140.24, ask: $140.26)
[14:32:17] DOGE/USDT: $0.3845 (bid: $0.3844, ask: $0.3846)
[14:32:22] SOL/USDT: $140.32 (update)
```

---

### Day 2: Technical Indicators Engine

**Deliverables:**
- RSI, Bollinger Bands, MACD, ATR, Stochastic calculations
- Indicator caching (rolling window buffers)
- Support/Resistance detection

**Key Files:**
```
src/
├── indicators/
│   ├── mod.rs                   # Indicator module
│   ├── rsi.rs                   # RSI (oversold/overbought)
│   ├── bollinger.rs             # Bollinger Bands
│   ├── macd.rs                  # MACD momentum
│   ├── atr.rs                   # ATR volatility
│   ├── stochastic.rs            # Stochastic %K %D
│   ├── support_resistance.rs    # Level detection
│   └── ichimoku.rs              # Leading indicator
└── cache/
    └── indicator_cache.rs       # Rolling window buffers
```

**Core Indicators:**
```rust
// RSI Calculation
pub fn calculate_rsi(closes: &[f64], period: usize) -> f64 {
    let gains: f64 = closes.windows(2)
        .filter(|w| w[1] > w[0])
        .map(|w| w[1] - w[0])
        .sum::<f64>() / period as f64;

    let losses: f64 = closes.windows(2)
        .filter(|w| w[1] < w[0])
        .map(|w| w[0] - w[1])
        .sum::<f64>() / period as f64;

    100.0 - (100.0 / (1.0 + (gains / losses)))
}

// Support/Resistance Detection
pub struct SupportResistance {
    pub support_level: f64,
    pub resistance_level: f64,
    pub support_touches: u32,      // How many times bounced
    pub resistance_touches: u32,
}

impl SupportResistance {
    pub fn detect(prices: &[f64]) -> Self {
        // Find local minima (support) and maxima (resistance)
        // Track how many times price bounced off each
    }
}
```

**Day 2 Goals:**
- [ ] RSI calculation (14-period standard)
- [ ] Bollinger Bands (20-period, 2 std dev)
- [ ] MACD (12/26/9 exponential)
- [ ] ATR (14-period volatility)
- [ ] Support/Resistance (detect from 100 candles)
- [ ] Test on historical SOL data (accuracy validation)

**Expected Output:**
```
[14:35:20] Technical Indicators Loaded
[14:35:20] SOL Indicators:
  ├─ RSI(14): 32.4 (OVERSOLD) ✓
  ├─ Bollinger: close=$140.25, lower=$135.80, upper=$144.70
  ├─ MACD: -0.82 (signal=-0.65, histogram=-0.17)
  ├─ ATR(14): 4.2 (normal)
  ├─ Stochastic: K=28, D=31 (oversold)
  └─ Support: $140 (4 bounces), Resistance: $145 (3 touches)
```

---

### Day 3: CEX Order Flow Detection

**Deliverables:**
- Bid/Ask imbalance calculation
- Order book monitoring
- Imbalance signal generation

**Key Files:**
```
src/
├── signals/
│   ├── mod.rs                   # Signal module
│   ├── order_flow.rs            # CEX imbalance detection
│   ├── confluence.rs            # Multi-signal scoring
│   └── models.rs                # Signal data structures
└── events/
    └── event_bus.rs             # Publish/subscribe signals
```

**Order Flow Detection:**
```rust
pub struct OrderFlowSignal {
    pub symbol: String,
    pub timestamp: i64,
    pub bid_volume: f64,           // Total bid quantity at depth
    pub ask_volume: f64,           // Total ask quantity at depth
    pub imbalance_ratio: f64,      // bid_volume / ask_volume
    pub imbalance_direction: Direction,  // LONG/SHORT
    pub confidence: f64,           // 0.0-1.0
}

impl OrderFlowSignal {
    pub fn calculate(orderbook: &OrderBook) -> Self {
        // Sum cumulative bid/ask at ±1%, ±2%, ±5% levels
        let bid_volume = orderbook.bids.iter().map(|(_, amt)| amt).sum();
        let ask_volume = orderbook.asks.iter().map(|(_, amt)| amt).sum();
        let imbalance_ratio = bid_volume / ask_volume;

        // Imbalance > 2.0 = strong buy signal
        let direction = if imbalance_ratio > 1.5 {
            Direction::LONG
        } else if imbalance_ratio < 0.67 {
            Direction::SHORT
        } else {
            Direction::NEUTRAL
        };

        let confidence = if imbalance_ratio > 3.0 {
            0.95  // Extreme imbalance
        } else if imbalance_ratio > 2.0 {
            0.85  // Strong imbalance
        } else if imbalance_ratio > 1.5 {
            0.70  // Moderate imbalance
        } else {
            0.50  // Weak signal
        };

        OrderFlowSignal {
            symbol: orderbook.symbol.clone(),
            timestamp: orderbook.timestamp,
            bid_volume,
            ask_volume,
            imbalance_ratio,
            imbalance_direction: direction,
            confidence,
        }
    }
}
```

**Day 3 Goals:**
- [ ] Parse Binance order book data
- [ ] Calculate bid/ask imbalance ratio
- [ ] Detect imbalance thresholds (1.5x, 2.0x, 3.0x)
- [ ] Generate confidence scores
- [ ] Test on live Binance data (paper trading)

**Expected Output:**
```
[14:37:45] CEX Order Flow Detected
SOL/USDT Order Book:
  ├─ Bid Volume (depth): $2.4M
  ├─ Ask Volume (depth): $800K
  ├─ Imbalance Ratio: 3.0x
  ├─ Signal: LONG BUY
  └─ Confidence: 0.95 (EXTREME IMBALANCE)
```

---

### Day 4: Trend Detection & Risk Management

**Deliverables:**
- Trend filter (ADX calculation)
- Support/Resistance-based stop loss
- Position sizing logic
- Risk validation gates

**Key Files:**
```
src/
├── risk/
│   ├── mod.rs                   # Risk module
│   ├── position_sizer.rs        # Calculate position size & leverage
│   ├── stop_loss.rs             # Stop loss calculation
│   ├── health_factor.rs         # Liquidation protection
│   └── circuit_breaker.rs       # Daily loss limits
├── trend/
│   ├── mod.rs
│   └── adx.rs                   # ADX trend strength
└── decision/
    └── engine.rs                # Final GO/NO-GO logic
```

**Risk Management:**
```rust
pub struct RiskParameters {
    pub max_position_pct: f64,      // Max 15% per trade
    pub max_leverage: f64,          // Max 15x
    pub min_health_factor: f64,     // >2.0 (prevent liquidation)
    pub daily_loss_limit: f64,      // Stop if lose $50
    pub max_concurrent_trades: usize, // Max 3 open
}

pub struct PositionSizing {
    pub confidence: f64,            // 0.65-0.98
    pub volatility: f64,            // ATR-based
    pub position_size_pct: f64,    // Calculated position
    pub leverage_factor: f64,       // 1x-15x
    pub expected_move_bps: i32,    // basis points
}

impl PositionSizing {
    pub fn calculate(
        confidence: f64,
        volatility: f64,
        capital: f64,
    ) -> Self {
        // Position size scales with confidence
        let position_size_pct = match confidence {
            c if c > 0.90 => 0.15,  // 15% for extreme confluence
            c if c > 0.80 => 0.12,
            c if c > 0.70 => 0.08,
            c if c > 0.65 => 0.05,
            _ => 0.00,  // SKIP if <0.65
        };

        // Leverage scales inversely with volatility
        let leverage_factor = if volatility < 2.0 {
            15.0  // Low volatility, use max leverage
        } else if volatility < 4.0 {
            10.0
        } else {
            5.0   // High volatility, reduce leverage
        };

        PositionSizing {
            confidence,
            volatility,
            position_size_pct,
            leverage_factor,
            expected_move_bps: ((confidence * 100.0) as i32),
        }
    }
}

pub struct StopLoss {
    pub entry_price: f64,
    pub support_level: f64,
    pub atr: f64,
    pub calculated_sl: f64,
    pub health_factor_sl: f64,  // Prevent liquidation
}

impl StopLoss {
    pub fn calculate(
        entry: f64,
        support: f64,
        atr: f64,
        leverage: f64,
    ) -> Self {
        // Use support or 1.5x ATR below entry, whichever is tighter
        let atr_based_sl = entry - (atr * 1.5);
        let calculated_sl = f64::max(support, atr_based_sl);

        // Prevent liquidation: don't allow losses >50% of capital
        let health_factor_sl = entry * (1.0 - (0.5 / leverage));

        StopLoss {
            entry_price: entry,
            support_level: support,
            atr,
            calculated_sl,
            health_factor_sl: f64::max(calculated_sl, health_factor_sl),
        }
    }
}
```

**Day 4 Goals:**
- [ ] ADX calculation (trend strength 0-100)
- [ ] Regime detection (trending vs ranging)
- [ ] Position size calculation (confidence → size)
- [ ] Leverage determination (volatility → leverage)
- [ ] Stop loss placement (support vs ATR)
- [ ] Health factor validation (prevent liquidations)
- [ ] Circuit breaker (daily loss limit $50)

**Expected Output:**
```
[14:39:10] Risk Management Initialized
SOL Trade Setup:
  ├─ Confidence: 0.88 (HIGH)
  ├─ Position Size: 12% of capital ($36 on $300)
  ├─ Leverage: 10x (volatility-adjusted)
  ├─ Entry: $140.25
  ├─ Stop Loss: $135.50 (support level)
  ├─ Health Factor: 3.2 (SAFE, >2.0)
  ├─ Daily Loss Limit: $50 remaining
  └─ GO DECISION: ✓ EXECUTE TRADE
```

---

### Day 5: Hyperliquid Executor & Integration

**Deliverables:**
- Hyperliquid API client (market orders, position management)
- Execution engine
- Trade logging & monitoring

**Key Files:**
```
src/
├── exchange/
│   ├── mod.rs
│   ├── hyperliquid.rs           # Hyperliquid client
│   ├── models.rs                # Order structures
│   └── execution.rs             # Order execution logic
├── monitoring/
│   ├── mod.rs
│   ├── trade_logger.rs          # Log all trades
│   ├── pnl_tracker.rs           # P&L calculation
│   └── alerts.rs                # Real-time alerts
└── main.rs                       # Orchestration
```

**Hyperliquid Executor:**
```rust
pub struct HyperliquidClient {
    pub api_key: String,
    pub secret: String,
    pub base_url: String,
    pub testnet: bool,
}

pub struct PlaceOrderRequest {
    pub symbol: String,
    pub is_buy: bool,
    pub size: f64,
    pub limit_px: f64,
    pub order_type: OrderType,  // MARKET, LIMIT
    pub reduce_only: bool,
    pub post_only: bool,
}

impl HyperliquidClient {
    pub async fn place_order(
        &self,
        request: PlaceOrderRequest,
    ) -> Result<OrderResponse> {
        // Send POST to /order
        // Sign with API key
        // Return order ID + fill info
    }

    pub async fn get_position(
        &self,
        symbol: &str,
    ) -> Result<Position> {
        // Get current position info
        // Return: size, entry price, P&L, health factor
    }

    pub async fn close_position(
        &self,
        symbol: &str,
    ) -> Result<OrderResponse> {
        // Market order to close current position
    }

    pub async fn get_health_factor(
        &self,
    ) -> Result<f64> {
        // Prevent liquidation
        // health_factor = (account_value + unrealized_pnl) / total_notional
    }
}

pub struct TradeLog {
    pub trade_id: String,
    pub timestamp: i64,
    pub strategy: String,          // Which strategy triggered
    pub signal_count: u32,         // How many signals aligned
    pub confidence: f64,
    pub direction: Direction,      // LONG/SHORT
    pub entry_price: f64,
    pub position_size: f64,
    pub leverage: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub exit_price: Option<f64>,
    pub pnl: Option<f64>,
    pub pnl_bps: Option<i32>,      // basis points
    pub duration_seconds: Option<i64>,
}
```

**Day 5 Goals:**
- [ ] Hyperliquid API authentication
- [ ] Place market orders (BUY/SHORT)
- [ ] Get position info (size, entry, P&L)
- [ ] Monitor health factor
- [ ] Close positions on SL/TP
- [ ] Log all trades to Supabase
- [ ] Send alerts (entry, exit, errors)

**Expected Output:**
```
[14:42:30] Trade Execution System Ready
[14:42:35] ✓ Connected to Hyperliquid Testnet
[14:42:40] TRADE ENTRY SIGNAL RECEIVED
  ├─ Strategy: Order Flow + Divergence Confluence
  ├─ Signal Count: 7/9
  ├─ Confidence: 0.88
  ├─ Direction: LONG
  ├─ Symbol: SOL/USDT
  ├─ Entry Price: $140.25
  ├─ Position: 0.26 SOL (12% of capital × 10x leverage)
  ├─ Stop Loss: $135.50
  ├─ Take Profit: $147.80
  └─ Order Placed: Order ID #123456
```

---

## Week 1 Integration Test

**After Day 5:**
```bash
# Run complete system
cargo run --release

# What happens:
1. Connect to Binance (CEX monitoring)
2. Calculate technical indicators
3. Detect order flow signals
4. Calculate confidence
5. Validate risk parameters
6. Connect to Hyperliquid Testnet
7. Monitor for signals
8. When confluence detected:
   - Place test order on testnet
   - Log trade information
   - Monitor P&L
   - Prepare for exit

# Test Scenario (Manual):
# Price at $140.25, RSI=32, imbalance=3.0x, divergence found
# Expected: ENTRY signal with 0.88 confidence
# Test Order: 0.26 SOL @ $140.25, SL=$135.50, TP=$147.80
```

---

## Architecture Summary: Week 1

```
Week 1 = FOUNDATION
├─ Data pipeline (CEX feeds)
├─ Technical analysis (9 indicators)
├─ Signal detection (order flow)
├─ Risk management (sizing, stops)
├─ Execution capability (testnet)
└─ Logging & monitoring

Lines of Code: ~2,000-2,500
Development Time: 40 hours
Ready for: Testnet validation (Week 2)
```

---

## File Structure: Week 1 Complete

```
tradingbots-fun/
├── Cargo.toml
├── src/
│   ├── main.rs                  # Entry point
│   ├── config.rs                # Settings
│   ├── data/
│   │   ├── mod.rs
│   │   ├── cex_client.rs
│   │   ├── models.rs
│   │   ├── price_cache.rs
│   │   └── normalization.rs
│   ├── indicators/
│   │   ├── mod.rs
│   │   ├── rsi.rs
│   │   ├── bollinger.rs
│   │   ├── macd.rs
│   │   ├── atr.rs
│   │   ├── stochastic.rs
│   │   ├── support_resistance.rs
│   │   └── ichimoku.rs
│   ├── signals/
│   │   ├── mod.rs
│   │   ├── order_flow.rs
│   │   ├── confluence.rs
│   │   └── models.rs
│   ├── risk/
│   │   ├── mod.rs
│   │   ├── position_sizer.rs
│   │   ├── stop_loss.rs
│   │   ├── health_factor.rs
│   │   └── circuit_breaker.rs
│   ├── trend/
│   │   ├── mod.rs
│   │   └── adx.rs
│   ├── exchange/
│   │   ├── mod.rs
│   │   ├── hyperliquid.rs
│   │   ├── models.rs
│   │   └── execution.rs
│   ├── monitoring/
│   │   ├── mod.rs
│   │   ├── trade_logger.rs
│   │   ├── pnl_tracker.rs
│   │   └── alerts.rs
│   └── decision/
│       ├── mod.rs
│       └── engine.rs
├── tests/
│   ├── indicators_test.rs
│   ├── order_flow_test.rs
│   └── position_sizing_test.rs
└── .env                         # API keys (never commit!)
```

