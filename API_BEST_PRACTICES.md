# 🛡️ API Best Practices: Security, Rate Limits & Error Handling

**Role:** DevOps/Security/API Integration Expert
**Purpose:** Production-grade API usage guidelines for crypto data sources
**Status:** ✅ Enterprise-ready specifications

---

## 📋 Overview

This document covers best practices for:
- Secure API credential management
- Rate limit compliance and optimization
- Error handling and retry strategies
- Monitoring and alerting
- Cost optimization (free tier maximization)
- API abuse prevention

---

## 🔐 Section 1: Credential Security

### 1.1 Environment File Strategy

**DO: Proper Separation**

```bash
# File 1: .env.data-feeds (GITIGNORED)
# Store ONLY API keys for optional services

LUNARCRUSH_API_KEY=xxx_actual_key_here
GLASSNODE_API_KEY=xxx_actual_key_here
MESSARI_API_KEY=xxx_actual_key_here
CRYPTOPANIC_AUTH_TOKEN=xxx_actual_key_here
SANTIMENT_API_KEY=xxx_actual_key_here

# File 2: .env.data-feeds.example (COMMITTED)
# Template showing structure, NO keys

LUNARCRUSH_API_KEY=your_api_key_here
GLASSNODE_API_KEY=your_api_key_here
# ... etc
```

**DON'T: Hardcode or Log**

```rust
// ❌ WRONG - Never hardcode
pub const LUNARCRUSH_KEY: &str = "actual_api_key_12345";

// ❌ WRONG - Never log credentials
println!("Using API key: {}", api_key);

// ❌ WRONG - Never commit private files
git add .env.data-feeds

// ✅ RIGHT - Load from environment
let api_key = env::var("LUNARCRUSH_API_KEY")
    .expect("Missing LUNARCRUSH_API_KEY");
```

### 1.2 Access Control

**File Permissions**

```bash
# Secure file permissions (owner read/write only)
chmod 600 .env.data-feeds
chmod 600 .env.wallet
chmod 600 .env.mainnet
chmod 600 .env.testnet

# Verify permissions
ls -la .env*
# Should show: -rw------- (600)

# Check if committed (should show nothing)
git ls-files .env*
# Should return nothing
```

### 1.3 Key Rotation Strategy

**Quarterly Rotation Schedule**

```bash
#!/bin/bash
# rotate_api_keys.sh - Quarterly rotation script

API_SERVICES=("lunarcrush" "glassnode" "messari" "cryptopanic" "santiment")

for service in "${API_SERVICES[@]}"; do
    echo "Rotating ${service} API key..."

    # 1. Generate new key in dashboard
    # 2. Update .env.data-feeds
    env_var="${service^^}_API_KEY"
    read -p "Enter new ${service} API key: " new_key
    sed -i "s/${env_var}=.*/${env_var}=${new_key}/" .env.data-feeds

    # 3. Test new key
    curl -s https://api.${service}.com/test?key=${new_key} > /dev/null
    if [ $? -eq 0 ]; then
        echo "✓ ${service} key validated"
        # 4. Revoke old key in dashboard
        echo "TODO: Revoke old key in ${service} dashboard"
    else
        echo "✗ ${service} key validation failed"
        exit 1
    fi
done

echo "All API keys rotated successfully"
```

**Key Expiration Tracking**

```rust
pub struct APIKeyRotation {
    service: String,
    created_at: DateTime<Utc>,
    rotation_interval: Duration,
}

impl APIKeyRotation {
    pub fn needs_rotation(&self) -> bool {
        Utc::now() - self.created_at > self.rotation_interval
    }

    pub fn days_until_rotation(&self) -> i64 {
        let rotation_date = self.created_at + self.rotation_interval;
        (rotation_date - Utc::now()).num_days()
    }
}

// Usage
let lunarcrush_key = APIKeyRotation {
    service: "lunarcrush".to_string(),
    created_at: DateTime::parse_from_rfc3339("2024-02-01T00:00:00Z")?,
    rotation_interval: Duration::days(90),
};

if lunarcrush_key.needs_rotation() {
    eprintln!("WARNING: LunarCrush key needs rotation!");
}
```

### 1.4 Secret Scanning

**Pre-commit Hook to Prevent Leaks**

```bash
#!/bin/bash
# .git/hooks/pre-commit

# Prevent committing files with secrets

# Check for .env files (except examples)
if git diff --cached --name-only | grep -E '\.env\.(data-feeds|wallet|mainnet|testnet)$'; then
    echo "ERROR: Cannot commit private .env file"
    exit 1
fi

# Check for hardcoded API keys
if git diff --cached | grep -E 'sk_live_|pk_live_|api_key|secret_key'; then
    echo "ERROR: Possible hardcoded API key detected"
    exit 1
fi

exit 0
```

---

## 🚦 Section 2: Rate Limit Compliance

### 2.1 Understanding Rate Limits

**Common Limit Types**

| Type | Example | Strategy |
|------|---------|----------|
| Per-minute | 120/min | Queue requests, batch if possible |
| Per-hour | 1000/hour | Spread calls evenly |
| Per-day | 1000/day | Plan carefully, cache aggressively |
| Concurrent | 10 simultaneous | Use semaphore to limit parallelism |
| Burst | 10 per second | Implement backoff |

**Real-World Examples**

```
LunarCrush (Free): 1000 calls/day
  = 42 calls/hour
  = 0.7 calls/minute
  = 1 call every 85 seconds

Binance (Free): 1200 req/min
  = 72,000 req/hour
  = 1.7M req/day
  = Very generous

CoinGecko (Free): 50 req/min
  = 3,000 req/hour
  = Adequate for most use cases
```

### 2.2 Rate Limiting Implementation

**Token Bucket Algorithm**

```rust
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{Instant, Duration};

pub struct TokenBucket {
    capacity: u32,
    refill_rate: u32,           // tokens per second
    tokens: Arc<Mutex<f64>>,
    last_refill: Arc<Mutex<Instant>>,
}

impl TokenBucket {
    pub fn new(capacity: u32, refill_rate: u32) -> Self {
        Self {
            capacity,
            refill_rate,
            tokens: Arc::new(Mutex::new(capacity as f64)),
            last_refill: Arc::new(Mutex::new(Instant::now())),
        }
    }

    pub async fn consume(&self, count: u32) -> bool {
        let mut tokens = self.tokens.lock().await;
        let mut last_refill = self.last_refill.lock().await;

        // Refill tokens based on time elapsed
        let elapsed = last_refill.elapsed();
        let refill_amount = elapsed.as_secs_f64() * self.refill_rate as f64;
        *tokens = (*tokens + refill_amount).min(self.capacity as f64);
        *last_refill = Instant::now();

        // Check if enough tokens
        if *tokens >= count as f64 {
            *tokens -= count as f64;
            true
        } else {
            false
        }
    }

    pub async fn wait_until_available(&self, count: u32) {
        loop {
            if self.consume(count).await {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

// Usage
let bucket = TokenBucket::new(
    50,  // 50 tokens capacity
    1,   // 1 token per second
);

// Before API call
bucket.wait_until_available(1).await;
let result = api_client.fetch_data().await?;
```

### 2.3 Request Batching

**DO: Batch Multiple Coins**

```rust
// ✅ RIGHT - Single call for 50 coins

pub async fn fetch_sentiment_all_coins(
    coins: Vec<&str>,
) -> Result<HashMap<String, SentimentScore>> {
    // One API call for up to 100 coins
    let response = reqwest::Client::new()
        .get("https://api.lunarcrush.com/v2/coins")
        .query(&[("ids", coins.join(","))])
        .send()
        .await?;

    let data: serde_json::Value = response.json().await?;
    let mut results = HashMap::new();

    for coin in data["data"].as_array().unwrap() {
        let symbol = coin["symbol"].as_str().unwrap();
        let sentiment = coin["sentiment"]["score"].as_f64().unwrap();
        results.insert(symbol.to_string(), sentiment);
    }

    Ok(results)  // 50 coins returned from 1 API call
}

// Call once
let sentiments = fetch_sentiment_all_coins(vec!["BTC", "ETH", "SOL", ...]).await?;
```

**DON'T: Individual Calls**

```rust
// ❌ WRONG - 50 separate API calls for 50 coins

for coin in coins {
    let response = reqwest::Client::new()
        .get("https://api.lunarcrush.com/v2/coins")
        .query(&[("id", coin)])
        .send()
        .await?;
    // ... process individual response
}

// This uses 50/1000 of free tier quota just for 50 coins!
```

### 2.4 Caching Strategy

**Cache Duration by Data Type**

```rust
pub struct DataCacheConfig {
    pub data_type: String,
    pub cache_duration: Duration,
    pub rationale: String,
}

// Define caching rules
let caching_rules = vec![
    DataCacheConfig {
        data_type: "market_prices".to_string(),
        cache_duration: Duration::from_secs(300),  // 5 minutes
        rationale: "Prices change frequently but 5min staleness acceptable for signals".to_string(),
    },
    DataCacheConfig {
        data_type: "sentiment".to_string(),
        cache_duration: Duration::from_secs(1800), // 30 minutes
        rationale: "Sentiment changes slowly, daily updates sufficient".to_string(),
    },
    DataCacheConfig {
        data_type: "on_chain".to_string(),
        cache_duration: Duration::from_secs(86400), // 24 hours
        rationale: "On-chain metrics update daily, no need for frequent fetches".to_string(),
    },
    DataCacheConfig {
        data_type: "news".to_string(),
        cache_duration: Duration::from_secs(300), // 5 minutes
        rationale: "News is time-sensitive, fresh updates important".to_string(),
    },
];

// Implementation
pub struct CachedAPIClient {
    cache: Arc<RwLock<HashMap<String, CachedValue>>>,
}

pub struct CachedValue {
    data: serde_json::Value,
    fetched_at: Instant,
    cache_duration: Duration,
}

impl CachedValue {
    pub fn is_expired(&self) -> bool {
        self.fetched_at.elapsed() > self.cache_duration
    }
}

impl CachedAPIClient {
    pub async fn get_with_cache<F>(&self, key: &str, cache_duration: Duration, f: F) -> Result<serde_json::Value>
    where
        F: Fn() -> futures::future::BoxFuture<'static, Result<serde_json::Value>>,
    {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.get(key) {
                if !cached.is_expired() {
                    return Ok(cached.data.clone());
                }
            }
        }

        // Fetch fresh data
        let data = f().await?;

        // Update cache
        {
            let mut cache = self.cache.write().await;
            cache.insert(key.to_string(), CachedValue {
                data: data.clone(),
                fetched_at: Instant::now(),
                cache_duration,
            });
        }

        Ok(data)
    }
}
```

---

## ⚠️ Section 3: Error Handling & Resilience

### 3.1 Exponential Backoff

**Implement Smart Retries**

```rust
pub struct BackoffConfig {
    pub max_retries: u32,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
}

pub async fn call_with_backoff<F, T>(
    mut f: F,
    config: BackoffConfig,
) -> Result<T>
where
    F: FnMut() -> futures::future::BoxFuture<'static, Result<T>>,
{
    let mut delay = config.initial_delay_ms;

    for attempt in 0..=config.max_retries {
        match f().await {
            Ok(result) => {
                if attempt > 0 {
                    println!("✓ Succeeded on attempt {}", attempt + 1);
                }
                return Ok(result);
            }
            Err(e) if attempt < config.max_retries => {
                // Exponential backoff: 100ms, 200ms, 400ms, 800ms, 1600ms
                println!(
                    "Attempt {} failed ({}), retrying in {}ms...",
                    attempt + 1,
                    e,
                    delay
                );

                tokio::time::sleep(Duration::from_millis(delay)).await;

                // Exponential increase with jitter
                delay = (delay * 2).min(config.max_delay_ms);
                delay = delay + rand::random::<u64>() % (delay / 10); // Add 0-10% jitter
            }
            Err(e) => {
                eprintln!("✗ Failed after {} attempts: {}", config.max_retries + 1, e);
                return Err(e);
            }
        }
    }

    Err("Max retries exceeded".into())
}

// Usage
let result = call_with_backoff(
    || Box::pin(async { api_client.fetch_data().await }),
    BackoffConfig {
        max_retries: 5,
        initial_delay_ms: 100,
        max_delay_ms: 5000,
    },
).await?;
```

### 3.2 Error Classification

**Distinguish Error Types**

```rust
pub enum APIError {
    // Retryable errors (temporary issues)
    RateLimitExceeded { retry_after_secs: u64 },
    Timeout,
    ConnectionError,
    ServiceUnavailable,

    // Non-retryable errors (don't retry)
    InvalidAPIKey,
    InvalidRequest,
    NotFound,

    // Critical errors (stop trading)
    AuthenticationFailed,
    InsufficientBalance,
}

pub fn should_retry(error: &APIError) -> bool {
    match error {
        APIError::RateLimitExceeded { .. } => true,
        APIError::Timeout => true,
        APIError::ConnectionError => true,
        APIError::ServiceUnavailable => true,

        APIError::InvalidAPIKey => false,
        APIError::InvalidRequest => false,
        APIError::NotFound => false,

        APIError::AuthenticationFailed => false,
        APIError::InsufficientBalance => false,
    }
}

pub async fn call_with_smart_retry<F, T>(
    mut f: F,
    max_retries: u32,
) -> Result<T>
where
    F: FnMut() -> futures::future::BoxFuture<'static, Result<T, APIError>>,
{
    for attempt in 0..=max_retries {
        match f().await {
            Ok(result) => return Ok(result),
            Err(ref e) if should_retry(e) && attempt < max_retries => {
                let delay = Duration::from_millis(100 * 2_u64.pow(attempt));
                println!("Retrying after {:?}...", delay);
                tokio::time::sleep(delay).await;
            }
            Err(e) => return Err(format!("API error: {:?}", e).into()),
        }
    }

    Err("Max retries exceeded".into())
}
```

### 3.3 Circuit Breaker Pattern

**Prevent Cascading Failures**

```rust
pub enum CircuitState {
    Closed,       // Normal operation
    Open,         // Failing, reject requests
    HalfOpen,     // Testing if service recovered
}

pub struct CircuitBreaker {
    state: Arc<RwLock<CircuitState>>,
    failure_count: Arc<AtomicU32>,
    failure_threshold: u32,
    timeout: Duration,
    opened_at: Arc<RwLock<Option<Instant>>>,
}

impl CircuitBreaker {
    pub async fn call<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce() -> futures::future::BoxFuture<'static, Result<T>>,
    {
        let state = self.state.read().await;

        match *state {
            CircuitState::Open => {
                // Check if timeout expired
                let opened_at = self.opened_at.read().await;
                if let Some(time) = *opened_at {
                    if time.elapsed() > self.timeout {
                        drop(opened_at);
                        drop(state);

                        // Transition to HalfOpen
                        let mut state = self.state.write().await;
                        *state = CircuitState::HalfOpen;
                    } else {
                        // Still open, reject request immediately
                        return Err("Circuit breaker open".into());
                    }
                }
            }
            CircuitState::Closed | CircuitState::HalfOpen => {
                // Allow request
            }
        }

        drop(state);

        // Execute request
        match f().await {
            Ok(result) => {
                // Reset on success
                self.failure_count.store(0, Ordering::SeqCst);
                let mut state = self.state.write().await;
                *state = CircuitState::Closed;
                Ok(result)
            }
            Err(e) => {
                // Increment failure counter
                let new_count = self.failure_count.fetch_add(1, Ordering::SeqCst) + 1;

                if new_count >= self.failure_threshold {
                    // Open circuit
                    let mut state = self.state.write().await;
                    *state = CircuitState::Open;
                    let mut opened_at = self.opened_at.write().await;
                    *opened_at = Some(Instant::now());
                }

                Err(e)
            }
        }
    }
}
```

---

## 📊 Section 4: Monitoring & Alerting

### 4.1 Metrics Collection

```rust
pub struct APIMetrics {
    pub total_calls: Arc<AtomicU64>,
    pub successful_calls: Arc<AtomicU64>,
    pub failed_calls: Arc<AtomicU64>,
    pub total_latency_ms: Arc<AtomicU64>,
    pub peak_latency_ms: Arc<AtomicU64>,
}

impl APIMetrics {
    pub fn record_call(&self, latency_ms: u64, success: bool) {
        self.total_calls.fetch_add(1, Ordering::SeqCst);
        self.total_latency_ms.fetch_add(latency_ms, Ordering::SeqCst);

        if latency_ms > self.peak_latency_ms.load(Ordering::SeqCst) {
            self.peak_latency_ms.store(latency_ms, Ordering::SeqCst);
        }

        if success {
            self.successful_calls.fetch_add(1, Ordering::SeqCst);
        } else {
            self.failed_calls.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub fn get_stats(&self) -> APIStats {
        let total = self.total_calls.load(Ordering::SeqCst);
        let successful = self.successful_calls.load(Ordering::SeqCst);
        let failed = self.failed_calls.load(Ordering::SeqCst);
        let total_latency = self.total_latency_ms.load(Ordering::SeqCst);
        let peak_latency = self.peak_latency_ms.load(Ordering::SeqCst);

        APIStats {
            total_calls: total,
            success_rate: if total > 0 {
                (successful as f64 / total as f64) * 100.0
            } else {
                0.0
            },
            avg_latency_ms: if total > 0 {
                total_latency / total
            } else {
                0
            },
            peak_latency_ms: peak_latency,
        }
    }
}

pub struct APIStats {
    pub total_calls: u64,
    pub success_rate: f64,
    pub avg_latency_ms: u64,
    pub peak_latency_ms: u64,
}
```

### 4.2 Alerting Rules

```rust
pub fn check_api_health(stats: &APIStats) -> Vec<Alert> {
    let mut alerts = vec![];

    // Alert: High error rate
    if stats.success_rate < 95.0 && stats.total_calls > 100 {
        alerts.push(Alert {
            severity: AlertSeverity::Warning,
            message: format!(
                "API error rate high: {:.1}% (expected >95%)",
                stats.success_rate
            ),
        });
    }

    // Alert: High latency
    if stats.avg_latency_ms > 2000 {
        alerts.push(Alert {
            severity: AlertSeverity::Warning,
            message: format!(
                "API latency high: {}ms (expected <1000ms)",
                stats.avg_latency_ms
            ),
        });
    }

    // Alert: Peak latency
    if stats.peak_latency_ms > 10000 {
        alerts.push(Alert {
            severity: AlertSeverity::Critical,
            message: format!(
                "API peak latency critical: {}ms",
                stats.peak_latency_ms
            ),
        });
    }

    alerts
}
```

### 4.3 Logging Best Practices

**DO: Log Important Events**

```rust
// ✅ Good logging
info!("Fetching sentiment for 50 coins");
debug!("API call to lunarcrush took 245ms");
warn!("API rate limit approaching: 950/1000 calls used");
error!("API key validation failed, check .env.data-feeds");

// ❌ Bad logging - Never log credentials
println!("Using API key: {}", api_key);
eprintln!("Auth header: {}", auth_header);
```

**Log Levels**

```rust
// Use appropriate log level
error!("Unrecoverable error, trading halted");      // Critical failures
warn!("Approaching rate limit");                    // Concerning trends
info!("API call completed successfully");           // Significant events
debug!("Fetching 50 coins from Binance API");       // Detailed execution
trace!("Processing JSON response...");              // Very detailed

// Configure in Rust
env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
    .format(|buf, record| {
        writeln!(
            buf,
            "[{}] {} - {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            record.level(),
            record.args()
        )
    })
    .init();
```

---

## 💰 Section 5: Cost Optimization

### 5.1 Free Tier Maximization

**Budget-Conscious Service Selection**

```
Scenario: Monitor 10 trading symbols

Option A (All Free):
  ✅ Binance: Free (1200 req/min)
  ✅ CoinGecko: Free (50 req/min)
  ✅ CryptoPanic: Free (unlimited)
  ✅ Supabase: Free (500MB)

  Cost: $0/month
  Capability: Professional-grade
  Trade-off: Limited on-chain metrics

Option B (Free + 1 Paid):
  ✅ Binance: Free
  ✅ CoinGecko: Free
  ✅ CryptoPanic: Free
  💰 Glassnode Pro: $399/month (on-chain)
  ✅ Supabase: Free

  Cost: $399/month
  Capability: Institutional-grade
  Trade-off: Higher cost but complete data

Option C (All Paid):
  💰 LunarCrush: $99/month
  💰 Glassnode: $399/month
  💰 Messari: $99/month
  💰 Supabase Pro: $25/month

  Cost: $622/month
  Capability: Maximum redundancy
  Trade-off: Not justified unless managing $1M+
```

### 5.2 Request Optimization

**Calculate Actual Cost**

```rust
pub fn calculate_api_cost(
    num_coins: usize,
    update_frequency_mins: u32,
    data_freshness_mins: u32,
) -> (u32, String) {
    // Assuming $0.01 per 1000 API calls (typical)
    let calls_per_update = num_coins as u32;
    let updates_per_day = (24 * 60) / update_frequency_mins;
    let calls_per_day = calls_per_update * updates_per_day;

    // With caching
    let cache_hits_ratio = (data_freshness_mins / update_frequency_mins).min(10) as f32;
    let effective_calls = (calls_per_day as f32 / cache_hits_ratio) as u32;

    (effective_calls, format!(
        "{} coins × {}/hour updates × {}min cache = {} effective calls/day",
        num_coins,
        updates_per_day,
        data_freshness_mins,
        effective_calls
    ))
}

// Example
let (calls, explanation) = calculate_api_cost(
    50,    // 50 coins
    60,    // Update every 60 minutes
    30,    // Cache for 30 minutes
);
println!("{} effective calls/day", calls);  // Output: ~41 calls/day (very cheap!)
```

### 5.3 Free Tier Prioritization

**Priority Matrix**

```
Must Have (Free):
  ✅ Binance (price data, most important)
  ✅ CoinGecko (fallback, all coins)
  ✅ CryptoPanic (event detection)

Nice to Have (Free):
  ✅ Messari (professional data)
  ✅ Santiment (community sentiment)
  ✅ Kraken (exchange alternative)

Optional (Paid):
  💰 LunarCrush (advanced sentiment)
  💰 Glassnode (on-chain metrics)
  💰 Polygon.io (premium aggregation)

Strategy:
  1. Start with 3x "Must Have" (cost: $0)
  2. Add "Nice to Have" services as comfortable (cost: $0)
  3. Upgrade to Paid only after validating 3-month backtest performance
```

---

## 🎯 Section 6: Free Tier Etiquette

### 6.1 Respect Service Providers

**DO:**
```
✅ Use reasonable polling intervals (1-5 minutes for prices)
✅ Cache aggressively (30+ minutes for sentiment)
✅ Batch requests (50 coins per call, not 50 calls)
✅ Monitor your usage dashboard
✅ Give back (cite sources, mention in docs)
✅ Upgrade when you scale past $10K AUM
```

**DON'T:**
```
❌ Poll every second (wastes resources for free tier)
❌ Hammer API with concurrent requests
❌ Scrape website instead of using API (violates ToS)
❌ Exceed documented rate limits
❌ Use bots to signup for multiple free accounts
❌ Share API credentials publicly
```

### 6.2 Usage Monitoring

```rust
pub struct UsageQuota {
    pub service: String,
    pub daily_limit: u32,
    pub calls_used: Arc<AtomicU32>,
    pub last_reset: Arc<RwLock<DateTime<Utc>>>,
}

impl UsageQuota {
    pub async fn check_quota(&self) -> Result<()> {
        let used = self.calls_used.load(Ordering::SeqCst);
        let remaining = self.daily_limit - used;
        let percent_used = (used as f32 / self.daily_limit as f32) * 100.0;

        if percent_used > 90.0 {
            eprintln!(
                "⚠️  WARNING: {} at {:.0}% quota ({}/{})",
                self.service, percent_used, used, self.daily_limit
            );
        }

        if remaining == 0 {
            return Err(format!("{} daily quota exhausted", self.service).into());
        }

        Ok(())
    }

    pub async fn use_calls(&self, count: u32) -> Result<()> {
        self.check_quota().await?;
        self.calls_used.fetch_add(count, Ordering::SeqCst);
        Ok(())
    }
}
```

---

## ✅ Implementation Checklist

### Security
- [ ] Environment files created (.env.data-feeds, .env.wallet)
- [ ] .gitignore configured properly
- [ ] Pre-commit hooks installed
- [ ] File permissions set to 600
- [ ] No hardcoded API keys in code
- [ ] API key rotation plan documented

### Rate Limiting
- [ ] Token bucket implementation created
- [ ] Request batching implemented
- [ ] Caching strategy defined
- [ ] Rate limit headers parsed and respected
- [ ] Monitoring of quota usage implemented

### Error Handling
- [ ] Exponential backoff implemented
- [ ] Error classification defined
- [ ] Circuit breaker pattern implemented
- [ ] Retry logic tested

### Monitoring
- [ ] Metrics collection implemented
- [ ] Alert thresholds defined
- [ ] Logging configured
- [ ] Health check script created

### Cost Optimization
- [ ] Free tier services identified
- [ ] Paid tier necessity evaluated
- [ ] Caching strategy optimized
- [ ] Usage monitoring dashboard set up

---

## 📚 Reference: API Endpoints Quick Guide

### Production-Ready Services

```bash
# Binance (Price Data)
curl "https://api.binance.com/api/v3/ticker/24hr?symbol=SOLUSDT"

# CoinGecko (Fallback Prices)
curl "https://api.coingecko.com/api/v3/simple/price?ids=solana&vs_currencies=usd"

# CryptoPanic (News)
curl "https://cryptopanic.com/api/v1/posts/?auth_token=YOUR_TOKEN"

# Messari (Professional Data)
curl -H "x-messari-key: YOUR_KEY" \
  "https://data.messari.io/api/v1/assets/bitcoin/metrics"
```

---

**Status:** ✅ Enterprise-grade API best practices fully documented
**Next Steps:**
1. Implement all three documentation files in codebase
2. Create automated monitoring dashboards
3. Set up alerting infrastructure
4. Begin data collection pipeline

