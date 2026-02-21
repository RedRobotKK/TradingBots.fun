# RedRobot HedgeBot - Quick Start Guide

## 🚀 Get Trading in 10 Minutes

### Step 1: Prerequisites (2 minutes)
```bash
# Install Docker
https://docs.docker.com/get-docker/

# Verify installation
docker --version
docker-compose --version
```

### Step 2: Get API Keys (3 minutes)
```
1. Binance: https://www.binance.com/account/api-management
   - Create new API key
   - Copy Key + Secret

2. Hyperliquid: https://app.hyperliquid.xyz
   - Settings → API
   - Copy Key + Secret
   - TEST ON TESTNET FIRST!
```

### Step 3: Configure (2 minutes)
```bash
# Clone project
git clone https://github.com/yourusername/redrobot-hedgebot.git
cd redrobot-hedgebot

# Create config file
cp .env.example .env

# Edit with your API keys
nano .env  # Or use your editor
# Fill in:
# - BINANCE_API_KEY
# - HYPERLIQUID_KEY
# - HYPERLIQUID_SECRET
# - MODE=testnet (for now)
```

### Step 4: Deploy (2 minutes)
```bash
# Start system
docker-compose up -d

# Watch logs
docker-compose logs -f

# Wait for "Trading cycle started"
# Ctrl+C to exit logs (doesn't stop system)
```

### Step 5: Monitor (1 minute)
```bash
# Check status
docker ps

# View trades
docker-compose exec postgres psql -U postgres -d redrobot \
  -c "SELECT * FROM trades ORDER BY created_at DESC LIMIT 5;"

# Stop if needed
docker-compose down
```

## ✅ Success Indicators

System is working when:
- ✅ Docker containers running (`docker ps`)
- ✅ No errors in logs (`docker-compose logs`)
- ✅ Database connected (`docker-compose exec postgres psql -U postgres`)
- ✅ First trades logged within 5-10 minutes

## ❌ If Something Goes Wrong

### Error: Connection refused (PostgreSQL)
```bash
# Ensure PostgreSQL container is running
docker-compose ps postgres

# If not, restart
docker-compose up postgres -d
docker-compose restart redrobot
```

### Error: Invalid API key
```bash
# Check your .env file
cat .env

# Verify keys are correct on Binance/Hyperliquid
# Update .env and restart
docker-compose restart redrobot
```

### Error: Orders not executing
```bash
# Check mode - should be testnet first
grep MODE .env

# Check logs for details
docker-compose logs redrobot | grep -i error
```

## 🔄 Trading Cycle

This is what happens every 500ms (automatic):

```
1. Fetch CEX data (Binance order book)
   ↓
2. Calculate technical indicators
   ↓
3. Detect order flow signals
   ↓
4. Make decision (BUY/SELL/SKIP)
   ↓
5. Risk validation (prevent bad trades)
   ↓
6. Execute order on Hyperliquid (if approved)
   ↓
7. Log trade to database
   ↓
8. Monitor position (close if needed)
```

## 📊 Understanding Logs

```
Example logs you'll see:

✓ Configuration loaded: Testnet
✓ Database initialized
✓ CEX client initialized
✓ Hyperliquid client initialized
📈 SOL/USDT | Action: BUY | Confidence: 87% | Strategy: Multi-Signal(4signals)
✓ Order placed: 550e8400-e29b-41d4-a716-446655440000
⏸ No signal (confidence too low)
❌ Health factor too low, skipping
```

## 🎯 First 24 Hours Checklist

- [ ] System deployed and running
- [ ] At least 5 trades logged
- [ ] No critical errors in logs
- [ ] Database synced correctly
- [ ] Risk management working (some trades skipped)
- [ ] Ready to review results

## 📈 After 24 Hours

1. **Stop testnet**: Let it run 24-72 hours
2. **Analyze trades**: Review in database
3. **Check win rate**: Should be 60-75%
4. **Review P&L**: Should be small positive or neutral
5. **Then**: Switch MODE to `mainnet` and deploy with $100

## 🔐 Safety Reminders

✅ DO:
- Start with $100 (testnet first, then small real money)
- Let it run 24+ hours before scaling
- Monitor logs daily
- Review trades regularly
- Keep .env backed up securely

❌ DON'T:
- Skip testnet validation
- Deploy with $500 immediately
- Commit .env to GitHub
- Modify risk limits
- Trade without monitoring

## 📞 Troubleshooting

### System won't start
```bash
# Full restart
docker-compose down
docker-compose up -d
docker-compose logs
```

### Very high slippage
```bash
# This is normal on small trades
# Monitor actual P&L vs expected
# Slippage already factored into backtests
```

### Not enough trades
```bash
# Check logs: Are signals firing?
docker-compose logs | grep "Signal\|Decision"

# If no signals:
# - May be wrong market regime
# - Wait for more volatile market
# - Check technical indicators
```

## 🚦 Scale Up Process

```
Week 1: Testnet ($0 risk)
  ├─ Run 24-72 hours
  ├─ Validate all systems
  └─ Check win rate >60%

Week 2: Small Real ($50-100)
  ├─ Run 3-5 days
  ├─ Document all trades
  └─ Verify no bugs

Week 3-4: Medium ($200-300)
  ├─ Confident in system
  ├─ Positive P&L proven
  └─ Ready to scale

Month 2+: Full Capital ($500+)
  ├─ System proven
  ├─ Consistent returns
  └─ Can add more features
```

## 📖 Next Steps

1. **Get API Keys** (5 min)
2. **Deploy System** (10 min)
3. **Monitor Logs** (24 hours)
4. **Review Trades** (30 min)
5. **Go Live** (when ready)

---

**Ready? Start with:** `cp .env.example .env` then `docker-compose up -d`

