# 🔌 Custom Data Feeds Guide: Add Your Own APIs

**Purpose:** Enable users to add custom data feed sources without modifying code
**Design:** Configuration-driven, not code-driven
**Security:** All custom APIs defined in environment files

---

## 🎯 **Why Custom APIs?**

```
Users might want to add:
  ✅ Regional exchanges (OKX, Huobi, FTX, etc.)
  ✅ Custom data providers (paid services)
  ✅ WebSocket-only feeds (for real-time data)
  ✅ Historical data services (Messari, Glassnode)
  ✅ Custom in-house APIs
  ✅ Proxy services (to bypass geofencing)

Without modifying code:
  ✅ Just update .env.data-feeds
  ✅ Add endpoint URL
  ✅ Set rate limit
  ✅ Enable it
  ✅ Bot uses it automatically
```

---

## 📝 **Configuration Format for Custom APIs**

### **In .env.data-feeds.example (SHAREABLE)**

```bash
# ============================================================================
# CUSTOM DATA FEEDS - User can add their own APIs
# ============================================================================
# Format: Add any custom APIs you want to use
# All custom APIs are optional and disabled by default

# CUSTOM API #1
# Example: OKX Exchange API
CUSTOM_API_1_NAME=okx
CUSTOM_API_1_URL=https://www.okx.com/api/v5/market/ticker
CUSTOM_API_1_ENABLED=false
CUSTOM_API_1_RATE_LIMIT=0               # 0 = no limit
CUSTOM_API_1_TIMEOUT_SECS=10
CUSTOM_API_1_REQUIRES_AUTH=false
CUSTOM_API_1_API_KEY_ENV_VAR=OKX_API_KEY   # If requires auth
CUSTOM_API_1_PRIORITY=5                 # 0-10 (higher = tried first)

# CUSTOM API #2
# Example: CoinMarketCap API
CUSTOM_API_2_NAME=coinmarketcap
CUSTOM_API_2_URL=https://pro-api.coinmarketcap.com/v1/cryptocurrency/quotes/latest
CUSTOM_API_2_ENABLED=false
CUSTOM_API_2_RATE_LIMIT=333             # Free tier: 333/day ≈ 0.23/min
CUSTOM_API_2_TIMEOUT_SECS=15
CUSTOM_API_2_REQUIRES_AUTH=true
CUSTOM_API_2_API_KEY_ENV_VAR=CMC_API_KEY
CUSTOM_API_2_PRIORITY=3

# CUSTOM API #3
# Example: Messari Historical Data
CUSTOM_API_3_NAME=messari
CUSTOM_API_3_URL=https://data.messari.io/api/v1/assets/bitcoin/metrics
CUSTOM_API_3_ENABLED=false
CUSTOM_API_3_RATE_LIMIT=120              # Community tier: 120/min
CUSTOM_API_3_TIMEOUT_SECS=15
CUSTOM_API_3_REQUIRES_AUTH=false
CUSTOM_API_3_PRIORITY=2

# Add more as needed: CUSTOM_API_4, CUSTOM_API_5, etc.

# ============================================================================
# NOTES:
# - Each API can have up to 10 fields (see above)
# - PRIORITY determines order: 10 = tried first, 0 = tried last
# - If REQUIRES_AUTH=true, set the API key in .env.data-feeds (private)
# - Leave disabled by default (ENABLED=false)
# - User enables only the ones they need
# ============================================================================
```

### **In .env.data-feeds (PRIVATE - with actual keys)**

```bash
# Custom API keys go here (if needed)

# OKX (if using)
OKX_API_KEY=your_actual_key_here
OKX_API_SECRET=your_secret_here

# CoinMarketCap (if using)
CMC_API_KEY=your_actual_key_here

# Messari (usually free, no key needed)
# MESSARI_API_KEY=

# Your proxy service (if using to bypass geo)
PROXY_API_KEY=your_proxy_key_here
PROXY_API_URL=https://your-proxy.example.com
```

---

## 💻 **Implementation: Loading Custom APIs**

### **Rust Code for Loading Custom APIs**

```rust
// src/config/custom_api_config.rs

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomApiConfig {
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub rate_limit: u32,
    pub timeout_secs: u64,
    pub requires_auth: bool,
    pub api_key_env_var: Option<String>,
    pub priority: u8,
}

pub struct CustomApiManager {
    apis: Vec<CustomApiConfig>,
}

impl CustomApiManager {
    pub fn from_env() -> Result<Self> {
        let mut apis = Vec::new();

        // Load up to 10 custom APIs
        for i in 1..=10 {
            let name_key = format!("CUSTOM_API_{}_NAME", i);
            if env::var(&name_key).is_err() {
                continue; // No more custom APIs
            }

            let config = CustomApiConfig {
                name: env::var(format!("CUSTOM_API_{}_NAME", i))?,
                url: env::var(format!("CUSTOM_API_{}_URL", i))?,
                enabled: env::var(format!("CUSTOM_API_{}_ENABLED", i))?
                    .parse::<bool>()
                    .unwrap_or(false),
                rate_limit: env::var(format!("CUSTOM_API_{}_RATE_LIMIT", i))?
                    .parse::<u32>()
                    .unwrap_or(0),
                timeout_secs: env::var(format!("CUSTOM_API_{}_TIMEOUT_SECS", i))?
                    .parse::<u64>()
                    .unwrap_or(10),
                requires_auth: env::var(format!("CUSTOM_API_{}_REQUIRES_AUTH", i))?
                    .parse::<bool>()
                    .unwrap_or(false),
                api_key_env_var: env::var(format!("CUSTOM_API_{}_API_KEY_ENV_VAR", i)).ok(),
                priority: env::var(format!("CUSTOM_API_{}_PRIORITY", i))?
                    .parse::<u8>()
                    .unwrap_or(5),
            };

            apis.push(config);
        }

        Ok(Self { apis })
    }

    pub fn get_enabled_apis(&self) -> Vec<&CustomApiConfig> {
        let mut enabled: Vec<_> = self.apis
            .iter()
            .filter(|api| api.enabled)
            .collect();

        // Sort by priority (highest first)
        enabled.sort_by(|a, b| b.priority.cmp(&a.priority));

        enabled
    }

    pub fn get_api_key(&self, api: &CustomApiConfig) -> Result<Option<String>> {
        if !api.requires_auth {
            return Ok(None);
        }

        if let Some(key_var) = &api.api_key_env_var {
            Ok(Some(env::var(key_var)?))
        } else {
            Err("Auth required but no API key env var configured".into())
        }
    }
}
```

---

## 🌐 **Common Custom API Examples**

### **Example 1: OKX Exchange**

```bash
# OKX is popular in Asia, works well as backup

CUSTOM_API_1_NAME=okx
CUSTOM_API_1_URL=https://www.okx.com/api/v5/market/ticker
CUSTOM_API_1_ENABLED=true              # User enables it
CUSTOM_API_1_RATE_LIMIT=0              # No limit
CUSTOM_API_1_TIMEOUT_SECS=10
CUSTOM_API_1_REQUIRES_AUTH=false       # Public API
CUSTOM_API_1_PRIORITY=6                # Try early

# Usage:
# GET /v5/market/ticker?instId=SOL-USDT
# Returns: OKX trading data
```

### **Example 2: CoinMarketCap**

```bash
# CoinMarketCap has premium historical data

CUSTOM_API_2_NAME=coinmarketcap
CUSTOM_API_2_URL=https://pro-api.coinmarketcap.com/v1
CUSTOM_API_2_ENABLED=false             # User enables if they have key
CUSTOM_API_2_RATE_LIMIT=333            # Free: 333/day
CUSTOM_API_2_TIMEOUT_SECS=15
CUSTOM_API_2_REQUIRES_AUTH=true        # Requires API key
CUSTOM_API_2_API_KEY_ENV_VAR=CMC_API_KEY
CUSTOM_API_2_PRIORITY=3

# In .env.data-feeds:
# CMC_API_KEY=your_api_key_here

# Usage:
# GET /v1/cryptocurrency/quotes/latest?slug=solana
# Returns: SOL price + data
```

### **Example 3: Messari (Historical Data)**

```bash
# Messari specializes in on-chain metrics

CUSTOM_API_3_NAME=messari
CUSTOM_API_3_URL=https://data.messari.io/api/v1/assets
CUSTOM_API_3_ENABLED=false
CUSTOM_API_3_RATE_LIMIT=120
CUSTOM_API_3_TIMEOUT_SECS=15
CUSTOM_API_3_REQUIRES_AUTH=false       # Free tier
CUSTOM_API_3_PRIORITY=2

# Usage:
# GET /api/v1/assets/bitcoin/metrics
# Returns: On-chain metrics + historical data
```

### **Example 4: Proxy Service (Bypass Geofencing)**

```bash
# Use proxy if Binance is geofenced

CUSTOM_API_4_NAME=binance_proxy
CUSTOM_API_4_URL=https://your-proxy.example.com/binance
CUSTOM_API_4_ENABLED=true              # If using proxy
CUSTOM_API_4_RATE_LIMIT=1200           # Same as Binance
CUSTOM_API_4_TIMEOUT_SECS=15
CUSTOM_API_4_REQUIRES_AUTH=true        # Proxy requires key
CUSTOM_API_4_API_KEY_ENV_VAR=PROXY_API_KEY
CUSTOM_API_4_PRIORITY=9                # Try before regular APIs

# In .env.data-feeds:
# PROXY_API_KEY=your_proxy_key_here
# PROXY_API_URL=https://your-proxy.example.com
```

### **Example 5: Huobi (Asian Exchange)**

```bash
# Huobi for redundancy in Asia

CUSTOM_API_5_NAME=huobi
CUSTOM_API_5_URL=https://api.huobi.pro/market/detail
CUSTOM_API_5_ENABLED=true
CUSTOM_API_5_RATE_LIMIT=0              # No rate limit published
CUSTOM_API_5_TIMEOUT_SECS=10
CUSTOM_API_5_REQUIRES_AUTH=false
CUSTOM_API_5_PRIORITY=4

# Usage:
# GET /market/detail?symbol=solusdt
# Returns: Trading data
```

---

## 📊 **Custom API Data Format Handling**

### **Problem: Different APIs Have Different Formats**

```
Binance format:
{
  "symbol": "SOLUSDT",
  "lastPrice": "123.45",
  "highPrice": "125.00"
}

CoinGecko format:
{
  "solana": {
    "usd": 123.45
  }
}

OKX format:
{
  "data": [{
    "instId": "SOL-USDT",
    "last": "123.45"
  }]
}

Problem: Need to normalize all to same format
```

### **Solution: Normalizer Interface**

```rust
// src/data_feeds/normalizers/mod.rs

pub trait DataNormalizer {
    fn normalize(&self, raw_response: &str) -> Result<Price>;
}

pub struct PriceNormalizer {
    api_name: String,
}

impl PriceNormalizer {
    pub fn for_api(name: &str) -> Box<dyn DataNormalizer> {
        match name {
            "binance" => Box::new(BinanceNormalizer),
            "coingecko" => Box::new(CoinGeckoNormalizer),
            "okx" => Box::new(OKXNormalizer),
            "coinmarketcap" => Box::new(CMCNormalizer),
            // ... more normalizers
            _ => Box::new(GenericNormalizer),
        }
    }
}

// Each API gets its own normalizer
struct BinanceNormalizer;
impl DataNormalizer for BinanceNormalizer {
    fn normalize(&self, raw: &str) -> Result<Price> {
        let data: BinanceResponse = serde_json::from_str(raw)?;
        Ok(Price {
            symbol: data.symbol,
            last: data.lastPrice.parse()?,
            bid: data.bidPrice.parse()?,
            ask: data.askPrice.parse()?,
            timestamp: chrono::Utc::now().timestamp(),
        })
    }
}

struct OKXNormalizer;
impl DataNormalizer for OKXNormalizer {
    fn normalize(&self, raw: &str) -> Result<Price> {
        let data: OKXResponse = serde_json::from_str(raw)?;
        let item = &data.data[0];
        Ok(Price {
            symbol: item.instId.clone(),
            last: item.last.parse()?,
            bid: item.bid.parse()?,
            ask: item.ask.parse()?,
            timestamp: chrono::Utc::now().timestamp(),
        })
    }
}
```

---

## 🔄 **Dynamic API Fallback Chain**

### **How It Works**

```
1. Load all custom APIs from .env
2. Filter to only ENABLED=true
3. Sort by PRIORITY (highest first)
4. Try each in order until one works
5. Cache result for 500ms
6. If all fail, try next backup

Example execution:
  1. Try: binance_proxy (PRIORITY=9)
  2. If fails: Try binance (PRIORITY=8)
  3. If fails: Try coingecko (PRIORITY=5)
  4. If fails: Try custom_api (PRIORITY=4)
  5. If all fail: Use cached price
  6. If no cache: Return error + stop trading
```

---

## ✅ **User Setup Guide for Custom APIs**

### **Step 1: Identify Your Custom API**

```bash
# What API do you want to add?
# Example: OKX Exchange API

API_NAME=okx
API_URL=https://www.okx.com/api/v5/market/ticker
RATE_LIMIT=0
REQUIRES_AUTH=false
PRIORITY=6
```

### **Step 2: Add to .env.data-feeds.example**

```bash
# Add to the example file (shareable):
CUSTOM_API_1_NAME=okx
CUSTOM_API_1_URL=https://www.okx.com/api/v5/market/ticker
CUSTOM_API_1_ENABLED=false
CUSTOM_API_1_RATE_LIMIT=0
CUSTOM_API_1_TIMEOUT_SECS=10
CUSTOM_API_1_REQUIRES_AUTH=false
CUSTOM_API_1_PRIORITY=6
```

### **Step 3: Enable in .env.data-feeds**

```bash
# User's private file:
CUSTOM_API_1_NAME=okx
CUSTOM_API_1_URL=https://www.okx.com/api/v5/market/ticker
CUSTOM_API_1_ENABLED=true        # Enable it!
CUSTOM_API_1_RATE_LIMIT=0
CUSTOM_API_1_TIMEOUT_SECS=10
CUSTOM_API_1_REQUIRES_AUTH=false
CUSTOM_API_1_PRIORITY=6
```

### **Step 4: If Requires Auth**

```bash
# Add API key to .env.data-feeds (private):
OKX_API_KEY=your_actual_key_here
OKX_API_SECRET=your_secret_here

# Reference in config:
CUSTOM_API_1_API_KEY_ENV_VAR=OKX_API_KEY
CUSTOM_API_1_REQUIRES_AUTH=true
```

### **Step 5: Test**

```bash
# Start bot
source .env.data-feeds
./redrobot

# Check logs for API usage:
tail -f /tmp/redrobot.log | grep "API.*okx\|API.*custom"

# Should show: "Using OKX API: Success" or similar
```

---

## 🎯 **Common Use Cases**

### **Use Case 1: Asia-Deployed Bot**

```bash
# Add regional exchanges for better rates

CUSTOM_API_1_NAME=okx
CUSTOM_API_1_ENABLED=true
CUSTOM_API_1_PRIORITY=8

CUSTOM_API_2_NAME=huobi
CUSTOM_API_2_ENABLED=true
CUSTOM_API_2_PRIORITY=7

PRIMARY_DATA_SOURCE=binance
FALLBACK_DATA_SOURCE=okx
```

### **Use Case 2: USA-Deployed Bot (Geofenced)**

```bash
# Can't use Binance, so add proxy

CUSTOM_API_1_NAME=binance_proxy
CUSTOM_API_1_ENABLED=true
CUSTOM_API_1_REQUIRES_AUTH=true
CUSTOM_API_1_PRIORITY=9

PRIMARY_DATA_SOURCE=coingecko
FALLBACK_DATA_SOURCE=binance_proxy
```

### **Use Case 3: Premium Historical Data**

```bash
# User wants best historical data

CUSTOM_API_1_NAME=messari
CUSTOM_API_1_ENABLED=true
CUSTOM_API_1_PRIORITY=2

CUSTOM_API_2_NAME=coinmarketcap
CUSTOM_API_2_ENABLED=true
CUSTOM_API_2_REQUIRES_AUTH=true
CUSTOM_API_2_PRIORITY=1

# For backtesting: Downloads from these APIs
```

---

## 📋 **Custom API Checklist**

```
Before adding custom API:

[ ] Identify the API endpoint
[ ] Check rate limits
[ ] Determine if authentication needed
[ ] Get API key (if required)
[ ] Determine response format
[ ] Write normalizer (if format differs)
[ ] Choose PRIORITY (0-10)
[ ] Test connectivity from your location
[ ] Add to .env.data-feeds.example
[ ] Enable in .env.data-feeds
[ ] Test with bot running
[ ] Document the addition
```

---

**Status:** User can configure custom APIs via .env files
**Security:** All custom APIs loaded from environment, not hardcoded
**Flexibility:** Users can add unlimited custom data sources
**Scalability:** Normalizers handle different API formats

