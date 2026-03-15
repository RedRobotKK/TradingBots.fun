# 🚀 Digital Ocean Deployment Guide - Step by Step

**For Beginners - Super Simple**
**Time Required:** 45 minutes
**Cost:** $6-12/month

---

## What You're Deploying

Your complete trading system:
- ✅ 26 strategies (21 technical + 5 institutional)
- ✅ Capital-efficient scoring system
- ✅ DCA/Pyramiding integration
- ✅ 4 institutional frameworks
- ✅ Real-time decision engine (<5ms)

---

## Prerequisites

1. GitHub account (free)
2. Digital Ocean account ($100 free credit)
3. Hyperliquid API key

---

## QUICK START - 10 STEPS

### Step 1: Prepare Your Code for GitHub

```bash
cd ~/Development/tradingbots-fun

# Initialize git if needed
git init
git config user.name "Your Name"
git config user.email "your@email.com"

# Add all files
git add -A

# Commit
git commit -m "Initial commit: Complete institutional trading system"
```

### Step 2: Create GitHub Repository

1. Go to github.com
2. Click "New Repository"
3. Name: tradingbots-fun
4. Set to PRIVATE
5. Click Create

### Step 3: Push Code to GitHub

```bash
# Add remote
git remote add origin https://github.com/YOUR_USERNAME/tradingbots-fun.git

# Push
git push -u origin main
```

### Step 4: Create Digital Ocean Account

1. Go to digitalocean.com
2. Sign up
3. Enter credit card (get $100 free credit)

### Step 5: Create Droplet

1. Click Create → Droplets
2. Choose:
   - Image: Ubuntu 22.04 LTS
   - Size: $6/month (2GB RAM)
   - Region: Closest to you
3. Click Create Droplet
4. Copy your IP address

### Step 6: Connect to Droplet

```bash
ssh root@YOUR_DROPLET_IP
```

### Step 7: Install System Dependencies + Rust

```bash
# Update package list
apt update && apt upgrade -y

# Install ALL required build dependencies in one command
# (These are required to compile Rust projects with networking/SSL)
apt-get install -y build-essential pkg-config libssl-dev

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

source ~/.cargo/env

rustc --version  # Verify

# What each package does:
# build-essential  → C compiler (cc/gcc) needed by Rust linker
# pkg-config       → Finds system libraries like OpenSSL
# libssl-dev       → OpenSSL development headers for HTTPS/TLS support
```

### Step 8: Clone Your Code

```bash
cd ~
git clone https://github.com/YOUR_USERNAME/tradingbots-fun.git
cd tradingbots-fun
```

### Step 9: Configure and Build

```bash
# Create .env file
cat > .env << 'EOF'
HYPERLIQUID_API_KEY=YOUR_API_KEY_HERE
HYPERLIQUID_SECRET=YOUR_SECRET_HERE
ACCOUNT_SIZE=10000
RISK_PER_TRADE=1.0
MAX_LEVERAGE=3.0
TESTNET_ONLY=true
ENABLE_LIVE_TRADING=false
LOG_LEVEL=INFO
EOF

# Build
cargo build --release
```

### Step 10: Run Your Trading System

```bash
# Install screen to keep it running
apt install screen -y

# Start a screen session
screen -S trading

# Inside screen, run:
cd ~/tradingbots-fun
./target/release/tradingbots-fun --config deployment.toml

# To detach: Press Ctrl+A, then Ctrl+D
# To reconnect: screen -r trading
```

---

## What Happens Next

Your trading system will:

1. ✅ Connect to Hyperliquid
2. ✅ Analyze all 26 strategies in real-time
3. ✅ Calculate scoring (0-100)
4. ✅ Detect market regime
5. ✅ Place trades automatically
6. ✅ Size positions correctly
7. ✅ Log everything
8. ✅ Run 24/7 without you

---

## IMPORTANT BEFORE LIVE TRADING

**ALWAYS start with TESTNET_ONLY=true**

Test for at least 1 week:
- [ ] Verify signals make sense
- [ ] Check position sizing
- [ ] Monitor P&L
- [ ] Review logs

Only when profitable in testnet:
1. Change TESTNET_ONLY=false
2. Start with $500-1000 account
3. Trade only STRONG signals (score 80+)
4. Wait 2 weeks before scaling

---

## Common Commands

```bash
# Check if running
ps aux | grep tradingbots-fun

# Check logs
tail -f /var/log/tradingbots/trading.log

# Stop
Ctrl+C

# Restart
./start-trading.sh

# Update code
git pull origin main
cargo build --release
```

---

## Troubleshooting

**"Command not found: cargo"**
```bash
source ~/.cargo/env
```

**"Connection refused"**
```bash
# Check your API key in .env
# Make sure it's not empty
```

**"Out of memory"**
```bash
# Upgrade to 4GB RAM droplet
# Or reduce strategies/logging
```

---

## That's It!

Your institutional-grade trading system is now:

✅ Deployed on the cloud
✅ Running 24/7
✅ Making trades automatically
✅ Managing risk properly
✅ Logging everything

**Go enjoy your shopping. Your system will work without you.** 🚀
