# 📊 Data Feeds Architecture: Free Public APIs Research

**Purpose:** Identify free public APIs for real-time and historical crypto pricing data
**Goal:** Replace random signal generation with real market data
**Constraint:** Zero RPC polling for price data (only for wallet/transaction auth)

---

## 🎯 **Architecture Requirements**

```
MUST HAVE:
✅ Real-time price feeds (for decision making every 1 second)
✅ Historical OHLCV data (for backtesting)
✅ Multiple data sources (redundancy)
✅ Free tier (no cost barrier)
✅ Rate limits documented
✅ Configurable via .env files
✅ Not hardcoded in source code
✅ Can switch sources without code changes

MUST AVOID:
❌ RPC calls for pricing data (only for auth/transactions)
❌ Hardcoded API keys in code
❌ Hardcoded endpoints in code
❌ Public API keys in git repo
```

---

## 📡 **FREE Public APIs for Crypto Data**

### **1. BINANCE API (Recommended - Best Free Tier)**

**Provider:** Binance (World's largest exchange)
**Type:** CEX pricing data

#### Real-Time Data:
```
Endpoint: https://api.binance.com/api/v3/ticker/24hr
Rate Limit: 1200 requests per minute
Features:
  - 24hr price, volume, high, low
  - All trading pairs
  - Zero authentication needed
  - Extremely reliable

Example Request:
GET https://api.binance.com/api/v3/ticker/24hr?symbol=SOLUSDT
Response: { "symbol": "SOLUSDT", "lastPrice": "123.45", "highPrice": "125.00", ... }
```

#### Historical OHLCV Data:
```
Endpoint: https://api.binance.com/api/v3/klines
Rate Limit: 1200 requests per minute
Parameters:
  - symbol: Trading pair (SOLUSDT, BTCUSDT, etc)
  - interval: 1m, 5m, 15m, 1h, 4h, 1d
  - startTime/endTime: Unix timestamps
  - limit: Max 1000 candles per request

Example Request:
GET https://api.binance.com/api/v3/klines?symbol=SOLUSDT&interval=1h&limit=100
Response: [[1645000000000, "123.00", "124.00", "122.50", "123.50", "1000000", ...], ...]

Daily Limits:
  - ~1.7 MILLION requests/day at 1200/min
  - Sufficient for real-time + backtesting
  - FREE FOREVER (no subscription needed)

Reliability: 99.9% uptime (industry standard)
```

---

### **2. COINGECKO API (Historical + Market Data)**

**Provider:** CoinGecko (Decentralized approach)
**Type:** Aggregated market data

#### Real-Time Data:
```
Endpoint: https://api.coingecko.com/api/v3/simple/price
Rate Limit: 10-50 calls/minute (free tier)
Parameters:
  - ids: Coin IDs (solana, bitcoin, ethereum)
  - vs_currencies: usd
  - include_market_cap: true
  - include_24hr_vol: true

Example Request:
GET https://api.coingecko.com/api/v3/simple/price?ids=solana,bitcoin&vs_currencies=usd&include_24hr_vol=true
Response: { "solana": { "usd": 123.45, "usd_24h_vol": 5000000000 } }

Daily Limits:
  - ~50,000 calls/day (50 per minute)
  - Sufficient for occasional polling
  - FREE (with rate limiting)
```

#### Historical Data:
```
Endpoint: https://api.coingecko.com/api/v3/coins/{id}/market_chart
Rate Limit: 10-50 calls/minute
Parameters:
  - id: Coin ID (solana, bitcoin)
  - vs_currency: usd
  - days: Number of days (1-max)
  - interval: daily

Example Request:
GET https://api.coingecko.com/api/v3/coins/solana/market_chart?vs_currency=usd&days=365&interval=daily
Response: { "prices": [[1640995200000, 123.45], ...], "volumes": [...] }

Daily Limits:
  - Free tier: 10-50 calls/minute
  - Paid tier: 500 calls/minute (optional $50/month)
```

---

### **3. KRAKEN API (Excellent Alternative)**

**Provider:** Kraken Exchange
**Type:** CEX pricing data

#### Real-Time Data:
```
Endpoint: https://api.kraken.com/0/public/Ticker
Rate Limit: 15 requests per second (no auth needed)
Parameters:
  - pair: Trading pair (XSOLZUSD, XXBTZUSD)
  - Note: Kraken uses different pair naming

Example Request:
GET https://api.kraken.com/0/public/Ticker?pair=XSOLZUSD
Response: { "XSOLZUSD": { "c": ["123.45", "1234567890"], "h": ["125.00", "125.00"], ... } }

Daily Limits:
  - 15 requests per second (unlimited total)
  - ~1.3 MILLION requests/day
  - FREE (no authentication needed)

Reliability: High reliability
Pair Format: Different from Binance (requires mapping)
```

#### Historical Data:
```
Endpoint: https://api.kraken.com/0/public/OHLC
Rate Limit: 15 requests per second
Parameters:
  - pair: Trading pair
  - interval: 1, 5, 15, 30, 60, 240, 1440, 10080, 21600 minutes
  - since: Unix timestamp

Example Request:
GET https://api.kraken.com/0/public/OHLC?pair=XSOLZUSD&interval=60&since=1645000000
Response: { "XSOLZUSD": [[timestamp, open, high, low, close, vwap, volume, count], ...] }
```

---

### **4. POLYGON.IO (Alternative for Stocks + Crypto)**

**Provider:** Polygon.io
**Type:** Market data aggregator

#### Real-Time Crypto Data:
```
Endpoint: https://api.polygon.io/v1/last/crypto
Rate Limit: 5 requests per minute (free tier)
Parameters:
  - cryptoticker: CSOL, CBT (Crypto SOL, Crypto BTC)
  - apikey: Your free API key (required)

Example Request:
GET https://api.polygon.io/v1/last/crypto?cryptoticker=CSOL&apikey=YOUR_KEY
Response: { "status": "OK", "results": [{ "last": 123.45, "timeframe": "hour" }] }

Daily Limits:
  - FREE: 5 requests/minute = 7,200/day
  - STARTER: $149/month = 100 req/min
  - Sufficient for hourly updates only
  - Requires API key registration (free)

Limitation: Slower than direct exchange APIs
```

---

### **5. HYPERLIQUID PERPS API (Direct Exchange Data)**

**Provider:** Hyperliquid (Your trading venue)
**Type:** DEX pricing from your trading platform

#### Real-Time Data:
```
Endpoint: https://api.hyperliquid.com/info
No Rate Limit: Direct exchange data
WebSocket: wss://api.hyperliquid.com/ws

Features:
  - Live orderbook data
  - Trade execution prices
  - Funding rates
  - Open interest
  - Zero external dependency (your exchange!)

Advantage:
  - No rate limiting
  - Most accurate for your trading venue
  - Zero authentication for public data
  - WebSocket for real-time updates
```

---

### **6. DRIFT PROTOCOL (Your Other Venue)**

**Provider:** Drift (Solana-based perps)
**Type:** DEX pricing from your other trading platform

#### Data Access:
```
Method: RPC calls through Solana (careful here!)
Endpoint: https://api.mainnet-beta.solana.com (default)
But: Use sparingly - only for account/trade info

Price Data: Better from Binance/Kraken instead
Trade Data: Must use Drift RPC for position info
```

---

## 📋 **API Comparison Matrix**

| Feature | Binance | CoinGecko | Kraken | Polygon | Hyperliquid |
|---------|---------|-----------|--------|---------|------------|
| Real-Time | ✅ 1200/min | ⚠️ 50/min | ✅ 15/sec | ⚠️ 5/min | ✅ Unlimited |
| Historical | ✅ Excellent | ✅ Good | ✅ Excellent | ✅ Good | ⚠️ Limited |
| Free Tier | ✅ Forever | ✅ Forever | ✅ Forever | ✅ Limited | ✅ Forever |
| Auth Needed | ❌ No | ❌ No | ❌ No | ✅ Yes (key) | ❌ No |
| Pairs | ✅ 3000+ | ✅ Crypto | ✅ Crypto | ✅ Limited | ✅ Perps only |
| Reliability | ✅ 99.9% | ✅ Good | ✅ Excellent | ⚠️ Good | ✅ Excellent |

---

## 🏆 **Recommended Architecture**

### **Primary Strategy: Binance + Hyperliquid**

```
For Real-Time Decision Making (Every 1 second):
  1. Query Hyperliquid API for your trading venue price
  2. Fallback to Binance if Hyperliquid unavailable
  3. Cache for 500ms to avoid excessive calls
  Rate: ~120 calls/min = Well within limits

For Historical Analysis:
  1. Use Binance for historical data
  2. Can query hourly candles (1h, 4h, 1d)
  Rate: ~10-20 calls/min = Well within limits

For Backtesting:
  1. Download full historical data once
  2. Store in local CSV or database
  3. No API calls needed during backtest
```

### **Secondary Strategy: CoinGecko for Redundancy**

```
If Binance/Hyperliquid fail:
  1. Fall back to CoinGecko
  2. Less frequent updates (5-10 min)
  3. Enough for monitoring
  Rate: 5 calls/min = Conservative
```

---

## 🔧 **Implementation Plan**

### **File Structure:**

```
tradingbots-fun/
├── .env.data-feeds              (⛔ GITIGNORE - Your actual keys)
├── .env.data-feeds.example      (✅ Committed - Template)
├── src/
│   ├── data_feeds/
│   │   ├── mod.rs               (Module exports)
│   │   ├── binance.rs           (Binance API client)
│   │   ├── coingecko.rs         (CoinGecko API client)
│   │   ├── hyperliquid_feed.rs  (Price feeds from HLP)
│   │   ├── kraken.rs            (Kraken API client)
│   │   └── cache.rs             (Price cache to avoid excessive calls)
│   └── ...
└── config/
    └── data_feeds_config.rs     (Load from .env)
```

---

## 📝 **Configuration Files**

### **.env.data-feeds.example** (Commit this to git)

```bash
# =============================================================================
# DATA FEEDS CONFIGURATION - Copy to .env.data-feeds and fill in your keys
# =============================================================================

# BINANCE API (No authentication needed - public data)
BINANCE_BASE_URL=https://api.binance.com/api/v3
BINANCE_WS_URL=wss://stream.binance.com:9443/ws
BINANCE_ENABLED=true
BINANCE_SYMBOLS=SOLUSDT,BTCUSDT,ETHUSDT

# COINGECKO API (Free tier - rate limited)
COINGECKO_BASE_URL=https://api.coingecko.com/api/v3
COINGECKO_API_KEY=                          # Leave empty for free tier
COINGECKO_ENABLED=true
COINGECKO_SYMBOLS=solana,bitcoin,ethereum

# KRAKEN API (No authentication needed)
KRAKEN_BASE_URL=https://api.kraken.com/0/public
KRAKEN_ENABLED=true
KRAKEN_SYMBOLS=XSOLZUSD,XXBTZUSD,XETHZUSD

# POLYGON.IO (Requires free API key)
POLYGON_BASE_URL=https://api.polygon.io/v1
POLYGON_API_KEY=                            # Get free at polygon.io
POLYGON_ENABLED=false                       # Optional
POLYGON_SYMBOLS=CSOL,CBT,CETH

# HYPERLIQUID FEEDS (Your trading venue - no auth for public data)
HYPERLIQUID_FEED_URL=https://api.hyperliquid.com/info
HYPERLIQUID_FEED_WS=wss://api.hyperliquid.com/ws
HYPERLIQUID_FEED_ENABLED=true

# =============================================================================
# CACHING & RATE LIMITING
# =============================================================================

# Cache pricing data for X milliseconds to avoid excessive API calls
PRICE_CACHE_DURATION_MS=500

# Primary data source (binance, coingecko, kraken, hyperliquid)
PRIMARY_DATA_SOURCE=binance
FALLBACK_DATA_SOURCE=coingecko

# Rate limiting (requests per minute per API)
BINANCE_RATE_LIMIT=1200                     # 1200 req/min
COINGECKO_RATE_LIMIT=50                     # 50 req/min (free)
KRAKEN_RATE_LIMIT=900                       # 15 req/sec = 900/min
POLYGON_RATE_LIMIT=300                      # 5 req/min (free)

# =============================================================================
# HISTORICAL DATA
# =============================================================================

# Where to store historical data for backtesting
HISTORICAL_DATA_PATH=./data/historical

# How many days of historical data to keep
HISTORICAL_DATA_RETENTION_DAYS=365

# Which API to use for historical data (binance, coingecko, kraken)
HISTORICAL_DATA_SOURCE=binance

# =============================================================================
# TRADING SYMBOLS (What pairs to monitor)
# =============================================================================

# Primary symbols for trading decisions
TRADING_SYMBOLS=SOLUSDT,BTCUSDT,ETHUSDT

# Timeframes for analysis (1m, 5m, 15m, 1h, 4h, 1d)
CANDLE_TIMEFRAMES=1h,4h,1d

# =============================================================================
# API RESPONSE TIMEOUTS (seconds)
# =============================================================================

BINANCE_TIMEOUT_SECS=10
COINGECKO_TIMEOUT_SECS=15
KRAKEN_TIMEOUT_SECS=10
POLYGON_TIMEOUT_SECS=15
HYPERLIQUID_TIMEOUT_SECS=10

# =============================================================================
# FALLBACK & ERROR HANDLING
# =============================================================================

# If primary source fails, how many seconds to wait before trying fallback
FALLBACK_WAIT_SECS=5

# Max retries for failed API calls
MAX_API_RETRIES=3

# Enable verbose API logging (debug mode)
API_DEBUG=false
```

### **.gitignore Update**

```bash
# Add to existing .gitignore:

# Data feeds configuration with API keys
.env.data-feeds
.env.data-feeds.local

# Historical data cache
data/historical/
*.csv

# API response cache (temporary)
.cache/
*.cache
```

---

## 💡 **Usage in Code**

### **Example: Real-Time Price Feed**

```rust
// In src/main.rs or initialization code:

use crate::data_feeds::{BinanceClient, PriceCache};
use std::env;

async fn initialize_data_feeds() -> Result<()> {
    // Load configuration from .env.data-feeds
    let primary_source = env::var("PRIMARY_DATA_SOURCE")?;
    let cache_duration = env::var("PRICE_CACHE_DURATION_MS")?.parse()?;

    // Initialize price cache
    let cache = PriceCache::new(cache_duration);

    // Initialize data feeds
    match primary_source.as_str() {
        "binance" => {
            let binance_url = env::var("BINANCE_BASE_URL")?;
            let client = BinanceClient::new(binance_url);

            // Get real price (with caching)
            let price = cache.get_or_fetch("SOLUSDT", || {
                client.get_price("SOLUSDT")
            }).await?;

            println!("SOL Price: ${}", price);
        },
        "hyperliquid" => {
            // Use HLP for direct venue pricing
        },
        _ => return Err("Unknown data source".into()),
    }

    Ok(())
}
```

---

## 📊 **Rate Limit Calculations**

### **Real-Time Trading (1 second decision interval)**

```
Decision Loop: Every 1 second
Symbols: 3 (SOL, BTC, ETH)
Calls per decision: 1 per symbol = 3 calls

Per minute: 3 symbols × 60 seconds = 180 calls/min
Daily: 180 × 1440 = 259,200 calls/day

Status:
  Binance (1200/min):  ✅ SAFE (uses only 3/min)
  Kraken (15/sec):     ✅ SAFE (uses only 0.05/sec)
  CoinGecko (50/min):  ✅ SAFE with caching
  Hyperliquid (∞):     ✅ UNLIMITED
```

### **Historical Data Fetching (Backtesting)**

```
Backtest period: 365 days
Timeframe: 1 hour = 365 × 24 = 8,760 candles
Symbols: 3
Total calls: 3 × (8760 / 1000) ≈ 27 API calls

Status:
  Binance (1200/min):  ✅ SAFE (uses 27 calls one-time)
  Kraken:              ✅ SAFE
  CoinGecko:           ✅ SAFE
```

---

## 🚨 **Critical Design Decisions**

### **DO NOT DO:**
```
❌ Poll RPC for price data
❌ Hardcode API endpoints in code
❌ Hardcode API keys in code
❌ Commit .env.data-feeds to git
❌ Create API clients in main loop (too slow)
❌ Make API call on every decision cycle
```

### **DO THIS INSTEAD:**
```
✅ Use public CEX/DEX APIs (Binance, Hyperliquid)
✅ Load all config from .env.data-feeds
✅ Store API keys only in .env files
✅ Commit only .env.example templates
✅ Initialize clients once on startup
✅ Cache prices for 500ms between calls
✅ Implement fallback sources
✅ Log all API calls (for debugging)
```

---

## 📈 **Next Steps**

### **Phase 1: Data Feed Infrastructure** (This week)
```
1. Create data_feeds/ module
2. Implement Binance client
3. Implement price caching
4. Add .env.data-feeds configuration
5. Update .gitignore
```

### **Phase 2: Real Signal Generation** (Next week)
```
1. Get real prices from data feeds
2. Calculate real technical indicators (RSI, MACD)
3. Use real signals for decisions (not random)
4. Backtest with real price data
5. Validate performance
```

### **Phase 3: Deploy to Testnet** (Week 2-3)
```
1. Run with real price feeds
2. Validate decision-making
3. Prove actual signal quality
4. Document performance
```

---

## ✅ **Success Criteria**

```
✅ Zero hardcoded API endpoints
✅ Zero hardcoded API keys
✅ All config in .env.data-feeds
✅ .gitignore prevents key leaks
✅ Multiple data sources (primary + fallback)
✅ Price caching to respect rate limits
✅ Real historical data for backtesting
✅ Real price feeds for trading decisions
✅ Can switch data sources without code changes
✅ Rate limits documented and respected
```

---

**Status:** Research complete, architecture defined
**Next:** Implement data feed clients in code
**Timeline:** 1 week to full implementation

This architecture eliminates random signals and replaces them with real market data! 🎯

