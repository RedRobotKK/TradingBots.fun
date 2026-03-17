# TradingBots.fun — Feature Backlog
*Last updated: March 2026 — full codebase audit*

---

## LEGEND
- 🔴 **Critical** — live bot risk or broken user flow
- 🟠 **High** — significant edge or revenue impact
- 🟡 **Medium** — quality of life, retention, optimization
- 🟢 **Nice to have** — polish, future scale

---

## SECTION 1: WORK IN PROGRESS (built but not finished)

### 🔴 1.1 — Account health checks never run
**File:** `risk.rs`, `exchange.rs`
`risk.rs` has a `should_trade()` function that checks account health factor,
daily loss limits, and margin. `exchange.rs::get_account()` returns hardcoded
`$1000 equity / health=999 / daily_pnl=0` instead of querying Hyperliquid.
**Result:** The bot never knows if it's undercollateralised. Liquidation risk
is completely invisible.
**Fix:** Implement `get_account()` to call HL `/info clearinghouseState`.
Call `should_trade()` from `execute_paper_trade()` before opening positions.

### 🔴 1.2 — Stop-loss never auto-enforced
**File:** `exchange.rs:273-285`
`get_positions()` and `close_position()` are both stubs (`#[allow(dead_code)]`).
Stop-loss prices are calculated and stored on positions but never triggered.
If price breaches the stop, the bot holds the position indefinitely.
**Fix:** Implement both stubs against the HL REST API. Add a per-cycle
sweep in the main loop comparing mark price to stop_loss for each open position.

### 🔴 1.3 — Stripe checkout URL hardcoded
**File:** `stripe.rs:204`
`TODO: replace with req Host header` — redirect URLs point to a hardcoded
`"https://yourdomain.com"`. Every new deployment needs a manual code edit.
**Fix:** Extract the `Host` header from the incoming request and use it
dynamically in the checkout session creation.

### 🟠 1.4 — Learner weights never persisted across restarts
**File:** `db.rs` — `upsert_signal_weights()` and `load_signal_weights()`
exist and are correct, but learner.rs never calls them. Every restart resets
weights to defaults. Weeks of learning are wiped on every deploy.
**Fix:** On startup, call `load_signal_weights()` to restore from DB.
After every 10-trade checkpoint, call `upsert_signal_weights()`.

### 🟠 1.5 — Trial expiry email never sends
**File:** `mailer.rs`, `funnel.rs`
`trial_expiry_html()` template is complete. Mailer is configured. But the
nightly job to fetch expired-trial tenants and send the email doesn't exist.
`mark_promo_sent()` funnel event and `PROMO_SENT` instrumentation are there
but never called.
**Fix:** Add a nightly task: query for tenants where trial ended > 0 days ago
and `promo_sent_at IS NULL`. Send email via Mailer. Stamp `promo_sent_at`.
Expected impact: +15-20 paid conversions/month.

### 🟠 1.6 — Daily AI analyst never executes
**File:** `daily_analyst.rs`
`analyse_day()` is called from main.rs at midnight UTC but only builds a
Claude prompt — it never sends it. `query_ai()` in `db.rs` is complete and
supports Claude/OpenAI/xAI/Ollama but is marked `#[allow(dead_code)]`.
**Fix:** Wire `analyse_day()` to call `query_ai()` with the built prompt.
Parse the JSON response for parameter suggestions. Write to `ai_analyses`
table. Expose in admin panel with one-click apply.

### 🟠 1.7 — DB maintenance job never scheduled
**File:** `db.rs` — `run_maintenance()` function exists but is never called.
Without pruning, equity_snapshot and trade tables slow down after 6 months.
**Fix:** Schedule nightly after daily analysis. Prune snapshots >90 days.
ANALYZE and REINDEX. Log bytes freed.

### 🟡 1.8 — Leaderboard prizes never awarded
**File:** `leaderboard.rs` — `award_campaign_prizes()` exists, never called.
Campaigns run and rankings are tracked but winners receive nothing.
**Fix:** Schedule end-of-campaign job. Call `award_campaign_prizes()`.
Write to `campaign_prizes` table. Send winner notification emails via Mailer.

### 🟡 1.9 — Invite/referral UI invisible to users
**File:** `invite.rs` — `generate_referral_code()` and `claim_invite_code()` work
but there is no UI surface. Users have no way to see or share their code.
**Fix:** Add `/app/referrals` route in `web_dashboard.rs`. Show referral code
with copy-to-clipboard. Show list of referrals (name, status, earned credit).

### 🟡 1.10 — Pyramid trades don't emit trade log events
**File:** `trade_log.rs` — `TradePyramid` event type is defined but never
emitted. `pyramid_position()` in `main.rs` completes silently.
**Fix:** Emit `TradePyramid` event from `pyramid_position()` with size, price,
new avg entry, and new R-multiple. Needed for attribution (Backlog item 3.2).

### 🟡 1.11 — Funnel analytics instrumentation gaps
**File:** `funnel.rs` — `page_view()`, `auth_success()`, `ad_impression()`
defined but never called from web_dashboard route handlers.
**Fix:** Add `funnel::page_view()` calls in dashboard middleware.
Add `funnel::auth_success()` after successful Privy JWT verification.
Required before cohort email segmentation is possible (Backlog 2.3).

### 🟡 1.12 — Coins list requires redeploy to update
**File:** `coins.rs` — coin dictionary is hardcoded. Adding a new coin
(e.g. a new L2 or meme coin) requires a code change and full redeploy.
**Fix:** Move coin list to DB table `supported_coins`. Admin panel gets
an "Add Coin" form. Bot reads from DB on startup.

---

## SECTION 2: NEW FEATURES (not yet built)

### 🔴 2.1 — Real-time position mark vs stop monitoring
The main loop currently has no sweep comparing live mark prices to stored
stop-loss levels. A position 10% against entry just keeps running.
**Build:** Every cycle, for each open position, compare `current_price`
to `stop_loss`. If breached, call `close_paper_position()` with reason
`"StopHit"`. Log it. This is separate from (and complementary to) the
exchange.rs live SL enforcement in item 1.2.

### 🟠 2.2 — Dynamic drawdown halt (daily / weekly / monthly)
The live circuit breaker is a 7-day rolling 8% trigger — too slow for
a bad single day. Add three faster halt levels:
- Daily P&L < -5% → pause entries for 6 hours, allow closes only
- Weekly P&L < -10% → pause 24 hours, require operator confirmation to resume
- Monthly P&L < -15% → full pause until next month
Track these in `BotState`. Reset daily at UTC midnight, weekly on Monday, monthly on 1st.
Reference implementation: `git show f1752e2:src/frameworks.rs` (DrawdownTracker)

### 🟠 2.3 — Cohort email segmentation
Funnel events table captures lifecycle stages but no cohort queries exist.
Define cohorts: trial_expired_no_convert, referred_user, power_user (>10
positions), organic. Send tailored emails per cohort. Cohort emails have
3-4× higher CTR than blasts.

### 🟠 2.4 — Strategy attribution — which signal makes money?
You cannot currently answer "which of the 13 signals made money this week?"
You only see aggregate Sharpe and win rate.
**Build:** When a trade closes, walk `SignalContribution` and record which
signals were active at entry. Group by signal over 50-trade windows.
Surface per-signal win rate and P&L in admin panel.
Reference implementation: `git show f1752e2:src/strategy_attribution.rs`

### 🟡 2.5 — Backtester
No way to validate parameter changes against history. Every tweak is tested
live in paper trading (slow). A backtester replays historical OHLCV through
`decision.rs::make_decision()` and returns Sharpe, win rate, max drawdown.
Reference implementation: `git show f1752e2:src/backtest.rs`
**To make functional:** wire `data.rs` candle fetch as the data source,
add `--backtest 7d` CLI flag.

### 🟡 2.6 — Sentiment gauge + funding rate heatmap on dashboard
`sentiment.rs` and `funding.rs` fetch data every cycle but it's invisible
to users. Add dashboard cards:
- Sentiment Gauge: galaxy_score, bull/bear bar, `quality()` label
- Funding Rate Heatmap: all tracked coins, rate + 4h change, highlight extremes
This builds user trust by showing *why* the bot is trading.

### 🟡 2.7 — Telegram / Discord trade notifications
Real-time push notification when a position opens, closes, or the circuit
breaker fires. Users want to know what the bot is doing without refreshing
the dashboard. One webhook URL per tenant in tenant settings.

### 🟡 2.8 — Correlation filter — don't stack correlated longs
If BTC and ETH are both in the LONG signal queue, entering both doubles
concentration risk since their correlation is ~0.85. Add a portfolio-level
check: if two new signals are >0.70 correlated, only enter the higher-
confidence one.

### 🟡 2.9 — On-chain data feed (whale wallet movements)
Supplement LunarCrush sentiment with Glassnode / Nansen-style on-chain
signals: exchange inflows (selling pressure), large wallet accumulation
(buy signal), stablecoin supply ratio (market dry powder). One additional
signal at the right weight can add 5-10% to win rate.

### 🟢 2.10 — Admin audit log
Multi-operator or multi-tenant admin changes (update signal weight, toggle
live mode, reset learner) leave no trace today. Add `audit_log` table:
(admin_id, action, old_val, new_val, timestamp). Display in admin panel.

### 🟢 2.11 — API rate limiting per tenant
No rate limiting on `/api/*` endpoints. A single tenant can hammer the
chart endpoint and degrade service for others. Add token-bucket rate
limiter: 100 req/min per tenant. Return 429 with upgrade prompt.

### 🟢 2.12 — Trade journal — operator notes on closed positions
After a position closes, allow the operator to add a note: "entered too
early", "MACD false signal in ranging market", etc. Store in
`closed_trades.operator_notes`. Weekly rollup of learnings in admin panel.
Behavioral feedback loop has historically improved win rate 8-12%.

---

## SECTION 3: PREVIOUSLY IDENTIFIED DEFERRED FEATURES

### 3.1 — Backtester
*(see 2.5 above — merged)*

### 3.2 — Strategy Attribution
*(see 2.4 above — merged)*

### 3.3 — Daily / Weekly Drawdown Limits
*(see 2.2 above — merged)*
Reference: `git show f1752e2:src/frameworks.rs`

---

## QUICK WIN RANKING
If you had one sprint to maximise impact:

| # | Item | Effort | Impact |
|---|------|--------|--------|
| 1 | Wire `should_trade()` + real `get_account()` | 1 day | 🔴 Eliminates liquidation risk |
| 2 | Fix Stripe hardcoded URL | 1 hour | 🔴 Unbreaks payments on any new deployment |
| 3 | Trial expiry email flow | 1 day | 🟠 +15-20 paid conversions/month |
| 4 | Persist learner weights to DB | 2 hours | 🟠 Learning survives restarts |
| 5 | Daily AI analyst → query_ai() | 2 hours | 🟠 Operator feedback loop activated |
| 6 | Daily/weekly drawdown halt | 1 day | 🟠 Stops bleeding during bad sessions |
| 7 | Real-time stop monitoring per position | 1 day | 🔴 Closes positions that breach SL |
| 8 | Strategy attribution (which signal works) | 3 days | 🟠 Know what's actually making money |

*Last updated: March 2026*
