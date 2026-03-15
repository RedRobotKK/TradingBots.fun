# 🧪 tradingbots-fun: Complete Testnet Setup & Validation Guide

**Status:** Ready for 72-Hour Validation
**Capital:** 100 USDC equivalent (testnet tokens)
**Duration:** 72 hours continuous run
**Cost:** $0 (testnet is completely free)
**Goal:** Prove the system works before mainnet deployment

---

## 📋 Quick Start (5 minutes)

```bash
# 1. Get testnet tokens
## Hyperliquid testnet: https://hyperliquid.io/testnet
## Solana devnet: solana airdrop 10 -u devnet

# 2. Edit .env.testnet with your testnet credentials
# 3. Build the project
cd /sessions/confident-eloquent-wozniak/mnt/Development/tradingbots-fun
cargo build --release

# 4. Start the bot
source .env.testnet
nohup ./target/release/tradingbots > /tmp/tradingbots_testnet.log 2>&1 &

# 5. Monitor it
tail -f /tmp/tradingbots_testnet.log
```

---

## 🔧 Step 1: Configure Testnet Credentials

### 1.1 Create Hyperliquid Testnet Account

```bash
# Visit: https://hyperliquid.io/testnet
# 1. Connect Phantom wallet
# 2. Click "Request Testnet Funding"
# 3. Get 100+ USDC testnet tokens
# 4. Copy your wallet address (starts with 0x)
# 5. Note: Testnet address ≠ Mainnet address
```

### 1.2 Create Solana Devnet Wallet

```bash
# If you don't have a devnet keypair:
solana-keygen new --outfile ~/.config/solana/devnet_keypair.json

# Get free devnet SOL
solana airdrop 10 ~/.config/solana/devnet_keypair.json -u devnet

# Verify balance
solana balance -u devnet
# Expected: Should show 10 SOL
```

### 1.3 Update .env.testnet

```bash
# Edit: /path/to/tradingbots-fun/.env.testnet

# Replace these values:
HYPERLIQUID_WALLET=0x<YOUR_TESTNET_WALLET>
HYPERLIQUID_PRIVATE_KEY=<YOUR_TESTNET_PRIVATE_KEY_HEX>

DRIFT_KEYPAIR_PATH=/home/user/.config/solana/devnet_keypair.json
DRIFT_PRIVATE_KEY=<BASE58_ENCODED_PRIVATE_KEY>

# Or use helper script:
chmod +x scripts/setup-testnet-credentials.sh
./scripts/setup-testnet-credentials.sh
```

---

## 🏗️ Step 2: Build Production Binary

### 2.1 Verify Dependencies

```bash
# Check Rust version (need 1.70+)
rustc --version

# Update if needed
rustup update

# Verify cargo
cargo --version
```

### 2.2 Clean and Build

```bash
cd /sessions/confident-eloquent-wozniak/mnt/Development/tradingbots-fun

# Clean previous builds
cargo clean

# Build with optimizations
cargo build --release --features full

# Expected compile time: 3-5 minutes
# Result: target/release/tradingbots (~42MB)
```

### 2.3 Verify Build

```bash
# Check binary exists
ls -lh target/release/tradingbots

# Test binary
./target/release/tradingbots --version
# Expected: tradingbots-fun 1.0.0

# Run all tests
cargo test --release -- --nocapture
# Expected: All tests passing
```

---

## 🚀 Step 3: Start Testnet Bot

### 3.1 Launch the Bot

**Option A: Foreground (for quick testing)**
```bash
source .env.testnet
./target/release/tradingbots

# You'll see output like:
# [2026-02-21T14:32:15Z] 🤖 Autonomous trader started
# [2026-02-21T14:32:16Z] ✓ Connected to Hyperliquid
# [2026-02-21T14:32:17Z] ✓ Connected to Drift
# [2026-02-21T14:33:00Z] 📊 Decision: BUY
```

**Option B: Background (for 72-hour run)**
```bash
source .env.testnet

# Start in background
nohup ./target/release/tradingbots > /tmp/tradingbots_testnet.log 2>&1 &

# Save PID
echo $! > /tmp/tradingbots.pid

# Verify it's running
ps -p $(cat /tmp/tradingbots.pid)

# View live logs
tail -f /tmp/tradingbots_testnet.log
```

**Option C: systemd Service (Recommended)**
```bash
# Create service file
sudo tee /etc/systemd/system/tradingbots-testnet.service > /dev/null <<EOF
[Unit]
Description=tradingbots-fun Testnet Validation
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=$USER
WorkingDirectory=$HOME/tradingbots-fun
EnvironmentFile=$HOME/tradingbots-fun/.env.testnet
ExecStart=$HOME/tradingbots-fun/target/release/tradingbots
Restart=always
RestartSec=10
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
EOF

# Enable and start
sudo systemctl daemon-reload
sudo systemctl enable tradingbots-testnet
sudo systemctl start tradingbots-testnet

# Check status
sudo systemctl status tradingbots-testnet
sudo journalctl -u tradingbots-testnet -f
```

### 3.2 Verify Bot Started

```bash
# Check if process is running
ps aux | grep tradingbots | grep -v grep

# Check logs for startup messages
head -20 /tmp/tradingbots_testnet.log

# Expected output:
# [2026-02-21T14:32:15Z] Loading configuration...
# [2026-02-21T14:32:16Z] Connecting to Hyperliquid testnet API...
# [2026-02-21T14:32:17Z] Connecting to Drift protocol...
# [2026-02-21T14:32:18Z] Account manager initialized (5 accounts)
# [2026-02-21T14:32:19Z] Capital manager ready
# [2026-02-21T14:32:20Z] Liquidation prevention active
# [2026-02-21T14:32:21Z] 🤖 Autonomous trader started
```

---

## 📊 Step 4: Monitor for 72 Hours

### 4.1 Use the Monitoring Dashboard

```bash
# Open dashboard.html in your browser
# File: /path/to/tradingbots-fun/dashboard.html
# Auto-refreshes every 30 seconds
# Shows:
#   - Live performance metrics
#   - Trading activity
#   - Risk management status
#   - System health
#   - Recent logs
#   - Validation checklist
```

### 4.2 Use Command-Line Monitoring

```bash
# Run the monitoring script
bash scripts/monitor-testnet.sh

# This shows:
✅ Bot Status
📊 Recent Log Activity
📈 Trade Statistics
❌ Error Count
💻 System Resources
⏱️  Runtime
🏥 Latest Health Check
✅ Validation Checklist
```

### 4.3 Manual Log Monitoring

```bash
# View last 20 lines
tail -20 /tmp/tradingbots_testnet.log

# View last 100 lines
tail -100 /tmp/tradingbots_testnet.log

# Follow in real-time
tail -f /tmp/tradingbots_testnet.log

# Search for specific events
grep "Trade executed" /tmp/tradingbots_testnet.log | wc -l
grep "ERROR" /tmp/tradingbots_testnet.log | wc -l
grep "Health check" /tmp/tradingbots_testnet.log | tail -10
```

### 4.4 Hourly Health Checks

**Every 4 hours, check:**
```bash
# Is bot still running?
pgrep -f "target/release/tradingbots" > /dev/null && echo "✅ Running" || echo "❌ Stopped"

# How many trades?
grep -c "Trade executed" /tmp/tradingbots_testnet.log

# Any critical errors?
grep "ERROR\|PANIC\|panic" /tmp/tradingbots_testnet.log | tail -5

# Recent performance
tail -50 /tmp/tradingbots_testnet.log | grep "P&L\|Health\|Win rate"
```

---

## ✅ Step 5: Validation Checklist

### Hour 0-12 (Initial Validation)
```
□ Bot started without errors
□ Connected to both APIs (Hyperliquid + Drift)
□ First trades executing (within 1 minute of startup)
□ No authentication errors
□ Logs show normal activity

Expected: Trading every 1 second, healthy checks every 5 seconds
```

### Hour 12-36 (Mid-Point Check)
```
□ >500 trades executed
□ Win rate 55-65%
□ No catastrophic errors
□ Max drawdown < -10%
□ Health factor > 1.5 (safe)
□ All 5 accounts have activity

Expected: Consistent trading pattern, small daily profit
```

### Hour 36-72 (Final Validation)
```
□ >1,500 trades executed
□ Win rate stable 55-65%
□ Liquidation prevention triggered at least once
□ Zero emergency stops
□ Health factor recovered after alerts
□ Rebalancing happening (every 5 min)
□ Performance matches expectations

Expected: Fully functional autonomous system
```

---

## 📈 Success Criteria

### The Bot is Working If:
```
✅ Trades executing every 1-5 seconds
✅ Win rate 55-65% (same as backtest)
✅ Daily P&L oscillates around +0.5% to +1.5%
✅ Health checks every 5 seconds show healthy status
✅ Rebalancing happens every 5 minutes
✅ No errors except occasional warnings
✅ Zero unexpected crashes/restarts
✅ API latency 20-50ms
✅ CPU usage < 10%
✅ Memory usage < 500MB
```

### Red Flags (Stop and Fix):
```
❌ No trades executing (connection issue)
❌ Win rate < 45% (signal generation problem)
❌ Repeated "ERROR" in logs
❌ Health factor trending down without recovery
❌ API connection timeouts
❌ Memory usage > 2GB
❌ Bot crashes repeatedly
❌ Orders failing consistently
```

---

## 🔧 Troubleshooting

### Issue: Bot starts but no trades execute

```bash
# Check API connectivity
curl https://testnet-api.hyperliquid.com/ping
# Expected: 200 OK

# Verify credentials
grep "HYPERLIQUID_WALLET\|DRIFT_RPC" .env.testnet
# Expected: Should show your testnet wallet

# Check logs for errors
grep "ERROR\|Connection\|Auth" /tmp/tradingbots_testnet.log
```

### Issue: Low win rate (< 50%)

```bash
# This might be normal in testnet with small data
# Check if it's a signal problem or just variance

# Analyze trades
grep "Trade executed" /tmp/tradingbots_testnet.log | head -50

# The backtest used full year 2024
# Testnet might have different market conditions
# Win rate variance of ±10% is acceptable in short runs
```

### Issue: Health factor dropping

```bash
# This is expected - the system will auto-correct
# Check liquidation prevention activating:
grep "Health Alert\|Reduce\|Liquidation" /tmp/tradingbots_testnet.log

# Normal sequence:
# Level 1 (1.5): Warning, reduce 10%
# Level 2 (1.2): Reduce positions 25%
# Level 3 (1.0): Emergency deleveraging

# If Level 3 triggered, that's still working correctly!
# It means the system protected itself
```

### Issue: "Permission denied" errors

```bash
# Fix file permissions
chmod +x target/release/tradingbots
chmod 600 .env.testnet

# If systemd service:
sudo chown -R $USER:$USER /path/to/tradingbots-fun
```

### Issue: Out of memory

```bash
# Check memory usage
ps aux | grep tradingbots | grep -v grep | awk '{print $6}'

# If > 2GB, there's a memory leak
# This shouldn't happen - report to development

# Restart the bot
pkill -f "target/release/tradingbots"
sleep 5
source .env.testnet
nohup ./target/release/tradingbots > /tmp/tradingbots_testnet.log 2>&1 &
```

---

## 📊 Data Collection

### Save Logs After 72 Hours

```bash
# Create results directory
mkdir -p testnet_results

# Copy logs
cp /tmp/tradingbots_testnet.log testnet_results/

# Extract metrics
bash scripts/extract-testnet-metrics.sh > testnet_results/metrics.txt

# Create summary report
cat > testnet_results/summary.txt << EOF
Testnet Validation Complete
Start Time: 2026-02-21T14:32:15Z
Duration: 72 hours
Total Trades: $(grep -c "Trade executed" /tmp/tradingbots_testnet.log)
Win Rate: $(grep "Win rate:" /tmp/tradingbots_testnet.log | tail -1)
Max Drawdown: $(grep "Max drawdown:" /tmp/tradingbots_testnet.log | tail -1)
Final P&L: $(grep "Total P&L:" /tmp/tradingbots_testnet.log | tail -1)
EOF

# Commit to GitHub
git add testnet_results/
git commit -m "Testnet validation results - 72 hour run"
git push origin main
```

---

## 🎉 After 72 Hours: Next Steps

### If Testnet Validation Passed:
```
✅ All systems working
✅ Performance as expected
✅ Ready for mainnet

PROCEED TO:
1. Read SMALL_CAPITAL_DEPLOYMENT.md
2. Fund mainnet accounts with $300-500
3. Deploy to mainnet
4. Start earning real returns
```

### If Issues Found:
```
⚠️  Issues encountered

NEXT STEPS:
1. Review logs and identify root cause
2. Fix the issue in code
3. Commit fix to GitHub
4. Run another 24-hour testnet validation
5. Once stable, proceed to mainnet
```

---

## 📋 Testnet Validation Checklist

```
PRE-DEPLOYMENT (Before 72 hours):
☐ Rust 1.70+ installed
☐ Cargo build successful
☐ All tests passing
☐ .env.testnet properly configured
☐ Testnet accounts created and funded
☐ Testnet tokens received (100+ USDC, 10+ SOL)
☐ Dashboard opened in browser
☐ Monitoring script made executable

DURING VALIDATION (72 hours):
☐ Hour 0: Bot started successfully
☐ Hour 6: First metrics collected
☐ Hour 12: Mid-point health check completed
☐ Hour 24: 24-hour checkpoint (P&L analyzed)
☐ Hour 36: Mid-point validation passed
☐ Hour 48: 48-hour checkpoint (W/R stable)
☐ Hour 60: Final health assessment
☐ Hour 72: Validation complete

POST-VALIDATION:
☐ Results saved to GitHub
☐ Summary report created
☐ Performance analysis completed
☐ Mainnet deployment plan reviewed
☐ Capital ($300-500) ready to deploy
☐ Mainnet credentials prepared
```

---

## 🚀 Ready to Deploy?

Once 72 hours of testnet validation is complete and everything passes:

1. **Push results to GitHub**
   ```bash
   git add -A
   git commit -m "Testnet validation passed - ready for mainnet"
   git push origin main
   ```

2. **Read deployment guide**
   ```
   Next: SMALL_CAPITAL_DEPLOYMENT.md
   ```

3. **Deploy to mainnet**
   ```
   Timeline: Within 24 hours of testnet completion
   ```

---

**Status:** ✅ Ready for 72-Hour Testnet Validation
**Timeline:** Start immediately
**Expected Completion:** 3 days
**Next Step:** Follow "Step 1: Configure Testnet Credentials" above

Good luck with your testnet validation! 🎉

