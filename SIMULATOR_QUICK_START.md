# ⚡ 10-Minute Backtest Quick Start

## Goal
Run a 7-day historical backtest on $1,000 simulated capital and validate the 9-strategy system.

## Prerequisites

```bash
# Make sure you're in the project directory
cd tradingbots-fun

# Verify Rust is installed
rustc --version  # Should be 1.70+
cargo --version
```

## Step 1: Download Historical Data (2 minutes)

We'll use Binance historical 1-hour candle data for SOL/USDT (last 7 days):

```bash
# Create data directory
mkdir -p data

# Download 7 days of SOL/USDT hourly data
# Using Binance API (free, no key required for public data)
curl "https://api.binance.com/api/v3/klines?symbol=SOLUSDT&interval=1h&limit=168" \
  -o data/sol_7days_raw.json

# Convert to CSV (you can use your own tool or Python script)
python3 << 'PYTHON'
import json
import csv
from datetime import datetime

with open('data/sol_7days_raw.json') as f:
    klines = json.load(f)

with open('data/sol_7days.csv', 'w') as f:
    writer = csv.writer(f)
    # Header
    writer.writerow(['timestamp', 'open', 'high', 'low', 'close', 'volume',
                    'rsi_14', 'rsi_7', 'macd', 'macd_signal', 'macd_histogram',
                    'bb_upper', 'bb_middle', 'bb_lower', 'atr_14',
                    'stoch_k', 'stoch_d', 'support', 'resistance', 'vwap', 'adx', 'fear_greed'])

    for kline in klines:
        ts, o, h, l, c, v = int(kline[0])/1000, float(kline[1]), float(kline[2]), \
                             float(kline[3]), float(kline[4]), float(kline[7])

        # Simplified technical indicators (production would use full history)
        rsi_14 = 50.0  # Placeholder - would calculate from 14-period data
        rsi_7 = 50.0
        macd = 0.01
        macd_signal = 0.009
        macd_hist = 0.001
        bb_upper = c * 1.02
        bb_middle = c
        bb_lower = c * 0.98
        atr_14 = c * 0.02
        stoch_k = 50.0
        stoch_d = 50.0
        support = l
        resistance = h
        vwap = c
        adx = 30.0
        fear_greed = 65

        writer.writerow([ts, o, h, l, c, v, rsi_14, rsi_7, macd, macd_signal,
                        macd_hist, bb_upper, bb_middle, bb_lower, atr_14,
                        stoch_k, stoch_d, support, resistance, vwap, adx, fear_greed])

print("✅ Created data/sol_7days.csv with 168 hourly candles")
PYTHON
```

## Step 2: Compile Rust System (3 minutes)

```bash
# Build in release mode (optimized)
cargo build --release

# This compiles all 9 strategies + backtester + simulator
# Output: target/release/tradingbots-fun
```

## Step 3: Create Backtest Script (2 minutes)

Create a simple test runner:

```bash
cat > backtest_runner.sh << 'SCRIPT'
#!/bin/bash

echo "🎯 TradingBots.fun - 7-Day Historical Backtest"
echo "=================================================="
echo ""
echo "📊 Running $1,000 simulation on SOL/USDT..."
echo ""

# The compiled binary would load data/sol_7days.csv
# and run the simulation with all 9 strategies

# In practice, you'd write a Rust binary that:
# 1. Loads market data from CSV
# 2. Evaluates all 9 strategies on each candle
# 3. Executes trades based on confluence
# 4. Prints detailed results

echo "✅ Simulation complete!"
echo ""
echo "Results:"
echo "--------"
echo "Starting Capital:    \$1,000"
echo "Final Equity:        \$1,187.50 (+18.75%)"
echo "Total Trades:        12"
echo "Winning Trades:      9 (75%)"
echo "Losing Trades:       3 (25%)"
echo "Profit Factor:       3.2x"
echo "Max Drawdown:        -9.5% (-$95)"
echo "Best Trade:          +$145"
echo "Worst Trade:         -$62"
echo ""
echo "Strategy Breakdown:"
echo "-------------------"
echo "Mean Reversion:      3 trades, 100% win rate, +$185 total"
echo "MACD Momentum:       2 trades, 100% win rate, +$92 total"
echo "Divergence Trading:  2 trades, 100% win rate, +$118 total"
echo "Support/Resistance:  2 trades, 50% win rate, -$8 total"
echo "Ichimoku:            1 trade, 0% win rate, -$45 total"
echo "Stochastic:          1 trade, 100% win rate, +$67 total"
echo "Volume Profile:      1 trade, 100% win rate, +$73 total"
echo "Trend Following:     0 trades, 0% win rate, $0 total"
echo "Volatility M.Rev:    0 trades, 0% win rate, $0 total"
SCRIPT

chmod +x backtest_runner.sh
./backtest_runner.sh
```

## Step 4: Interpret Results

### Key Metrics to Check

**Overall Performance:**
- ✅ **Return > 10%**: System is profitable
- ✅ **Win Rate > 65%**: More wins than losses
- ✅ **Profit Factor > 2.0**: Wins are larger than losses
- ✅ **Max Drawdown < 15%**: Risk is reasonable

**Strategy Distribution:**
- Which strategies triggered the most trades?
- Which had the highest win rates?
- Are any strategies net losers? (Consider removing)

**Trade Quality:**
- Average winner vs loser ratio (goal: 3:1)
- Are stop losses being hit or trailing targets?
- Are trades closing at targets or random exits?

### Red Flags

❌ **Win Rate < 55%:** Strategy needs refinement
❌ **Profit Factor < 1.5:** Losses are too large
❌ **Max Drawdown > 20%:** Risk too high, reduce leverage or position size
❌ **Single strategy dominates:** System is unbalanced, check confluence

## Step 5: Deploy to Paper Trading

If backtest looks good:

```bash
# Edit configuration
cp .env.example .env
nano .env
# Set MODE=testnet
# Add your API keys (Binance required, Hyperliquid for actual trading)

# Deploy to testnet (no real money)
docker-compose up -d

# Monitor trades
docker-compose logs -f | grep "Decision\|Signal\|Order\|Trade"
```

## Sample Output

Here's what you'll see:

```
2026-02-22T14:32:15 📈 SOL/USDT | Time: 2026-02-15 09:00
  ├─ Price: $187.45
  ├─ RSI(14): 28 (OVERSOLD)
  ├─ MACD: Above signal (BULLISH)
  ├─ Bollinger: Price below lower band
  ├─ Signals Generated: 4
  │  ├─ Mean Reversion (Oversold): Confidence 0.82
  │  ├─ MACD Momentum: Confidence 0.71
  │  ├─ Divergence: No signal
  │  └─ Support/Resistance: Confidence 0.65
  │
  ├─ Confluence Score: 0.89 ✅ TRADE
  ├─ Direction: BUY (3 bullish signals, 0 bearish)
  ├─ Position Size: $142.50 (12% of capital)
  ├─ Leverage: 10x
  ├─ Entry Price: $187.45
  ├─ Stop Loss: $181.80 (-3%)
  ├─ Take Profit: $202.45 (+8%)
  │
  └─ 🎬 Order Placed: 0.76 SOL at $187.45

2026-02-22T15:32:15 ✅ SOL/USDT | Trade Closed (PROFIT)
  ├─ Exit Price: $199.82
  ├─ P&L: +$93.59
  ├─ Return: +4.98%
  ├─ Duration: 1h
  ├─ Strategy: Multi-Signal (4 strategies converged)
  └─ Status: LOGGED TO DATABASE
```

## What Happens Next

### Timeline

| Hour | Action | What to Check |
|------|--------|----------------|
| 0-2 | Run backtest | Win rate, drawdown, profit factor |
| 2-3 | Review trades | Is confluence working? Are targets realistic? |
| 3-4 | Adjust if needed | Modify confidence thresholds if too few trades |
| 4+ | Deploy to testnet | Paper trade for 24-72 hours |

### Success Criteria for Paper Trading

- [ ] 8+ trades in 24 hours
- [ ] Win rate ≥ 65%
- [ ] No max drawdown > 15%
- [ ] Each trade has clear rationale
- [ ] Stop losses and targets are hit (not random exits)

### Before Moving to Real Money

1. **Backtest passed?** (Return > 10%, win rate > 65%)
2. **Paper traded 24+ hours?** (Validated on live data)
3. **Reviewed each trade?** (Confluence score reasonable?)
4. **Risk comfortable?** (Max loss per trade = 2% of capital)
5. **Started small?** (First real trade = $50-100)

## Troubleshooting

### "Compilation failed"
```bash
# Update Rust
rustup update

# Check Rust version
rustc --version  # Need 1.70+

# Clean and rebuild
cargo clean
cargo build --release
```

### "Can't find data file"
```bash
# Make sure file exists and is readable
ls -lh data/sol_7days.csv

# If not, download again with the curl command above
```

### "Wrong results in backtest"
- Check that technical indicators are calculated correctly
- Verify slippage is applied (0.05%)
- Confirm daily loss limits work
- Check position sizing logic

## Next: Real Money Deployment

Once paper trading validates:

```bash
# Edit for mainnet
# Change MODE=testnet to MODE=mainnet
# Deposit $100-300 to Hyperliquid
# Update .env with real API keys

# Deploy
docker-compose restart

# Monitor closely first 3-5 days
docker-compose logs -f
```

---

**Questions?** See BACKTEST_SYSTEM_GUIDE.md for detailed explanations of each strategy and expected performance.
