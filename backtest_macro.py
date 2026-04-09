#!/usr/bin/env python3
"""
Backtest: Macro Regime Detection + Pico-Top SHORT Scalp Logic
=============================================================
Mirrors the Rust implementation exactly:
  - MacroRegime::classify()  → price > MA20 AND MA5 > MA10 = BULL
  - MacroRegime::consensus() → BTC primary, ETH confirms alts
  - Pico-top gate            → large green candle (≥2%) + RSI>68 + price≥BB_upper
  - BEAR bounce LONG gate    → large red candle (≥2%) + RSI<32 + price≤BB_lower

Uses Binance public API (no key needed).
Fetches: BTC + ETH daily candles (last 365 days)
         BTC 1h candles (last 90 days) for pico-top simulation
"""

import json, sys, time, math, urllib.request, urllib.error
from datetime import datetime, timezone

# ── Binance public API ────────────────────────────────────────────────────────

def fetch_klines(symbol: str, interval: str, limit: int) -> list:
    url = (
        f"https://api.binance.com/api/v3/klines"
        f"?symbol={symbol}&interval={interval}&limit={limit}"
    )
    try:
        with urllib.request.urlopen(url, timeout=15) as r:
            raw = json.loads(r.read())
        # [open_time, open, high, low, close, volume, ...]
        return [
            {
                "ts":    raw_c[0] // 1000,
                "open":  float(raw_c[1]),
                "high":  float(raw_c[2]),
                "low":   float(raw_c[3]),
                "close": float(raw_c[4]),
                "vol":   float(raw_c[5]),
            }
            for raw_c in raw
        ]
    except Exception as e:
        print(f"  ⚠  fetch_klines({symbol}, {interval}) failed: {e}")
        return []

# ── Indicator helpers (mirrors indicators.rs) ────────────────────────────────

def sma(candles: list, period: int) -> float | None:
    if len(candles) < period or period == 0:
        return None
    closes = [c["close"] for c in candles[-period:]]
    return sum(closes) / period

def daily_mas(candles: list) -> tuple[float, float, float]:
    ma5  = sma(candles, 5)  or 0.0
    ma10 = sma(candles, 10) or 0.0
    ma20 = sma(candles, 20) or 0.0
    return ma5, ma10, ma20

def rsi(candles: list, period: int = 14) -> float:
    """Wilder's RSI — same as indicators.rs"""
    closes = [c["close"] for c in candles]
    if len(closes) < period + 1:
        return 50.0
    gains, losses = [], []
    for i in range(1, len(closes)):
        d = closes[i] - closes[i-1]
        gains.append(max(d, 0.0))
        losses.append(max(-d, 0.0))
    # initial averages (simple)
    avg_gain = sum(gains[:period]) / period
    avg_loss = sum(losses[:period]) / period
    for i in range(period, len(gains)):
        avg_gain = (avg_gain * (period - 1) + gains[i]) / period
        avg_loss = (avg_loss * (period - 1) + losses[i]) / period
    if avg_loss < 1e-10:
        return 100.0
    rs = avg_gain / avg_loss
    return 100.0 - 100.0 / (1.0 + rs)

def bollinger(candles: list, period: int = 20, k: float = 2.0):
    """Returns (mid, upper, lower)"""
    closes = [c["close"] for c in candles]
    if len(closes) < period:
        mid = closes[-1]
        return mid, mid * 1.02, mid * 0.98
    sl = closes[-period:]
    mid = sum(sl) / period
    std = math.sqrt(sum((x - mid)**2 for x in sl) / period)
    return mid, mid + k * std, mid - k * std

# ── MacroRegime (mirrors decision.rs) ────────────────────────────────────────

class MacroRegime:
    BULL = "BULL"
    BEAR = "BEAR"
    TRANSITION = "TRANSITION"

    @staticmethod
    def classify(price: float, ma5: float, ma10: float, ma20: float) -> str:
        if price == 0.0 or ma20 == 0.0 or ma10 == 0.0 or ma5 == 0.0:
            return MacroRegime.TRANSITION
        above_ma20     = price > ma20
        ma5_above_ma10 = ma5 > ma10
        if above_ma20 and ma5_above_ma10:
            return MacroRegime.BULL
        if not above_ma20 and not ma5_above_ma10:
            return MacroRegime.BEAR
        return MacroRegime.TRANSITION

    @staticmethod
    def consensus(btc: str, eth: str) -> str:
        if btc == MacroRegime.BULL and eth == MacroRegime.BULL:
            return MacroRegime.BULL
        if btc == MacroRegime.BEAR and eth == MacroRegime.BEAR:
            return MacroRegime.BEAR
        if btc == MacroRegime.BULL and eth == MacroRegime.TRANSITION:
            return MacroRegime.BULL
        if btc == MacroRegime.BEAR and eth == MacroRegime.TRANSITION:
            return MacroRegime.BEAR
        return MacroRegime.TRANSITION

# ── Pico-top detection (mirrors execute_paper_trade entry gate) ───────────────

def is_pico_top(candles_window: list, rsi_val: float, bb_upper: float) -> bool:
    """
    Returns True when:
      - Any of the last 3 candles has ≥2% green body
      - RSI > 68
      - Current close ≥ BB_upper * 0.998
    """
    if len(candles_window) < 3:
        return False
    recent3 = candles_window[-3:]
    large_green = any(
        c["close"] > c["open"] and
        (c["close"] - c["open"]) / max(c["open"], 1e-8) >= 0.02
        for c in recent3
    )
    current_price = candles_window[-1]["close"]
    rsi_overbought = rsi_val > 68.0
    at_bb_upper    = current_price >= bb_upper * 0.998
    return large_green and rsi_overbought and at_bb_upper

def is_bear_bounce(candles_window: list, rsi_val: float, bb_lower: float) -> bool:
    """Mirror of pico_top for BEAR regime LONG entries."""
    if len(candles_window) < 3:
        return False
    recent3 = candles_window[-3:]
    large_red = any(
        c["open"] > c["close"] and
        (c["open"] - c["close"]) / max(c["open"], 1e-8) >= 0.02
        for c in recent3
    )
    current_price = candles_window[-1]["close"]
    rsi_oversold = rsi_val < 32.0
    at_bb_lower  = current_price <= bb_lower * 1.002
    return large_red and rsi_oversold and at_bb_lower

# ── Section 1: Daily macro regime timeline ────────────────────────────────────

def run_macro_regime_backtest():
    print("\n" + "═"*70)
    print("  SECTION 1: DAILY MACRO REGIME TIMELINE (BTC + ETH, last 365 days)")
    print("═"*70)

    print("  Fetching BTC daily candles…")
    btc_d = fetch_klines("BTCUSDT", "1d", 365)
    print("  Fetching ETH daily candles…")
    eth_d = fetch_klines("ETHUSDT", "1d", 365)

    if not btc_d or not eth_d:
        print("  ✗ Could not fetch data — skipping section 1")
        return []

    regimes = []
    regime_counts = {MacroRegime.BULL: 0, MacroRegime.BEAR: 0, MacroRegime.TRANSITION: 0}
    transitions = []  # (date, old, new)
    last_regime = None

    # Align to the shorter of the two series
    n = min(len(btc_d), len(eth_d))
    btc_d, eth_d = btc_d[-n:], eth_d[-n:]

    print(f"\n  {'Date':<12} {'BTC Close':>10} {'BTC Regime':<12} {'ETH Regime':<12} {'Consensus':<12}")
    print("  " + "-"*60)

    for i in range(20, n):   # need at least 20 candles for MA20
        btc_slice = btc_d[:i+1]
        eth_slice = eth_d[:i+1]

        btc_ma5, btc_ma10, btc_ma20 = daily_mas(btc_slice)
        eth_ma5, eth_ma10, eth_ma20 = daily_mas(eth_slice)

        btc_price = btc_slice[-1]["close"]
        eth_price = eth_slice[-1]["close"]

        btc_regime = MacroRegime.classify(btc_price, btc_ma5, btc_ma10, btc_ma20)
        eth_regime = MacroRegime.classify(eth_price, eth_ma5, eth_ma10, eth_ma20)
        consensus  = MacroRegime.consensus(btc_regime, eth_regime)

        regime_counts[consensus] += 1
        ts = datetime.fromtimestamp(btc_slice[-1]["ts"], tz=timezone.utc)
        date_str = ts.strftime("%Y-%m-%d")

        if consensus != last_regime:
            if last_regime is not None:
                transitions.append((date_str, last_regime, consensus))
            last_regime = consensus

        regimes.append({
            "date": date_str,
            "btc_price": btc_price,
            "btc_ma20": btc_ma20,
            "btc_regime": btc_regime,
            "eth_regime": eth_regime,
            "consensus": consensus,
        })

        # Print only last 30 rows (to keep output manageable)
        if i >= n - 30:
            bull_str = "🐂" if consensus == MacroRegime.BULL else ("🐻" if consensus == MacroRegime.BEAR else "◎")
            print(f"  {date_str:<12} {btc_price:>10,.0f}  {btc_regime:<12} {eth_regime:<12} {bull_str} {consensus}")

    total = sum(regime_counts.values())
    print(f"\n  ── Regime distribution (last {total} trading days) ──")
    for r, cnt in regime_counts.items():
        bar = "█" * int(cnt / total * 30)
        print(f"    {r:<12}  {cnt:>4}d  ({cnt/total*100:4.1f}%)  {bar}")

    print(f"\n  ── Regime transitions ──")
    for date, old, new in transitions[-10:]:  # last 10 flips
        arrow = "🐂" if new == MacroRegime.BULL else ("🐻" if new == MacroRegime.BEAR else "◎")
        print(f"    {date}  {old} → {arrow} {new}")

    return regimes

# ── Section 2: Pico-top SHORT backtest on BTC 1h ─────────────────────────────

def run_pico_top_backtest(daily_regimes: list):
    print("\n" + "═"*70)
    print("  SECTION 2: PICO-TOP SHORT SCALP BACKTEST (BTC 1h, last 90 days)")
    print("═"*70)
    print("  Conditions: large green (≥2%) + RSI>68 + price≥BB_upper(20,2)")
    print("  Entry: close of signal candle   Exit: 12h or +3% / -2% stop")

    print("  Fetching BTC 1h candles…")
    btc_1h = fetch_klines("BTCUSDT", "1h", 90 * 24)

    if not btc_1h:
        print("  ✗ Could not fetch 1h data — skipping section 2")
        return

    # Build a date→regime lookup from daily backtest
    regime_map = {}
    for r in daily_regimes:
        regime_map[r["date"]] = r["consensus"]

    def get_regime_at(ts_unix: int) -> str:
        date_str = datetime.fromtimestamp(ts_unix, tz=timezone.utc).strftime("%Y-%m-%d")
        return regime_map.get(date_str, MacroRegime.TRANSITION)

    # Scan 1h candles
    LOOKBACK = 50   # candles needed for indicators
    trades = []
    blocked_count = 0
    skipped_no_setup = 0

    for i in range(LOOKBACK, len(btc_1h) - 12):
        window = btc_1h[i - LOOKBACK: i + 1]
        candle = btc_1h[i]
        regime = get_regime_at(candle["ts"])

        # Only evaluate SHORT signals (SELL in bot terms) in BULL macro
        if regime != MacroRegime.BULL:
            continue

        current_rsi = rsi(window)
        _, bb_upper, bb_lower = bollinger(window)
        current_price = candle["close"]

        pico = is_pico_top(window, current_rsi, bb_upper)
        if not pico:
            # Count how many are blocked due to pico conditions not met
            # (only when RSI is moderately elevated to avoid counting all hours)
            if current_rsi > 60:
                skipped_no_setup += 1
            continue

        # --- Valid pico-top setup, simulate SHORT ---
        entry_price = current_price
        target      = entry_price * (1.0 - 0.025)   # +2.5% profit (shorting)
        stop        = entry_price * (1.0 + 0.020)    # -2.0% stop loss
        max_hold    = 12  # hours

        # Forward test: scan next 12 candles
        exit_price  = None
        exit_reason = None
        exit_hours  = 0
        pnl_pct     = 0.0

        for j in range(1, max_hold + 1):
            if i + j >= len(btc_1h):
                break
            fc = btc_1h[i + j]
            # Hit stop (price went UP — bad for short)
            if fc["high"] >= stop:
                exit_price  = stop
                exit_reason = "STOP"
                exit_hours  = j
                pnl_pct     = (entry_price - stop) / entry_price * 100.0  # negative
                break
            # Hit target (price went DOWN — good for short)
            if fc["low"] <= target:
                exit_price  = target
                exit_reason = "TARGET"
                exit_hours  = j
                pnl_pct     = (entry_price - target) / entry_price * 100.0  # positive
                break

        if exit_price is None:
            # Time exit
            exit_price  = btc_1h[min(i + max_hold, len(btc_1h)-1)]["close"]
            exit_reason = "TIME"
            exit_hours  = max_hold
            pnl_pct     = (entry_price - exit_price) / entry_price * 100.0

        date_str = datetime.fromtimestamp(candle["ts"], tz=timezone.utc).strftime("%Y-%m-%d %H:%M")
        trades.append({
            "date":   date_str,
            "entry":  entry_price,
            "exit":   exit_price,
            "reason": exit_reason,
            "hours":  exit_hours,
            "pnl":    pnl_pct,
            "rsi":    current_rsi,
        })

    # ── Results ──────────────────────────────────────────────────────────────
    if not trades:
        print("\n  No pico-top setups found in this period.")
        return

    wins   = [t for t in trades if t["pnl"] > 0]
    losses = [t for t in trades if t["pnl"] <= 0]
    total_pnl = sum(t["pnl"] for t in trades)
    win_rate  = len(wins) / len(trades) * 100.0
    avg_win   = sum(t["pnl"] for t in wins)   / len(wins)   if wins   else 0.0
    avg_loss  = sum(t["pnl"] for t in losses) / len(losses) if losses else 0.0
    profit_factor = (
        sum(t["pnl"] for t in wins) / abs(sum(t["pnl"] for t in losses))
        if losses and sum(t["pnl"] for t in losses) != 0 else float("inf")
    )

    print(f"\n  {'Date (UTC)':<20} {'Entry':>8} {'Exit':>8} {'Exit':<8} {'PnL%':>6}")
    print("  " + "-"*58)
    for t in trades[-20:]:  # show last 20
        icon = "✅" if t["pnl"] > 0 else "❌"
        print(
            f"  {t['date']:<20} {t['entry']:>8,.0f} {t['exit']:>8,.0f}"
            f"  {t['reason']:<8} {icon} {t['pnl']:>+5.2f}%"
        )

    print(f"\n  ── Performance Summary ──────────────────────────────")
    print(f"    Total trades:     {len(trades)}")
    print(f"    Wins / Losses:    {len(wins)} / {len(losses)}")
    print(f"    Win rate:         {win_rate:.1f}%")
    print(f"    Avg win:         {avg_win:+.2f}%")
    print(f"    Avg loss:        {avg_loss:+.2f}%")
    print(f"    Profit factor:    {profit_factor:.2f}x")
    print(f"    Total PnL:       {total_pnl:+.2f}%")
    print(f"    Blocked (no pico, RSI>60): {skipped_no_setup}")

    # Exits by reason
    reasons = {}
    for t in trades:
        reasons[t["reason"]] = reasons.get(t["reason"], 0) + 1
    print(f"\n  ── Exit reasons ─────")
    for r, cnt in sorted(reasons.items()):
        print(f"    {r:<8} {cnt:>4}  ({cnt/len(trades)*100:.0f}%)")

# ── Section 3: BEAR→BULL transition timing ────────────────────────────────────

def run_transition_timing(daily_regimes: list):
    print("\n" + "═"*70)
    print("  SECTION 3: BEAR→BULL TRANSITION TIMING vs ACTUAL BTC BOTTOM")
    print("═"*70)
    print("  How quickly does the macro detector flip to BULL after real bottom?\n")

    # Find local price minima (20d lookback/forward) in data
    prices = [(r["date"], r["btc_price"]) for r in daily_regimes]
    regimes = [(r["date"], r["consensus"]) for r in daily_regimes]

    # Find each BEAR→BULL transition
    transitions = []
    for i in range(1, len(regimes)):
        if regimes[i-1][1] != MacroRegime.BULL and regimes[i][1] == MacroRegime.BULL:
            transitions.append(regimes[i][0])

    if not transitions:
        print("  No BEAR→BULL transitions found in dataset.")
        return

    for flip_date in transitions:
        # Find the price at transition
        flip_idx = next((i for i,(d,_) in enumerate(prices) if d == flip_date), None)
        if flip_idx is None:
            continue
        flip_price = prices[flip_idx][1]

        # Find local minimum in ±15 days window
        window_start = max(0, flip_idx - 15)
        window_end   = min(len(prices)-1, flip_idx + 5)
        window_prices = prices[window_start:window_end+1]
        local_min_date, local_min_price = min(window_prices, key=lambda x: x[1])

        lag_days = (
            datetime.strptime(flip_date, "%Y-%m-%d") -
            datetime.strptime(local_min_date, "%Y-%m-%d")
        ).days

        entry_vs_bottom = (flip_price - local_min_price) / local_min_price * 100.0

        print(f"  Flip date:     {flip_date}  @ ${flip_price:,.0f}")
        print(f"  Local bottom:  {local_min_date}  @ ${local_min_price:,.0f}")
        print(f"  Lag:           {lag_days} days after bottom")
        if lag_days >= 0:
            print(f"  Entry vs bottom: +{entry_vs_bottom:.1f}% (entered after bottom — expected for MA-based)")
        else:
            print(f"  Entry vs bottom: {entry_vs_bottom:.1f}% (entered BEFORE bottom — lucky)")
        print()

# ── Main ──────────────────────────────────────────────────────────────────────

if __name__ == "__main__":
    print()
    print("╔══════════════════════════════════════════════════════════════════════╗")
    print("║    TRADINGBOTS.FUN — MACRO REGIME + PICO-TOP BACKTEST               ║")
    print("║    Mirrors src/decision.rs + src/indicators.rs + main.rs logic      ║")
    print("╚══════════════════════════════════════════════════════════════════════╝")

    daily_regimes = run_macro_regime_backtest()
    if daily_regimes:
        run_pico_top_backtest(daily_regimes)
        run_transition_timing(daily_regimes)

    print("\n" + "═"*70)
    print("  BACKTEST COMPLETE")
    print("═"*70 + "\n")
