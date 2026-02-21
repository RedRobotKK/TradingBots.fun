# 🔄 Multi-CEX Data Architecture: Sub-Millisecond Trading Infrastructure

**Role:** DevOps/Security/API Integration Expert + Trading Infrastructure Architect
**Purpose:** Comprehensive multi-exchange data collection for arbitrage, funding trades, and microstructure signals
**Latency Target:** Sub-millisecond competitive advantage
**Status:** ✅ Enterprise-grade architecture specifications

---

## 🎯 Executive Summary

This architecture enables:
- **Spot Arbitrage:** Buy low on one exchange, sell high on another (mev-style)
- **Funding Rate Trading:** Capture interest rate differentials between spot and futures
- **Microstructure Signals:** Order flow imbalances, liquidation cascades, whale movements
- **Real-time Data:** Order books, funding rates, open interest, long/short ratios
- **Sub-millisecond Latency:** WebSocket-based feeds for competitive edge

### Supported Exchanges

| Exchange | Type | Free API | WebSocket | Order Book | Funding | Open Int | Long/Short |
|----------|------|----------|-----------|-----------|---------|----------|-----------|
| **Binance** | CEX | ✅ Spot | ✅ Yes | ✅ Yes | ✅ Futures | ✅ Yes | ❌ No |
| **Bybit** | CEX/Dex | ✅ Free | ✅ Yes | ✅ Yes | ✅ Perpetual | ✅ Yes | ✅ Yes |
| **OKX** | CEX | ✅ Free | ✅ Yes | ✅ Yes | ✅ Swap | ✅ Yes | ✅ Yes |
| **Coinbase** | CEX | ⚠️ Signed | ✅ Yes | ✅ Yes | ❌ No | ❌ No | ❌ No |
| **Kraken** | CEX | ✅ Free | ❌ WebSocket | ✅ REST | ✅ Futures | ✅ Yes | ❌ No |
| **Kucoin** | CEX | ✅ Free | ✅ Yes | ✅ Yes | ✅ Perpetual | ✅ Yes | ⚠️ Limited |
| **Bybit** | Derivatives | ✅ Free | ✅ Yes | ✅ Yes | ✅ Yes | ✅ Yes | ✅ Yes |
| **Deribit** | Options | ✅ Free | ✅ Yes | ✅ Yes | ✅ Perpetual | ✅ Yes | ⚠️ Limited |
| **Hyperliquid** | DEX | ✅ Free | ✅ Yes | ✅ Real-time | ✅ Perpetual | ✅ Yes | ✅ Yes |
| **Drift** | DEX | ✅ Free | ✅ WebSocket | ⚠️ Blockchain | ✅ Perpetual | ✅ Yes | ✅ Yes |

---

## 📡 WebSocket Connection Architecture

### Why WebSocket Over REST?

```
REST Polling (Bad for latency):
  GET /orderbook → 50-200ms latency
  GET /funding → 50-200ms latency
  → Need to poll every 1-5 seconds
  → 20-50 API calls/second for real-time
  → Hit rate limits quickly
  → Max 100-500 updates/second per exchange

WebSocket (Good for latency):
  SUBSCRIBE orderbook → <5ms latency
  → Push updates only when data changes
  → 1000s of updates/second capacity
  → Never hit rate limits (no polling)
  → True real-time <5ms
  → Perfect for microsecond arbitrage
```

### Multi-Exchange WebSocket Connections

```rust
// Architecture: Maintain persistent WebSocket to each exchange

pub struct MultiExchangeConnector {
    binance: Arc<BinanceWSClient>,
    bybit: Arc<BybitWSClient>,
    okx: Arc<OKXWSClient>,
    coinbase: Arc<CoinbaseWSClient>,
    kraken: Arc<KrakenWSClient>,
    kucoin: Arc<KucoinWSClient>,
    hyperliquid: Arc<HyperliquidWSClient>,
    drift: Arc<DriftWSClient>,
}

impl MultiExchangeConnector {
    pub async fn subscribe_all_orderbooks(&self, symbols: Vec<&str>) -> Result<()> {
        // Subscribe to order books on ALL exchanges simultaneously
        // Each exchange receives updates as they occur

        let mut tasks = vec![];

        // Binance
        for symbol in &symbols {
            tasks.push(
                self.binance.subscribe_orderbook(*symbol, 100) // Top 100 bids/asks
            );
        }

        // Bybit
        for symbol in &symbols {
            tasks.push(
                self.bybit.subscribe_orderbook(*symbol, 500) // Deeper book
            );
        }

        // OKX
        for symbol in &symbols {
            tasks.push(
                self.okx.subscribe_orderbook(*symbol, 400)
            );
        }

        // Coinbase
        for symbol in &symbols {
            tasks.push(
                self.coinbase.subscribe_orderbook(*symbol, 100)
            );
        }

        // Kraken (REST polling as fallback since no WS)
        for symbol in &symbols {
            tasks.push(
                self.kraken.poll_orderbook_periodic(*symbol, Duration::from_millis(100))
            );
        }

        futures::future::join_all(tasks).await;

        Ok(())
    }

    pub async fn subscribe_all_funding_rates(&self, symbols: Vec<&str>) -> Result<()> {
        // Subscribe to funding rate updates on perpetual exchanges
        let mut tasks = vec![];

        // Bybit funding rate updates
        for symbol in &symbols {
            tasks.push(
                self.bybit.subscribe_funding_rate(*symbol)
            );
        }

        // OKX funding rate updates
        for symbol in &symbols {
            tasks.push(
                self.okx.subscribe_funding_rate(*symbol)
            );
        }

        // Kucoin funding updates
        for symbol in &symbols {
            tasks.push(
                self.kucoin.subscribe_funding_rate(*symbol)
            );
        }

        // Hyperliquid funding (real-time)
        for symbol in &symbols {
            tasks.push(
                self.hyperliquid.subscribe_funding(*symbol)
            );
        }

        futures::future::join_all(tasks).await;

        Ok(())
    }

    pub async fn subscribe_all_sentiment(&self, symbols: Vec<&str>) -> Result<()> {
        // Subscribe to long/short ratio and open interest
        let mut tasks = vec![];

        // Bybit long/short ratio
        for symbol in &symbols {
            tasks.push(
                self.bybit.subscribe_long_short_ratio(*symbol, "5min".to_string())
            );
        }

        // OKX open interest
        for symbol in &symbols {
            tasks.push(
                self.okx.subscribe_open_interest(*symbol)
            );
        }

        // Hyperliquid funding and sentiment
        for symbol in &symbols {
            tasks.push(
                self.hyperliquid.subscribe_sentiment(*symbol)
            );
        }

        futures::future::join_all(tasks).await;

        Ok(())
    }
}
```

---

## 📊 Detailed Exchange API Reference

### 1. Binance (Spot + Futures)

**WebSocket Endpoints:**
```
Spot Price Stream:
  wss://stream.binance.com:9443/ws/solusdt@trade
  → Updates every trade

Order Book Stream (25ms updates):
  wss://stream.binance.com:9443/ws/solusdt@depth20@100ms
  → Top 20 bids/asks every 100ms

Full Order Book Snapshot:
  GET https://api.binance.com/api/v3/depth?symbol=SOLUSDT&limit=5000
  → Initial book for local reconstruction

Funding Rate (Futures):
  GET https://fapi.binance.com/fapi/v1/fundingRate?symbol=SOLUSDT
  → 8 updates per day (every 8 hours)
  → Next update time at fundingTime
```

**Implementation:**
```rust
pub struct BinanceSpotOrderBook {
    last_update_id: u64,
    bids: BTreeMap<Decimal, Decimal>,  // price -> quantity
    asks: BTreeMap<Decimal, Decimal>,
    last_update_time: Instant,
}

impl BinanceSpotOrderBook {
    pub async fn subscribe_and_maintain(symbol: &str) -> Result<()> {
        // 1. Fetch initial snapshot
        let snapshot = Self::fetch_snapshot(symbol).await?;

        // 2. Connect to WebSocket for updates
        let ws = tokio_tungstenite::connect(
            format!("wss://stream.binance.com:9443/ws/{}@depth20@100ms",
                symbol.to_lowercase())
        ).await?;

        // 3. Apply updates to maintain live book
        let (mut write, mut read) = ws.split();
        while let Some(msg) = read.next().await {
            let depth: BinanceDepthUpdate = serde_json::from_str(&msg?)?;

            // Validate update is sequential
            if depth.first_update_id <= snapshot.last_update_id {
                continue;  // Skip if older than snapshot
            }

            // Apply bid/ask updates
            for (price, qty) in depth.bids {
                if qty == 0.0 {
                    self.bids.remove(&price);
                } else {
                    self.bids.insert(price, qty);
                }
            }

            for (price, qty) in depth.asks {
                if qty == 0.0 {
                    self.asks.remove(&price);
                } else {
                    self.asks.insert(price, qty);
                }
            }

            self.last_update_time = Instant::now();
        }

        Ok(())
    }

    pub fn get_spread(&self) -> Result<Decimal> {
        let best_bid = self.bids.iter().next_back()?.0; // Highest bid
        let best_ask = self.asks.iter().next()?.0;      // Lowest ask
        Ok(best_ask - best_bid)
    }

    pub fn get_mid_price(&self) -> Result<Decimal> {
        let best_bid = *self.bids.iter().next_back()?.0;
        let best_ask = *self.asks.iter().next()?.0;
        Ok((best_bid + best_ask) / 2)
    }
}
```

**Key Metrics:**
```
Rate Limit: 1200 requests/minute (very generous)
Order Book Latency: 100ms updates (good but not best)
Funding Rates: 8 times/day (not real-time)
Best For: Spot arbitrage, general trading
```

---

### 2. Bybit (CEX + Perpetuals - Best for Funding Trades)

**WebSocket Endpoints:**
```
Order Book (Real-time):
  wss://stream.bybit.com/v5/public/spot
  Subscribe: {"op":"subscribe","args":["orderbook.500.SOLUSDT"]}
  → 500-level order book, <10ms updates

Funding Rate:
  wss://stream.bybit.com/v5/public/linear
  Subscribe: {"op":"subscribe","args":["funding.SOLUSDT"]}
  → Real-time funding rate changes

Long/Short Ratio:
  wss://stream.bybit.com/v5/public/linear
  Subscribe: {"op":"subscribe","args":["longShortSentiment.SOLUSDT"]}
  → Real-time sentiment (5min aggregation)

Open Interest:
  wss://stream.bybit.com/v5/public/linear
  Subscribe: {"op":"subscribe","args":["openInterest.SOLUSDT"]}
  → Total contracts open
```

**Implementation (Funding Rate Trading):**
```rust
pub struct BybitFundingRateMonitor {
    symbol: String,
    current_funding_rate: Arc<RwLock<Decimal>>,
    next_funding_time: Arc<RwLock<DateTime<Utc>>>,
    historical_rates: Arc<RwLock<VecDeque<FundingRateSnapshot>>>,
}

pub struct FundingRateSnapshot {
    timestamp: DateTime<Utc>,
    funding_rate: Decimal,
    mark_price: Decimal,
    index_price: Decimal,
    open_interest: Decimal,
}

impl BybitFundingRateMonitor {
    pub async fn detect_funding_arbitrage(&self, spot_price: Decimal) -> Option<ArbitrageOpportunity> {
        let funding_rate = *self.current_funding_rate.read().await;
        let index_price = spot_price; // Approximation

        // Funding arbitrage: If funding rate is positive and high (>0.1% per 8h)
        // Buy spot, short perpetual, collect funding
        if funding_rate > Decimal::from_str("0.001").unwrap() {
            // Positive funding = longs pay shorts
            // Strategy: Buy spot (@index), short perp, earn funding

            return Some(ArbitrageOpportunity {
                strategy: "Spot-Perp Arbitrage".to_string(),
                action: "BUY_SPOT_SHORT_PERP".to_string(),
                expected_daily_yield: funding_rate * 3, // 3x per day
                entry_price_spot: spot_price,
                entry_price_perp: index_price,
                duration: Duration::from_secs(28800), // 8 hours
            });
        }

        None
    }

    pub async fn monitor_funding_changes(&self) {
        // Track funding rate changes over 8h period
        loop {
            let rate = *self.current_funding_rate.read().await;
            let snapshot = FundingRateSnapshot {
                timestamp: Utc::now(),
                funding_rate: rate,
                mark_price: 0.0.into(), // Would fetch from market
                index_price: 0.0.into(),
                open_interest: 0.0.into(),
            };

            let mut history = self.historical_rates.write().await;
            history.push_back(snapshot);
            if history.len() > 1000 {
                history.pop_front();
            }

            // Alert if rate spikes (potential arbitrage)
            if let Some(prev) = history.iter().rev().nth(1) {
                let rate_change = (rate - prev.funding_rate) / prev.funding_rate;
                if rate_change.abs() > Decimal::from_str("0.5").unwrap() {
                    println!("⚠️  Funding rate spike: {}% change detected", rate_change * 100);
                }
            }

            tokio::time::sleep(Duration::from_secs(300)).await; // Check every 5min
        }
    }
}
```

**Key Metrics:**
```
Rate Limit: 1000 requests/minute
Order Book Latency: <10ms (excellent)
Funding Updates: Real-time (<100ms)
Long/Short Ratio: 5-minute aggregation
Best For: Funding rate trades, order flow analysis
```

---

### 3. OKX (CEX + Perpetuals - Best for Sophisticated Traders)

**WebSocket Endpoints:**
```
Order Book (Real-time):
  wss://ws.okx.com:8443/ws/v5/public
  Subscribe: {"op":"subscribe","args":[{"channel":"books","instId":"SOL-USDT"}]}
  → Full order book with <5ms latency

Open Interest:
  Subscribe: {"op":"subscribe","args":[{"channel":"open-interest","instId":"SOL-USDT"}]}
  → Real-time open interest changes

Funding Rate & Sentiment:
  Subscribe: {"op":"subscribe","args":[{"channel":"funding","instId":"SOL-USDT"}]}
  → Funding rates and long/short ratio
```

**Implementation (Multi-Asset Monitoring):**
```rust
pub struct OKXMultiAssetMonitor {
    instruments: Vec<OKXInstrument>,
    order_books: Arc<RwLock<HashMap<String, OrderBook>>>,
    funding_rates: Arc<RwLock<HashMap<String, FundingData>>>,
    open_interests: Arc<RwLock<HashMap<String, Decimal>>>,
}

pub struct OKXInstrument {
    inst_id: String,        // e.g., "SOL-USDT"
    inst_type: String,      // "SPOT", "SWAP", "FUTURES"
    underlying: String,     // "SOL"
    quote_currency: String, // "USDT"
}

impl OKXMultiAssetMonitor {
    pub async fn start_monitoring(&mut self) {
        // Subscribe to multiple channels for each instrument
        for inst in &self.instruments {
            // 1. Order book for spot arbitrage detection
            let orderbook_sub = json!({
                "op": "subscribe",
                "args": [{
                    "channel": "books",
                    "instId": inst.inst_id
                }]
            });

            // 2. Perpetual funding for funding trades
            let funding_sub = json!({
                "op": "subscribe",
                "args": [{
                    "channel": "funding",
                    "instId": format!("{}-USDT-SWAP", inst.underlying)
                }]
            });

            // 3. Open interest for sentiment analysis
            let oi_sub = json!({
                "op": "subscribe",
                "args": [{
                    "channel": "open-interest",
                    "instId": format!("{}-USDT-SWAP", inst.underlying)
                }]
            });

            // Send all subscriptions concurrently
            tokio::spawn(self.send_subscriptions(
                vec![orderbook_sub, funding_sub, oi_sub]
            ));
        }

        // Listen for updates
        self.listen_for_updates().await;
    }

    pub async fn detect_multi_exchange_arbitrage(
        &self,
        symbol: &str,
        other_exchanges: &HashMap<String, OrderBook>,
    ) -> Vec<ArbitrageOpportunity> {
        let okx_book = match self.order_books.read().await.get(symbol) {
            Some(book) => book.clone(),
            None => return vec![],
        };

        let mut opportunities = vec![];

        // Compare OKX prices against other exchanges
        for (exchange_name, other_book) in other_exchanges {
            let okx_best_bid = okx_book.best_bid();
            let okx_best_ask = okx_book.best_ask();

            let other_best_bid = other_book.best_bid();
            let other_best_ask = other_book.best_ask();

            // Arbitrage 1: OKX cheaper than other exchange
            if okx_best_ask < other_best_bid {
                let spread = (other_best_bid - okx_best_ask) / okx_best_ask;
                if spread > Decimal::from_str("0.001").unwrap() { // >0.1% profit
                    opportunities.push(ArbitrageOpportunity {
                        strategy: format!("BUY_OKX_SELL_{}", exchange_name),
                        profit_percentage: spread * 100,
                        buy_price: okx_best_ask,
                        sell_price: other_best_bid,
                        volume_available: okx_book.ask_volume[&okx_best_ask],
                    });
                }
            }

            // Arbitrage 2: Other exchange cheaper than OKX
            if other_best_ask < okx_best_bid {
                let spread = (okx_best_bid - other_best_ask) / other_best_ask;
                if spread > Decimal::from_str("0.001").unwrap() {
                    opportunities.push(ArbitrageOpportunity {
                        strategy: format!("BUY_{}_SELL_OKX", exchange_name),
                        profit_percentage: spread * 100,
                        buy_price: other_best_ask,
                        sell_price: okx_best_bid,
                        volume_available: other_book.ask_volume[&other_best_ask],
                    });
                }
            }
        }

        opportunities
    }
}
```

**Key Metrics:**
```
Rate Limit: 2000 requests/minute
Order Book Latency: <5ms (best in class)
Funding Updates: Real-time
Open Interest: Real-time with delta updates
Best For: Serious arbitrage, sophisticated signals
```

---

### 4. Coinbase

**REST API** (No WebSocket for public data):
```
Order Book:
  GET https://api.exchange.coinbase.com/products/{product_id}/book
  ?level=2 (best bid/ask)
  ?level=3 (full book)

Ticker:
  GET https://api.exchange.coinbase.com/products/{product_id}/ticker

Trade History:
  GET https://api.exchange.coinbase.com/products/{product_id}/trades
```

**Real-Time via WebSocket** (Requires auth for filtered data):
```
wss://ws-feed.exchange.coinbase.com
→ Requires authentication to subscribe
```

**Implementation (Polling Strategy):**
```rust
pub struct CoinbaseOrderBook {
    product_id: String,
    last_update: Instant,
    poll_interval: Duration,
}

impl CoinbaseOrderBook {
    pub async fn start_polling(&self) {
        loop {
            // Fetch full order book
            let response = reqwest::Client::new()
                .get(format!(
                    "https://api.exchange.coinbase.com/products/{}/book?level=3",
                    self.product_id
                ))
                .send()
                .await;

            if let Ok(resp) = response {
                let book: CoinbaseBookSnapshot = resp.json().await.ok().unwrap();

                // Process book
                self.process_orderbook(&book).await;
            }

            // Poll every 100ms (achieves 10 updates/sec)
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}
```

**Key Metrics:**
```
Rate Limit: Strict (10 req/sec, 15,000 req/hour)
Order Book Latency: 100-200ms via polling
Best For: US-centric trades, compliance-focused
Limitation: No real WebSocket limits arbitrage opportunities
```

---

### 5. Kraken (Reliable but No WebSocket)

**REST API Only:**
```
Order Book:
  GET https://api.kraken.com/0/public/Depth?pair=SOLUSDT&count=500

Ticker:
  GET https://api.kraken.com/0/public/Ticker?pair=SOLUSDT

OHLC:
  GET https://api.kraken.com/0/public/OHLC?pair=SOLUSDT&interval=1
```

**Polling Strategy:**
```rust
pub struct KrakenOrderBook {
    symbol: String,
}

impl KrakenOrderBook {
    pub async fn poll_continuously(&self) {
        // Since Kraken has no WebSocket, must poll
        // Mitigate with smart caching and batching

        loop {
            // Get tier pricing to catch spreads
            let (bids, asks) = self.fetch_orderbook().await.ok().unwrap();

            // Process for arbitrage
            let best_bid = bids[0].price;
            let best_ask = asks[0].price;

            // Store for comparison against other exchanges
            // ...

            // Poll every 100-200ms as fast as rate limits allow
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}
```

**Key Metrics:**
```
Rate Limit: 15 req/sec (good)
Order Book Latency: 100-200ms via polling
Best For: Reliable European trading, regulatory compliance
Limitation: Polling means always 100ms+ behind other exchanges
```

---

### 6. Kucoin (Good all-rounder)

**WebSocket Endpoints:**
```
Order Book (Real-time):
  wss://stream.kucoin.com/socket.io
  Topic: /market/level2:{symbol}
  → Real-time order book updates

Funding Rate:
  Topic: /contract/funding:{symbol}
  → Real-time funding changes

Open Interest:
  Topic: /contract/openInterest:{symbol}
  → Real-time open interest
```

---

### 7. Hyperliquid (DEX - No Counterparty Risk)

**WebSocket Endpoints:**
```
Real-time Data:
  wss://api.hyperliquid.xyz/ws

Order Book:
  {"method": "subscribe", "subscription": {"type": "l2Book", "coin": "SOL"}}
  → Full order book, <5ms latency

Funding Rate:
  {"method": "subscribe", "subscription": {"type": "funding", "coin": "SOL"}}
  → Real-time funding

Liquidations:
  {"method": "subscribe", "subscription": {"type": "liquidations", "coin": "SOL"}}
  → Liquidation cascade detection
```

---

### 8. Drift (DEX - On-Chain Order Book)

**WebSocket Endpoints:**
```
Real-time indexing of on-chain positions:
  → Order book reconstructed from blockchain
  → No counterparty risk
  → < 100ms latency for liquidation detection
```

---

## 🔍 Order Book Storage & Analysis

### Order Book Data Structure

```sql
-- Real-time order book snapshots
CREATE TABLE order_book_snapshots (
    id BIGSERIAL PRIMARY KEY,
    timestamp TIMESTAMPTZ NOT NULL,
    exchange VARCHAR(50) NOT NULL,     -- "binance", "bybit", "okx"
    symbol VARCHAR(20) NOT NULL,       -- "SOL"
    quote_currency VARCHAR(10),        -- "USDT"

    -- Best bid/ask
    best_bid NUMERIC(20, 8),
    best_ask NUMERIC(20, 8),
    bid_size NUMERIC(20, 8),
    ask_size NUMERIC(20, 8),

    -- Mid price and spread
    mid_price NUMERIC(20, 8),
    spread_bps INTEGER,                -- Spread in basis points

    -- Order book imbalance (microstructure signal)
    bid_depth_10 NUMERIC(30, 2),       -- USD value of top 10 bids
    ask_depth_10 NUMERIC(30, 2),       -- USD value of top 10 asks
    imbalance_ratio NUMERIC(10, 4),    -- bid_depth / ask_depth (>1 = bullish)

    -- Full book pressure (next 100 levels)
    bid_depth_100 NUMERIC(30, 2),
    ask_depth_100 NUMERIC(30, 2),

    created_at TIMESTAMPTZ DEFAULT NOW()
);

SELECT create_hypertable('order_book_snapshots', 'timestamp',
    if_not_exists => TRUE);

CREATE INDEX ON order_book_snapshots (exchange, symbol, timestamp DESC);
CREATE INDEX ON order_book_snapshots (timestamp DESC);
```

### Arbitrage Detection Query

```sql
-- Find arbitrage opportunities across exchanges
WITH latest_books AS (
    SELECT
        exchange,
        symbol,
        best_bid,
        best_ask,
        bid_size,
        ask_size,
        ROW_NUMBER() OVER (PARTITION BY exchange, symbol ORDER BY timestamp DESC) as rn
    FROM order_book_snapshots
    WHERE timestamp > NOW() - INTERVAL '5 seconds'
)
SELECT
    symbol,
    MIN(best_ask) as buy_exchange_ask,
    (SELECT exchange FROM latest_books b2 WHERE b2.best_ask = MIN(best_ask)) as buy_on,
    MAX(best_bid) as sell_exchange_bid,
    (SELECT exchange FROM latest_books b3 WHERE b3.best_bid = MAX(best_bid)) as sell_on,
    ((MAX(best_bid) - MIN(best_ask)) / MIN(best_ask)) * 100 as spread_percent
FROM latest_books
WHERE rn = 1
GROUP BY symbol
HAVING ((MAX(best_bid) - MIN(best_ask)) / MIN(best_ask)) > 0.001  -- >0.1% arbitrage
ORDER BY spread_percent DESC;
```

---

## 💰 Funding Rate & Open Interest Data

### Funding Rate Schema

```sql
CREATE TABLE funding_rates (
    id BIGSERIAL PRIMARY KEY,
    timestamp TIMESTAMPTZ NOT NULL,
    exchange VARCHAR(50) NOT NULL,
    symbol VARCHAR(20) NOT NULL,

    -- Current funding rate
    current_rate NUMERIC(10, 6),       -- e.g., 0.0001 = 0.01%
    funding_rate_annual NUMERIC(10, 2), -- Annualized

    -- Historical
    rate_change_1h NUMERIC(10, 6),
    rate_change_24h NUMERIC(10, 6),
    rate_extreme_24h_high NUMERIC(10, 6),
    rate_extreme_24h_low NUMERIC(10, 6),

    -- Sentiment
    funding_positive_count BIGINT,     -- How many longs paying
    funding_negative_count BIGINT,     -- How many shorts paying

    -- Mark vs index price
    mark_price NUMERIC(20, 8),
    index_price NUMERIC(20, 8),
    basis NUMERIC(10, 4),              -- (mark - index) / index

    next_funding_time TIMESTAMPTZ,     -- When rates settle

    created_at TIMESTAMPTZ DEFAULT NOW()
);

SELECT create_hypertable('funding_rates', 'timestamp', if_not_exists => TRUE);
CREATE INDEX ON funding_rates (exchange, symbol, timestamp DESC);
```

### Funding Arbitrage Detection Query

```sql
-- Identify high funding rate opportunities for spot-perp arb
SELECT
    f.exchange,
    f.symbol,
    f.current_rate * 3 as expected_daily_yield,  -- 3x per day
    f.current_rate * 365 as annualized_yield,
    f.mark_price,
    f.basis,
    CASE
        WHEN f.current_rate > 0.0005 THEN 'BUY_SPOT_SHORT_PERP'
        WHEN f.current_rate < -0.0005 THEN 'SHORT_SPOT_BUY_PERP'
        ELSE 'NEUTRAL'
    END as recommended_action
FROM funding_rates f
WHERE timestamp > NOW() - INTERVAL '1 minute'
    AND current_rate > 0.0001  -- Only opportunities >0.01% per 8h
ORDER BY current_rate DESC
LIMIT 20;
```

---

## 📊 Long/Short Ratio & Sentiment

### Long/Short Data Schema

```sql
CREATE TABLE long_short_sentiment (
    id BIGSERIAL PRIMARY KEY,
    timestamp TIMESTAMPTZ NOT NULL,
    exchange VARCHAR(50) NOT NULL,
    symbol VARCHAR(20) NOT NULL,

    -- Ratio data
    long_count BIGINT,              -- Number of long positions
    short_count BIGINT,             -- Number of short positions
    long_ratio NUMERIC(5, 4),       -- Long / (Long + Short)
    short_ratio NUMERIC(5, 4),      -- Short / (Long + Short)

    -- Volume-weighted
    long_volume NUMERIC(30, 2),     -- USD volume of longs
    short_volume NUMERIC(30, 2),    -- USD volume of shorts
    long_volume_ratio NUMERIC(5, 4),

    -- Change metrics
    long_change_1h NUMERIC(10, 4),  -- % change in long interest
    short_change_1h NUMERIC(10, 4),

    -- Sentiment interpretation
    GENERATED ALWAYS AS (
        CASE
            WHEN long_ratio > 0.65 THEN 'VERY_BULLISH'
            WHEN long_ratio > 0.55 THEN 'BULLISH'
            WHEN long_ratio > 0.45 THEN 'NEUTRAL'
            WHEN long_ratio > 0.35 THEN 'BEARISH'
            ELSE 'VERY_BEARISH'
        END
    ) STORED as sentiment,

    created_at TIMESTAMPTZ DEFAULT NOW()
);

SELECT create_hypertable('long_short_sentiment', 'timestamp', if_not_exists => TRUE);
CREATE INDEX ON long_short_sentiment (exchange, symbol, timestamp DESC);
```

---

## 🚀 Real-Time Data Pipeline Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    Multi-CEX Data Collector                      │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐   │
│  │ WebSocket       │ │ WebSocket       │ │ WebSocket       │   │
│  │ Binance         │ │ Bybit           │ │ OKX             │   │
│  │ (100ms latency) │ │ (10ms latency)  │ │ (5ms latency)   │   │
│  └────────┬────────┘ └────────┬────────┘ └────────┬────────┘   │
│           │                   │                   │             │
│  ┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐   │
│  │ REST Polling    │ │ WebSocket       │ │ WebSocket       │   │
│  │ Coinbase        │ │ Kucoin          │ │ Hyperliquid     │   │
│  │ Kraken          │ │ (WebSocket)     │ │ (DEX native)    │   │
│  └────────┬────────┘ └────────┬────────┘ └────────┬────────┘   │
│           │                   │                   │             │
│           └───────────────────┼───────────────────┘             │
│                               ▼                                  │
│                    ┌──────────────────────┐                     │
│                    │ Data Normalization   │                     │
│                    │ & Aggregation        │                     │
│                    │ (1ms processing)     │                     │
│                    └──────────┬───────────┘                     │
│                               ▼                                  │
│                    ┌──────────────────────┐                     │
│                    │ Real-Time Analysis   │                     │
│                    │ - Arbitrage detect   │                     │
│                    │ - Spread monitoring  │                     │
│                    │ - Funding signals    │                     │
│                    └──────────┬───────────┘                     │
│                               ▼                                  │
│                    ┌──────────────────────┐                     │
│                    │ Trading Decisions    │                     │
│                    │ (sub-millisecond)    │                     │
│                    └──────────┬───────────┘                     │
│                               │                                  │
└───────────────────────────────┼──────────────────────────────────┘
                                ▼
                        ┌────────────────┐
                        │ Execute Trades │
                        │ Multi-Exchange │
                        └────────────────┘
```

---

## 🔧 Implementation Checklist

- [ ] Set up WebSocket connections to all 8 exchanges
- [ ] Implement order book reconstruction (handle snapshots + deltas)
- [ ] Create funding rate monitoring system
- [ ] Create long/short ratio tracking
- [ ] Implement arbitrage detection algorithms
- [ ] Create data schemas in Supabase
- [ ] Implement cross-exchange spread monitoring
- [ ] Build liquidation cascade detector
- [ ] Create risk management layer (position limits per exchange)
- [ ] Implement execution layer (simultaneous multi-exchange orders)
- [ ] Set up monitoring and alerting
- [ ] Backtest on 6-month historical data

---

**Status:** ✅ Multi-CEX architecture fully specified
**Next:** Order book reconstruction, arbitrage detection, execution layer

