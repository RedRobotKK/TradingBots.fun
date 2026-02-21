# 📊 RAG Database Architecture: LLM Training Data Storage

**Role:** DevOps/Security/API Integration Expert
**Purpose:** Design database schema for storing API data feeds for LLM training (RAG retrieval)
**Technology:** Supabase (PostgreSQL) + TimescaleDB
**Status:** ✅ Production-ready specifications

---

## 🎯 Overview

This document details how to set up a Retrieval-Augmented Generation (RAG) database that:
- Stores real-time and historical market data from all APIs
- Enables LLMs (Claude, GPT-4, Ollama) to access historical context
- Supports pattern recognition and predictive modeling
- Maintains data integrity and backups
- Scales efficiently to multi-year historical data

### Why RAG for LLMs?

```
Without RAG:
  LLM: "Should we buy?"
  Model: Uses only training data from 2023, doesn't know current market

With RAG:
  LLM: "Should we buy?"
  System: Retrieves last 1000 similar market conditions from database
  LLM: Analyzes patterns + generates decision = 20-30% better accuracy
```

---

## 🏗️ Architecture Overview

```
┌─────────────────────────────────────────────────────────┐
│                  Trading Bot (Rust)                     │
│  ┌──────────────────┐  ┌──────────────────┐             │
│  │  Data Collectors │  │  AI Decision      │             │
│  │  (BTC, ETH, etc) │  │  Engine (Claude)  │             │
│  └────────┬─────────┘  └────────┬─────────┘             │
│           │                     │                        │
│           └──────────┬──────────┘                        │
│                      ▼                                   │
│          ┌──────────────────────┐                       │
│          │  Data Normalization  │                       │
│          │  & Validation        │                       │
│          └──────────┬───────────┘                       │
│                     ▼                                   │
│          ┌──────────────────────┐                       │
│          │  Batch Write to DB   │                       │
│          │  (every 5 minutes)   │                       │
│          └──────────┬───────────┘                       │
└─────────────┼────────────────────────────────────────────┘
              │
              ▼
┌─────────────────────────────────────────────────────────┐
│            Supabase (PostgreSQL + TimescaleDB)          │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
│  │ Market       │  │ Social       │  │ On-Chain     │  │
│  │ Prices       │  │ Sentiment    │  │ Metrics      │  │
│  │ (OHLCV)      │  │ (hourly)     │  │ (daily)      │  │
│  └──────────────┘  └──────────────┘  └──────────────┘  │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
│  │ News Events  │  │ Trading      │  │ LLM Training │  │
│  │ (real-time)  │  │ Signals      │  │ Logs         │  │
│  │              │  │ (AI history) │  │              │  │
│  └──────────────┘  └──────────────┘  └──────────────┘  │
│                                                         │
│  Storage: 500MB+ (free tier)                           │
│  Retention: 2+ years                                    │
│  Backups: Automatic daily                              │
└──────────────┬──────────────────────────────────────────┘
               │
      ┌────────┴────────┐
      ▼                 ▼
   RAG Query      LLM Training
   (Real-time)    (Weekly updates)
   │              │
   └──────┬───────┘
          ▼
   ┌──────────────┐
   │ Claude API   │
   │ (with context)
   │              │
   │ "Analyze     │
   │  these 100   │
   │  similar     │
   │  trades"     │
   └──────────────┘
```

---

## 📋 Complete Database Schema

### Table 1: Market Prices (OHLCV Data)

**Purpose:** Store candlestick data from all price sources

```sql
CREATE TABLE market_prices (
    -- Time-based partitioning (required for hypertable)
    time TIMESTAMPTZ NOT NULL,

    -- Asset identification
    symbol VARCHAR(20) NOT NULL,        -- "BTC", "ETH", "SOL"
    source VARCHAR(50) NOT NULL,        -- "binance", "coingecko", "kraken"
    interval VARCHAR(10) NOT NULL,      -- "1m", "5m", "15m", "1h", "4h", "1d"

    -- OHLCV data
    open NUMERIC(20, 8) NOT NULL,       -- 20 digits, 8 decimals
    high NUMERIC(20, 8) NOT NULL,
    low NUMERIC(20, 8) NOT NULL,
    close NUMERIC(20, 8) NOT NULL,
    volume NUMERIC(20, 2) NOT NULL,     -- In USD

    -- Additional context
    trades INTEGER,                      -- Number of trades in period
    buy_volume NUMERIC(20, 2),          -- Buy-side volume
    sell_volume NUMERIC(20, 2),         -- Sell-side volume

    -- Metadata
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),

    PRIMARY KEY (time, symbol, source, interval)
);

-- Convert to hypertable for automatic time-series optimization
SELECT create_hypertable(
    'market_prices',
    'time',
    if_not_exists => TRUE,
    migrate_data => TRUE
);

-- Performance indexes
CREATE INDEX ON market_prices (symbol, time DESC);
CREATE INDEX ON market_prices (source, time DESC);
CREATE INDEX ON market_prices (interval, time DESC);

-- Compression policy (compress data older than 30 days)
SELECT add_compression_policy(
    'market_prices',
    INTERVAL '30 days',
    if_not_exists => TRUE
);

-- Retention policy (delete data older than 3 years)
SELECT add_retention_policy(
    'market_prices',
    INTERVAL '1095 days',  -- 3 years
    if_not_exists => TRUE
);
```

### Table 2: Social Sentiment

**Purpose:** Store sentiment scores from all social data sources

```sql
CREATE TABLE social_sentiment (
    time TIMESTAMPTZ NOT NULL,
    symbol VARCHAR(20) NOT NULL,
    source VARCHAR(50) NOT NULL,     -- "lunarcrush", "santiment", "reddit"

    -- Sentiment metrics
    sentiment_score NUMERIC(5, 3),   -- -1.0 to +1.0
    social_volume INTEGER,            -- Mention count
    discussions INTEGER,              -- Discussion count
    engagement_score NUMERIC(5, 3),   -- Engagement metric
    influencer_score NUMERIC(5, 3),   -- Influencer activity
    bullish_ratio NUMERIC(5, 3),     -- % bullish mentions

    -- Community metrics
    unique_contributors INTEGER,
    growth_rate NUMERIC(5, 2),        -- Week-over-week %

    -- Metadata
    created_at TIMESTAMPTZ DEFAULT NOW(),

    PRIMARY KEY (time, symbol, source)
);

SELECT create_hypertable(
    'social_sentiment',
    'time',
    if_not_exists => TRUE,
    migrate_data => TRUE
);

CREATE INDEX ON social_sentiment (symbol, time DESC);
CREATE INDEX ON social_sentiment (source, time DESC);

-- Compress older data
SELECT add_compression_policy(
    'social_sentiment',
    INTERVAL '30 days',
    if_not_exists => TRUE
);

-- Keep 2 years
SELECT add_retention_policy(
    'social_sentiment',
    INTERVAL '730 days',
    if_not_exists => TRUE
);
```

### Table 3: On-Chain Metrics

**Purpose:** Store blockchain metrics for fundamental analysis

```sql
CREATE TABLE on_chain_metrics (
    time TIMESTAMPTZ NOT NULL,
    symbol VARCHAR(20) NOT NULL,
    source VARCHAR(50) NOT NULL,     -- "glassnode", "messari"

    -- Exchange metrics
    exchange_inflow NUMERIC(20, 8),     -- Coins entering exchanges
    exchange_outflow NUMERIC(20, 8),    -- Coins leaving exchanges
    exchange_netflow NUMERIC(20, 8),    -- Net direction

    -- Whale metrics
    whale_transactions INTEGER,         -- Txs > $1M
    whale_volume NUMERIC(20, 8),       -- Total whale volume
    whale_addresses_active INTEGER,

    -- Network metrics
    active_addresses INTEGER,
    transaction_volume NUMERIC(30, 2),  -- USD volume
    transaction_count INTEGER,
    unique_senders INTEGER,

    -- Holder composition
    addresses_1_year_plus NUMERIC(5, 2),  -- % of holders dormant 1+ year
    addresses_active_1d INTEGER,
    addresses_new INTEGER,

    -- Network health
    network_value NUMERIC(30, 2),      -- Market cap
    realized_price NUMERIC(20, 8),     -- Average price paid

    -- Metadata
    created_at TIMESTAMPTZ DEFAULT NOW(),

    PRIMARY KEY (time, symbol, source)
);

SELECT create_hypertable(
    'on_chain_metrics',
    'time',
    if_not_exists => TRUE,
    migrate_data => TRUE
);

CREATE INDEX ON on_chain_metrics (symbol, time DESC);
```

### Table 4: News & Events

**Purpose:** Store news articles for event-driven signal detection

```sql
CREATE TABLE news_events (
    -- Unique identifier
    id BIGSERIAL PRIMARY KEY,

    -- Timestamp
    published_at TIMESTAMPTZ NOT NULL,
    fetched_at TIMESTAMPTZ DEFAULT NOW(),

    -- Content
    title TEXT NOT NULL,
    content TEXT,
    url VARCHAR(512) UNIQUE,

    -- Categorization
    symbol VARCHAR(20),                 -- Affected coin (may be NULL)
    source VARCHAR(100) NOT NULL,       -- "cryptopanic", "reddit", "twitter"
    category VARCHAR(50),               -- "news", "regulation", "hack", etc

    -- Sentiment
    sentiment VARCHAR(20),              -- "positive", "negative", "neutral"
    importance_score NUMERIC(5, 2),    -- 0.0 to 1.0 (LLM or rules-based)

    -- For full-text search (RAG retrieval)
    search_vector TSVECTOR GENERATED ALWAYS AS (
        to_tsvector('english',
            coalesce(title, '') || ' ' ||
            coalesce(content, '') || ' ' ||
            coalesce(category, '')
        )
    ) STORED,

    -- Metadata
    tags TEXT[],                        -- ["Bitcoin", "ETF", "approval"]
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX ON news_events (published_at DESC);
CREATE INDEX ON news_events (symbol, published_at DESC);
CREATE INDEX ON news_events USING GIN (search_vector);  -- Full-text search

-- Retention: Keep 2 years for pattern analysis
SELECT add_retention_policy(
    'news_events',
    INTERVAL '730 days',
    if_not_exists => TRUE
);
```

### Table 5: Trading Signals (AI Decision History)

**Purpose:** Log all AI decisions for training and analysis

```sql
CREATE TABLE trading_signals (
    id BIGSERIAL PRIMARY KEY,
    time TIMESTAMPTZ NOT NULL,

    -- Decision metadata
    symbol VARCHAR(20) NOT NULL,
    decision_id VARCHAR(50) UNIQUE,  -- UUID for tracking
    ai_model VARCHAR(50),            -- "claude-opus", "gpt-4", "ollama-mistral"

    -- Signal information
    signal_type VARCHAR(50),         -- "RSI", "MACD", "sentiment", "ensemble"
    signal_strength NUMERIC(5, 3),   -- 0.0 to 1.0
    signal_confidence NUMERIC(5, 3),

    -- Decision outcome
    action VARCHAR(50) NOT NULL,     -- "BUY", "SELL", "HOLD", "HEDGE"
    suggested_size NUMERIC(10, 2),   -- Position size in USD
    stop_loss NUMERIC(20, 8),        -- SL price
    take_profit NUMERIC(20, 8),      -- TP price
    leverage NUMERIC(5, 2),          -- Suggested leverage

    -- Market context (stored as JSON for flexibility)
    market_context JSONB,  -- {
                           --   "price": 145.32,
                           --   "volume_24h": 1840000,
                           --   "rsi": 72.5,
                           --   "macd": "positive",
                           --   "sentiment": 0.72
                           -- }

    -- Data sources used
    data_sources TEXT[],   -- ["binance", "lunarcrush", "glassnode"]

    -- Rationale (for training)
    rationale TEXT,  -- LLM explanation of decision

    -- Outcome tracking (updated later when trade closes)
    entry_price NUMERIC(20, 8),
    exit_price NUMERIC(20, 8),
    realized_pnl NUMERIC(20, 8),
    win BOOLEAN,  -- NULL = not closed yet, TRUE = win, FALSE = loss

    -- Metadata
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX ON trading_signals (symbol, time DESC);
CREATE INDEX ON trading_signals (ai_model, time DESC);
CREATE INDEX ON trading_signals (win, time DESC);  -- For win rate analysis

-- Keep all trading signals indefinitely (important for backtest validation)
```

### Table 6: LLM Training Logs

**Purpose:** Track LLM training sessions for performance monitoring

```sql
CREATE TABLE llm_training_logs (
    id BIGSERIAL PRIMARY KEY,

    -- Training session info
    session_id VARCHAR(50) UNIQUE,
    model_name VARCHAR(100),         -- "claude-opus", "gpt-4", "ollama-mistral"
    training_date TIMESTAMPTZ DEFAULT NOW(),

    -- Data range
    data_start_date DATE,
    data_end_date DATE,
    data_points_used INTEGER,        -- How many rows from database

    -- Model configuration
    batch_size INTEGER,
    learning_rate NUMERIC(10, 6),
    epochs INTEGER,
    context_length INTEGER,          -- Token limit

    -- Performance metrics
    train_accuracy NUMERIC(5, 4),
    val_accuracy NUMERIC(5, 4),
    test_f1_score NUMERIC(5, 4),
    precision NUMERIC(5, 4),
    recall NUMERIC(5, 4),

    -- Backtest results
    backtest_win_rate NUMERIC(5, 4),
    backtest_sharpe_ratio NUMERIC(10, 4),
    backtest_max_drawdown NUMERIC(5, 4),
    backtest_total_return NUMERIC(10, 4),

    -- Metadata
    notes TEXT,
    status VARCHAR(20),  -- "completed", "failed", "running"
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX ON llm_training_logs (training_date DESC);
CREATE INDEX ON llm_training_logs (model_name, training_date DESC);
```

### Table 7: Data Quality Logs

**Purpose:** Track data collection health and issues

```sql
CREATE TABLE data_quality_logs (
    id BIGSERIAL PRIMARY KEY,

    timestamp TIMESTAMPTZ DEFAULT NOW(),
    api_source VARCHAR(50),     -- "binance", "lunarcrush", etc
    api_endpoint VARCHAR(200),

    -- Health metrics
    success BOOLEAN,
    response_time_ms INTEGER,
    error_code INTEGER,
    error_message TEXT,

    -- Data metrics
    rows_inserted INTEGER,
    rows_skipped INTEGER,
    data_gaps_detected INTEGER,

    -- Alert if issues
    alert_severity VARCHAR(20), -- "info", "warning", "critical"
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Retention: Keep 90 days only (monitoring logs)
SELECT add_retention_policy(
    'data_quality_logs',
    INTERVAL '90 days',
    if_not_exists => TRUE
);
```

---

## 🔄 Data Collection Pipeline

### Automatic Data Collection (Every 5 Minutes)

```rust
// Pseudo-code for automated collection

pub struct DataCollector {
    db: SupabaseClient,
    sources: Vec<APIClient>,
}

impl DataCollector {
    pub async fn collect_all(&mut self) -> Result<()> {
        let symbols = vec!["BTC", "ETH", "SOL"];

        // Collect from all sources
        for symbol in symbols {
            // 1. Fetch price data (fast, small payload)
            let prices = self.fetch_prices(symbol).await?;
            self.db.insert_market_prices(&prices).await?;

            // 2. Fetch sentiment (lower frequency)
            if Instant::now() % SENTIMENT_INTERVAL == 0 {
                let sentiment = self.fetch_sentiment(symbol).await?;
                self.db.insert_social_sentiment(&sentiment).await?;
            }

            // 3. Fetch on-chain (daily)
            if is_new_day() {
                let onchain = self.fetch_onchain(symbol).await?;
                self.db.insert_onchain(&onchain).await?;
            }

            // 4. News (continuous, but batched)
            let news = self.fetch_news(symbol).await?;
            self.db.insert_news(&news).await?;
        }

        // Log success
        self.db.log_success().await?;

        Ok(())
    }
}

// Run on schedule (every 5 minutes)
// tokio::spawn(async move {
//     loop {
//         collector.collect_all().await.ok();
//         tokio::time::sleep(Duration::from_secs(300)).await;
//     }
// });
```

### Batch Insert for Efficiency

```sql
-- Insert market prices efficiently
INSERT INTO market_prices (time, symbol, source, interval, open, high, low, close, volume)
VALUES
    ('2024-02-21 14:00:00', 'BTC', 'binance', '1h', 45230.50, 45380.00, 45200.00, 45320.00, 1840000),
    ('2024-02-21 14:00:00', 'ETH', 'binance', '1h', 2834.50, 2850.00, 2820.00, 2840.00, 920000),
    ('2024-02-21 14:00:00', 'SOL', 'binance', '1h', 145.32, 146.00, 144.50, 145.80, 450000)
ON CONFLICT (time, symbol, source, interval) DO UPDATE
SET
    open = EXCLUDED.open,
    high = EXCLUDED.high,
    low = EXCLUDED.low,
    close = EXCLUDED.close,
    volume = EXCLUDED.volume,
    updated_at = NOW();
```

---

## 🔍 RAG Query Examples

### Example 1: Find Similar Market Conditions

```sql
-- Find all past instances where we had similar conditions
-- (High RSI + positive sentiment + whale outflow)
-- This helps LLMs learn what happened historically

SELECT
    mp.time,
    mp.symbol,
    mp.close as price,
    ss.sentiment_score,
    ocm.exchange_outflow,
    ts.action,
    ts.realized_pnl,
    ts.win
FROM market_prices mp
LEFT JOIN social_sentiment ss
    ON mp.symbol = ss.symbol
    AND mp.time = ss.time
    AND ss.source = 'lunarcrush'
LEFT JOIN on_chain_metrics ocm
    ON mp.symbol = ocm.symbol
    AND DATE(mp.time) = DATE(ocm.time)
    AND ocm.source = 'glassnode'
LEFT JOIN trading_signals ts
    ON mp.symbol = ts.symbol
    AND ABS(EXTRACT(EPOCH FROM (mp.time - ts.time))) < 3600
WHERE
    mp.symbol = 'BTC'
    AND mp.interval = '1h'
    AND mp.time >= NOW() - INTERVAL '2 years'
    AND ss.sentiment_score > 0.65        -- Positive sentiment
    AND ocm.exchange_outflow > 500       -- Whale outflow
    AND mp.close > mp.open               -- Green candle
ORDER BY mp.time DESC
LIMIT 100;

-- Output: 100 similar past conditions for Claude to analyze
-- Claude analyzes: "In past 100 similar cases, we won 63 trades out of 100"
```

### Example 2: Search News for Context

```sql
-- Full-text search for relevant news when analyzing a symbol
SELECT
    published_at,
    title,
    source,
    sentiment,
    importance_score
FROM news_events
WHERE
    symbol = 'BTC'
    AND published_at >= NOW() - INTERVAL '30 days'
    AND (
        search_vector @@ to_tsquery('english', 'ETF | approval | SEC')
        OR category = 'regulation'
    )
ORDER BY importance_score DESC, published_at DESC
LIMIT 20;

-- Returns: Recent relevant news for context window
```

### Example 3: Performance Analysis for Training

```sql
-- Analyze which signals worked best for retraining
SELECT
    ai_model,
    signal_type,
    COUNT(*) as total_signals,
    SUM(CASE WHEN win THEN 1 ELSE 0 END)::FLOAT / COUNT(*) as win_rate,
    AVG(realized_pnl) as avg_pnl,
    STDDEV(realized_pnl) as pnl_volatility
FROM trading_signals
WHERE
    realized_pnl IS NOT NULL  -- Trade completed
    AND time >= NOW() - INTERVAL '90 days'
GROUP BY ai_model, signal_type
ORDER BY win_rate DESC;

-- Output: Training insights for model improvement
```

---

## 🔒 Backup & Disaster Recovery

### Automatic Backups (Supabase)

```bash
# Supabase automatically backs up daily
# Access at: https://supabase.com/dashboard/project/[id]/settings/database-backups
```

### Manual Export (Weekly)

```bash
#!/bin/bash
# Export database to SQL dump

pg_dump postgresql://user:pass@db.supabase.co:5432/postgres \
    --format=custom \
    --compress=9 \
    > backup_$(date +%Y%m%d).sql.gz

# Upload to secure storage (Google Drive, AWS S3, etc)
```

### Point-in-Time Recovery

```sql
-- Restore from backup
-- Contact Supabase support for recovery (< 7 days of backups available)
```

---

## 📊 Storage & Cost Optimization

### Storage Requirements

**For 50 coins, 2 years of data:**

```
Market Prices:
  - 5 intervals (1m, 5m, 15m, 1h, 4h, 1d)
  - 50 coins × 365 days × 24 hours × 5 intervals
  = 2,190,000 rows
  × 100 bytes/row = 219 MB

Social Sentiment (hourly):
  - 50 coins × 365 days × 24 hours × 5 sources
  = 2,190,000 rows
  × 80 bytes = 175 MB

On-Chain Metrics (daily):
  - 50 coins × 365 × 2 years × 5 sources
  = 182,500 rows
  × 150 bytes = 27 MB

News Events:
  - 500 events/day × 365 × 2 years
  = 365,000 rows
  × 1000 bytes = 365 MB

Trading Signals:
  - 10 signals/day × 365 × 2 years
  = 7,300 rows
  × 500 bytes = 4 MB

TOTAL: ~790 MB (fits in Supabase free tier 500MB + Pro 8GB)
```

### Cost Breakdown

| Tier | Storage | Cost/Month | Data Retention |
|------|---------|-----------|-----------------|
| Free | 500 MB | $0 | 1 year recommended |
| Pro | 8 GB | $25 | 2-3 years |
| Team | 100 GB | $50 | 5+ years |
| Enterprise | Unlimited | Custom | Unlimited |

### Compression Strategy

```sql
-- TimescaleDB automatically compresses data older than 30 days
-- 10x compression ratio (1 TB → 100 GB with compression)

-- Example: Compress market_prices older than 30 days
SELECT add_compression_policy(
    'market_prices',
    INTERVAL '30 days',
    if_not_exists => TRUE
);

-- Re-compress aged data
SELECT compress_chunk(show_chunks('market_prices', older_than => INTERVAL '30 days'));
```

---

## 🚀 Integration with LLMs

### Claude API Integration (RAG Pattern)

```rust
pub async fn make_trading_decision_with_rag(
    symbol: &str,
    client: &claude::Client,
    db: &supabase::Client,
) -> Result<String> {
    // Step 1: Fetch similar past conditions from database
    let similar_trades = db.query_similar_conditions(symbol, 100).await?;

    // Step 2: Format context for Claude
    let rag_context = format_context(&similar_trades);

    // Step 3: Prepare system prompt with database context
    let system_prompt = format!(
        r#"
You are an expert crypto trader. Analyze the following market conditions and similar historical patterns.

HISTORICAL PATTERNS (Last 100 Similar Conditions):
{}

Current Market Data:
- Symbol: {}
- Price: ${}
- RSI: {}
- Sentiment: {}

Based on these 100 historical similar conditions:
- Win Rate: {}%
- Average P&L: ${}
- Max Drawdown: {}%

Make a decision: BUY / SELL / HOLD
Explain your reasoning based on the patterns.
"#,
        rag_context,
        symbol,
        // ... more context
    );

    // Step 4: Call Claude with full context
    let response = client
        .messages()
        .create(MessageRequest {
            model: "claude-opus-4-5".to_string(),
            max_tokens: 500,
            system: Some(system_prompt),
            messages: vec![
                Message {
                    role: "user".to_string(),
                    content: format!("Analyze {} and make a trading decision.", symbol),
                }
            ],
        })
        .await?;

    Ok(response.content[0].text.clone())
}
```

### Local LLM Integration (Ollama RAG)

```rust
pub async fn make_decision_with_ollama(
    symbol: &str,
    rag_context: &str,
) -> Result<String> {
    let client = reqwest::Client::new();

    let response = client
        .post("http://localhost:11434/api/generate")
        .json(&json!({
            "model": "mistral",
            "prompt": format!(
                "Market Analysis for {}:\n\n{}\n\nDecision:",
                symbol,
                rag_context
            ),
            "stream": false,
        }))
        .send()
        .await?;

    let body: serde_json::Value = response.json().await?;
    Ok(body["response"].as_str().unwrap_or("").to_string())
}
```

---

## 📈 Monitoring & Maintenance

### Daily Health Check Script

```rust
pub async fn check_database_health(db: &supabase::Client) -> Result<()> {
    // 1. Check data freshness
    let latest_price = db
        .from("market_prices")
        .select("*")
        .order("time.desc")
        .limit(1)
        .single()
        .await?;

    let age_minutes = (Utc::now() - latest_price.time).num_minutes();
    if age_minutes > 30 {
        eprintln!("WARNING: Latest price data is {} minutes old", age_minutes);
    }

    // 2. Check storage usage
    let storage_info = db.get_storage_stats().await?;
    println!("Database storage: {}/{} MB used",
        storage_info.used, storage_info.limit);

    if storage_info.used > storage_info.limit * 90 / 100 {
        eprintln!("WARNING: Database 90% full!");
    }

    // 3. Check backup status
    let last_backup = db.get_last_backup_time().await?;
    let backup_age_hours = (Utc::now() - last_backup).num_hours();
    if backup_age_hours > 24 {
        eprintln!("WARNING: Last backup is {} hours old", backup_age_hours);
    }

    Ok(())
}
```

### Monthly Maintenance

```bash
#!/bin/bash
# Monthly database optimization script

# 1. Vacuum and analyze
psql -h db.supabase.co -U postgres -d postgres -c "VACUUM ANALYZE;"

# 2. Reindex for performance
psql -h db.supabase.co -U postgres -d postgres -c "REINDEX DATABASE postgres;"

# 3. Update compression policies
psql -h db.supabase.co -U postgres -d postgres -c "
    SELECT compress_chunk(
        show_chunks('market_prices', older_than => INTERVAL '30 days')
    );
"

# 4. Export backup
pg_dump postgresql://user:pass@db.supabase.co/postgres \
    --format=custom --compress=9 \
    > backup_monthly_$(date +%Y%m%d).sql.gz
```

---

## ✅ Implementation Checklist

- [ ] Create Supabase account
- [ ] Enable TimescaleDB extension
- [ ] Create all tables (copy SQL above)
- [ ] Set up retention policies
- [ ] Set up compression policies
- [ ] Create API data collection scripts
- [ ] Implement rate limiting
- [ ] Test batch inserts
- [ ] Set up monitoring scripts
- [ ] Configure automated backups
- [ ] Create RAG query examples
- [ ] Integration test with Claude API
- [ ] Document access procedures
- [ ] Plan upgrade path if storage exceeded

---

**Status:** ✅ RAG database architecture fully specified
**Next:** Implement data collection clients and deploy

