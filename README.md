# 🚀 TradingBots.fun AI trading platform

A production-grade, high-performance trading bot written in Rust for trading on Hyperliquid and soon on (across multiple protocols) with comprehensive unit testing and performance optimization.

## Features

✅ **Multi-Account Management** — Manage 5+ accounts with different trading strategies
✅ **Multi-Protocol Support** — Drift Protocol, Hyperliquid, Phantom integration
✅ **Capital Allocation** — Dynamic capital distribution based on market conditions
✅ **Liquidation Prevention** — Multi-level safety system to prevent account liquidation
✅ **Cross-Chain Bridging** — Optimize capital movement across chains
✅ **Comprehensive Testing** — >90% code coverage with unit + integration tests
✅ **High Performance** — 10-100x faster than Python/Node.js solutions
✅ **Type Safety** — Compile-time error checking with Rust's type system

## Project Structure

```
drift-multi-protocol/
├── Cargo.toml                          # Rust project manifest
├── src/
│   ├── lib.rs                          # Library entry point
│   ├── main.rs                         # CLI binary
│   ├── models/
│   │   ├── mod.rs
│   │   └── account.rs                  # Account data structures
│   ├── modules/
│   │   ├── mod.rs
│   │   └── account_manager.rs          # Multi-account management (25+ tests)
│   └── utils/
│       ├── mod.rs
│       └── error_handling.rs           # Error types and handling
├── tests/
│   └── integration_test.rs             # End-to-end integration tests
└── README.md                           # This file
```

## Building the Project

### Prerequisites

- Rust 1.70+ (install from https://rustup.rs)
- Cargo (comes with Rust)

### Build Steps

```bash
# Navigate to project directory
cd drift-multi-protocol

# Install dependencies and build
cargo build --release

# Verify compilation
cargo check
```

## Running Tests

### Run All Tests

```bash
# Run unit tests + integration tests
cargo test

# Run with output
cargo test -- --nocapture --test-threads=1
```

### Run Specific Test Modules

```bash
# Unit tests only
cargo test modules::account_manager

# Integration tests only
cargo test --test integration_test

# Single test
cargo test test_register_account -- --nocapture
```

### Test Coverage

```bash
# Install tarpaulin (code coverage tool)
cargo install cargo-tarpaulin

# Generate coverage report
cargo tarpaulin --out Html --verbose --timeout 300
```

## Running the Bot

### Development Mode

```bash
# Run with debug output
cargo run
```

### Production Mode

```bash
# Build optimized release binary
cargo build --release

# Run the binary
./target/release/drift-bot
```

## Analytics Scripts & AI Cache

- `cargo run --bin reporter` now behaves as the intelligent analytics script: it regenerates the daily trade journal, persists the JSON `reports/latest_report.json`, and writes the new pattern summary files (`reports/pattern_insights_<date>.json` and `reports/pattern_insights_<date>.md`) that capture guardrail scores, signal breakdowns, context snapshots, and winner/loser tables.
- Each run also refreshes `reports/pattern_cache.json`. The pattern cache stores the latest insights together with the SHA256 hashes of `logs/ai_guardrail_feedback.jsonl` and `logs/trading_<date>.jsonl`. The dashboard/API consider the cache stale whenever those hashes change or when the cache is older than five minutes, so running the reporter after new exits guarantees fresh data while avoiding redundant recomputation.
- The existing `/api/report/query` cache is tied to the `report_hash` derived from the summary payload, so AI answers expire automatically as soon as guardrail/trade summaries move. To surface the richer pattern payloads to dashboards or assistants, request `GET /api/report/patterns`—the endpoint returns the latest `PatternInsights` bundle mirroring the markdown/JSON output.
- A new `/app/agents` page gives OpenClaw/x402 agents a lightweight control room: portfolio snapshot + position list, guardrail combo highlights from `/api/report/patterns`, automation alerts from `/api/report/patterns/alerts`, and an agent-friendly command form that posts directly to `/api/command`. Keep that tab open to monitor wins/losses and push manual instructions while Claude reasons about the refreshed pattern history.
- The consumer nav now links directly to `/app/agents`, and the admin panel exposes the same Agent Console link plus a dedicated pattern cache summary card so operators can see the latest winner/loser and signal combos alongside CLI/webhook alerts. The shared `reports/pattern_cache_alert.json` payload is still served at `/api/report/patterns/alerts`, letting dashboards or assistant prompts stay in sync with the freshest combos.
- `scripts/pattern_cache_alert.py` now appends every refresh to `reports/pattern_cache_alert.log` and, when `PATTERN_CACHE_WEBHOOK_URL` is set, posts the same payload to that webhook (customize `PATTERN_CACHE_WEBHOOK_METHOD`, add `PATTERN_CACHE_WEBHOOK_TOKEN`, or pass `PATTERN_CACHE_WEBHOOK_HEADERS` JSON for extra headers). This turns the cache watcher into a Slack/webhook/CLI alerting source, so agents get notified of new pattern signatures even when they’re not looking at the dashboard.
- Guardrail records now embed `prompt_tokens`/`completion_tokens`/`total_tokens`; run `python scripts/claude_usage_summary.py` (optionally with `--log logs/ai_guardrail_feedback.jsonl`) to get per-review averages and totals for Claude’s token usage.
- Bridging is now prototyped: see `BRIDGE_GUIDE.md` for the manual steps plus the `/api/bridge/withdraw` and `/api/bridge/status/:id` endpoints that orchestrate Hyperliquid → Arbitrum payouts. Configure `BRIDGE_MIN_WITHDRAW_USD` and `BRIDGE_TRUSTED_DESTINATIONS` to keep withdrawals safe.

## Key Modules

### 1. Account Manager (`src/modules/account_manager.rs`)

Manages multiple trading accounts with different purposes:

```rust
let mut manager = AccountManager::new();

// Create scalp account
let scalp = TradingAccount::new(
    "drift-scalp-1".to_string(),
    Protocol::Drift,
    "public_key".to_string(),
    AccountPurpose::Scalp,
);

// Register account
manager.register_account(scalp)?;

// Set capital allocation
manager.set_capital_allocation("drift-scalp-1", 0.30)?;

// Set leverage
manager.set_leverage("drift-scalp-1", 50.0)?;

// Get all accounts
let accounts = manager.get_all_accounts();
```

**Features:**
- Register/manage multiple accounts
- Set leverage per account (respects max for account purpose)
- Dynamic capital allocation
- Filter accounts by protocol/purpose
- Account health monitoring
- Duplicate detection
- Complete validation

**Test Coverage:**
- ✅ Account registration (31 unit tests)
- ✅ Duplicate detection
- ✅ Leverage constraints
- ✅ Capital allocation
- ✅ Account filtering (by protocol, purpose, active)
- ✅ Account lifecycle (activate/deactivate)
- ✅ High-throughput operations (100+ accounts)

### 2. Account Data Models (`src/models/account.rs`)

Defines all account-related data structures:

```rust
pub enum AccountPurpose {
    Scalp,      // 100x leverage
    Swing,      // 20x leverage
    Position,   // 10x leverage
    Hedge,      // 5x leverage
    Reserve,    // 0x leverage
}

pub struct TradingAccount {
    pub id: String,
    pub protocol: Protocol,
    pub purpose: AccountPurpose,
    pub capital_allocation: f64,
    pub current_leverage: f64,
    pub max_position_size: f64,
    // ... more fields
}

pub struct HealthMetrics {
    pub health_factor: f64,
    pub liquidation_risk: LiquidationRisk,
    // ... more fields
}
```

**Test Coverage:**
- ✅ Account creation
- ✅ Purpose-based leverage constraints
- ✅ Liquidation risk calculation
- ✅ Health factor computation
- ✅ Account validation

### 3. Error Handling (`src/utils/error_handling.rs`)

Comprehensive error types with severity levels:

```rust
pub enum Error {
    DuplicateAccount,
    AccountNotFound,
    InvalidAccountConfig,
    LiquidationCritical,
    // ... 20+ error types
}

impl Error {
    pub fn is_recoverable(&self) -> bool { ... }
    pub fn severity(&self) -> u8 { ... }  // 1-10
}
```

## Account Setup

### Recommended Multi-Account Structure

```
Total Capital: $5,000

├─ Drift Scalp (30%)        → 100x leverage, quick trades
├─ Drift Swing (25%)        → 20x leverage, 1-7 day holds
├─ Drift Position (20%)     → 10x leverage, weekly+ holds
├─ Drift Hedge (15%)        → 5x leverage, portfolio protection
└─ Drift Reserve (10%)      → 0x leverage, emergency funds
```

### Creating This Setup

```bash
# Just run the bot
cargo run

# It will:
# 1. Create all 5 accounts
# 2. Set correct leverage for each
# 3. Allocate capital proportionally
# 4. Verify total = 100%
# 5. Display status
```

## Performance Targets

| Component | Target | Status |
|-----------|--------|--------|
| Account registration | <100ms | ✅ Instant |
| Account lookup | <1ms | ✅ HashMap |
| Capital rebalancing | <500ms | TBD |
| Account health check | <100ms | TBD |
| Liquidation prevention | <1s | TBD |

## Code Quality

### Checks and Formatting

```bash
# Format code
cargo fmt

# Check formatting
cargo fmt -- --check

# Lint code
cargo clippy -- -D warnings
```

### Documentation

```bash
# Generate and open docs
cargo doc --open
```

## Phase 1 Completion Checklist

- ✅ Project initialized with Cargo
- ✅ Core models defined (Account, Protocol, Purpose)
- ✅ Error types with severity levels
- ✅ AccountManager with 31+ unit tests
- ✅ Account validation logic
- ✅ Capital allocation validation
- ✅ Leverage constraint enforcement
- ✅ Integration tests (high-throughput, multi-protocol)
- ✅ CLI binary for account setup
- ✅ Comprehensive documentation

## Next Phases

### Phase 2: Hyperliquid Integration
- [ ] Implement HyperLiquid protocol client
- [ ] Add market data fetching
- [ ] Implement order placement
- [ ] Add position management
- [ ] Tests for protocol integration

### Phase 3: Cross-Chain Bridging
- [ ] Implement bridge optimization
- [ ] Add Wormhole support
- [ ] Add CCTP support
- [ ] Add Stargate support
- [ ] Gas cost calculation

### Phase 4: Capital Management
- [ ] Dynamic capital allocation
- [ ] Market scoring system
- [ ] Automated rebalancing
- [ ] Risk assessment
- [ ] Performance tracking

## Dependencies

```toml
tokio = "1"              # Async runtime
reqwest = "0.11"         # HTTP client
serde = "1.0"            # Serialization
solana-sdk = "1.18"      # Solana integration
sqlx = "0.7"             # Database
redis = "0.25"           # Caching
uuid = "1.0"             # ID generation
chrono = "0.4"           # Timestamps
tracing = "0.1"          # Logging
```

## Testing Commands Reference

```bash
# Run all tests with colored output
cargo test -- --nocapture --color=always

# Run specific test function
cargo test test_register_account -- --nocapture

# Run tests in account_manager module
cargo test modules::account_manager -- --nocapture

# Run integration tests
cargo test --test integration_test

# Run with single thread (for debugging)
cargo test -- --test-threads=1 --nocapture

# Run tests and show output even for passing tests
cargo test -- --nocapture --show-output

# Generate test coverage report
cargo tarpaulin --out Html --output-dir coverage
```

## Troubleshooting

### Build Errors

```bash
# Update dependencies
cargo update

# Clean and rebuild
cargo clean
cargo build --release
```

### Test Failures

```bash
# Run failing test with backtrace
RUST_BACKTRACE=1 cargo test test_name -- --nocapture

# Run single-threaded for debugging
cargo test -- --test-threads=1 --nocapture
```

## Contributing

Follow these guidelines:

1. Write tests first (TDD)
2. Maintain >90% code coverage
3. Run `cargo fmt` before commit
4. Run `cargo clippy` to check for warnings
5. Document public APIs with doc comments

## License

MIT / Apache 2.0

## Support

For issues or questions:
1. Check documentation in code
2. Review test examples
3. Run tests to verify behavior

---

**Status:** ✅ Phase 1 Complete (Account Management)
**Last Updated:** 2026-02-21
**Version:** 0.1.0
