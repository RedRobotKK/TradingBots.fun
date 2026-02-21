# Week 2: API Integration (GMGN + Supabase)

## Week 2 Overview

```
WEEK 2 = INTELLIGENCE LAYER
├─ GMGN API integration (whale detection)
├─ Supabase PostgreSQL connection (data logging)
├─ Whale movement classification
├─ Confidence adjustment logic
├─ Historical pattern database
└─ Trade metadata enrichment

Expected Completion: 5-6 days
Lines of Code: ~1,500-2,000
Goal: Autonomous whale detection + rich trade logging
```

---

## Day 6-7: GMGN Integration (Whale Movement Detection)

### GMGN Data Structure

**GMGN provides real-time Solana transaction stream:**

```rust
// GMGN Trade Event (from WebSocket)
pub struct GMGNTradeEvent {
    pub timestamp: i64,
    pub tx_signature: String,
    pub token_address: String,
    pub token_symbol: String,
    pub trader_wallet: String,
    pub amount: f64,
    pub price: f64,
    pub side: String,              // "buy" or "sell"
    pub wallet_label: Option<String>, // "known whale", "bot", etc.
}

// Whale Profile
pub struct WhaleProfile {
    pub wallet_address: String,
    pub label: String,              // "Whale X", "Smart Money Bot"
    pub total_trades: u32,
    pub profitable_trades: u32,
    pub sell_accuracy: f64,         // % of sells that preceded dumps
    pub buy_accuracy: f64,          // % of buys that preceded pumps
    pub avg_profit_per_trade: f64,
    pub created_at: i64,
    pub updated_at: i64,
}

// Whale Movement Detection
pub struct WhaleMovement {
    pub whale_address: String,
    pub whale_label: String,
    pub token: String,
    pub timestamp: i64,
    pub action: WhaleAction,        // DEPOSIT_TO_CEX, STAKE, LP
    pub destination: String,
    pub amount: f64,
    pub amount_usd: f64,
    pub whale_sell_accuracy: f64,   // Historical accuracy
    pub whale_hold_accuracy: f64,
}

pub enum WhaleAction {
    DepositToCex { exchange: String },
    StakingLock { duration_days: u32 },
    ProvidingLiquidity { protocol: String },
    AccumulatingAtLevel { price: f64 },
    Unknown,
}
```

### GMGN API Implementation

**File: `src/external_apis/gmgn.rs`**

```rust
use tokio_tungstenite::{connect_async, tungstenite::Message};
use futures::{SinkExt, StreamExt};
use serde_json::json;

pub struct GMGNClient {
    ws_url: String,
    is_connected: bool,
    known_whale_wallets: std::collections::HashMap<String, WhaleProfile>,
}

impl GMGNClient {
    pub fn new() -> Self {
        GMGNClient {
            ws_url: "wss://api.gmgn.ai/ws".to_string(),
            is_connected: false,
            known_whale_wallets: HashMap::new(),
        }
    }

    /// Initialize known whale wallets from database
    pub async fn load_whale_profiles(&mut self, db: &SupabaseClient) -> Result<()> {
        // Load from supabase table: whale_profiles
        let profiles = db.query_whale_profiles().await?;

        for profile in profiles {
            self.known_whale_wallets.insert(
                profile.wallet_address.clone(),
                profile,
            );
        }

        println!("✓ Loaded {} known whale profiles",
            self.known_whale_wallets.len());
        Ok(())
    }

    /// Connect to GMGN WebSocket
    pub async fn connect(&mut self) -> Result<()> {
        let (ws_stream, _) = connect_async(&self.ws_url)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to connect: {}", e))?;

        self.is_connected = true;

        let (mut write, mut read) = ws_stream.split();

        // Subscribe to Solana whale trades
        let subscribe_msg = json!({
            "method": "subscribe",
            "channels": ["trades:solana"],
            "filters": {
                "min_amount_usd": 50000,        // Only large trades
                "include_wallet_labels": true,
            }
        });

        write.send(Message::Text(subscribe_msg.to_string()))
            .await?;

        println!("✓ Subscribed to GMGN Solana trade stream");

        // Keep connection alive and process events
        self.process_events(read).await
    }

    /// Process incoming trade events
    async fn process_events(
        &mut self,
        mut read: futures::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>
            >
        >,
    ) -> Result<()> {
        while let Some(msg) = read.next().await {
            match msg? {
                Message::Text(text) => {
                    if let Ok(event) = serde_json::from_str::<GMGNTradeEvent>(&text) {
                        // Check if this is a known whale
                        if let Some(whale_profile) =
                            self.known_whale_wallets.get(&event.trader_wallet)
                        {
                            self.handle_whale_movement(&event, whale_profile).await?;
                        }
                    }
                },
                Message::Close(_) => {
                    self.is_connected = false;
                    println!("⚠️ GMGN connection closed, attempting reconnect...");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    // Reconnect with exponential backoff
                    return self.connect().await;
                },
                _ => {}
            }
        }
        Ok(())
    }

    /// Classify whale action and signal importance
    async fn handle_whale_movement(
        &self,
        event: &GMGNTradeEvent,
        whale: &WhaleProfile,
    ) -> Result<()> {
        // Determine what the whale is doing
        let action = self.classify_action(&event, whale).await?;

        // Create whale movement signal
        let movement = WhaleMovement {
            whale_address: event.trader_wallet.clone(),
            whale_label: whale.label.clone(),
            token: event.token_symbol.clone(),
            timestamp: event.timestamp,
            action,
            destination: "TBD".to_string(),
            amount: event.amount,
            amount_usd: event.price * event.amount,
            whale_sell_accuracy: whale.sell_accuracy,
            whale_hold_accuracy: whale.buy_accuracy,
        };

        // Emit signal for confidence calculation
        println!(
            "🐋 WHALE MOVEMENT: {} - {} {} (${:.0})",
            whale.label,
            event.side.to_uppercase(),
            event.token_symbol,
            movement.amount_usd
        );

        Ok(())
    }

    /// Classify what whale is doing (buying, selling to CEX, staking, etc.)
    async fn classify_action(
        &self,
        event: &GMGNTradeEvent,
        whale: &WhaleProfile,
    ) -> Result<WhaleAction> {
        match event.side.as_str() {
            "sell" => {
                // Is this whale moving to CEX or just exiting position?
                // Check next transaction from this wallet (API call to GMGN)
                let next_tx = self.get_next_transaction(&event.trader_wallet).await?;

                if self.is_cex_deposit(&next_tx) {
                    Ok(WhaleAction::DepositToCex {
                        exchange: self.identify_cex(&next_tx),
                    })
                } else {
                    Ok(WhaleAction::Unknown)
                }
            },
            "buy" => {
                Ok(WhaleAction::AccumulatingAtLevel {
                    price: event.price,
                })
            },
            _ => Ok(WhaleAction::Unknown),
        }
    }

    async fn get_next_transaction(&self, wallet: &str) -> Result<TransactionInfo> {
        // Call GMGN API to get next transaction from this wallet
        // Determines if they're moving to CEX or staking
        todo!()
    }

    fn is_cex_deposit(&self, tx: &TransactionInfo) -> bool {
        // Check if destination is known CEX wallet
        let cex_wallets = vec![
            "Binance", "Kraken", "Bybit", "OKX", "Kucoin"
        ];
        cex_wallets.iter().any(|c| tx.destination.contains(c))
    }

    fn identify_cex(&self, tx: &TransactionInfo) -> String {
        // Identify which CEX
        if tx.destination.contains("Binance") {
            "Binance".to_string()
        } else if tx.destination.contains("Kraken") {
            "Kraken".to_string()
        } else {
            "Unknown".to_string()
        }
    }
}

// REST API fallback (for querying whale info)
pub async fn fetch_whale_profile(wallet: &str) -> Result<WhaleProfile> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://api.gmgn.ai/wallet/{}?chains=solana&metrics=true",
        wallet
    );

    let response = client.get(&url)
        .send()
        .await?
        .json::<WhaleProfile>()
        .await?;

    Ok(response)
}
```

**GMGN REST API for Whale Data:**

```rust
// Query whale's trade history
pub struct WhalePnL {
    pub wallet: String,
    pub total_trades: i32,
    pub win_rate: f64,
    pub avg_profit: f64,
    pub biggest_win: f64,
    pub biggest_loss: f64,
}

pub async fn get_whale_pnl(wallet: &str) -> Result<WhalePnL> {
    let client = reqwest::Client::new();
    let resp = client.get(&format!(
        "https://api.gmgn.ai/wallet/{}/pnl",
        wallet
    ))
    .send()
    .await?
    .json()
    .await?;

    Ok(resp)
}

// Query specific token holders
pub async fn get_token_top_holders(token: &str) -> Result<Vec<WhaleProfile>> {
    let client = reqwest::Client::new();
    let resp = client.get(&format!(
        "https://api.gmgn.ai/token/{}/holders?limit=100",
        token
    ))
    .send()
    .await?
    .json()
    .await?;

    Ok(resp)
}
```

**Day 6-7 Goals:**
- [ ] Connect to GMGN WebSocket
- [ ] Stream Solana whale trades in real-time
- [ ] Identify known whale wallets
- [ ] Classify whale actions (selling, staking, accumulating)
- [ ] Build whale profile accuracy tracking
- [ ] Log whale movements to event bus

**Expected Output:**
```
[16:22:10] GMGN Client Initialized
[16:22:11] ✓ Loaded 47 known whale profiles
[16:22:12] ✓ Connected to GMGN WebSocket
[16:22:15] 🐋 WHALE DETECTED: Whale X sold $2.4M SOL
[16:22:16] ✓ Whale X historical sell accuracy: 78%
[16:22:16] ⚠️ BEARISH SIGNAL: Whale X liquidation likely within 24h
[16:22:16] Confidence adjustment: -0.22
```

---

## Day 8-9: Supabase Integration (Trade Logging & RAG Database)

### Supabase Schema Design

**Create Supabase PostgreSQL Project:**

```sql
-- Enable TimescaleDB extension for time-series data
CREATE EXTENSION IF NOT EXISTS timescaledb;

-- Main tables:

-- 1. Market Prices (time-series)
CREATE TABLE market_prices (
    time TIMESTAMPTZ NOT NULL,
    symbol VARCHAR(20) NOT NULL,
    source VARCHAR(50),           -- 'binance', 'hyperliquid', etc.
    open NUMERIC,
    high NUMERIC,
    low NUMERIC,
    close NUMERIC,
    volume NUMERIC,
    bid NUMERIC,
    ask NUMERIC,
    imbalance_ratio NUMERIC,
    PRIMARY KEY (time, symbol, source)
);
SELECT create_hypertable('market_prices', 'time', if_not_exists => TRUE);
CREATE INDEX ON market_prices (symbol, time DESC);

-- 2. Technical Indicators
CREATE TABLE technical_indicators (
    time TIMESTAMPTZ NOT NULL,
    symbol VARCHAR(20) NOT NULL,
    rsi_14 NUMERIC,
    bollinger_upper NUMERIC,
    bollinger_lower NUMERIC,
    bollinger_middle NUMERIC,
    macd NUMERIC,
    macd_signal NUMERIC,
    atr_14 NUMERIC,
    stochastic_k NUMERIC,
    stochastic_d NUMERIC,
    ichimoku_tenkan NUMERIC,
    ichimoku_kijun NUMERIC,
    support_level NUMERIC,
    resistance_level NUMERIC,
    adx NUMERIC,
    PRIMARY KEY (time, symbol)
);
SELECT create_hypertable('technical_indicators', 'time', if_not_exists => TRUE);

-- 3. Order Flow Signals
CREATE TABLE order_flow_signals (
    time TIMESTAMPTZ NOT NULL,
    symbol VARCHAR(20) NOT NULL,
    bid_volume NUMERIC,
    ask_volume NUMERIC,
    imbalance_ratio NUMERIC,
    imbalance_direction VARCHAR(10),    -- 'LONG', 'SHORT', 'NEUTRAL'
    confidence NUMERIC,
    PRIMARY KEY (time, symbol)
);
SELECT create_hypertable('order_flow_signals', 'time', if_not_exists => TRUE);

-- 4. Whale Movements
CREATE TABLE whale_movements (
    id BIGSERIAL PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    whale_address VARCHAR(100),
    whale_label VARCHAR(100),
    token VARCHAR(20),
    action VARCHAR(50),             -- 'DEPOSIT_TO_CEX', 'STAKING', 'LP'
    destination VARCHAR(100),
    amount NUMERIC,
    amount_usd NUMERIC,
    whale_sell_accuracy NUMERIC,
    whale_hold_accuracy NUMERIC,
    confidence_adjustment NUMERIC,
    confidence_reasoning TEXT
);
CREATE INDEX ON whale_movements (whale_address, created_at DESC);
CREATE INDEX ON whale_movements (token, created_at DESC);

-- 5. Whale Profiles (cached from GMGN)
CREATE TABLE whale_profiles (
    wallet_address VARCHAR(100) PRIMARY KEY,
    label VARCHAR(100),
    total_trades INTEGER,
    profitable_trades INTEGER,
    sell_accuracy NUMERIC,
    buy_accuracy NUMERIC,
    avg_profit_per_trade NUMERIC,
    created_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ
);

-- 6. Trading Signals
CREATE TABLE trading_signals (
    id BIGSERIAL PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    symbol VARCHAR(20),
    signal_type VARCHAR(50),        -- 'ORDER_FLOW', 'DIVERGENCE', 'MEAN_REVERSION'
    direction VARCHAR(10),          -- 'LONG', 'SHORT'
    confidence NUMERIC,
    signal_count INTEGER,
    rationale TEXT,
    strategies JSONB,               -- Which strategies triggered
    order_flow_data JSONB,
    technical_data JSONB,
    whale_data JSONB,
    sentiment_data JSONB
);
CREATE INDEX ON trading_signals (symbol, created_at DESC);
CREATE INDEX ON trading_signals (created_at DESC);

-- 7. Trades (final execution)
CREATE TABLE trades (
    id BIGSERIAL PRIMARY KEY,
    trade_id VARCHAR(100) UNIQUE,
    created_at TIMESTAMPTZ NOT NULL,
    symbol VARCHAR(20),
    direction VARCHAR(10),          -- 'LONG', 'SHORT'
    entry_price NUMERIC,
    entry_amount NUMERIC,
    position_size_pct NUMERIC,
    leverage_factor NUMERIC,
    stop_loss NUMERIC,
    take_profit NUMERIC,

    -- Strategy metadata
    strategy_name VARCHAR(100),
    signal_count INTEGER,
    confidence NUMERIC,
    rationale TEXT,

    -- Execution
    order_id VARCHAR(100),
    filled_price NUMERIC,
    filled_amount NUMERIC,
    execution_time_ms INTEGER,
    slippage_bps INTEGER,

    -- Result
    exit_price NUMERIC,
    exit_time TIMESTAMPTZ,
    pnl_usd NUMERIC,
    pnl_percent NUMERIC,
    pnl_bps INTEGER,
    duration_seconds INTEGER,
    status VARCHAR(20),             -- 'OPEN', 'CLOSED', 'STOPPED_OUT'

    -- Risk metrics
    max_drawdown_pct NUMERIC,
    health_factor_at_entry NUMERIC,
    health_factor_at_exit NUMERIC,
    liquidation_risk VARCHAR(20)    -- 'LOW', 'MEDIUM', 'HIGH'
);
SELECT create_hypertable('trades', 'created_at', if_not_exists => TRUE);
CREATE INDEX ON trades (symbol, created_at DESC);
CREATE INDEX ON trades (status, created_at DESC);

-- 8. Portfolio State
CREATE TABLE portfolio_state (
    time TIMESTAMPTZ NOT NULL,
    total_equity NUMERIC,
    total_cash NUMERIC,
    total_positions_notional NUMERIC,
    open_trades INTEGER,
    daily_pnl NUMERIC,
    daily_pnl_percent NUMERIC,
    monthly_pnl NUMERIC,
    monthly_pnl_percent NUMERIC,
    max_drawdown NUMERIC,
    health_factor NUMERIC,
    risk_score NUMERIC,
    PRIMARY KEY (time)
);
SELECT create_hypertable('portfolio_state', 'time', if_not_exists => TRUE);

-- 9. System Logs
CREATE TABLE system_logs (
    id BIGSERIAL PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    level VARCHAR(20),              -- 'INFO', 'WARN', 'ERROR'
    component VARCHAR(50),
    message TEXT,
    metadata JSONB
);
CREATE INDEX ON system_logs (created_at DESC, level);
```

### Supabase Rust Client Implementation

**File: `src/external_apis/supabase.rs`**

```rust
use supabase_rs::SupabaseClient;
use serde_json::json;

pub struct SupabaseConnector {
    client: SupabaseClient,
    project_url: String,
    api_key: String,
}

impl SupabaseConnector {
    pub fn new(url: &str, key: &str) -> Self {
        SupabaseConnector {
            client: SupabaseClient::new(url.to_string(), key.to_string()),
            project_url: url.to_string(),
            api_key: key.to_string(),
        }
    }

    // Log market price
    pub async fn log_price(&self, price_data: &PriceData) -> Result<()> {
        self.client
            .from("market_prices")
            .insert(json!({
                "time": price_data.timestamp,
                "symbol": price_data.symbol,
                "source": "binance",
                "open": price_data.open,
                "high": price_data.high,
                "low": price_data.low,
                "close": price_data.close,
                "volume": price_data.volume,
            }))
            .execute()
            .await?;

        Ok(())
    }

    // Log technical indicators
    pub async fn log_indicators(
        &self,
        symbol: &str,
        indicators: &TechnicalIndicators,
    ) -> Result<()> {
        self.client
            .from("technical_indicators")
            .insert(json!({
                "time": Utc::now().to_rfc3339(),
                "symbol": symbol,
                "rsi_14": indicators.rsi,
                "bollinger_upper": indicators.bollinger_upper,
                "bollinger_lower": indicators.bollinger_lower,
                "macd": indicators.macd,
                "macd_signal": indicators.macd_signal,
                "atr_14": indicators.atr,
                "stochastic_k": indicators.stochastic_k,
                "stochastic_d": indicators.stochastic_d,
                "ichimoku_tenkan": indicators.ichimoku_tenkan,
                "ichimoku_kijun": indicators.ichimoku_kijun,
                "support_level": indicators.support_level,
                "resistance_level": indicators.resistance_level,
                "adx": indicators.adx,
            }))
            .execute()
            .await?;

        Ok(())
    }

    // Log order flow signal
    pub async fn log_order_flow(&self, signal: &OrderFlowSignal) -> Result<()> {
        self.client
            .from("order_flow_signals")
            .insert(json!({
                "time": Utc::now().to_rfc3339(),
                "symbol": signal.symbol,
                "bid_volume": signal.bid_volume,
                "ask_volume": signal.ask_volume,
                "imbalance_ratio": signal.imbalance_ratio,
                "imbalance_direction": format!("{:?}", signal.imbalance_direction),
                "confidence": signal.confidence,
            }))
            .execute()
            .await?;

        Ok(())
    }

    // Log whale movement
    pub async fn log_whale_movement(&self, movement: &WhaleMovement) -> Result<()> {
        self.client
            .from("whale_movements")
            .insert(json!({
                "whale_address": movement.whale_address,
                "whale_label": movement.whale_label,
                "token": movement.token,
                "action": format!("{:?}", movement.action),
                "destination": movement.destination,
                "amount": movement.amount,
                "amount_usd": movement.amount_usd,
                "whale_sell_accuracy": movement.whale_sell_accuracy,
                "whale_hold_accuracy": movement.whale_hold_accuracy,
            }))
            .execute()
            .await?;

        Ok(())
    }

    // Log complete trade (with all metadata)
    pub async fn log_trade(&self, trade: &ExecutedTrade) -> Result<()> {
        self.client
            .from("trades")
            .insert(json!({
                "trade_id": trade.id,
                "created_at": trade.timestamp,
                "symbol": trade.symbol,
                "direction": format!("{:?}", trade.direction),
                "entry_price": trade.entry_price,
                "position_size_pct": trade.position_size_pct,
                "leverage_factor": trade.leverage,
                "stop_loss": trade.stop_loss,
                "take_profit": trade.take_profit,
                "strategy_name": trade.strategy,
                "signal_count": trade.signal_count,
                "confidence": trade.confidence,
                "rationale": trade.rationale,
                "order_id": trade.order_id,
                "filled_price": trade.filled_price,
                "health_factor_at_entry": trade.health_factor,
            }))
            .execute()
            .await?;

        Ok(())
    }

    // Update trade on exit
    pub async fn update_trade_exit(
        &self,
        trade_id: &str,
        exit_data: &TradeExit,
    ) -> Result<()> {
        self.client
            .from("trades")
            .update(json!({
                "exit_price": exit_data.exit_price,
                "exit_time": Utc::now().to_rfc3339(),
                "pnl_usd": exit_data.pnl_usd,
                "pnl_percent": exit_data.pnl_percent,
                "pnl_bps": exit_data.pnl_bps,
                "duration_seconds": exit_data.duration_seconds,
                "status": "CLOSED",
            }))
            .eq("trade_id", trade_id)
            .execute()
            .await?;

        Ok(())
    }

    // Query whale profiles (for caching in memory)
    pub async fn query_whale_profiles(&self) -> Result<Vec<WhaleProfile>> {
        let profiles = self.client
            .from("whale_profiles")
            .select("*")
            .execute()
            .await?
            .json::<Vec<WhaleProfile>>()
            .await?;

        Ok(profiles)
    }

    // Get recent trades for analysis
    pub async fn get_recent_trades(&self, limit: i32) -> Result<Vec<Trade>> {
        let trades = self.client
            .from("trades")
            .select("*")
            .order("created_at.desc")
            .limit(limit)
            .execute()
            .await?
            .json::<Vec<Trade>>()
            .await?;

        Ok(trades)
    }

    // Calculate statistics (using Supabase PostgREST)
    pub async fn get_trade_statistics(&self) -> Result<TradeStatistics> {
        // Query: SELECT AVG(pnl_bps), COUNT(*), SUM(pnl_usd) FROM trades
        let stats = self.client
            .rpc(
                "get_trade_stats",
                json!({
                    "last_days": 30
                }),
            )
            .execute()
            .await?
            .json::<TradeStatistics>()
            .await?;

        Ok(stats)
    }
}

pub struct ExecutedTrade {
    pub id: String,
    pub timestamp: i64,
    pub symbol: String,
    pub direction: Direction,
    pub entry_price: f64,
    pub position_size_pct: f64,
    pub leverage: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub strategy: String,
    pub signal_count: u32,
    pub confidence: f64,
    pub rationale: String,
    pub order_id: String,
    pub filled_price: f64,
    pub health_factor: f64,
}

pub struct TradeExit {
    pub exit_price: f64,
    pub pnl_usd: f64,
    pub pnl_percent: f64,
    pub pnl_bps: i32,
    pub duration_seconds: i64,
}
```

**Day 8-9 Goals:**
- [ ] Create Supabase PostgreSQL database (TimescaleDB)
- [ ] Create all 9 tables with proper schema
- [ ] Implement Supabase Rust client
- [ ] Test logging: prices, indicators, signals, trades
- [ ] Verify data persistence and retrieval
- [ ] Set up backups and recovery

**Expected Output:**
```
[17:45:30] Supabase Database Connected
[17:45:31] ✓ Created 9 tables with TimescaleDB
[17:45:32] ✓ Verified connection pool (10 connections)
[17:46:00] Market prices logged: 120 records/sec
[17:46:05] Technical indicators cached
[17:46:10] Trade #001 logged to database
[17:46:10]   ├─ Entry: $140.25, Position: 0.26 SOL
[17:46:10]   ├─ Strategy: Order Flow Confluence
[17:46:10]   ├─ Confidence: 0.88
[17:46:10]   └─ Recorded for RAG retrieval
```

---

## Day 10: Integration & Testing

### Complete Data Flow Integration

```rust
// File: src/integrations/mod.rs

pub async fn integrate_all_apis() -> Result<()> {
    // Initialize all external APIs
    let mut gmgn_client = GMGNClient::new();
    let supabase = SupabaseConnector::new(
        &env::var("SUPABASE_URL")?,
        &env::var("SUPABASE_KEY")?,
    );

    // Load existing whale profiles
    gmgn_client.load_whale_profiles(&supabase).await?;

    // Start event processing
    tokio::spawn(async move {
        gmgn_client.connect().await.ok();
    });

    // Main loop: Process signals and log everything
    loop {
        // 1. Get latest market data
        let prices = get_latest_prices().await?;
        for price in &prices {
            supabase.log_price(price).await?;
        }

        // 2. Calculate indicators
        let indicators = calculate_indicators(&prices)?;
        for (symbol, ind) in &indicators {
            supabase.log_indicators(symbol, ind).await?;
        }

        // 3. Detect order flow signals
        let signals = detect_order_flow(&get_order_books().await?)?;
        for signal in &signals {
            supabase.log_order_flow(signal).await?;
        }

        // 4. Check for whale movements (from GMGN events)
        if let Some(whale_movement) = get_latest_whale_movement() {
            supabase.log_whale_movement(&whale_movement).await?;
        }

        // 5. Aggregate confidence and make decision
        let decision = aggregate_signals_and_decide(&prices, &indicators, &signals).await?;

        if let Some(trade) = decision.trade {
            // Execute trade and log
            let result = execute_trade(&trade).await?;
            supabase.log_trade(&result).await?;
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
```

**Day 10 Goals:**
- [ ] Connect all APIs (CEX + GMGN + Supabase)
- [ ] Test data flow end-to-end
- [ ] Log 100+ complete data cycles
- [ ] Verify database integrity
- [ ] Test RAG retrieval (query historical data)
- [ ] Ready for Week 3 testing

**Expected Output:**
```
[18:30:00] Full Integration Test Started
[18:30:01] ✓ GMGN connected (47 whale profiles loaded)
[18:30:01] ✓ Supabase ready (test write successful)
[18:30:02] ✓ CEX monitoring (Binance, Bybit, OKX)
[18:30:05] === Test Scenario: SOL Oversold Setup ===
[18:30:05] Market: RSI=32, Imbalance=3.0x, Support=$140
[18:30:06] Signal: 7/9 strategies triggered
[18:30:06] Confidence: 0.88 (HIGH)
[18:30:07] Whale Check: No dumping detected
[18:30:07] DECISION: ENTER LONG
[18:30:08] ✓ Trade logged to Supabase
[18:30:08] ✓ All data persisted
[18:30:10] === Integration Test Complete ===
```

---

## Week 2 Deliverables Summary

```
WEEK 2 COMPLETION:
├─ GMGN WebSocket integration (real-time whale detection)
├─ Whale profile caching (47+ known whales tracked)
├─ Supabase PostgreSQL database (9 production tables)
├─ Complete trade logging system
├─ RAG-ready historical database (for Claude analysis)
├─ Confidence adjustment engine (whale intel)
└─ System monitoring & alerting

Lines of Code: +1,500-2,000 (Total: ~3,500-4,500)
Development Time: 40-50 hours
Total System LOC: ~3,500-4,500
Ready for: Testnet trading (Week 3)
```

