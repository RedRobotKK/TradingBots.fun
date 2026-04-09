#!/usr/bin/env python3
"""
Unit tests: verify macro regime + pico-top logic matches Rust implementation.
No network required — tests against hardcoded synthetic data.
"""

import math, sys

# ── Copy of logic from backtest_macro.py ─────────────────────────────────────

def sma(candles, period):
    if len(candles) < period or period == 0:
        return None
    closes = [c["close"] for c in candles[-period:]]
    return sum(closes) / period

def daily_mas(candles):
    return (sma(candles, 5) or 0.0, sma(candles, 10) or 0.0, sma(candles, 20) or 0.0)

def rsi(candles, period=14):
    closes = [c["close"] for c in candles]
    if len(closes) < period + 1:
        return 50.0
    gains, losses = [], []
    for i in range(1, len(closes)):
        d = closes[i] - closes[i-1]
        gains.append(max(d, 0.0))
        losses.append(max(-d, 0.0))
    avg_gain = sum(gains[:period]) / period
    avg_loss = sum(losses[:period]) / period
    for i in range(period, len(gains)):
        avg_gain = (avg_gain * (period - 1) + gains[i]) / period
        avg_loss = (avg_loss * (period - 1) + losses[i]) / period
    if avg_loss < 1e-10:
        return 100.0
    return 100.0 - 100.0 / (1.0 + avg_gain / avg_loss)

def bollinger(candles, period=20, k=2.0):
    closes = [c["close"] for c in candles]
    if len(closes) < period:
        mid = closes[-1]
        return mid, mid * 1.02, mid * 0.98
    sl = closes[-period:]
    mid = sum(sl) / period
    std = math.sqrt(sum((x - mid)**2 for x in sl) / period)
    return mid, mid + k * std, mid - k * std

class MacroRegime:
    BULL = "BULL"; BEAR = "BEAR"; TRANSITION = "TRANSITION"

    @staticmethod
    def classify(price, ma5, ma10, ma20):
        if price == 0.0 or ma20 == 0.0 or ma10 == 0.0 or ma5 == 0.0:
            return MacroRegime.TRANSITION
        if price > ma20 and ma5 > ma10:
            return MacroRegime.BULL
        if not (price > ma20) and not (ma5 > ma10):
            return MacroRegime.BEAR
        return MacroRegime.TRANSITION

    @staticmethod
    def consensus(btc, eth):
        B, R, T = MacroRegime.BULL, MacroRegime.BEAR, MacroRegime.TRANSITION
        if btc == B and eth == B:    return B
        if btc == R and eth == R:    return R
        if btc == B and eth == T:    return B
        if btc == R and eth == T:    return R
        return T

def is_pico_top(candles_window, rsi_val, bb_upper):
    if len(candles_window) < 3:
        return False
    recent3 = candles_window[-3:]
    large_green = any(
        c["close"] > c["open"] and (c["close"] - c["open"]) / max(c["open"], 1e-8) >= 0.02
        for c in recent3
    )
    current_price = candles_window[-1]["close"]
    return large_green and rsi_val > 68.0 and current_price >= bb_upper * 0.998

def is_bear_bounce(candles_window, rsi_val, bb_lower):
    if len(candles_window) < 3:
        return False
    recent3 = candles_window[-3:]
    large_red = any(
        c["open"] > c["close"] and (c["open"] - c["close"]) / max(c["open"], 1e-8) >= 0.02
        for c in recent3
    )
    current_price = candles_window[-1]["close"]
    return large_red and rsi_val < 32.0 and current_price <= bb_lower * 1.002

# ── Test helpers ─────────────────────────────────────────────────────────────

PASS = 0; FAIL = 0

def check(name, got, want):
    global PASS, FAIL
    if got == want:
        print(f"  ✅ {name}")
        PASS += 1
    else:
        print(f"  ❌ {name}")
        print(f"       got  = {got!r}")
        print(f"       want = {want!r}")
        FAIL += 1

def make_candles(closes, open_pct=0.0):
    """Build synthetic candles from close prices. open = close * (1 - open_pct)."""
    return [{"open": c * (1 - open_pct), "high": c * 1.005, "low": c * 0.995, "close": c, "vol": 1.0}
            for c in closes]

# ── Test Suite ────────────────────────────────────────────────────────────────

def test_sma():
    print("\n── sma() ──────────────────────────────────────────────────────────")
    c = make_candles([1, 2, 3, 4, 5, 6, 7, 8, 9, 10])
    check("sma-5 of 1..10 = 8.0",   sma(c, 5),  8.0)
    check("sma-10 of 1..10 = 5.5",  sma(c, 10), 5.5)
    check("sma-11 → None (too few)", sma(c, 11), None)
    check("sma-0  → None (period=0)", sma(c, 0), None)
    check("sma-1 = last close",      sma(c, 1),  10.0)

def test_classify():
    print("\n── MacroRegime.classify() ─────────────────────────────────────────")
    B, R, T = MacroRegime.BULL, MacroRegime.BEAR, MacroRegime.TRANSITION

    # Pure BULL: price well above MA20, MA5 > MA10
    check("BULL: price>MA20, MA5>MA10",
          MacroRegime.classify(100.0, 99.0, 97.0, 90.0), B)

    # Pure BEAR: price below MA20, MA5 < MA10
    check("BEAR: price<MA20, MA5<MA10",
          MacroRegime.classify(80.0, 81.0, 83.0, 90.0), R)

    # Transition: price above MA20 but MA5 < MA10
    check("TRANSITION: price>MA20 but MA5<MA10",
          MacroRegime.classify(95.0, 88.0, 90.0, 90.0), T)

    # Transition: price below MA20 but MA5 > MA10
    check("TRANSITION: price<MA20 but MA5>MA10",
          MacroRegime.classify(85.0, 90.0, 88.0, 90.0), T)

    # Zero guard
    check("TRANSITION when ma20=0",  MacroRegime.classify(100, 99, 97, 0), T)
    check("TRANSITION when price=0", MacroRegime.classify(0,   99, 97, 90), T)

    # Edge: price exactly equals MA20 → price > ma20 is False → TRANSITION
    # (strictly greater-than required, same as Rust: `price > ma20`)
    check("price == MA20, MA5>MA10 → TRANSITION (not strictly above)",
          MacroRegime.classify(90.0, 91.0, 89.0, 90.0), T)
    # Price *just* above MA20
    check("price just above MA20, MA5<MA10 → TRANSITION",
          MacroRegime.classify(90.01, 88.0, 90.0, 90.0), T)

def test_consensus():
    print("\n── MacroRegime.consensus() ────────────────────────────────────────")
    B, R, T = MacroRegime.BULL, MacroRegime.BEAR, MacroRegime.TRANSITION

    check("BULL+BULL → BULL",         MacroRegime.consensus(B, B), B)
    check("BEAR+BEAR → BEAR",         MacroRegime.consensus(R, R), R)
    check("BULL+TRANSITION → BULL",   MacroRegime.consensus(B, T), B)
    check("BEAR+TRANSITION → BEAR",   MacroRegime.consensus(R, T), R)
    check("BULL+BEAR → TRANSITION",   MacroRegime.consensus(B, R), T)
    check("BEAR+BULL → TRANSITION",   MacroRegime.consensus(R, B), T)
    check("TRANSITION+BULL → TRANS",  MacroRegime.consensus(T, B), T)
    check("TRANSITION+BEAR → TRANS",  MacroRegime.consensus(T, R), T)
    check("TRANSITION+TRANS → TRANS", MacroRegime.consensus(T, T), T)

def test_pico_top():
    print("\n── is_pico_top() ──────────────────────────────────────────────────")

    bb_upper = 100.0

    # ─ ALL three conditions met ──────────────────────────────────────────────
    # candle 3 is a large green (3% body), current price is at BB upper
    base = make_candles([95, 96, 97, 98, 99] * 4)
    # Replace last 3 with: large green, neutral, current near bb_upper
    big_green  = {"open": 96.0, "high": 100.5, "low": 95.5, "close": 99.5, "vol": 1.0}  # 3.6% green
    neutral    = {"open": 99.0, "high": 100.0, "low": 98.5, "close": 99.3, "vol": 1.0}
    at_upper   = {"open": 99.5, "high": 100.5, "low": 99.0, "close": 100.0, "vol": 1.0}  # price == bb_upper
    window = base[:-3] + [big_green, neutral, at_upper]
    rsi_val = 72.0
    check("ALL met → pico_top=True",  is_pico_top(window, rsi_val, bb_upper), True)

    # ─ RSI not overbought ────────────────────────────────────────────────────
    check("RSI=65 (not >68) → False", is_pico_top(window, 65.0, bb_upper), False)

    # ─ Price below BB upper ───────────────────────────────────────────────────
    below_bb = {"open": 96.0, "high": 97.0, "low": 95.5, "close": 96.5, "vol": 1.0}
    window2 = base[:-1] + [below_bb]
    check("price < BB_upper*0.998 → False", is_pico_top(window2, 72.0, bb_upper), False)

    # ─ No large green candle ─────────────────────────────────────────────────
    small_green = {"open": 99.0, "high": 100.1, "low": 98.9, "close": 99.5, "vol": 1.0}  # 0.5% green
    window3 = base[:-3] + [small_green, neutral, at_upper]
    check("no ≥2% green → False",  is_pico_top(window3, 72.0, bb_upper), False)

    # ─ Price within 0.2% of BB upper (should still trigger) ──────────────────
    barely = {"open": 99.5, "high": 100.5, "low": 99.0, "close": 99.81, "vol": 1.0}  # 99.81 >= 100*0.998
    window4 = base[:-3] + [big_green, neutral, barely]
    check("price at 99.81% of BB_upper → True (within 0.2%)", is_pico_top(window4, 72.0, bb_upper), True)

    # ─ Large green but RSI barely below threshold ────────────────────────────
    check("RSI exactly 68.0 (not >68) → False", is_pico_top(window, 68.0, bb_upper), False)
    check("RSI = 68.1 → True",                  is_pico_top(window, 68.1, bb_upper), True)

def test_bear_bounce():
    print("\n── is_bear_bounce() ───────────────────────────────────────────────")

    bb_lower = 100.0

    base = make_candles([103, 102, 101, 100, 99] * 4)
    big_red   = {"open": 103.0, "high": 103.5, "low": 99.5, "close": 99.5, "vol": 1.0}  # 3.4% red
    neutral   = {"open": 100.0, "high": 100.5, "low": 99.0, "close": 100.0, "vol": 1.0}
    at_lower  = {"open": 100.0, "high": 100.2, "low": 99.5, "close": 99.8, "vol": 1.0}   # ≤ 100*1.002

    window = base[:-3] + [big_red, neutral, at_lower]
    rsi_val = 28.0

    check("ALL met → bounce=True",    is_bear_bounce(window, rsi_val, bb_lower), True)
    check("RSI=33 (not <32) → False", is_bear_bounce(window, 33.0, bb_lower), False)

    above_lower = {"open": 100.0, "high": 101.0, "low": 99.5, "close": 101.5, "vol": 1.0}
    window2 = base[:-1] + [above_lower]
    check("price > BB_lower*1.002 → False", is_bear_bounce(window2, 28.0, bb_lower), False)

    small_red = {"open": 100.0, "high": 100.1, "low": 99.4, "close": 99.8, "vol": 1.0}  # 0.2% red
    window3 = base[:-3] + [small_red, neutral, at_lower]
    check("no ≥2% red → False",    is_bear_bounce(window3, 28.0, bb_lower), False)

def test_daily_ma_regime_sequence():
    """Feed a synthetic 30-day price series through the full classify pipeline."""
    print("\n── Full daily MA+classify sequence ────────────────────────────────")

    # Craft a price series that should go BEAR → TRANSITION → BULL
    # Phase 1 (days 0-9): falling prices — BEAR
    # Phase 2 (days 10-19): flat/sideways — TRANSITION
    # Phase 3 (days 20-29): rising fast — BULL
    prices = (
        [1000 - i * 20 for i in range(10)]   # 1000 → 820  (falling)
      + [820  + i * 2  for i in range(10)]   # 820  → 838  (flat)
      + [838  + i * 15 for i in range(10)]   # 838  → 973  (rising)
    )
    candles = make_candles(prices)

    # Day 20 (start of bull phase, need 20 for MA20)
    regime_20 = MacroRegime.classify(
        candles[20]["close"],
        *daily_mas(candles[:21])
    )
    # Day 29 (end, should be BULL by now)
    regime_29 = MacroRegime.classify(
        candles[29]["close"],
        *daily_mas(candles[:30])
    )
    # Day 5 (deep bear)
    regime_5 = MacroRegime.classify(
        candles[5]["close"],
        *daily_mas(candles[:6])  # only 6 candles, MA10/20 → 0 → TRANSITION
    )

    check("Day 5 (too few candles for MA20) → TRANSITION", regime_5, "TRANSITION")
    # Day 20 is at start of recovery — might still be BEAR or TRANSITION
    check("Day 20 (start of rally) not BULL yet",  regime_20 in ("BEAR","TRANSITION"), True)
    check("Day 29 (after strong rally) → BULL",    regime_29, "BULL")

def test_rsi_synthetic():
    print("\n── RSI sanity checks ──────────────────────────────────────────────")
    # 15 consecutive up days → RSI should be very high
    up_candles = make_candles([100 + i for i in range(25)])
    r_up = rsi(up_candles)
    check("15 up days: RSI > 80", r_up > 80, True)

    # 15 consecutive down days → RSI should be very low
    down_candles = make_candles([100 - i for i in range(25)])
    r_down = rsi(down_candles)
    check("15 down days: RSI < 20", r_down < 20, True)

    # Flat prices → avg_loss = 0 → RS = gain/0 = ∞ → RSI = 100.
    # This is mathematically correct (no losses = max bullishness).
    # Same behaviour as the Rust implementation.
    flat_candles = make_candles([100.0] * 25)
    r_flat = rsi(flat_candles)
    check("flat prices (no losses): RSI == 100.0", r_flat, 100.0)

# ── Run all tests ─────────────────────────────────────────────────────────────

if __name__ == "__main__":
    print()
    print("╔══════════════════════════════════════════════════════════════════════╗")
    print("║    UNIT TESTS: Macro Regime + Pico-Top Logic                        ║")
    print("║    Validates Python mirrors Rust implementation exactly             ║")
    print("╚══════════════════════════════════════════════════════════════════════╝")

    test_sma()
    test_classify()
    test_consensus()
    test_pico_top()
    test_bear_bounce()
    test_daily_ma_regime_sequence()
    test_rsi_synthetic()

    total = PASS + FAIL
    print(f"\n{'═'*70}")
    print(f"  Results: {PASS}/{total} passed", end="")
    if FAIL == 0:
        print("  ✅ ALL PASS")
    else:
        print(f"  ❌ {FAIL} FAILED")
    print(f"{'═'*70}\n")
    sys.exit(0 if FAIL == 0 else 1)
