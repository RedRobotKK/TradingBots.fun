# Dashboard Specification: Real-Time Monitoring

## Dashboard Architecture

```
DASHBOARD TECHNOLOGY STACK:
═════════════════════════════════════════════════════════════

Frontend:
├─ Framework: React + TypeScript
├─ Real-time: WebSocket for live updates (<100ms lag)
├─ Charts: Recharts (price, P&L, indicators)
├─ Styling: Tailwind CSS
└─ Build: Vite (fast development)

Backend:
├─ API Server: Rust Actix-web (same codebase)
├─ WebSocket: tokio-tungstenite
├─ Database: Supabase (PostgreSQL)
└─ Live Feed: CEX data (Binance, GMGN whale movements)

Deployment:
├─ Frontend: Vercel (free tier sufficient)
├─ Backend: Digital Ocean ($5-10/month VPS)
└─ Database: Supabase (free tier sufficient)
```

---

## Dashboard Pages

### 1. System Status Dashboard (Main)

```
┌─────────────────────────────────────────────────────────────────┐
│                    REDROBOT TRADING SYSTEM                      │
│                      [Status: LIVE ✓]                           │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ACCOUNT METRICS            │  DAILY PERFORMANCE                │
│  ────────────────────────   │  ──────────────────────────────   │
│  Capital:  $300.00          │  Today P&L:      +$12.50 (+4.2%)  │
│  Equity:   $312.50 ↑        │  Win Rate:       72% (18/25)      │
│  Health:   4.2 (SAFE) ✓     │  Best Trade:     +2.34%           │
│  Margin:   34% used         │  Worst Trade:    -0.89%           │
│                             │  Avg Win:        +0.82%           │
│                             │  Avg Loss:       -0.63%           │
│                             │  Profit Factor:  1.87              │
│                                                                 │
├─────────────────────────────────────────────────────────────────┤
│  POSITIONS (2 OPEN)                                             │
│  ─────────────────────────────────────────────────────────────  │
│                                                                 │
│  SOL/USDT (LONG)             │  BTC/USDT (LONG)                │
│  ├─ Entry:      $140.25      │  ├─ Entry:      $43,200         │
│  ├─ Current:    $142.80      │  ├─ Current:    $43,450         │
│  ├─ Unrealized: +$1.08       │  ├─ Unrealized: +$250           │
│  ├─ Leverage:   10x          │  ├─ Leverage:   5x              │
│  ├─ Strategy:   Order Flow   │  ├─ Strategy:   Trend Follow    │
│  ├─ Time Open:  2h 14m       │  ├─ Time Open:  1h 47m          │
│  └─ SL: $135.50              │  └─ SL: $41,800                 │
│                                                                 │
├─────────────────────────────────────────────────────────────────┤
│  ACTIVE ALERTS                                                  │
│  ──────────────────────────────────────────────────────────────  │
│                                                                 │
│  ⚠️  WARNING: Health Factor Decreasing (4.2 → 3.8 in 2 hours)  │
│      Action: Reduce position size on next entry                │
│                                                                 │
│  ✓  INFO: Whale movement detected (whaleX bought $5M SOL)      │
│      Action: Increased confidence on next SOL signals          │
│                                                                 │
│  ✓  OK: All systems operational                                │
│      Database sync: OK | CEX feeds: OK | GMGN: OK              │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

**Key Metrics Display:**

```rust
pub struct DashboardMetrics {
    // Account
    pub total_capital: f64,
    pub current_equity: f64,
    pub daily_pnl: f64,
    pub monthly_pnl: f64,
    pub health_factor: f64,
    pub margin_used_pct: f64,

    // Performance
    pub win_rate: f64,
    pub profit_factor: f64,
    pub sharpe_ratio: f64,
    pub max_drawdown: f64,
    pub trades_today: i32,
    pub best_trade_pnl: f64,
    pub worst_trade_pnl: f64,

    // Positions
    pub open_positions: Vec<OpenPosition>,
    pub closed_trades_today: i32,

    // System
    pub system_uptime: Duration,
    pub last_trade_time: i64,
    pub last_signal_time: i64,
}

pub struct OpenPosition {
    pub symbol: String,
    pub direction: String,              // LONG / SHORT
    pub entry_price: f64,
    pub current_price: f64,
    pub unrealized_pnl: f64,
    pub position_size: f64,
    pub leverage: f64,
    pub entry_time: i64,
    pub strategy: String,
    pub confidence: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
}
```

---

### 2. Strategy & Signals Dashboard

```
┌─────────────────────────────────────────────────────────────────┐
│                    STRATEGY BREAKDOWN                           │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  STRATEGY PERFORMANCE (Last 30 Days)                            │
│                                                                 │
│  1. Order Flow + Confluence     │ Trades:  12  │ Win%: 83%     │
│     └─ Avg P&L:  +1.24%        │ Sharpe:  2.1  │ Max DD: -12%  │
│                                                                 │
│  2. Trend Following Pullback    │ Trades:  18  │ Win%: 76%     │
│     └─ Avg P&L:  +0.87%        │ Sharpe:  1.8  │ Max DD: -8%   │
│                                                                 │
│  3. Divergence Trading          │ Trades:  8   │ Win%: 79%     │
│     └─ Avg P&L:  +1.03%        │ Sharpe:  2.3  │ Max DD: -10%  │
│                                                                 │
│  4. Mean Reversion              │ Trades:  15  │ Win%: 68%     │
│     └─ Avg P&L:  +0.65%        │ Sharpe:  1.4  │ Max DD: -15%  │
│                                                                 │
├─────────────────────────────────────────────────────────────────┤
│  SIGNAL HISTORY (Last 24 Hours)                                 │
│  ──────────────────────────────────────────────────────────────  │
│                                                                 │
│  14:35:22 | Order Flow + Confluence | SOL | LONG | 0.88      │
│            Entry: $140.25 | Exit: $142.80 | ✓ +1.81%          │
│            Signals: Imbalance 3.0x + RSI 32 + Divergence      │
│                                                                 │
│  15:47:15 | Trend Pullback | BTC | LONG | 0.75               │
│            Entry: $43,100 | Current: $43,450 | ↑ +0.81%       │
│            Signals: Support + 20MA bounce + MACD +            │
│                                                                 │
│  16:22:08 | Mean Reversion | ETH | LONG | 0.71               │
│            Entry: $2,310 | Exit: $2,295 | ✗ -0.65%            │
│            Signals: RSI 28 + Bollinger lower touch            │
│                                                                 │
│  [Scroll for more...]                                          │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

---

### 3. Technical Analysis Dashboard

```
┌──────────────────────────────────────────────────┬──────────────┐
│              SOL/USDT Technical Analysis         │ 1h timeframe│
├──────────────────────────────────────────────────┴──────────────┤
│                                                                 │
│  PRICE CHART (with indicators)                                 │
│  ────────────────────────────────────────────────────────────  │
│                                                                 │
│   150 ├─────────────────────┐                                  │
│       │                     │ RSI: 45 (neutral)                │
│   145 ├─────────────────────┤─ Resistance                      │
│       │ Price: $142.80      │                                  │
│   140 ├─────────────────────┤─ Support (strong)                │
│       │    ✓ Entry here     │                                  │
│   135 ├─────────────────────┘─ Support (weak)                  │
│       │                                                        │
│   130 └────────────────────────────────────────               │
│       └─ 20 candles ─────────────────────────→ Now            │
│                                                                 │
│  INDICATORS                                                    │
│  ┌────────────────────────────────────────────────────────┐   │
│  │ RSI (14):        45.2 (Neutral)                        │   │
│  │ Bollinger Bands: $138.50 - $145.30 (wide)             │   │
│  │ MACD:            +1.24 (positive, strengthening)      │   │
│  │ Stochastic K%:   32 (oversold region)                 │   │
│  │ Stochastic D%:   35                                    │   │
│  │ ATR:             3.2 (low volatility)                 │   │
│  │ Support:         $140 (strong, 4 touches)             │   │
│  │ Resistance:      $145 (weak, 2 touches)               │   │
│  │ Ichimoku Trend:  Bullish (price above cloud)          │   │
│  │ ADX:             18 (ranging market)                   │   │
│  │ Regime:          CONSOLIDATING (watch for breakout)   │   │
│  └────────────────────────────────────────────────────────┘   │
│                                                                 │
│  SIGNAL CONFLUENCE                                             │
│  ┌────────────────────────────────────────────────────────┐   │
│  │ ✓ Support bounce confirmed                            │   │
│  │ ✓ Divergence (RSI higher, price lower)                │   │
│  │ ✓ MACD turning positive                               │   │
│  │ ✓ Stochastic about to cross bullish                   │   │
│  │ ✗ ADX shows ranging, not trending yet                 │   │
│  │ ✗ Resistance above ($145) not broken                  │   │
│  │ ✓ Volume increasing on support                        │   │
│  │ ✓ Fear & Greed: 24 (extreme fear, contrarian long)   │   │
│  │                                                        │   │
│  │ TOTAL SIGNALS: 6/9 → Confidence: 0.82 (GOOD ENTRY)   │   │
│  └────────────────────────────────────────────────────────┘   │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

---

### 4. Whale Intelligence Dashboard

```
┌─────────────────────────────────────────────────────────────────┐
│              WHALE MOVEMENT & ON-CHAIN ACTIVITY                │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  RECENT WHALE MOVEMENTS (Last 6 Hours)                         │
│  ───────────────────────────────────────────────────────────── │
│                                                                 │
│  16:15:22 | Whale X      | SOL     | DEPOSIT_TO_BINANCE       │
│            Amount: $8.2M | Accuracy: 78% (likely to sell)     │
│            Confidence Impact: -0.22 (BEARISH signal)          │
│            Status: Monitoring for dump within 24h             │
│                                                                 │
│  15:42:10 | Whale Y      | SOL     | STAKING (365-day lock)  │
│            Amount: $3.5M | Duration: 365 days                  │
│            Supply Removed: 0.012% of total SOL                │
│            Confidence Impact: +0.20 (BULLISH signal)          │
│            Status: Supply reduction = price support           │
│                                                                 │
│  14:28:35 | Whale Z      | BTC     | ACCUMULATION (support)  │
│            Amount: $5.2M | Price: $43,100 (historical level)  │
│            Whale History: 85% profitable on BTC buys          │
│            Confidence Impact: +0.18 (BULLISH signal)          │
│            Status: Smart money accumulating                    │
│                                                                 │
│  WHALE PROFILES (Known Traders We Track)                       │
│  ─────────────────────────────────────────────────────────────  │
│                                                                 │
│  Whale X (Professional Trader)                                 │
│  ├─ Total Trades: 284                                          │
│  ├─ Win Rate: 78%                                              │
│  ├─ Sell Accuracy: 78% (after CEX deposit, price drops)       │
│  ├─ Avg Trade Size: $2.5M                                      │
│  ├─ Specialization: SOL, DOGE                                  │
│  └─ Last 7 Trades: +5/7 wins (71%)                            │
│                                                                 │
│  Whale Y (Hodler/Staker)                                       │
│  ├─ Total Trades: 42                                           │
│  ├─ Win Rate: 89%                                              │
│  ├─ Hold Pattern: Locks tokens for months                      │
│  ├─ Avg Trade Size: $3.2M                                      │
│  ├─ Specialization: SOL, ETH                                   │
│  └─ Last Signal: Bullish (just staked $3.5M)                  │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

---

### 5. Risk Management Dashboard

```
┌─────────────────────────────────────────────────────────────────┐
│                    RISK METRICS & CONTROLS                     │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ACCOUNT HEALTH                                                │
│  ───────────────                                               │
│  Health Factor:      4.2 [████████████░░░░░░]  SAFE ✓         │
│  Threshold Warning:  3.0                                       │
│  Liquidation Risk:   2.0 (CRITICAL)                            │
│                                                                 │
│  Position concentration (total notional): 34%                  │
│  Max allowed: 100% (can open more positions)                   │
│                                                                 │
│  DAILY LOSS LIMIT                                              │
│  ──────────────────                                            │
│  Daily Loss Limit:   $50 (hard stop)                           │
│  Daily P&L Today:    +$12.50                                   │
│  Remaining Budget:   $62.50                                    │
│  Status:             ✓ Safe (no risk of hitting limit)        │
│                                                                 │
│  MAX DRAWDOWN TRACKING                                         │
│  ──────────────────────                                        │
│  Peak Equity (ever): $312.50 (on 2026-02-21)                 │
│  Current Drawdown:   -0.8% (from peak)                         │
│  Max Drawdown Limit: -5.0% (circuit breaker)                  │
│  Status:             ✓ Safe (room for 4% more decline)        │
│                                                                 │
│  POSITION LIMITS                                               │
│  ────────────────                                              │
│  Open Positions:     2 / 3 max                                 │
│  Leverage Usage:     10x (avg) / 15x (max)                     │
│  Position Sizes:     $105 (SOL), $57 (BTC) / $45 max each    │
│  Status:             ✓ All within limits                      │
│                                                                 │
│  LIQUIDATION ALERTS                                            │
│  ──────────────────                                            │
│  ⚠️  SOL position: Health factor declining                    │
│      Current: 3.2 | Warning: 3.0 | Liquidation: 2.0          │
│      Action: Reduce position if factor hits 2.5               │
│                                                                 │
│  ✓  BTC position: Healthy                                     │
│      Health factor: 8.5 (very safe)                           │
│                                                                 │
│  SLIPPAGE TRACKING                                             │
│  ──────────────────                                            │
│  Expected Slippage:  0.2% (on $2K orders)                     │
│  Actual Slippage:    0.18% (good execution)                   │
│  Anomalies (1%+):    0 detected                               │
│  Status:             ✓ Normal market conditions               │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

---

## Backend API Endpoints (for Dashboard)

```rust
// actix-web API server running alongside trading system

// GET /api/metrics
// Returns: Current account metrics, positions, P&L
GET /api/metrics -> DashboardMetrics

// GET /api/positions
// Returns: All open positions with unrealized P&L
GET /api/positions -> Vec<OpenPosition>

// GET /api/trades?limit=50&symbol=SOL
// Returns: Trade history with full details
GET /api/trades -> Vec<TradeLog>

// GET /api/signals?limit=20
// Returns: Last 20 signals generated
GET /api/signals -> Vec<SignalEvent>

// GET /api/whale-movements?limit=10
// Returns: Recent whale activities detected
GET /api/whale-movements -> Vec<WhaleMovement>

// GET /api/technical/:symbol?timeframe=1h
// Returns: Technical indicators for symbol
GET /api/technical/:symbol -> TechnicalIndicators

// WebSocket /ws/live
// Real-time updates: prices, indicators, trades, alerts
WebSocket /ws/live

// GET /api/performance/daily
// Returns: Daily P&L breakdown
GET /api/performance/daily -> DailyPerformance

// GET /api/performance/monthly
// Returns: Monthly P&L breakdown
GET /api/performance/monthly -> MonthlyPerformance

// GET /api/backtest/results
// Returns: Backtest statistics
GET /api/backtest/results -> BacktestResults
```

---

## Real-Time WebSocket Updates

```rust
pub enum WebsocketMessage {
    // Price updates
    PriceUpdate {
        symbol: String,
        price: f64,
        bid: f64,
        ask: f64,
        timestamp: i64,
    },

    // Indicator updates
    IndicatorUpdate {
        symbol: String,
        rsi: f64,
        macd: f64,
        atr: f64,
        // ... others
    },

    // Trade signals
    SignalDetected {
        symbol: String,
        direction: String,
        confidence: f64,
        strategy: String,
        signal_count: i32,
    },

    // Trade execution
    TradeExecuted {
        trade_id: String,
        symbol: String,
        direction: String,
        entry_price: f64,
        leverage: f64,
    },

    // Trade exit
    TradeExited {
        trade_id: String,
        exit_price: f64,
        pnl: f64,
        pnl_percent: f64,
    },

    // Whale movement
    WhaleMovement {
        whale_label: String,
        token: String,
        action: String,
        amount_usd: f64,
        confidence_adjustment: f64,
    },

    // Alerts
    Alert {
        level: String,  // INFO, WARN, ERROR
        message: String,
        timestamp: i64,
    },

    // Position update
    PositionUpdate {
        symbol: String,
        unrealized_pnl: f64,
        health_factor: f64,
    },
}

// Example WebSocket message sent every second
{
    "type": "PriceUpdate",
    "symbol": "SOL/USDT",
    "price": 142.80,
    "bid": 142.79,
    "ask": 142.81,
    "timestamp": 1708608000000
}
```

---

## Mobile-Friendly Alert System

```
SMS/Push Alerts (optional integration):
└─ Trade executed: "$140.25 SOL LONG, 0.14 SOL, +$5 target"
└─ Trade exited: "$142.80 exit, +1.81% profit ✓"
└─ Health warning: "Health factor 3.2, reduce size on next trade"
└─ Whale detected: "Whale depositing $8.2M SOL to Binance (bearish)"
└─ System error: "CEX connection failed, retrying..."
└─ Daily summary: "Today: +4.2%, 6 trades, 83% win rate"
```

---

## Dashboard Deployment (React Frontend)

**File: `frontend/src/components/Dashboard.tsx`**

```typescript
import React, { useEffect, useState } from 'react';
import { LineChart, BarChart, Tooltip } from 'recharts';
import { useWebSocket } from './hooks/useWebSocket';

export const Dashboard = () => {
    const [metrics, setMetrics] = useState<DashboardMetrics>(null);
    const [positions, setPositions] = useState<OpenPosition[]>([]);
    const [trades, setTrades] = useState<TradeLog[]>([]);
    const ws = useWebSocket('ws://localhost:8000/ws/live');

    useEffect(() => {
        // Subscribe to real-time updates
        ws.on('PriceUpdate', (msg) => {
            // Update chart data
        });

        ws.on('TradeExecuted', (msg) => {
            // Add trade to history, update positions
            setPositions(prev => [...prev, {
                symbol: msg.symbol,
                entry_price: msg.entry_price,
                // ...
            }]);
        });

        ws.on('Alert', (msg) => {
            // Show notification
            showAlert(msg.level, msg.message);
        });
    }, [ws]);

    return (
        <div className="dashboard">
            <MetricsPanel metrics={metrics} />
            <PositionsPanel positions={positions} />
            <TradeHistory trades={trades} />
            <TechnicalChart symbol="SOL" />
            <WhaleActivityFeed />
            <RiskMetrics />
        </div>
    );
};
```

---

## Summary: Dashboard Provides Full Transparency

✓ Know exactly what strategy triggered each trade
✓ See real-time P&L and win rate
✓ Monitor whale movements and on-chain intelligence
✓ Track technical indicators feeding decisions
✓ Alert on risk threshold violations
✓ Understand confidence scoring and signal confluence
✓ Historical trade analysis for learning

**Next Step:** This dashboard becomes your "eyes" into the system. Every decision is transparent and explainable.

