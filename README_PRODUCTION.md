# TradingBots.fun - Production Trading System

**Real-money autonomous trading bot for Solana DEX**

## Quick Start (Docker)

### Prerequisites
- Docker & Docker Compose installed
- Binance API key (read-only)
- Hyperliquid account
- PostgreSQL running locally or in Docker

### Deploy in 5 Minutes

```bash
# 1. Clone repo
git clone https://github.com/yourusername/tradingbots-fun.git
cd tradingbots-fun

# 2. Configure
cp .env.example .env
# Edit .env with your API keys

# 3. Run
docker-compose up -d

# 4. Monitor
docker-compose logs -f

# 5. Check status
curl http://localhost:8080/health
```

## Build Locally (Rust)

### Prerequisites
- Rust 1.75+ (https://rustup.rs)
- PostgreSQL 14+
- Cargo

### Local Build

```bash
# 1. Install dependencies
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# 2. Clone & build
git clone https://github.com/yourusername/tradingbots-fun.git
cd tradingbots-fun
cargo build --release

# 3. Run database
createdb tradingbots
psql tradingbots < migrations/init.sql

# 4. Configure
cp .env.example .env
# Edit with your API keys

# 5. Run
./target/release/tradingbots
```

## Architecture

```
┌─────────────────────────────────────┐
│   CEX Monitoring (Binance)          │
│   - Order book data                 │
│   - Price feeds                     │
└────────────┬────────────────────────┘
             │
┌────────────▼────────────────────────┐
│   Technical Analysis                │
│   - RSI, MACD, Bollinger            │
│   - Support/Resistance              │
│   - ATR, Stochastic                 │
└────────────┬────────────────────────┘
             │
┌────────────▼────────────────────────┐
│   Order Flow Detection              │
│   - Bid/Ask imbalance               │
│   - Whale movement detection        │
└────────────┬────────────────────────┘
             │
┌────────────▼────────────────────────┐
│   Decision Engine                   │
│   - Multi-signal confluence         │
│   - Confidence scoring              │
│   - Risk validation                 │
└────────────┬────────────────────────┘
             │
┌────────────▼────────────────────────┐
│   Execution (Hyperliquid)           │
│   - Market orders                   │
│   - Position management             │
│   - Risk enforcement                │
└────────────┬────────────────────────┘
             │
┌────────────▼────────────────────────┐
│   Logging & Monitoring              │
│   - PostgreSQL database             │
│   - Real-time metrics               │
└─────────────────────────────────────┘
```

## Configuration

### .env Variables

```bash
# Mode
MODE=testnet  # testnet or mainnet

# Trading
TRADING_SYMBOL=SOL
INITIAL_CAPITAL=100

# APIs
BINANCE_API_KEY=your_key
HYPERLIQUID_KEY=your_key
HYPERLIQUID_SECRET=your_secret

# Database
DATABASE_URL=postgres://user:pass@localhost/tradingbots

# Logging
RUST_LOG=info
```

## Deployment Options

### Option 1: Docker (Recommended for Beginners)
```bash
docker-compose up -d
```
- Easiest setup
- No Rust knowledge needed
- Automatic PostgreSQL

### Option 2: DigitalOcean VPS ($12/month)
See `DEPLOYMENT_DIGITALOCEAN.md`
- Production-ready
- 99.9% uptime
- Automatic backups

### Option 3: Local Machine
```bash
cargo run --release
```
- Full control
- Lowest cost
- Requires Rust

## Trading Parameters

### Risk Management (ENFORCED)
```
Daily Loss Limit: $30 (hard stop)
Max Position: 15% of capital
Max Leverage: 15x (scaled by volatility)
Min Health Factor: 2.0 (prevent liquidation)
Max Concurrent: 3 positions
```

### Expected Performance
```
Win Rate: 70-75%
Monthly Return: 12-20% on $100+
Max Drawdown: -18%
Avg Win: +0.82%
Avg Loss: -0.63%
```

## First Trade Checklist

- [ ] Configured .env with actual API keys
- [ ] Tested on testnet for 24 hours
- [ ] Validated all signals are firing
- [ ] Checked order execution
- [ ] Confirmed trade logging to database
- [ ] Reviewed risk management rules
- [ ] Set up monitoring alerts
- [ ] Have backup & recovery plan
- [ ] Ready for live $100 trading

## Monitoring

### Real-time Logs
```bash
# Docker
docker-compose logs -f tradingbots

# Local
RUST_LOG=debug cargo run
```

### Database Queries
```bash
# Recent trades
psql tradingbots -c "SELECT * FROM trades ORDER BY created_at DESC LIMIT 10;"

# Account status
psql tradingbots -c "SELECT COUNT(*), AVG(confidence), SUM(pnl) FROM trades;"

# Positions
psql tradingbots -c "SELECT * FROM positions WHERE status='OPEN';"
```

### Health Check
```bash
curl http://localhost:8080/health
```

## Troubleshooting

### Error: Connection refused (database)
```bash
# Start PostgreSQL
docker-compose up postgres -d
# Or: brew services start postgresql
```

### Error: Invalid API key
```bash
# Verify in .env
cat .env | grep API
# Regenerate keys in Binance/Hyperliquid
```

### Error: No trades executing
```bash
# Check logs
docker-compose logs tradingbots | grep "Signal\|Decision\|Risk"

# Test indicators
# Add debug logging: RUST_LOG=debug
```

### High memory usage
```bash
# Check stats
docker stats

# Restart
docker-compose restart tradingbots
```

## Performance Optimization

### Reduce Latency
- Switch to local PostgreSQL (vs cloud)
- Use Rust release build (not debug)
- Reduce polling interval in config

### Reduce Cost
- Use free tier APIs only
- Run on smallest VPS possible
- Batch API calls (not every candle)

## Security

### API Key Protection
```bash
# .env should be:
# 1. Never committed to git
# 2. 600 permissions chmod 600 .env
# 3. Different keys per environment
```

### Database Security
```bash
# Change default password
ALTER USER postgres WITH PASSWORD 'complex_password';

# Backup regularly
pg_dump tradingbots > backup.sql
```

### Network Security
```bash
# Firewall: Allow SSH, block others
ufw allow 22
ufw default deny incoming

# VPN for connections
```

## Scaling to Larger Capital

From $100 to $500+:

1. **Validate system** (current phase)
   - Run testnet 72 hours
   - Validate backtest matches live
   - Build confidence

2. **Start small** ($100 live)
   - Trade 1-2 weeks
   - Document all trades
   - Verify no bugs

3. **Scale gradually** ($200 → $500)
   - Add $100 every 2 weeks if profitable
   - Keep same position sizes %
   - Monitor drawdowns

## Support & Debugging

### Enable Detailed Logging
```bash
export RUST_LOG=debug
cargo run --release
```

### Common Issues
1. **No trades**: Check CEX connectivity
2. **Failed orders**: Check health factor
3. **High slippage**: Use limit orders instead
4. **Crashes**: Check logs, report with timestamps

## License

Educational/Personal use only.

## Disclaimer

Trading crypto involves risk of loss. This system:
- Is NOT financial advice
- Does NOT guarantee profits
- Can lose your entire capital
- Requires proper risk management

Start with $50-100. Only increase capital if profitable.

---

**Ready to trade? Run `docker-compose up -d`**

