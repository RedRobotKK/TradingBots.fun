# 📊 Data Hierarchy: What Actually Matters for AI Trading Decisions

**Purpose:** Show the exact data pipeline and why each data feed is critical (or not)
**Audience:** You (to understand what to build)
**Honesty:** 100% - Showing what works vs what sounds good

---

## 🎯 The Data Stack: From RAW to DECISION

```
LEVEL 1: RAW DATA (Real-Time Feeds)
├─ CEX Order Books: Binance, Bybit, OKX (100-500ms latency) ✅ CRITICAL
│  └─ Why: Shows retail behavior, earliest signal detection
├─ DEX Order Books: Hyperliquid, Drift (<50ms latency) ✅ CRITICAL
│  └─ Why: Where we actually execute, entry price validation
├─ Volatility Metrics: Rolling 1m/5m/1h/1d measures ✅ CRITICAL
│  └─ Why: Determines safe leverage, risk calculation
├─ Portfolio State: Positions, capital, health factor ✅ CRITICAL
│  └─ Why: Risk management enforcement
└─ Sentiment Feeds: LunarCrush, on-chain metrics (1-5min staleness) ✅ IMPORTANT
   └─ Why: Confirmation signal, not primary signal

        ↓ (Process all in <50ms)

LEVEL 2: PROCESSED DATA (Signal Generation)
├─ Imbalance Ratios: bid_vol / ask_vol ✅ PRIMARY SIGNAL
│  └─ Calculation: Simple division, 0-2s stale is fine
├─ Order Book Shocks: Depth % change rate ✅ ACCELERATION SIGNAL
│  └─ Calculation: (current - previous) / previous
├─ Signal Confidence Scores: 0.65-0.95 ✅ CORE DECISION METRIC
│  └─ Calculation: Weighted combination of signals
├─ Volatility-Adjusted Leverage: 1-15x ✅ RISK METRIC
│  └─ Calculation: Inverse function of volatility
└─ Liquidation Safety Margins: Distance in bps ✅ VETO GATE
   └─ Calculation: (Health factor - liquidation threshold) / leverage

        ↓ (Combine all signals)

LEVEL 3: DECISION LAYER (AI Logic)
├─ Signal Confidence: 0.65-0.95 range ✅ GO/NO-GO GATE
│  └─ Rule: Skip if <0.65, trade if >0.65, aggressive if >0.80
├─ Risk Validation: <2% per trade, <5% total ✅ HARD LIMIT
│  └─ Rule: Reject ANY trade that violates limits
├─ RAG Pattern Matching: Historical similar signals ✅ CONTEXT
│  └─ Rule: Increase confidence if past performance good
├─ Portfolio Correlation: Current positions ✅ DIVERSIFICATION
│  └─ Rule: Skip if too correlated with existing positions
└─ Time Window Validity: Signal <5 seconds old ✅ FRESHNESS
   └─ Rule: Ignore stale signals, always use fresh data

        ↓ (Output)

LEVEL 4: EXECUTION (Trade or Skip)
├─ GO: Calculate position size + leverage ✅ EXECUTE
│  └─ Entry: Limit order with 10s timeout
│  └─ Target: Profit taking at +50-200 bps
│  └─ Stop: Loss protection at -100 bps
└─ SKIP: Save capital for better opportunities ✅ DISCIPLINE
   └─ Reason: Low confidence, risk unacceptable, or uncertain
```

---

## 🔢 Data Quality Scoring

### How Fresh Does Data Need To Be?

| Data Feed | Needed Freshness | Why | Impact if Stale |
|-----------|-----------------|-----|-----------------|
| DEX Mark Price | <10ms | Entry point | ❌ Miss signal |
| Order Book Imbalance | <500ms | Signal source | ⚠️ Slower decision |
| Volatility | <5 seconds | Risk calculation | ⚠️ Over-leverage |
| Portfolio State | <100ms | Risk check | ❌ Hit limits |
| CEX Data | <2 seconds | Confirmation | ⚠️ False signal |
| LunarCrush | <5 minutes | Sentiment | ✅ Still valid |
| On-chain metrics | <1 hour | Context | ✅ Still valid |

---

## 💡 Confidence Scoring: What Raises/Lowers It?

### Starting Point: 0.50 (Neutral)

**What ADDS Confidence:**

```
CEX Bid-Ask Imbalance:
  Ratio 1.3x → +0.05
  Ratio 1.5x → +0.10
  Ratio 1.8x → +0.15
  Ratio 2.0x+ → +0.20

DEX Confirmation:
  Matches CEX → +0.15
  Slightly different → +0.05
  Diverges → -0.20 (RED FLAG)

Sentiment Agreement:
  Very bullish (+0.7) → +0.10
  Moderately bullish (+0.5) → +0.05
  Neutral → +0.00

Order Book Shock:
  Depth +20% → +0.05
  Depth +30% → +0.10
  Depth +50% → +0.15

Volatility Status:
  Low vol (<30%) → +0.05
  Medium vol (30-50%) → +0.00
  High vol (50%+) → -0.10

Risk Check:
  All constraints met → +0.00 (required for any trade)
  Any constraint violated → -1.0 (SKIP SIGNAL)

RAG Historical Pattern:
  Similar pattern won 70%+ past → +0.10
  Similar pattern won 50-70% → +0.00
  Similar pattern won <50% → -0.10
```

**What I DEMAND Before Trading:**

```
Absolute Requirements (ALL must be YES):
├─ Confidence score ≥ 0.65
├─ Risk ≤ 2% of capital
├─ Health factor > 2.0
├─ Volatility acceptable for leverage
├─ DEX confirmation (not just CEX)
├─ No existing conflicting position
├─ Data freshness adequate
└─ No external risk factors (major news)
```

---

## 📈 Real World Example: Complete Data Pipeline

### Setup: You have $500, monitoring SOL/USDT

```
T+0ms: Signal Detected ────────────────────────────────────────

Raw Data Arrives:
├─ Binance SOL order book
│  ├─ Bid depth 1M: 5000 SOL
│  ├─ Ask depth 1M: 2500 SOL
│  ├─ Ratio: 2.0x (STRONG BUY signal)
│  └─ Timestamp: T+0 (fresh)
│
├─ Hyperliquid SOL order book
│  ├─ Bid depth: 3000 SOL
│  ├─ Ask depth: 2800 SOL
│  ├─ Ratio: 1.07x (NEUTRAL, not confirming)
│  └─ Timestamp: T+2ms (fresh)
│
├─ Current volatility: 2.1% (daily) = 33% annualized
│
├─ LunarCrush sentiment: +0.68 (bullish)
│
├─ On-chain: +12% more active addresses than yesterday
│
├─ Bybit funding: 0.07% per 8h (normal, not spiking)
│
└─ Your portfolio: $500 available, health factor 3.1

T+5ms: Signal Processing ───────────────────────────────────

My Decision Process:
"I see a 2.0x imbalance on Binance (buy signal)"
"But Hyperliquid shows only 1.07x imbalance (divergence)"

This is CRITICAL: CEX and DEX don't match!

My thinking:
  ├─ Possibility 1: Binance signal is correct, HLP hasn't moved yet
  │                → Could be early signal (good)
  ├─ Possibility 2: Binance is manipulated/fake volume
  │                → Could be trap (bad)
  └─ Solution: I'm skeptical of divergence

Score so far:
  Base: 0.50
  + 0.15 (Binance imbalance 2.0x): 0.65
  - 0.15 (HLP divergence): 0.50 ← NEUTRAL, not bullish
  + 0.10 (sentiment bullish): 0.60
  + 0.05 (on-chain positive): 0.65
  + 0.00 (vol OK, no bonus): 0.65

DECISION: Borderline. 0.65 is minimum threshold.
ANALYSIS: This is 65% confidence = very risky trade.

Position sizing if I trade:
  ├─ For 0.65 confidence: Only 2% risk
  ├─ Base size: 0.02 × $500 = $10
  ├─ Safe leverage with 33% vol: 2-3x
  ├─ Final: $10 position with 2x leverage = $20 notional
  ├─ Risk if stopped: $20 × 0.01 = $0.20
  └─ Expected profit if correct: $20 × 0.50 = $10

PROBLEM: Risk/reward is only 1:50 (bad).
Expected value: 0.65 × $10 - 0.35 × $0.20 = $6.43 - $0.07 = $6.36

Wait, that's positive... but tiny position ($10)

MY DECISION: ⏭️ SKIP THIS SIGNAL

Reason: CEX/DEX divergence is red flag
        Confidence only 0.65 (bare minimum)
        Position would be too small to matter
        Better signals will come

T+100ms: Signal Expires ────────────────────────────────────

15 seconds pass, Binance signal drops (other buyers exhausted it)
Hyperliquid stayed at 1.07x (confirming it was false)
Price actually dropped slightly (-5 bps)

If I had traded: Would have lost -$0.20 on $10 position

By SKIPPING: I saved $0.20 and waited for better signal
This is how you win at trading: Skip bad signals, catch good ones
```

---

## 🎯 Data I Need (Prioritized)

### For 90%+ Confidence Trades

```
MUST HAVE (Top Priority):
1. CEX Order Book (Binance) - Latest 500-1000 levels
   ├─ Update every 100-500ms
   ├─ Need: Bid volume, ask volume, spread
   └─ Cost: Free (REST API)

2. DEX Order Book (Hyperliquid) - Latest 200+ levels
   ├─ Update: Real-time (WebSocket)
   ├─ Need: Mark price, bid, ask, depth
   └─ Cost: Free (API included)

3. Volatility Measurement
   ├─ Calculate 1m rolling vol every 1 second
   ├─ Calculate 5m rolling vol every 5 seconds
   ├─ Need: Last 60 close prices
   └─ Cost: Free (calculated from price data)

4. Portfolio State (Your Positions)
   ├─ Current positions, entry prices, PnL
   ├─ Health factor (refreshed every 100ms)
   ├─ Capital available
   └─ Cost: Free (Hyperliquid API)

IMPORTANT (High Priority):
5. Funding Rates (Bybit Perpetuals)
   ├─ Update every 1 second
   ├─ Need: Current rate, next funding time
   └─ Cost: Free (Bybit WebSocket)

6. LunarCrush Sentiment
   ├─ Update every 5-15 minutes
   ├─ Need: Sentiment score, volume, growth
   └─ Cost: Free tier ($0, need API key signup)

7. On-Chain Metrics (Glassnode)
   ├─ Update every 1 hour
   ├─ Need: Active addresses, transactions, whale moves
   └─ Cost: Free tier ($0, need API key signup)

NICE TO HAVE (Lower Priority):
8. Long/Short Ratio (Bybit)
   ├─ Update every 5 minutes
   ├─ Need: Long %, short %
   └─ Cost: Free (Bybit API)

9. RAG Historical Database
   ├─ Stored in Supabase PostgreSQL
   ├─ Need: Past similar signals, outcomes, metrics
   └─ Cost: Free tier ($0)
```

---

## ❌ Data I DON'T Need (And Why Not)

### Overrated/Worthless Data Sources

```
❌ News Headlines
   Why: By time I read them, market already moved
   Alternative: On-chain metrics (real money signals)

❌ Social Media Sentiment (raw)
   Why: Too noisy, easily manipulated
   Alternative: Aggregated LunarCrush (professional analysis)

❌ Technical Analysis Indicators (alone)
   Why: Lagging indicators, everyone has same view
   Alternative: Order flow (what's ACTUALLY happening)

❌ Historical Backtests (as prediction)
   Why: Past ≠ future, overfitting risk
   Alternative: Current volatility + recent pattern matching

❌ Influencer Calls
   Why: Conflicts of interest, often pump & dumps
   Alternative: Order flow + sentiment (what whales actually doing)

❌ Crypto Whale Tracking (public data)
   Why: Delayed, manipulated, not real-time
   Alternative: Exchange flows data (real money moving)

❌ TV/News Crypto Commentary
   Why: Entertainment, not information
   Alternative: On-chain metrics (facts)

❌ Correlation Matrix (static)
   Why: Changes daily, not predictive
   Alternative: Current position overlap check
```

---

## 💻 The Complete Data Schema (What Gets Stored)

### Real-Time Cache (Fast Lookup)

```
Last 1 Minute (Every Entry):
├─ CEX order books (bid/ask, volumes, spread)
├─ DEX prices (mark, bid, ask)
├─ Your positions (size, entry, PnL)
├─ Health factor
└─ Recent trades (last 100)

Storage: In-memory (Redis or Vec in Rust)
TTL: 60 seconds
Purpose: Real-time decision making
```

### Recent History (Pattern Matching)

```
Last 24 Hours (Every Signal):
├─ All signals generated (CEX detected)
├─ All decisions made (GO or SKIP)
├─ All trades executed (entry, exit, PnL)
├─ Sentiment snapshots (every 5 minutes)
├─ Volatility measurements (every 1 minute)
└─ On-chain metrics snapshots (every 1 hour)

Storage: Supabase PostgreSQL
Purpose: RAG pattern matching, learning
Queries: "Similar signal win rate?", "What happened last time?"
```

### Historical Backtest (Long-term Learning)

```
Last 6-12 Months:
├─ Daily OHLCV (from public sources)
├─ Monthly pattern analysis
├─ Seasonal trends
├─ Correlation changes
└─ Volatility regimes

Storage: Supabase (TimescaleDB for efficiency)
Purpose: Quarterly review, strategy refinement
Queries: "Does this strategy work?", "What's the best time window?"
```

---

## 🎓 Confidence Score Examples

### Example 1: HIGH Confidence (90%)
```
CEX Binance SOL:
  ├─ Imbalance: 2.2x (bid 2200, ask 1000)
  ├─ Depth shock: +35% in 2 seconds
  └─ Signal strength: +0.25

DEX Hyperliquid SOL:
  ├─ Imbalance: 2.0x (matching CEX!)
  ├─ Mark price: $145.32 (confirmed entry)
  └─ Depth OK: +0.15

Sentiment:
  ├─ LunarCrush: +0.74 (very bullish)
  ├─ Active addresses: +18% vs yesterday
  └─ Funding: 0.09% (slightly bullish) +0.15

Risk:
  ├─ Volatility: 28% (acceptable)
  ├─ Health factor: 3.2 (safe)
  ├─ Capital available: $500
  └─ Pass all checks: +0.00

RAG Historical:
  ├─ Similar patterns: Won 72% historically
  └─ Signal strength bonus: +0.10

TOTAL CONFIDENCE: 0.25 + 0.15 + 0.15 + 0.25 + 0.10 = 0.90 ✅

DECISION: EXECUTE with 10% position size, 10x leverage = $500 notional
```

### Example 2: MEDIUM Confidence (72%)
```
CEX shows imbalance: 1.6x → +0.12
DEX shows imbalance: 1.5x → +0.10 (matches, good)
Sentiment: +0.62 → +0.07 (moderately bullish)
Volatility: 35% → +0.00 (neutral)
RAG: 58% win rate → +0.00 (mixed history)
Risk: All checks pass → +0.00

TOTAL: 0.12 + 0.10 + 0.07 + 0.00 + 0.00 = 0.29

Wait, that's only 29%? What's missing?

Oh, I'm forgetting:
Base confidence: 0.50 (starting point)
So total: 0.50 + 0.29 = 0.79 (wait, I calculated 0.72)

Actually, let me recalculate:
Base: 0.50
Imbalance signals: +0.12
DEX match: +0.10
Sentiment: +0.07
= 0.79 confidence

Hmm, that's slightly higher than my estimate. Let me use 0.79.

DECISION: Trade with 5% position size, 5x leverage = $125 notional
```

### Example 3: LOW Confidence (45%) - SKIP
```
CEX shows imbalance: 1.2x (weak) → +0.05
DEX shows: 1.0x (no imbalance, divergence!) → -0.15
Sentiment: 0.48 (neutral) → +0.00
Volatility: 52% (high) → -0.10
News: Major SEC hearing today → -0.10
Risk: Some constraints tight → Caution

Total: 0.50 + 0.05 - 0.15 + 0.00 - 0.10 - 0.10 = 0.20

But wait, CEX/DEX divergence is major red flag (-0.20 alone)

Recalculate:
Base: 0.50
Weak CEX signal: +0.05
Strong DEX divergence: -0.20
Neutral sentiment: +0.00
High vol reduction: -0.10
News risk: -0.10
= 0.15 (SKIP threshold <0.65)

DECISION: ❌ SKIP - Confidence too low, risks too high
```

---

## 🎯 Final Checklist: Do I Have What I Need?

✅ **BEFORE FIRST TRADE, I need:**
- [ ] Binance order book API working (100ms updates)
- [ ] Hyperliquid WebSocket connected (<50ms latency)
- [ ] Volatility calculation accurate (rolling 1m/5m)
- [ ] Health factor tracking working
- [ ] LunarCrush sentiment API connected
- [ ] Portfolio position tracking accurate
- [ ] Risk engine enforcing constraints
- [ ] RAG database populated with historical data
- [ ] Decision algorithm scoring confidence correctly
- [ ] Execution layer placing orders properly
- [ ] Stop loss/take profit working
- [ ] Monitoring dashboard showing real-time state

**Total dependencies: 12 systems working together**

---

**Status:** ✅ Complete data hierarchy documented
**Ready for:** Implementation in Rust with all data pipelines

