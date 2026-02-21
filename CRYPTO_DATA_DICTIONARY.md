# 📚 Crypto Data Dictionary: Multi-Exchange Symbol Standardization

**Role:** Data Architecture & Integration Specialist
**Purpose:** Standardize coin symbols, metadata, and mappings across all CEX exchanges
**Status:** ✅ Production-ready data dictionary

---

## 🎯 Overview

Different exchanges use different symbol conventions:

```
Binance:    SOLUSDT (futures), SOLusdt (spot)
Bybit:      SOLUSDT (perpetual)
OKX:        SOL-USDT (swap), SOL-USDT-SWAP
Coinbase:   SOL-USD, SOL-USDT
Kraken:     SOLUSUSDT
Kucoin:     SOL-USDT
Hyperliquid: SOL

Without standardization: 7 different symbols for 1 asset!
```

This document maps all symbols to a canonical format.

---

## 📊 Canonical Symbol Format

### Standard: `{ASSET}-{QUOTE}` (Spot) & `{ASSET}-{QUOTE}-PERP` (Perpetuals)

**Examples:**
```
SOL-USDT          (SOL paired with USDT, spot)
SOL-USDC          (SOL paired with USDC, spot)
BTC-USD           (BTC paired with USD, spot)
SOL-USDT-PERP     (SOL perpetual, USDT quoted)
BTC-USDT-PERP     (BTC perpetual, USDT quoted)
```

**Rules:**
- Always uppercase
- Hyphen-separated: `{BASE}-{QUOTE}`
- Perpetuals: Append `-PERP`
- Use 4-letter currency codes (USDT, USDC, BUSD)
- No slashes, no spaces

---

## 🔄 Exchange Symbol Mapping

### Master Coin List (Top 100 by Volume)

```sql
CREATE TABLE crypto_master_coins (
    id BIGSERIAL PRIMARY KEY,

    -- Canonical identifiers
    canonical_symbol VARCHAR(20) NOT NULL,      -- e.g., "SOL"
    canonical_name VARCHAR(100),                -- "Solana"
    cmc_id INTEGER,                            -- CoinMarketCap ID for lookups
    coingecko_id VARCHAR(50),                  -- CoinGecko ID

    -- Blockchain info
    blockchain VARCHAR(50),                     -- "solana", "ethereum", "bitcoin"
    contract_address JSONB,  -- {"ethereum": "0x...", "solana": "..."}
    decimals JSONB,          -- {"ethereum": 6, "solana": 9}

    -- Classification
    asset_type VARCHAR(20),                     -- "token", "coin", "derivative"
    category VARCHAR(50),                       -- "layer-1", "defi", "nft", "meme"

    -- Core metadata
    market_cap_usd NUMERIC(30, 2),
    volume_24h_usd NUMERIC(30, 2),
    circulating_supply NUMERIC(30, 8),
    total_supply NUMERIC(30, 8),

    -- Exchange support tracking
    supported_exchanges TEXT[],  -- ["binance", "bybit", "okx", "coinbase", "kraken", "kucoin", "hyperliquid"]

    -- Last updated
    updated_at TIMESTAMPTZ DEFAULT NOW(),

    UNIQUE(canonical_symbol)
);

-- Example data for top coins
INSERT INTO crypto_master_coins VALUES
('SOL', 'Solana', 5426, 'solana', 'solana', '...', 9, 'coin', 'layer-1', 85000000000, 2500000000, 425000000, 500000000, ARRAY['binance', 'bybit', 'okx', 'coinbase', 'kraken', 'kucoin', 'hyperliquid']),
('BTC', 'Bitcoin', 1, 'bitcoin', 'bitcoin', '...', 8, 'coin', 'layer-0', 1200000000000, 25000000000, 21000000, 21000000, ARRAY['binance', 'bybit', 'okx', 'coinbase', 'kraken', 'kucoin', 'hyperliquid']),
('ETH', 'Ethereum', 1027, 'ethereum', 'ethereum', '...', 18, 'coin', 'layer-1', 200000000000, 15000000000, 120000000, 120000000, ARRAY['binance', 'bybit', 'okx', 'coinbase', 'kraken', 'kucoin', 'hyperliquid']),
-- ... (100 rows total)
;

CREATE INDEX ON crypto_master_coins (canonical_symbol);
CREATE INDEX ON crypto_master_coins (canonical_name);
CREATE INDEX ON crypto_master_coins (cmc_id);
```

### Exchange-Specific Symbol Mapping

```sql
CREATE TABLE exchange_symbol_mapping (
    id BIGSERIAL PRIMARY KEY,

    -- Reference to master coin
    crypto_id BIGINT REFERENCES crypto_master_coins(id),

    -- Exchange info
    exchange VARCHAR(50) NOT NULL,              -- "binance", "bybit", "okx"
    exchange_symbol VARCHAR(50) NOT NULL,       -- Exchange's symbol: "SOLUSDT", "SOL-USDT"

    -- Product type
    product_type VARCHAR(20) NOT NULL,          -- "spot", "perpetual", "futures", "margin"

    -- Quote currency
    quote_currency VARCHAR(10),                 -- "USDT", "USDC", "USD"

    -- Exchange-specific details
    min_order_value NUMERIC(20, 8),            -- Minimum order in quote currency
    min_quantity NUMERIC(20, 8),               -- Minimum order in base currency
    price_precision INTEGER,                    -- Decimal places for price
    quantity_precision INTEGER,                 -- Decimal places for quantity

    -- Trading details
    maker_fee NUMERIC(10, 6),                  -- 0.001 = 0.1%
    taker_fee NUMERIC(10, 6),
    liquidation_fee NUMERIC(10, 6),            -- For perpetuals only

    -- Liquidity
    average_spread_bps INTEGER,                -- Average bid-ask spread in basis points
    average_daily_volume_usd NUMERIC(30, 2),

    -- WebSocket availability
    has_websocket BOOLEAN,
    websocket_orderbook_precision INTEGER,     -- How many price levels
    orderbook_update_speed_ms INTEGER,         -- Typical update latency

    -- Status
    is_active BOOLEAN DEFAULT TRUE,
    last_verified TIMESTAMPTZ,

    UNIQUE(exchange, exchange_symbol)
);

-- Examples
INSERT INTO exchange_symbol_mapping VALUES
-- SOL on Binance Spot
(DEFAULT, (SELECT id FROM crypto_master_coins WHERE canonical_symbol = 'SOL'),
 'binance', 'SOLUSDT', 'spot', 'USDT', 10, 0.01, 8, 2, 0.0001, 0.001, NULL, 5, 1000000, TRUE, 20, 50, TRUE, NOW()),

-- SOL on Bybit Perpetual
(DEFAULT, (SELECT id FROM crypto_master_coins WHERE canonical_symbol = 'SOL'),
 'bybit', 'SOLUSDT', 'perpetual', 'USDT', 10, 1, 2, 4, 0.0001, 0.0003, 0.005, 2, 500000, TRUE, 500, 10, TRUE, NOW()),

-- SOL on OKX Swap
(DEFAULT, (SELECT id FROM crypto_master_coins WHERE canonical_symbol = 'SOL'),
 'okx', 'SOL-USDT', 'perpetual', 'USDT', 5, 0.1, 2, 1, 0.0002, 0.0004, 0.005, 1, 300000, TRUE, 400, 5, TRUE, NOW()),

-- SOL on Coinbase
(DEFAULT, (SELECT id FROM crypto_master_coins WHERE canonical_symbol = 'SOL'),
 'coinbase', 'SOL-USDT', 'spot', 'USDT', 25, 0.001, 2, 3, 0.004, 0.006, NULL, 8, 800000, FALSE, NULL, 200, FALSE, NOW()),

-- BTC on Kraken
(DEFAULT, (SELECT id FROM crypto_master_coins WHERE canonical_symbol = 'BTC'),
 'kraken', 'XBTUSDT', 'spot', 'USDT', 100, 0.00001, 2, 5, 0.0016, 0.0026, NULL, 3, 2000000, FALSE, NULL, 500, FALSE, NOW()),
;

CREATE INDEX ON exchange_symbol_mapping (exchange, exchange_symbol);
CREATE INDEX ON exchange_symbol_mapping (crypto_id, exchange);
CREATE INDEX ON exchange_symbol_mapping (product_type);
```

---

## 🔍 Quote Currency Standardization

Different exchanges use different stablecoins:

```sql
CREATE TABLE stablecoin_mapping (
    id BIGSERIAL PRIMARY KEY,

    canonical_quote VARCHAR(10),    -- "USDT", "USDC", "BUSD", "USDD"
    exchange VARCHAR(50),
    exchange_quote VARCHAR(10),     -- What exchange calls it

    -- Conversion rate to canonical (for normalized comparisons)
    conversion_rate NUMERIC(10, 8) DEFAULT 1.0,  -- How many USDT = 1 unit

    -- Quote details
    blockchain VARCHAR(50),         -- "ethereum", "solana", "binance-smart-chain"
    contract_address VARCHAR(100),

    -- Liquidity and slippage
    average_slippage_bps NUMERIC(10, 4),  -- 10 bps = 0.1% slippage

    UNIQUE(canonical_quote, exchange, exchange_quote)
);

INSERT INTO stablecoin_mapping VALUES
-- USDT variations
('USDT', 'binance', 'USDT', 1.0, 'ethereum', '0xdac17f958d2ee523a2206206994597c13d831ec7', 2),
('USDT', 'binance', 'BUSDT', 1.0, 'binance-smart-chain', '0x55d398326f99059fF775485246999027B3197955', 1),
('USDT', 'bybit', 'USDT', 1.0, 'ethereum', '0xdac17f958d2ee523a2206206994597c13d831ec7', 2),

-- USDC variations
('USDC', 'coinbase', 'USDC', 1.0, 'ethereum', '0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48', 2),
('USDC', 'coinbase', 'USDC-SOL', 0.999, 'solana', 'EPjFWaJrnUguWMKY3ursJMnZMEVmYkSn9KVDnUzRN5g8', 3),

-- USD (fiat-backed stablecoins)
('USD', 'coinbase', 'USD', 1.0, 'fiat', NULL, 1),

-- BUSD (deprecated, but still traded)
('BUSD', 'binance', 'BUSD', 0.998, 'ethereum', '0x4fabb145d64652a948d72533023f6e7a623c7c53', 5),
;
```

---

## 📋 Complete Exchange Symbol Reference

### Binance Spot Symbols (Top 20)

| Canonical | Binance Symbol | Min Order | Fees |
|-----------|----------------|-----------|------|
| BTC-USDT | BTCUSDT | 10 USDT | 0.1% |
| ETH-USDT | ETHUSDT | 10 USDT | 0.1% |
| SOL-USDT | SOLUSDT | 10 USDT | 0.1% |
| BNB-USDT | BNBUSDT | 10 USDT | 0.1% |
| XRP-USDT | XRPUSDT | 10 USDT | 0.1% |
| ADA-USDT | ADAUSDT | 10 USDT | 0.1% |
| DOGE-USDT | DOGEUSDT | 10 USDT | 0.1% |
| AVAX-USDT | AVAXUSDT | 10 USDT | 0.1% |
| LINK-USDT | LINKUSDT | 10 USDT | 0.1% |
| MATIC-USDT | MATICUSDT | 10 USDT | 0.1% |

### Bybit Perpetuals (Top 20)

| Canonical | Bybit Symbol | Min Order | Leverage | Fees |
|-----------|--------------|-----------|----------|------|
| BTC-USDT-PERP | BTCUSDT | 10 USDT | 1-125x | 0.02/0.06% |
| ETH-USDT-PERP | ETHUSDT | 10 USDT | 1-100x | 0.01/0.05% |
| SOL-USDT-PERP | SOLUSDT | 10 USDT | 1-20x | 0.02/0.06% |
| DOGE-USDT-PERP | DOGEUSDT | 10 USDT | 1-20x | 0.02/0.06% |
| ARB-USDT-PERP | ARBUSDT | 10 USDT | 1-10x | 0.01/0.05% |

### OKX Swaps (Top 20)

| Canonical | OKX Symbol | Min Order | Leverage | Fees |
|-----------|-----------|-----------|----------|------|
| BTC-USDT-PERP | BTC-USDT-SWAP | 5 USDT | 1-125x | 0.02/0.05% |
| ETH-USDT-PERP | ETH-USDT-SWAP | 5 USDT | 1-100x | 0.02/0.05% |
| SOL-USDT-PERP | SOL-USDT-SWAP | 5 USDT | 1-50x | 0.02/0.05% |

### Coinbase Pro (Spot Only)

| Canonical | Coinbase Symbol | Min Order | Fees |
|-----------|-----------------|-----------|------|
| BTC-USD | BTC-USD | 10 USD | 0.4-0.6% |
| ETH-USD | ETH-USD | 10 USD | 0.4-0.6% |
| SOL-USD | SOL-USD | 25 USD | 0.4-0.6% |

---

## 🔗 Symbol Resolver (Rust Implementation)

```rust
pub struct SymbolResolver {
    mapping_cache: Arc<RwLock<HashMap<String, ExchangeSymbol>>>,
    db: SupabaseClient,
}

impl SymbolResolver {
    pub async fn resolve_canonical_to_exchange(
        &self,
        canonical: &str,          // "SOL-USDT"
        exchange: &str,           // "binance"
        product_type: &str,       // "spot" or "perpetual"
    ) -> Result<String> {
        // Query: canonical_symbol = "SOL", exchange = "binance", product_type = "spot"
        // Returns: "SOLUSDT"

        let query = format!(
            r#"
            SELECT exchange_symbol FROM exchange_symbol_mapping
            WHERE crypto_id = (SELECT id FROM crypto_master_coins WHERE canonical_symbol = '{}')
                AND exchange = '{}'
                AND product_type = '{}'
            LIMIT 1
            "#,
            canonical.split('-').next().unwrap(),  // Extract "SOL" from "SOL-USDT"
            exchange,
            product_type,
        );

        let result = self.db.query(&query).await?;
        Ok(result[0]["exchange_symbol"].as_str().unwrap().to_string())
    }

    pub async fn resolve_exchange_to_canonical(
        &self,
        exchange_symbol: &str,    // "SOLUSDT"
        exchange: &str,           // "binance"
    ) -> Result<String> {
        // Query: exchange_symbol = "SOLUSDT", exchange = "binance"
        // Returns: "SOL" (canonical)

        let query = format!(
            r#"
            SELECT c.canonical_symbol
            FROM exchange_symbol_mapping esm
            JOIN crypto_master_coins c ON esm.crypto_id = c.id
            WHERE esm.exchange_symbol = '{}' AND esm.exchange = '{}'
            LIMIT 1
            "#,
            exchange_symbol,
            exchange,
        );

        let result = self.db.query(&query).await?;
        Ok(result[0]["canonical_symbol"].as_str().unwrap().to_string())
    }

    pub async fn get_best_trading_pair(
        &self,
        symbol: &str,                    // "SOL"
        base_exchange: &str,            // "binance"
        quote_currency: &str,           // "USDT"
    ) -> Result<Option<ExchangeSymbol>> {
        // Find which exchange has best liquidity for SOL-USDT trading
        // Return: ("bybit", "SOLUSDT", 1.5 bps spread, 500M 24h vol)

        let query = format!(
            r#"
            SELECT esm.exchange, esm.exchange_symbol, esm.average_spread_bps, esm.average_daily_volume_usd
            FROM exchange_symbol_mapping esm
            JOIN crypto_master_coins c ON esm.crypto_id = c.id
            WHERE c.canonical_symbol = '{}'
                AND esm.quote_currency = '{}'
                AND esm.is_active = TRUE
            ORDER BY esm.average_daily_volume_usd DESC
            LIMIT 1
            "#,
            symbol,
            quote_currency,
        );

        let result = self.db.query(&query).await?;

        if result.is_empty() {
            return Ok(None);
        }

        Ok(Some(ExchangeSymbol {
            exchange: result[0]["exchange"].as_str().unwrap().to_string(),
            symbol: result[0]["exchange_symbol"].as_str().unwrap().to_string(),
            spread_bps: result[0]["average_spread_bps"].as_i64().unwrap(),
            volume_24h: result[0]["average_daily_volume_usd"].as_f64().unwrap(),
        }))
    }
}

// Usage
let resolver = SymbolResolver::new(db);

// Get Binance symbol for SOL spot trading
let binance_symbol = resolver
    .resolve_canonical_to_exchange("SOL-USDT", "binance", "spot")
    .await?;  // Returns "SOLUSDT"

// Get canonical name for exchange symbol
let canonical = resolver
    .resolve_exchange_to_canonical("SOLUSDT", "binance")
    .await?;  // Returns "SOL"

// Find best exchange for SOL-USDT trading
let best = resolver
    .get_best_trading_pair("SOL", "binance", "USDT")
    .await?;  // Returns best liquidity exchange
// Result: ("bybit", "SOLUSDT", 2 bps, $500M volume)
```

---

## 📊 Coin Metadata Schema

```sql
CREATE TABLE coin_metadata (
    id BIGSERIAL PRIMARY KEY,

    crypto_id BIGINT REFERENCES crypto_master_coins(id),

    -- Price information (real-time)
    current_price_usd NUMERIC(20, 8),
    price_24h_high NUMERIC(20, 8),
    price_24h_low NUMERIC(20, 8),
    price_7d_high NUMERIC(20, 8),
    price_7d_low NUMERIC(20, 8),
    price_all_time_high NUMERIC(20, 8),
    price_all_time_low NUMERIC(20, 8),

    -- Volume data
    volume_24h_usd NUMERIC(30, 2),
    volume_7d_usd NUMERIC(30, 2),

    -- Changes
    change_1h NUMERIC(10, 4),        -- -2.5 = -2.5%
    change_24h NUMERIC(10, 4),
    change_7d NUMERIC(10, 4),
    change_30d NUMERIC(10, 4),

    -- Supply
    circulating_supply NUMERIC(30, 8),
    total_supply NUMERIC(30, 8),
    max_supply NUMERIC(30, 8),

    -- Dominance
    market_cap_dominance NUMERIC(10, 4),  -- 0-100, % of total crypto market

    -- Community
    github_commits_30d INTEGER,
    github_stars INTEGER,
    twitter_followers INTEGER,
    subreddit_subscribers INTEGER,

    -- Sentiment
    social_volume_24h INTEGER,
    news_sentiment NUMERIC(5, 2),         -- -1.0 (very negative) to +1.0 (very positive)

    -- On-chain
    active_addresses_24h BIGINT,
    transaction_volume_24h NUMERIC(30, 2),
    whale_transactions_24h INTEGER,

    -- Updated timestamp
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX ON coin_metadata (crypto_id);
CREATE INDEX ON coin_metadata (updated_at DESC);
CREATE INDEX ON coin_metadata (market_cap_dominance DESC);
```

---

## 🔄 Data Collection & Refresh Schedules

```rust
pub struct MetadataRefreshScheduler {
    db: SupabaseClient,
}

impl MetadataRefreshScheduler {
    pub async fn start_metadata_updates(&self) {
        // Refresh schedule:
        // - Prices: Every 5 seconds (real-time)
        // - 24h volumes: Every 1 minute
        // - Supply/market cap: Every 5 minutes
        // - Community metrics: Every hour
        // - GitHub/development: Every 6 hours

        tokio::spawn(self.update_prices_realtime());
        tokio::spawn(self.update_volumes_1min());
        tokio::spawn(self.update_supply_5min());
        tokio::spawn(self.update_community_1hour());
        tokio::spawn(self.update_development_6hour());
    }

    async fn update_prices_realtime(&self) {
        loop {
            // Fetch from multiple price sources
            let prices = futures::future::join_all(vec![
                self.fetch_binance_prices(),
                self.fetch_coinbase_prices(),
                self.fetch_coingecko_prices(),
            ]).await;

            // Average prices from multiple sources
            // Update database

            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    async fn update_volumes_1min(&self) {
        loop {
            // Fetch 24h volumes from all exchanges
            // Aggregate and store

            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    }

    // ... similar for other refresh intervals
}
```

---

## ✅ Implementation Checklist

- [ ] Create crypto_master_coins table with top 100 coins
- [ ] Create exchange_symbol_mapping table with all exchange symbols
- [ ] Create stablecoin_mapping for quote currency standardization
- [ ] Implement SymbolResolver for canonical ↔ exchange conversions
- [ ] Populate symbol mappings for all supported coins
- [ ] Create coin_metadata table
- [ ] Implement metadata refresh scheduler
- [ ] Build symbol lookup API endpoints
- [ ] Create monitoring for symbol consistency
- [ ] Test symbol resolution across all exchanges
- [ ] Document symbol conventions for team
- [ ] Set up alerts for missing symbols

---

## 📋 Quick Reference: Most Traded Symbols

### Top 20 by Volume

| # | Canonical | Binance | Bybit | OKX | Coinbase | Status |
|---|-----------|---------|-------|-----|----------|--------|
| 1 | BTC-USDT | BTCUSDT | BTCUSDT | BTC-USDT | BTC-USD | ✅ |
| 2 | ETH-USDT | ETHUSDT | ETHUSDT | ETH-USDT | ETH-USD | ✅ |
| 3 | SOL-USDT | SOLUSDT | SOLUSDT | SOL-USDT | SOL-USD | ✅ |
| 4 | DOGE-USDT | DOGEUSDT | DOGEUSDT | DOGE-USDT | DOGE-USD | ✅ |
| 5 | ADA-USDT | ADAUSDT | ADAUSDT | ADA-USDT | ❌ | ✅ |
| 6 | LINK-USDT | LINKUSDT | LINKUSDT | LINK-USDT | LINK-USD | ✅ |
| 7 | MATIC-USDT | MATICUSDT | MATICUSDT | MATIC-USDT | ❌ | ✅ |
| 8 | XRP-USDT | XRPUSDT | XRPUSDT | XRP-USDT | XRP-USD | ✅ |
| 9 | AVAX-USDT | AVAXUSDT | AVAXUSDT | AVAX-USDT | ❌ | ✅ |
| 10 | ARB-USDT | ARBUSDT | ARBUSDT | ARB-USDT | ❌ | ✅ |

---

**Status:** ✅ Crypto data dictionary fully specified
**Next:** Production implementation of symbol resolver and metadata system

