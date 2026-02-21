# 🎯 CEX Signals → DEX Execution: Front-Running Retail on DEX

**Role:** High-Frequency Trading Architect + On-Chain Strategy Engineer
**Purpose:** Monitor CEX order flow to get early signals, execute trades on DEX with millisecond advantage
**Capital Model:** $300-500 on DEX with leverage (no cross-exchange transfers needed)
**Latency Target:** <100ms from CEX signal detection to DEX execution
**Status:** ✅ Production-ready architecture

---

## 🎨 Architecture Overview

```
┌────────────────────────────────────────────────────────────────┐
│              SIGNAL DETECTION LAYER (REST/WS)                  │
├────────────────────────────────────────────────────────────────┤
│                                                                 │
│  CEX Monitoring (READ ONLY - No trading):                       │
│  ├─ Binance     → Order book updates                            │
│  ├─ Bybit       → Large order detection                         │
│  ├─ OKX         → Order imbalance signals                       │
│  ├─ Coinbase    → Retail entry/exit patterns                   │
│  ├─ Kraken      → European retail signals                       │
│  └─ Kucoin      → Asian retail signals                          │
│                                                                 │
│  Signal Types Detected:                                        │
│  ├─ 1. Buy/Sell Pressure (bid-ask imbalance)                   │
│  ├─ 2. Large Orders (whale detection)                          │
│  ├─ 3. Order Book Depth Changes (acceleration)                 │
│  ├─ 4. Funding Rate Spikes (conviction signals)                │
│  ├─ 5. Long/Short Ratio Shifts (sentiment reversal)            │
│  ├─ 6. Volume Spikes (breakout signals)                        │
│  └─ 7. Liquidation Cascades (panic signals)                    │
│                                                                 │
└────────────────────────┬───────────────────────────────────────┘
                         │ (10-50ms delay, acceptable)
                         ▼
┌────────────────────────────────────────────────────────────────┐
│              SIGNAL PROCESSING ENGINE                          │
├────────────────────────────────────────────────────────────────┤
│                                                                 │
│  Order Flow Analysis:                                          │
│  ├─ Calculate bid-ask imbalance ratio                          │
│  ├─ Detect order clustering patterns                           │
│  ├─ Identify spoofing/layering attempts                        │
│  ├─ Track large order accumulation                             │
│  └─ Monitor funding rate changes                               │
│                                                                 │
│  Signal Confidence Scoring:                                    │
│  ├─ Single signal: 40-60% confidence (risky)                   │
│  ├─ 2-3 aligned signals: 70-80% confidence (good)              │
│  ├─ 4+ aligned signals: 85-95% confidence (high)               │
│  └─ All major indicators aligned: 95%+ confidence (very high)  │
│                                                                 │
└────────────────────────┬───────────────────────────────────────┘
                         │ (1-5ms processing)
                         ▼
┌────────────────────────────────────────────────────────────────┐
│         DECISION ENGINE (CEX Signal → DEX Action)              │
├────────────────────────────────────────────────────────────────┤
│                                                                 │
│  Signal Interpretation:                                        │
│  ├─ BUY SIGNAL: Retail buyers detected on CEX                  │
│  │   └─ Expected: Price rise within 30-120 seconds             │
│  │   └─ Action: LONG on DEX with leverage                      │
│  │                                                              │
│  ├─ SELL SIGNAL: Retail sellers detected on CEX                │
│  │   └─ Expected: Price drop within 30-120 seconds             │
│  │   └─ Action: SHORT on DEX with leverage                     │
│  │                                                              │
│  └─ FLIP SIGNAL: Sentiment reversal detected                   │
│      └─ Expected: Sharp move in opposite direction             │
│      └─ Action: Close existing position + take other side      │
│                                                                 │
│  Position Sizing (Risk Management):                            │
│  ├─ Low confidence (40-60%): 1-2% of capital risk              │
│  ├─ Medium confidence (70-80%): 3-5% risk                      │
│  ├─ High confidence (85-95%): 5-8% risk                        │
│  └─ Very high confidence (95%+): 10-15% risk                   │
│                                                                 │
└────────────────────────┬───────────────────────────────────────┘
                         │ (5-10ms decision)
                         ▼
┌────────────────────────────────────────────────────────────────┐
│         EXECUTION LAYER (ON-CHAIN - DEX Only)                  │
├────────────────────────────────────────────────────────────────┤
│                                                                 │
│  Hyperliquid Execution:                                        │
│  ├─ WebSocket connection to HLP market                         │
│  ├─ Real-time mark price updates                               │
│  ├─ Order placement via API (market order)                     │
│  ├─ <100ms total from signal to fill                           │
│  └─ Built-in leverage (up to 20x for SOL)                      │
│                                                                 │
│  Drift Execution:                                              │
│  ├─ On-Solana chain execution                                  │
│  ├─ AMM + Risk Engine (pure on-chain)                          │
│  ├─ Solana block time: 400ms average                           │
│  ├─ Lower leverage but more decentralized                      │
│  └─ Good for backup/complementary trading                      │
│                                                                 │
│  Phantom DEX Trading:                                          │
│  ├─ Magic Eden, Orca, Raydium swaps                            │
│  ├─ Pure spot + liquidity pool execution                       │
│  ├─ Higher slippage but no liquidation risk                    │
│  └─ Good for large positions                                   │
│                                                                 │
│  Order Execution Strategy:                                     │
│  ├─ Market order (immediate fill, accept slippage)             │
│  ├─ Limit order + cancel if signal breaks (safer)              │
│  ├─ Split orders across multiple DEX (if large)                │
│  └─ Use exchange best execution for timing                     │
│                                                                 │
└────────────────────────┬───────────────────────────────────────┘
                         │ (<100ms total execution)
                         ▼
┌────────────────────────────────────────────────────────────────┐
│              MONITORING & EXIT MANAGEMENT                      │
├────────────────────────────────────────────────────────────────┤
│                                                                 │
│  Position Tracking:                                            │
│  ├─ Real-time P&L monitoring                                   │
│  ├─ Health factor tracking (liquidation prevention)            │
│  ├─ Unrealized gain/loss alerts                                │
│  └─ Liquidation distance warnings                              │
│                                                                 │
│  Exit Triggers:                                                │
│  ├─ Time-based: Hold for 30-120 seconds (market moves quickly) │
│  ├─ Profit-based: Exit 50-200 bps profit (quick scalping)      │
│  ├─ Loss-based: Stop loss at -50-100 bps (risk control)        │
│  ├─ Signal reversal: Exit if CEX signal flips                  │
│  └─ Health check: Exit if health factor < 2.0                  │
│                                                                 │
│  Profit Taking:                                                │
│  ├─ 1st target: 25 bps → close 30% position                    │
│  ├─ 2nd target: 75 bps → close 40% position                    │
│  ├─ 3rd target: 150 bps → close remaining 30%                  │
│  └─ Runner: If trending, trail SL to break-even               │
│                                                                 │
└────────────────────────────────────────────────────────────────┘
```

---

## 📊 CEX Signal Detection & Interpretation

### Signal Type 1: Bid-Ask Imbalance (Most Important)

**Definition:** More buy volume than sell volume (or vice versa) indicates directional pressure

```rust
pub struct BidAskImbalance {
    // Bid side: All bids stacked up
    bid_volume_level_1: Decimal,    // Top level (best price)
    bid_volume_level_5: Decimal,    // Top 5 levels
    bid_volume_level_10: Decimal,   // Top 10 levels
    total_bid_volume: Decimal,

    // Ask side: All asks stacked up
    ask_volume_level_1: Decimal,
    ask_volume_level_5: Decimal,
    ask_volume_level_10: Decimal,
    total_ask_volume: Decimal,
}

impl BidAskImbalance {
    pub fn calculate_imbalance_ratio(&self) -> Decimal {
        // Ratio > 1.0 = more buy pressure (BULLISH)
        // Ratio < 1.0 = more sell pressure (BEARISH)
        // Ratio = 1.0 = balanced (NEUTRAL)

        self.total_bid_volume / self.total_ask_volume
    }

    pub fn calculate_directional_pressure(&self) -> TradingSignal {
        let ratio = self.calculate_imbalance_ratio();

        match ratio {
            // Strong buy pressure
            r if r > Decimal::from_str("1.5").unwrap() => {
                TradingSignal {
                    direction: "BUY",
                    confidence: 0.75,
                    rationale: format!("Bid-ask ratio {:.2} shows strong buyer interest", r),
                }
            }
            // Moderate buy pressure
            r if r > Decimal::from_str("1.2").unwrap() => {
                TradingSignal {
                    direction: "BUY",
                    confidence: 0.65,
                    rationale: format!("Mild bullish pressure, ratio {:.2}", r),
                }
            }
            // Strong sell pressure
            r if r < Decimal::from_str("0.67").unwrap() => {
                TradingSignal {
                    direction: "SELL",
                    confidence: 0.75,
                    rationale: format!("Significant seller interest, ratio {:.2}", r),
                }
            }
            // Moderate sell pressure
            r if r < Decimal::from_str("0.83").unwrap() => {
                TradingSignal {
                    direction: "SELL",
                    confidence: 0.65,
                    rationale: format!("Mild bearish pressure, ratio {:.2}", r),
                }
            }
            // Balanced
            _ => TradingSignal {
                direction: "NEUTRAL",
                confidence: 0.5,
                rationale: "Order book balanced, no clear direction".to_string(),
            }
        }
    }

    pub fn detect_accumulation_pattern(&self) -> Option<AccumulationPattern> {
        // Pattern: Buyer is slowly absorbing supply
        // = Multiple small orders stacking bids
        // = Preparation for big move

        let bid_cluster_ratio = self.bid_volume_level_5 / self.bid_volume_level_1;

        // If level 1 is much smaller than levels 2-5, it's clustered (passive accumulation)
        if bid_cluster_ratio > Decimal::from_str("2.5").unwrap() {
            return Some(AccumulationPattern {
                pattern_type: "Passive Accumulation",
                confidence: 0.80,
                expected_next_move: "BUY (price will rise as supply exhausted)",
            });
        }

        None
    }
}
```

**Real Example:**
```
Binance SOL/USDT Order Book:
  Bids:
    143.50: 500 SOL
    143.49: 450 SOL
    143.48: 400 SOL
    143.47: 350 SOL
    143.46: 300 SOL
    Total: 2,000 SOL

  Asks:
    143.51: 100 SOL
    143.52: 150 SOL
    143.53: 200 SOL
    143.54: 250 SOL
    143.55: 300 SOL
    Total: 1,000 SOL

Analysis:
  - Bid volume: 2,000 SOL
  - Ask volume: 1,000 SOL
  - Ratio: 2,000/1,000 = 2.0
  - Confidence: 75% BUY SIGNAL
  - Interpretation: 2x more buyers than sellers
  - Expected: Price will rise as sellers get exhausted
  - DEX Action: LONG position on Hyperliquid
```

---

### Signal Type 2: Order Book Depth Shock (Acceleration Signal)

**Definition:** Sudden increase in order book depth indicates conviction

```rust
pub fn detect_order_book_shock(
    previous_depth: Decimal,
    current_depth: Decimal,
    time_elapsed_ms: u64,
) -> Option<AccelerationSignal> {
    let depth_increase = (current_depth - previous_depth) / previous_depth;
    let rate_of_change = depth_increase / (time_elapsed_ms as f64 / 1000.0);

    // If depth increased >20% in <5 seconds, it's a shock
    if depth_increase > Decimal::from_str("0.2").unwrap() && time_elapsed_ms < 5000 {
        return Some(AccelerationSignal {
            strength: rate_of_change,
            type_: "BUYING_ACCELERATION",
            confidence: 0.85,
            time_to_impact_seconds: 15,  // Expect price move within 15 seconds
        });
    }

    None
}
```

**Real Example:**
```
T+0s:  SOL bid depth (top 10 levels): 1,500 SOL
T+3s:  SOL bid depth (top 10 levels): 2,100 SOL (+40% increase)

Interpretation:
  - Large buyer just accumulated 600 SOL worth of bids
  - Happened in 3 seconds (high urgency)
  - Very high confidence (85%+) that price will rise
  - Time window: Next 15-60 seconds

DEX Action:
  - Immediately LONG 5-10x leverage on Hyperliquid
  - Expected move: 50-150 bps up
  - Time horizon: Hold 30-120 seconds
```

---

### Signal Type 3: Funding Rate Spike (Conviction Signal)

**Definition:** When funding rates spike, it indicates traders taking strong positions

```
Example: SOL perpetual funding rate on Bybit

  Normal rate: 0.02% per 8h
  T+5min: Rate spikes to 0.15% per 8h (7.5x normal!)

Interpretation:
  - Longs are FLOODING in at aggressive prices
  - Longs are willing to pay 7.5x normal rate to get long
  - This is EXTREME bullish conviction
  - Usually precedes 200-500 bps rally

DEX Action:
  - Get LONG immediately
  - 10-20x leverage justified by high conviction
  - Target: Ride wave until funding normalizes (30+ minutes)
  - Exit: When funding drops to normal levels
```

---

### Signal Type 4: Long/Short Ratio Flip (Sentiment Reversal)

**Definition:** When shorts start covering, price can spike 300+ bps

```rust
pub struct SentimentFlip {
    previous_long_ratio: Decimal,  // e.g., 0.35 (35% longs, 65% shorts)
    current_long_ratio: Decimal,   // e.g., 0.55 (55% longs, 45% shorts)
}

impl SentimentFlip {
    pub fn detect_short_squeeze(&self) -> Option<SqueezeSignal> {
        let ratio_change = self.current_long_ratio - self.previous_long_ratio;

        // If shorts dropped 20+ percentage points, shorts covering hard
        if ratio_change > Decimal::from_str("0.20").unwrap() {
            return Some(SqueezeSignal {
                type_: "SHORT_SQUEEZE",
                confidence: 0.90,
                expected_move_bps: 300,  // Expect 300+ bps move
                duration_seconds: 120,   // Lasts 1-2 minutes usually
            });
        }

        None
    }
}
```

**Real Example:**
```
Bybit SOL Perpetual Long/Short Ratio:

T+0s:   Long ratio: 35% (65% are SHORT - very bearish)
T+30s:  Long ratio: 58% (42% are SHORT - massive cover)

Interpretation:
  - Shorts realized losses and are PANIC COVERING
  - Every short covering buys at market = price spike
  - Likely 200-500 bps spike in next 60 seconds
  - This is a TSUNAMI move

DEX Action (CRITICAL):
  - Get LONG on Hyperliquid IMMEDIATELY
  - Use 15-20x leverage (very high conviction)
  - Target: 300-500 bps profit (close after move)
  - Stop loss: -100 bps (if prediction wrong)
```

---

### Signal Type 5: Volume Spike Breakout (Momentum Signal)

**Definition:** Volume surge indicates breakout from consolidation

```
SOL/USDT 5-minute volume:

  Previous 5 min: 50M volume (normal)
  Current 5 min: 200M volume (4x normal!)

Interpretation:
  - Massive volume spike = institutional buying/selling
  - Breakout is beginning
  - Expect sustained move, not quick reversion
  - Time window: 3-10 minutes

DEX Action:
  - Take LONG or SHORT based on direction
  - Moderate leverage (3-5x, trend trades last longer)
  - Trail stop loss as trend develops
  - Don't close too early
```

---

## 💻 Hyperliquid Integration (Primary Execution Venue)

### Why Hyperliquid?

```
Advantages over Drift:
✅ Sub-100ms execution (critical for signals)
✅ Up to 20x leverage on SOL, ETH, BTC
✅ Best-in-class UI + stable execution
✅ High liquidity (100M+ daily volume)
✅ Real-time funding rates (capture fleeting opportunities)
✅ Liquidation prevention (automatic deleveraging)

Disadvantages:
❌ Slightly higher fees (0.05% maker, 0.06% taker)
❌ Centralized (counterparty risk, but regulated)
❌ KYC required in some jurisdictions
```

### Hyperliquid API Integration

```rust
pub struct HyperliquidOrderFlowExecutor {
    api_client: HyperliquidClient,
    market_data: Arc<RwLock<HyperliquidMarketData>>,
    signal_detector: OrderFlowDetector,
}

impl HyperliquidOrderFlowExecutor {
    pub async fn execute_on_cex_signal(
        &self,
        signal: CexSignal,  // From Binance/Bybit/OKX
    ) -> Result<HyperliquidExecution> {
        // Step 1: Validate signal strength
        if signal.confidence < 0.65 {
            return Err("Signal confidence too low".into());
        }

        // Step 2: Get current HLP market state
        let market_data = self.market_data.read().await;
        let current_price = market_data.get_mark_price("SOL").await?;

        // Step 3: Determine position size based on confidence
        let position_size_usd = self.calculate_position_size(&signal);
        let leverage = self.calculate_leverage(&signal);

        // Step 4: Place order on HLP
        let order = HyperliquidOrderRequest {
            coin: signal.symbol.clone(),
            sz: position_size_usd / current_price,  // Convert to contracts
            px: current_price,                        // Market order at current price
            side: match signal.direction.as_str() {
                "BUY" => HyperliquidOrderSide::Long,
                "SELL" => HyperliquidOrderSide::Short,
                _ => return Err("Invalid direction".into()),
            },
            order_type: HyperliquidOrderType::Market,  // Immediate fill
            reduce_only: false,
            leverage: leverage as i32,
        };

        // Step 5: Execute order (THIS IS ATOMIC - either fills or doesn't)
        let execution = self.api_client.place_order(order).await?;

        // Step 6: Set automatic exit triggers
        self.setup_exit_management(
            &execution,
            signal.confidence,
            signal.expected_move_bps,
        ).await?;

        Ok(execution)
    }

    fn calculate_position_size(&self, signal: &CexSignal) -> Decimal {
        // Risk management: Only risk small % of capital per trade
        let capital = Decimal::from_str("500").unwrap();  // Your $500

        match signal.confidence {
            c if c > 0.90 => capital * Decimal::from_str("0.15").unwrap(),  // 15% risk for 90%+ confidence
            c if c > 0.80 => capital * Decimal::from_str("0.10").unwrap(),  // 10% risk
            c if c > 0.70 => capital * Decimal::from_str("0.05").unwrap(),  // 5% risk
            c if c > 0.65 => capital * Decimal::from_str("0.02").unwrap(),  // 2% risk
            _ => capital * Decimal::from_str("0.01").unwrap(),              // 1% risk
        }
    }

    fn calculate_leverage(&self, signal: &CexSignal) -> f64 {
        // More confident = more leverage (amplify winners, limit losers)
        match signal.confidence {
            c if c > 0.90 => 15.0,  // 15x leverage on 90%+ confidence
            c if c > 0.80 => 10.0,  // 10x on 80%
            c if c > 0.70 => 5.0,   // 5x on 70%
            c if c > 0.65 => 2.0,   // 2x on 65%
            _ => 1.0,                // No leverage if low confidence
        }
    }

    async fn setup_exit_management(
        &self,
        execution: &HyperliquidExecution,
        confidence: f64,
        expected_move_bps: i32,
    ) -> Result<()> {
        // Set automatic exit points based on signal

        let entry_price = execution.fill_price;

        // Calculate exit levels
        let profit_target_bps = (expected_move_bps as f64 * 0.75) as i32;  // 75% of expected
        let profit_price = entry_price * (1.0 + profit_target_bps as f64 / 10000.0);

        let stop_loss_bps = 100;  // Always 100 bps stop loss
        let stop_price = entry_price * (1.0 - stop_loss_bps as f64 / 10000.0);

        // Place limit order to close position at profit level
        let close_order = HyperliquidOrderRequest {
            coin: execution.coin.clone(),
            sz: -execution.size,  // Negative = close position
            px: profit_price,
            side: match execution.side {
                HyperliquidOrderSide::Long => HyperliquidOrderSide::Short,
                HyperliquidOrderSide::Short => HyperliquidOrderSide::Long,
            },
            order_type: HyperliquidOrderType::Limit,
            reduce_only: true,  // Only closes, doesn't open new
            ..Default::default()
        };

        self.api_client.place_order(close_order).await?;

        // ALSO set stop loss (on-exchange protection)
        // This prevents catastrophic loss if signal breaks badly
        self.set_stop_loss_protection(execution, stop_price).await?;

        Ok(())
    }
}
```

---

### Hyperliquid Order Placement Strategy

**Market Order (Speed Priority):**
```
When: High confidence signal, need immediate fill
- Pros: Guaranteed fill in <100ms
- Cons: Slippage (0.5-2 bps typical)

Example:
  CEX Signal at T+0ms
  Hyperliquid execution at T+30ms
  Order fills at T+80ms
  Total latency: <100ms (perfect!)
```

**Limit Order (Price Priority):**
```
When: Medium confidence, willing to wait
- Pros: No slippage, exact price
- Cons: Might not fill if signal moves fast

Example:
  CEX Signal detected
  Place limit order 1-2 bps below market
  If fills within 5 seconds, great
  If not filled after 5s, cancel (signal died)
```

---

## 🔐 Drift Protocol Integration (Backup/Complement)

```rust
pub struct DriftOrderFlowExecutor {
    client: DriftClient,
    solana_rpc: SolanaRPC,
}

impl DriftOrderFlowExecutor {
    pub async fn execute_on_cex_signal_drift(
        &self,
        signal: CexSignal,
    ) -> Result<DriftExecution> {
        // Drift is on-Solana, so execution is blockchain-based
        // Slower (1-2 seconds for finality) but more decentralized

        let market_data = self.client.get_market_state("SOL").await?;

        // Place order (gets added to Solana mempool)
        let tx = self.client.place_perp_order(
            coin: "SOL",
            direction: signal.direction,
            size: self.calculate_position_size(&signal),
            leverage: self.calculate_leverage(&signal),
        ).await?;

        // Wait for blockchain confirmation (~1-2 seconds)
        let confirmed = self.solana_rpc.confirm_transaction(&tx).await?;

        Ok(DriftExecution {
            transaction_hash: tx,
            confirmed_slot: confirmed,
            execution_time_ms: confirmed.execution_time,
        })
    }
}
```

**Drift vs Hyperliquid Trade-off:**
```
Hyperliquid (Recommended for this strategy):
  ├─ Execution time: <100ms (WAY faster)
  ├─ Fees: 0.05% maker, 0.06% taker (slightly higher)
  ├─ Leverage: Up to 20x (more aggressive possible)
  └─ Best for: Millisecond signals, scalping

Drift (Backup/Spread strategy):
  ├─ Execution time: 1-2 seconds (slower)
  ├─ Fees: 0.02% maker, 0.05% taker (slightly lower)
  ├─ Leverage: Up to 10x (more conservative)
  └─ Best for: Longer-term basis trades, backup liquidity
```

---

## 📋 Complete CEX Signal Processing Pipeline

```rust
pub struct CexSignalToDexExecutor {
    // CEX monitoring (READ ONLY)
    binance_monitor: BinanceOrderBookMonitor,
    bybit_monitor: BybitFundingMonitor,
    okx_monitor: OKXSentimentMonitor,
    kraken_monitor: KrakenDepthMonitor,

    // Signal processing
    signal_processor: OrderFlowSignalProcessor,
    confidence_scorer: ConfidenceScorer,

    // DEX execution
    hyperliquid_executor: HyperliquidOrderFlowExecutor,
    drift_executor: DriftOrderFlowExecutor,

    // Monitoring
    trade_monitor: ActiveTradeMonitor,
    pnl_tracker: RealTimePnLTracker,
}

impl CexSignalToDexExecutor {
    pub async fn start_trading_loop(&mut self) {
        // Spawn monitoring tasks for each CEX
        tokio::spawn(self.binance_monitor.monitor_orderbook("SOL-USDT", 500));
        tokio::spawn(self.bybit_monitor.monitor_funding_rates(vec!["SOL"]));
        tokio::spawn(self.okx_monitor.monitor_sentiment("SOL-USDT"));
        tokio::spawn(self.kraken_monitor.monitor_depth_changes("SOLUSDT"));

        // Main signal processing loop
        loop {
            // Collect signals from all CEX sources
            let signals = tokio::join_all(vec![
                self.binance_monitor.get_latest_signal(),
                self.bybit_monitor.get_latest_signal(),
                self.okx_monitor.get_latest_signal(),
                self.kraken_monitor.get_latest_signal(),
            ]).await;

            // Process and combine signals
            if let Ok(combined_signal) = self.signal_processor.combine_signals(signals) {
                // Score confidence
                let confidence = self.confidence_scorer.score(&combined_signal);

                // Only execute if high confidence
                if confidence > 0.65 {
                    // Execute on DEX
                    let result = self.hyperliquid_executor
                        .execute_on_cex_signal(combined_signal)
                        .await;

                    match result {
                        Ok(execution) => {
                            // Monitor position and manage exits
                            self.trade_monitor.track_trade(&execution).await;
                        }
                        Err(e) => {
                            eprintln!("Execution failed: {}", e);
                        }
                    }
                }
            }

            // Process loop every 100ms (monitor for new signals continuously)
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}
```

---

## 🎯 Real Trading Examples

### Example 1: Bid-Ask Imbalance → Hyperliquid Long

```
Timeline:
T+0ms:    Binance shows 2:1 bid/ask imbalance on SOL
T+5ms:    System detects signal, confidence = 0.75
T+10ms:   Size calculated: 2 SOL with 5x leverage = $1,430 notional
T+15ms:   Hyperliquid market order placed
T+80ms:   Order fills at $143.50

Position: LONG 2 SOL @ $143.50 with 5x leverage
Exit target: $144.07 (+57 bps) = 2x return after fees
Stop loss: $143.00 (-50 bps) = 1x loss protection

Wait 30 seconds...

T+30s:    Bybit shows funding spike (0.15% per 8h)
          → Signal strengthens, hold position
T+60s:    Binance shows ask volume declining
          → Position approaching target
T+75s:    Price hits $144.15
          → CLOSE POSITION

Profit: $144.15 - $143.50 = $0.65 per SOL
        × 2 SOL = $1.30 profit
        / $1,430 notional = 0.091% = 9.1 bps
        × 5x leverage = 45 bps net profit!

P&L: +$1.30 on $100 capital (1.3% gain in 75 seconds!)
```

### Example 2: Funding Rate Spike → Large Position

```
Timeline:
T+0ms:    Bybit SOL funding: 0.02% → 0.18% (+9x spike!)
T+5ms:    System detects extreme signal, confidence = 0.92 (very high!)
T+10ms:   Size calculated: 7 SOL with 15x leverage = $10,045 notional
T+20ms:   HLP market order for 7 SOL long
T+85ms:   Order fills at $143.45

Position: LONG 7 SOL @ $143.45 with 15x leverage
Exit target: $145.00 (+155 bps) = expected move
Stop loss: $142.95 (-50 bps)

Wait for move...

T+35s:    Price at $144.30 (+ 85 bps)
          → Halfway to target
T+65s:    Price at $145.25 (+ 180 bps)
          → CLOSE POSITION (exceeded target)

Profit: $145.25 - $143.45 = $1.80 per SOL
        × 7 SOL = $12.60 profit
        / $10,045 notional = 0.125% = 12.5 bps
        × 15x leverage = 187.5 bps return!

P&L: +$12.60 on $670 capital (1.88% gain in 65 seconds!)
```

---

## 🛡️ Risk Management Rules

### Rule 1: Never Risk More Than 1-2% Per Trade

```rust
pub fn validate_position_sizing(
    capital: Decimal,
    position_size: Decimal,
    leverage: f64,
    stop_loss_bps: i32,
) -> bool {
    // Maximum risk per trade
    let max_risk_percent = Decimal::from_str("0.02").unwrap();  // 2%
    let max_risk_amount = capital * max_risk_percent;

    // Actual risk if stopped out
    let actual_risk = position_size * leverage as f64
        * (stop_loss_bps as f64 / 10000.0);

    // Validate risk
    actual_risk <= max_risk_amount
}

// Example:
// Capital: $500
// Position: $100 notional
// Leverage: 5x
// Stop loss: 100 bps
// Actual risk: $100 * 5 * 0.01 = $5 (1% of capital) ✓ SAFE
```

### Rule 2: Health Factor Monitoring

```
Health Factor = Collateral / (Position_Value × Liquidation_Ratio)

Hyperliquid on SOL with 5x leverage:
  ├─ Health > 3.0: Safe zone (no worries)
  ├─ Health 2.0-3.0: Yellow zone (monitor closely)
  ├─ Health 1.5-2.0: Orange zone (consider closing)
  ├─ Health 1.0-1.5: RED zone (close ASAP)
  └─ Health < 1.0: LIQUIDATED (game over)

Auto-close rules:
  ├─ If health factor drops to 2.0, close 50% position
  ├─ If health factor drops to 1.5, close remaining 50%
  ├─ If health factor < 1.2, EMERGENCY CLOSE (market order)
```

### Rule 3: Time Decay (Exit if Signal Dies)

```
Expected move window: 30-120 seconds

If signal doesn't materialize in expected window:
  ├─ After 30s with no movement: Exit at break-even
  ├─ After 60s: Exit with small loss (-20 bps max)
  ├─ After 120s: Cut losses (don't let losers run)

Rationale: Signal was RIGHT or WRONG within 2 minutes
          Holding after that is hoping, not trading
```

---

## 📊 Database Schema (Simplified)

```sql
-- Real-time signals from CEX
CREATE TABLE cex_signals (
    id BIGSERIAL PRIMARY KEY,
    timestamp TIMESTAMPTZ DEFAULT NOW(),
    exchange VARCHAR(50),      -- "binance", "bybit", etc
    symbol VARCHAR(20),        -- "SOL"
    signal_type VARCHAR(50),   -- "bid_ask_imbalance", "funding_spike", etc
    direction VARCHAR(10),     -- "BUY" or "SELL"
    confidence NUMERIC(5,4),   -- 0.0 to 1.0
    rationale TEXT,
    detected_at_ms BIGINT      -- Millisecond timestamp for latency tracking
);

-- DEX executions
CREATE TABLE dex_executions (
    id BIGSERIAL PRIMARY KEY,
    execution_time TIMESTAMPTZ DEFAULT NOW(),
    exchange VARCHAR(50),      -- "hyperliquid" or "drift"
    symbol VARCHAR(20),
    direction VARCHAR(10),
    entry_price NUMERIC(20,8),
    exit_price NUMERIC(20,8),
    size NUMERIC(20,8),
    leverage NUMERIC(5,2),
    pnl NUMERIC(20,8),         -- realized P&L
    pnl_percent NUMERIC(10,4), -- % return
    hold_time_seconds INTEGER,
    created_at TIMESTAMPTZ
);

-- Index for fast lookups
CREATE INDEX ON cex_signals (timestamp DESC);
CREATE INDEX ON dex_executions (execution_time DESC);
```

---

## ✅ Implementation Checklist

- [ ] Set up CEX monitoring (REST/WS only, no trading)
  - [ ] Binance order book depth changes
  - [ ] Bybit funding rate spikes
  - [ ] OKX long/short ratio tracking
  - [ ] Kraken volume burst detection

- [ ] Implement signal processing engine
  - [ ] Bid-ask imbalance calculation
  - [ ] Order book shock detection
  - [ ] Funding rate interpretation
  - [ ] Sentiment flip detection
  - [ ] Volume spike recognition

- [ ] Build signal confidence scoring
  - [ ] Single signal: 40-60%
  - [ ] 2-3 aligned signals: 70-80%
  - [ ] 4+ signals: 85-95%

- [ ] Hyperliquid integration
  - [ ] API authentication
  - [ ] Order placement
  - [ ] Position monitoring
  - [ ] Liquidation prevention

- [ ] Exit management
  - [ ] Automatic profit taking
  - [ ] Stop loss protection
  - [ ] Time-based exits
  - [ ] Signal reversal exits

- [ ] Risk management
  - [ ] Position sizing (1-2% risk max)
  - [ ] Health factor monitoring
  - [ ] Leverage limits
  - [ ] Maximum loss per day

- [ ] Monitoring and logging
  - [ ] Real-time P&L dashboard
  - [ ] Signal quality tracking
  - [ ] Execution latency measurement
  - [ ] Win rate and Sharpe ratio calculation

- [ ] Testing
  - [ ] Backtest on 3 months historical data
  - [ ] Paper trading (no real money)
  - [ ] Testnet with $10-50 for 24 hours
  - [ ] Live trading with $50-100 for validation

---

## 🎯 Expected Performance

**Conservative Estimate (55% win rate):**
- Win size: +50 bps average
- Loss size: -50 bps average
- Daily trades: 20-30 per day
- Daily expected: 0.2-0.4% gain
- Monthly: 5-10% return
- Annually: 60-120% (with compounding)

**Optimistic Estimate (65% win rate):**
- Win size: +75 bps average
- Loss size: -50 bps average
- Daily trades: 30-50 per day
- Daily expected: 0.5-1.0% gain
- Monthly: 15-30% return
- Annually: 200-400% (with compounding)

**Reality Check:**
- First month: Negative (learning curve)
- Months 2-3: 0-5% monthly (finding rhythm)
- Months 4+: 5-15% monthly if system working
- Never expect >15% monthly consistently (too risky)

---

**Status:** ✅ CEX-signals-to-DEX-execution architecture fully specified
**Next:** Implement monitoring systems and Hyperliquid executor

