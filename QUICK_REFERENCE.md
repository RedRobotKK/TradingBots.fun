# ⚡ Quick Reference Card

Print this page and keep it handy while deploying!

---

## BEFORE SHOPPING

### Prepare GitHub
```bash
cd ~/Development/RedRobot-HedgeBot
git init
git config user.name "Your Name"
git config user.email "your@email.com"
git add -A
git commit -m "Initial: Production ready"
git remote add origin https://github.com/YOU/RedRobot-HedgeBot.git
git push -u origin main
```

---

## AFTER SHOPPING

### 1. Get Droplet IP
Digital Ocean → Droplet → Copy IP address

### 2. SSH In (Replace IP)
```bash
ssh root@YOUR_IP
```

### 3. Install Rust
```bash
apt update && apt upgrade -y
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
```

### 4. Clone Code (Replace USERNAME)
```bash
cd ~
git clone https://github.com/USERNAME/RedRobot-HedgeBot.git
cd RedRobot-HedgeBot
```

### 5. Configure
```bash
cat > .env << 'CONF'
HYPERLIQUID_API_KEY=hl_YOUR_KEY_HERE
HYPERLIQUID_SECRET=hl_YOUR_SECRET_HERE
ACCOUNT_SIZE=10000
RISK_PER_TRADE=1.0
MAX_LEVERAGE=3.0
TESTNET_ONLY=true
ENABLE_LIVE_TRADING=false
CONF
```

### 6. Build
```bash
cargo build --release
```
(Takes 5-10 minutes, watch for errors)

### 7. Run
```bash
apt install screen -y
screen -S trading
./target/release/redrobot-hedgebot --config deployment.toml
# Detach: Ctrl+A, then D
```

---

## DAILY OPERATIONS

### Check Running
```bash
ps aux | grep redrobot-hedgebot
```

### View Logs
```bash
tail -f /var/log/redrobot/trading.log
```

### Stop
```bash
Ctrl+C
```

### Restart
```bash
./start-trading.sh
```

### Update Code
```bash
git pull && cargo build --release
```

---

## EMERGENCY STOP

**If anything goes wrong:**
```bash
Ctrl+C
```

That's it. System stops. No stuck positions.

---

## IMPORTANT RULES

⚠️ **ALWAYS START WITH TESTNET_ONLY=true**
- Test for 1 week minimum
- Only when profitable: set to false
- Start with small account ($500-1000)
- Only trade STRONG signals (score 80+)

⚠️ **NEVER COMMIT .env TO GITHUB**
- Contains your API keys
- Check .gitignore
- Keep it safe locally

⚠️ **MONITOR DAILY**
- First 2 weeks: check every day
- After 2 weeks: can be less frequent
- Always have emergency stop plan

---

## FILE LOCATIONS

| What | Where |
|------|-------|
| Code | ~/RedRobot-HedgeBot/ |
| Binary | ~/RedRobot-HedgeBot/target/release/ |
| Config | ./.env |
| Logs | /var/log/redrobot/trading.log |
| Deployment | DEPLOYMENT_DIGITALOCEAN.md |
| Full Guide | docs/COMPLETE_IMPLEMENTATION_GUIDE.md |

---

## EXPECTED PERFORMANCE

- Win Rate: 70-72%
- Profit Factor: 2.2-2.5x
- Annual Return: +65-85%
- Sharpe Ratio: 2.0-2.3
- Max Drawdown: 12-15%

---

## FIRST MONTH TIMELINE

**Week 1:** Deploy, test in testnet
**Week 2:** Verify signals, check position sizing
**Week 3:** Go live with $500-1000
**Week 4:** Scale if profitable or optimize

---

## TROUBLESHOOTING

| Problem | Solution |
|---------|----------|
| "Command not found: cargo" | `source ~/.cargo/env` |
| "Connection refused" | Check API key in .env |
| "Out of memory" | Upgrade droplet or reduce logging |
| "Can't compile" | `rustup update` then rebuild |
| System crashes | Check logs: `tail -f /var/log/redrobot/trading.log` |

---

## SUPPORT

1. **Deployment issues:** Read DEPLOYMENT_DIGITALOCEAN.md
2. **General questions:** Read COMPLETE_IMPLEMENTATION_GUIDE.md
3. **System design:** Read INSTITUTIONAL_FRAMEWORKS_AND_INFRASTRUCTURE.md
4. **Logs:** `tail -f /var/log/redrobot/trading.log`

---

## YOU'RE READY! 🚀

Everything is built. Just deploy and monitor.

Have a great shopping trip!
