# TradingBots.fun — Feature Backlog

Items here were identified during the March 2026 codebase audit as genuinely
valuable work that was deprioritised in favour of stabilising the live pipeline.
All three have deleted reference implementations recoverable via:
`git show f1752e2:src/<filename>`

---

## 1. Backtester
**Recover from:** `git show f1752e2:src/backtest.rs`
**Why it matters:**
Every parameter change made since launch (AI-close guardrails, DCA window,
signal reversal hold, confidence floor) has been validated only by forward
paper trading. A backtester would let you replay those changes against
historical data before deploying, catching regressions in minutes instead
of days.

**What needs doing to make it functional:**
- Wire `data.rs` candle fetching to feed historical OHLCV into the engine
- Connect `decision.rs` make_decision() as the strategy under test
- Persist results to `logs/backtest_YYYY-MM-DD.jsonl` in the same format
  as live trade logs so `daily_analyst.rs` can analyse them
- Add a CLI flag (`--backtest 7d`) to run from main.rs without starting the live loop

**Reference commit:** `f1752e2` (deletion commit — file fully intact in history)

---

## 2. Strategy Attribution
**Recover from:** `git show f1752e2:src/strategy_attribution.rs`
**Why it matters:**
The live bot has 13 signals (RSI, MACD, EMA cross, order flow, sentiment,
funding rate, etc.) but zero visibility into which ones are actually
generating P&L. `learner.rs` adjusts weights from outcomes but never
surfaces "funding rate signal contributed +$142 this week, chart patterns
cost -$67." Without attribution you are flying blind on which signals
deserve higher weights.

**What needs doing:**
- Hook `AttributedTrade` recording into `close_paper_position()` — capture
  which `SignalContribution` fields were true at entry
- Persist attribution rows to DB (new table) or to `.jsonl` sidecar
- Add a `/api/attribution` endpoint in `web_dashboard.rs` so the dashboard
  can show per-signal win rate and P&L contribution
- Feed weekly attribution summary into `daily_analyst.rs` prompt so the
  AI analysis knows which signals are working

**Reference commit:** `f1752e2`

---

## 3. Daily / Weekly Drawdown Limits (DrawdownTracker)
**Recover from:** `git show f1752e2:src/frameworks.rs` (DrawdownTracker struct)
**Why it matters:**
The live circuit breaker only fires at 8% rolling 7-day peak-to-trough.
A bad single day (e.g. -5% in 4 hours) keeps the bot trading until the
rolling window catches up. `DrawdownTracker` had daily (-5%), weekly (-10%),
and monthly (-15%) hard stops that would pause trading much faster.

**What needs doing:**
- Add `daily_pnl`, `weekly_pnl`, `monthly_pnl` tracking to `BotState`
  (reset at UTC midnight / Monday / month start)
- Feed realised P&L from `close_paper_position()` into these accumulators
- Add a `drawdown_halt: bool` flag to `BotState`; set it when any limit hit
- In `execute_paper_trade()` check `drawdown_halt` before opening new positions
- Surface halt status on dashboard with reason ("Daily -5% limit reached")
- Auto-clear at next reset period (midnight resets daily, etc.)

**Reference commit:** `f1752e2`

---

*Last updated: March 2026*
