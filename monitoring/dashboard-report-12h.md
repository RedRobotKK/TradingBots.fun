# TradingBots.fun — Monitoring Analysis Report (Updated)

**Generated:** 2026-04-09T07:46 UTC
**Data window:** 2026-04-02T03:01 UTC → 2026-04-09T07:46 UTC (~7 days of log history)
**Valid snapshots:** 40 of 131 total rows (91 ERROR rows due to Chrome disconnection / network egress blocking — see Infrastructure Issues below)
**Latest snapshot:** 2026-04-09T07:46 UTC — BULL regime, cycle 300, equity $996,847, all 12 positions LONG, win rate 83.3%, profit factor 18.505

---

## Summary

| Metric | Start | End | Change |
|--------|-------|-----|--------|
| Realized PnL | $17,377.77 | $121,382.36 | **+$104,004.59** |
| Equity | $924,265.96 | $934,439.23 | **+$10,173.27** |
| Total Trades | 19 | 131 | **+112 new trades** |
| Win Rate | 84.2% | 50.0% | **−34.2 pp** |
| Profit Factor | 12.041 | 2.780 | **−9.261** |
| Max Drawdown | 3.16% | 73.93% | **+70.77 pp** |

The bot generated $104k in realized PnL over the session and grew equity by ~$10k. However this masks a severe and ongoing degradation in execution quality. Win rate collapsed by more than a third, profit factor dropped from exceptional to mediocre, and a 73.93% max drawdown flag appeared midway through the session and never fully cleared — a critical risk signal that demands immediate investigation.

---

## Performance Trend

The session divides cleanly into three phases:

**First third (snapshots 1–13, 03:01–09:31 UTC):** The bot started with outstanding metrics — win rate 84.2%, profit factor 12.041, Sharpe 1.20. This reflects a portfolio well-positioned at the start of the BEAR regime. Over 41 trades, win rate fell to 73.2% and profit factor to 4.91 — still excellent but clearly declining. The early long exposure (BABY:L, HEMI:L) was present then exited, leaving a pure short book.

**Middle third (snapshots 14–26, 10:43–16:39 UTC):** This is where degradation became acute. Win rate fell from 69.8% to 50.5%, profit factor from 3.60 to 2.78, and Sharpe from 0.52 to 0.31. The max drawdown jumped from 3.16% to 14.14% around 10:43 UTC, then to 73.93% at 16:39 UTC — a massive step-change. During this phase the bot executed roughly 70 trades, with results approaching coin-flip quality. The transition from skilled SHORT momentum to near-random performance is a textbook sign of regime maturation or strategy overfitting to early favorable conditions.

**Final third (snapshots 27–39, 17:22–22:12 UTC):** Performance stabilized but at the lower level. Win rate flatlined at 50–51%, profit factor held 2.78–2.98, Sharpe stayed near 0.31. Realized PnL continued growing (the bot remains net profitable at 2x+ profit factor), but there is no recovery trend. The bot is in a "grinding" state — taking many marginally profitable trades without the decisive edge seen at session open.

**Verdict:** The bot degrades over the session. Initial strong performance is tied to catching early BEAR regime momentum. As that momentum matured, the bot continued entering at similar frequency but with dramatically lower win probability.

---

## Regime Analysis

| Regime | Snapshots | Duration |
|--------|-----------|----------|
| TRANSITION | 2 | ~20 min |
| BEAR | 37 | ~19 hours |
| BULL | 0 | — |
| Total Regime Changes | 1 | TRANSITION → BEAR |

The session was almost entirely a single BEAR regime. TRANSITION lasted only the first two snapshots (03:01–03:21 UTC), then locked into BEAR at 03:42 UTC and never changed. No BULL regime occurred across the entire valid data window.

The TRANSITION snapshots showed the highest metrics (win rate 84.2%, profit factor 12.0) but this reflects inherited positions from a prior session, not live TRANSITION signals. All active trading occurred in BEAR regime.

The consistency of BEAR classification across 19 hours — with no flip even during equity rallies — suggests the macro regime detector either has high inertia or a very high threshold for BULL confirmation. If the detector is stuck in BEAR, the bot will miss long opportunities and may over-commit to shorts near turning points.

---

## Position Patterns

**Average open positions:** ~12.4 (range: 11–14, very stable)

**LONG:SHORT ratio across session phases:**

| Phase | Longs | Shorts |
|-------|-------|--------|
| Early (03:01–03:42) | 2 | 12 |
| Mid (06:02–16:39) | 0 | 12 |
| Late (17:22–22:12) | 1 (BTC) | 12 |

The bot ran a fully-short book for the majority of the session — 17 consecutive snapshots with zero long exposure. This is directionally consistent with BEAR regime but creates concentrated downside risk if regime flips without warning.

**Most frequent symbols (by appearance count):**

| Symbol | Side | Approx. Appearances |
|--------|------|---------------------|
| BLUR | SHORT | 30+ |
| BLAST | SHORT | 30+ |
| APE | SHORT | 20+ |
| ETH | SHORT | ~15 |
| LDO | SHORT | ~14 |
| AIXBT | SHORT | ~13 |
| GRASS | SHORT | ~12 |
| WIF | SHORT | ~10 |
| GMT | SHORT | ~10 |
| BTC | LONG | ~13 |

BLUR and BLAST are persistent "anchor" shorts appearing in almost every snapshot from mid-session onward. BTC is the only recurring long, added in late session as a minimal hedge.

**Symbol format anomaly:** At snapshot 37 (21:13 UTC), symbol names switched from ticker format ("BTC:L") to full display names ("Bitcoin:L", "SushiSwap:S", "Sonic:S"). Any downstream tooling that parses symbol names will break on this inconsistency.

---

## Drawdown Profile

| Timestamp | max_dd | cur_dd | Notes |
|-----------|--------|--------|-------|
| 03:01 | 3.16% | 0.00% | Session start |
| 03:42 | 3.16% | 0.54% | First unrealized drawdown |
| 07:57 | 6.00% | 6.00% | max_dd reset — possible capital event |
| 10:43 | **14.14%** | 11.78% | First major spike |
| 13:13 | 0.00% | 0.50% | Recovery — large wins closed |
| 16:39 | **73.93%** | 2.56% | **Critical: 73.93% max_dd appears** |
| 20:16 | 1.20% | 0.90% | Brief apparent reset |
| 20:33 | **73.93%** | 17.33% | Returns immediately |
| 22:12 | 73.93% | 3.60% | Still present at session end |

The 73.93% max drawdown flag is the most alarming signal in the dataset. It appeared between snapshot 24 (14:40 UTC, max_dd=3.10%) and snapshot 25 (16:39 UTC, max_dd=73.93%) — a jump of over 70 percentage points occurring during a monitoring gap (there is an ERROR snapshot at 15:11 UTC between these two). This is not a gradual decline; it's either a catastrophic trade that happened during the gap, or a tracking/accounting anomaly.

The brief reset to 1.20% at 20:16 UTC — coinciding with capital (~$959k) nearly equalling equity (~$959k), suggesting a near-full position reset — did not persist. The 73.93% flag returned one snapshot later.

If this drawdown is real (73.93% of peak equity lost at some point), the account was effectively destroyed at the nadir. The fact that equity remained near $937–$963k throughout this period is inconsistent with a 73.93% account-level drawdown from peak equity of ~$963k. This strongly suggests a calculation bug — perhaps max_dd is tracking a sub-strategy or individual symbol drawdown, not total portfolio drawdown.

---

## Best & Worst Intervals

**Top equity-gaining windows (consecutive valid snapshots):**

| Interval | Equity Change |
|----------|--------------|
| 12:44→13:13 | **+$18,479** |
| 18:02→18:44 | +$13,800 |
| 11:42→12:22 | +$9,551 |
| 19:53→20:16 | +$5,814 |
| 17:42→18:02 | +$5,393 |

**Top equity-losing windows:**

| Interval | Equity Change | Notes |
|----------|--------------|-------|
| 03:42→06:02 | **−$37,747** | 2.5-hr gap, likely not one 20-min window |
| 06:02→06:22 | −$9,558 | Continued decline |
| 21:13→21:34 | −$6,182 | Late session giveback |
| 20:33→20:54 | −$2,888 | Post-drawdown volatility |

The best 20-minute window (12:44→13:13) coincides with a massive $44k jump in realized PnL — large winning shorts were closed simultaneously. The worst clean interval (21:13→21:34) was −$6,182, a minor giveback by comparison.

---

## Key Observations

**1. Win rate collapse is the primary risk.** Going from 84.2% to 50.0% across 112 trades is not variance — it's structural decay. The bot's entry signals stop being predictive once the initial trending momentum exhausts. This is consistent with a momentum strategy that excels at regime onset but degrades as regime matures and price action becomes choppier.

**2. Capital accounting is volatile and unreliable.** The `capital` field swings from $26,951 to $958,635 across the session, with a dramatic jump to ~$953k at 19:53 UTC where capital nearly equalled equity (implying nearly zero open unrealized PnL). These swings are likely due to how the bot accounts for settled vs open positions, but the volatility makes capital-based risk calculations unreliable.

**3. Zero-long periods create concentrated directional risk.** Running 12 shorts and 0 longs for 11 hours — even in a confirmed BEAR regime — is aggressive. A sudden short squeeze or regime flip could cause outsized damage before the system reacts. The eventual addition of BTC:L as a hedge is a positive signal but arrives late.

**4. Monitoring infrastructure has been non-functional for 7 days.** Of 130 total rows in the CSV, 91 are ERROR — a 70% data loss rate. Valid data collection stopped at 2026-04-02T22:12 UTC and has not resumed. The root causes are Chrome extension disconnection and network egress blocking in the sandbox. This means the bot has been running without any monitoring, alerting, or data collection for a full week. Any anomalies, drawdowns, or failures since April 3 are undetected.

**5. Symbol naming format changed mid-session.** At 21:13 UTC, "BTC:L" became "Bitcoin:L", "SUSHI:S" became "SushiSwap:S", etc. This is a breaking change for any parser or analysis tool consuming symbol data from the API.

**6. The 73.93% max_dd flag is persistent and unresolved.** It appeared on April 2, survived multiple snapshot cycles, and was still present at the last valid reading. Whether it's a real event or a calculation bug, it has not self-corrected.

---

## Recommended Actions

### Immediate — Risk Management

**1. Investigate the 73.93% max drawdown.** Pull raw trade history for 2026-04-02 14:40–16:39 UTC. Determine if this represents a real peak-to-trough loss or a calculation bug. If real, halt live trading immediately — a 73.93% drawdown implies catastrophic failure of position sizing controls. If a bug, fix the drawdown tracking logic and add a sanity check: `max_dd` cannot exceed total account equity loss from peak.

**2. Fix the monitoring infrastructure immediately.** The Chrome extension approach is too fragile for production use. Replace it with a lightweight server-side script (Python cron job or systemd service) that directly calls `https://tradingbots.fun/api/state` with authentication and writes to the same CSV format. The bot has been running blind for 7 days.

### Short-term — Strategy Tuning

**3. Add a win-rate circuit breaker.** When rolling 20-trade win rate drops below 60%, reduce position size by 50%. When it drops below 55%, halt new entries or reduce to minimal sizing. The current system trades at full size even at 50% win rate — that's a pure coin flip with fees and slippage eating into returns.

**4. Implement regime-age decay on entry signals.** The bot performs excellently at regime onset (win rate 84%+) but degrades sharply as the regime matures. Add a regime-age factor: raise entry signal thresholds or reduce position size after 4–6 hours in the same regime. This would have preserved much of the early-session edge rather than chasing diminishing returns.

**5. Require a minimum long hedge at all times.** Even in BEAR regime, mandate 1–2 long positions (e.g., BTC or ETH) as tail-risk protection against sudden short squeezes. The bot eventually found this equilibrium with BTC:L late in session — build it in from regime entry.

### Medium-term — System Quality

**6. Normalize symbol naming in the API.** Fix the ticker vs. full-name inconsistency. Canonicalize all symbols to a consistent format (preferably tickers) before they leave the API layer. This affects all downstream analysis, alerting, and logging.

**7. Audit capital accounting logic.** Add a reconciliation check that flags when `|capital_t - capital_{t-1}|` exceeds 20%. The current wild capital swings suggest the accounting model conflates settled PnL, open unrealized PnL, and margin in ways that are difficult to interpret. Clear, stable capital tracking is essential for reliable position sizing.

**8. Review the macro regime detector sensitivity.** 19 hours without a BULL classification — even during equity rallies to new highs — suggests the detector may be over-fitted to BEAR conditions or has too high a transition threshold. A more responsive detector would help the bot capture counter-trend long opportunities and reduce SHORT concentration risk.

---

## Latest Snapshot — 2026-04-09T07:46 UTC (NEW)

**This snapshot represents a full bot reset since the prior session ended Apr 2.** Key metrics:

| Field | Value |
|---|---|
| Cycle | 300 |
| Regime | **BULL** |
| Capital (free) | $70,503.81 |
| Equity | $996,846.65 |
| PnL | $2,447.93 |
| Open Positions | 12 (all LONG) |
| Win Rate | 83.3% |
| Profit Factor | 18.505 |
| Sharpe | 1.410 |
| Expectancy | 11.297 |
| Max Drawdown | 7.74% |
| Current Drawdown | 0.00% |
| Total Trades | 12 (10W / 2L) |
| Symbols | SOL, AIXBT, SYRUP, ALGO, STRK, SKR, PUMP, MEW, KAS, kPEPE, 2Z, ALT |
| Last Signal | PROVE SELL |

The 73.93% max_dd from the previous session is gone — confirming the bot was fully reset. Current metrics (83.3% win rate, pf 18.5) match the TRANSITION-era peak from Session 1 and are consistent with strong early BULL positioning. The sample is still small (12 trades), but initial signals are positive.

---

## Infrastructure Status

**Monitoring:** RECOVERING — Chrome extension was disconnected from 2026-04-02T15:11 UTC to 2026-04-09T07:46 UTC (91 consecutive ERROR rows). The extension reconnected successfully for today's snapshot.

**Recommended fix:** Replace Chrome-based monitoring with:
```bash
# Example: direct cron-based monitoring (no browser required)
curl -s -H "Cookie: session=..." https://tradingbots.fun/api/state | python3 parse_and_append.py
```
This approach has no dependency on a running browser and will survive system restarts.

---

*Report generated automatically by the tradingbots-monitor scheduled task.*
*Based on 40 valid snapshots out of 131 total collected rows (2026-04-02T03:01 → 2026-04-09T07:46 UTC).*
