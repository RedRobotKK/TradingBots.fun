#!/usr/bin/env python3
"""
Signal Quality Backtest — TradingBots.fun
============================================
Tests all 10 bot signals across 8 assets × 2 years of 15-minute data.

Signals tested:
  1. RSI          — Wilder's RSI(14), mean-reversion extremes
  2. Bollinger    — Band position + squeeze breakout
  3. MACD         — Histogram momentum direction
  4. EMA Cross    — EMA(8/21) crossover
  5. Z-Score      — 20-bar statistical mean-reversion depth
  6. Volume       — Volume ratio conviction (20-bar mean)
  7. Trend        — 10-bar % change (legacy)
  8. Funding Rate — Perpetual funding (Binance USDT-M, 8h contrarian)
  9. Order Flow   — Proxy: candle body asymmetry (bid/ask imbalance)
 10. Sentiment    — Not backtestable (live API only) — excluded

Metrics per signal:
  • IC        — Pearson correlation with forward return (primary)
  • Hit Rate  — % of signals where price moved in predicted direction
  • T-Stat    — Statistical significance vs 50% random baseline
  • Frequency — How often the signal fires (too rare = fragile)
  • Grade     — A–F: IC×0.40 + hit_rate×0.40 + t_stat×0.20

Horizons: 1h (4 bars), 4h (16 bars), 1d (96 bars) at 15m resolution
Regime:   Trending (ADX>27), Neutral (19–27), Ranging (<19) per bar

Usage:
  python3 signal_quality_backtest.py

Output:
  signal_quality_report.html
"""

import json
import math
import os
import sys
import time
import urllib.request
import urllib.parse

from datetime import datetime, timezone

# ─────────────────────────────────────────────────────────────────────────────
# Configuration
# ─────────────────────────────────────────────────────────────────────────────

SYMBOLS   = ["BTC", "ETH", "SOL", "BNB", "AVAX", "LINK", "ARB", "SUI"]
INTERVAL  = "15m"
YEARS     = 2          # lookback
LIMIT_PER_CALL = 1000  # Binance max

FORWARD_HORIZONS = {
    "1h":  4,    # bars at 15m
    "4h":  16,
    "1d":  96,
}
PRIMARY_HORIZON = "4h"

# ─────────────────────────────────────────────────────────────────────────────
# Binance data fetching
# ─────────────────────────────────────────────────────────────────────────────

BASE_SPOT    = "https://api.binance.com"
BASE_FUTURES = "https://fapi.binance.com"


def _get_json(url: str, retries: int = 3) -> object:
    for attempt in range(retries):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": "TradingBots.funBot/1.0"})
            with urllib.request.urlopen(req, timeout=20) as r:
                return json.loads(r.read())
        except Exception as e:
            if attempt == retries - 1:
                raise
            print(f"  ↺ retry {attempt+1}: {e}")
            time.sleep(2 ** attempt)


def fetch_klines(symbol: str, interval: str, start_ms: int, end_ms: int) -> list:
    """Fetch all klines between start_ms and end_ms in paginated 1000-bar chunks."""
    bars = []
    cur = start_ms
    while cur < end_ms:
        url = (
            f"{BASE_SPOT}/api/v3/klines"
            f"?symbol={symbol}USDT&interval={interval}"
            f"&startTime={cur}&endTime={end_ms}&limit={LIMIT_PER_CALL}"
        )
        data = _get_json(url)
        if not data or not isinstance(data, list):
            break
        for d in data:
            bars.append({
                "t":  int(d[0]),
                "o":  float(d[1]),
                "h":  float(d[2]),
                "l":  float(d[3]),
                "c":  float(d[4]),
                "v":  float(d[5]),
            })
        cur = int(data[-1][0]) + 1
        if len(data) < LIMIT_PER_CALL:
            break
        time.sleep(0.12)   # polite to free tier
    return bars


def fetch_funding_history(symbol: str, start_ms: int, end_ms: int) -> dict:
    """
    Returns {timestamp_ms: rate} for all 8h funding settlements.
    Only available for symbols listed on Binance USDT-M futures.
    """
    rates = {}
    limit  = 1000
    cur    = start_ms
    while cur < end_ms:
        url = (
            f"{BASE_FUTURES}/fapi/v1/fundingRate"
            f"?symbol={symbol}USDT&startTime={cur}&endTime={end_ms}&limit={limit}"
        )
        try:
            data = _get_json(url)
        except Exception as e:
            print(f"  ⚠ Funding history unavailable for {symbol}: {e}")
            return rates
        if not data or not isinstance(data, list):
            break
        for d in data:
            rates[int(d["fundingTime"])] = float(d["fundingRate"])
        cur = int(data[-1]["fundingTime"]) + 1
        if len(data) < limit:
            break
        time.sleep(0.12)
    return rates


def forward_fill_funding(klines: list, funding_map: dict) -> list:
    """
    Attach the most-recent funding rate to each bar (forward-fill every 8h).
    Sets bar["funding"] = last known rate at bar open time.
    """
    if not funding_map:
        for b in klines:
            b["funding"] = 0.0
        return klines

    sorted_times = sorted(funding_map.keys())
    idx = 0
    for b in klines:
        # Advance until we overshoot bar open time
        while idx + 1 < len(sorted_times) and sorted_times[idx + 1] <= b["t"]:
            idx += 1
        if sorted_times[idx] <= b["t"]:
            b["funding"] = funding_map[sorted_times[idx]]
        else:
            b["funding"] = 0.0
    return klines

# ─────────────────────────────────────────────────────────────────────────────
# Indicator helpers  (mirror decision.rs logic exactly)
# ─────────────────────────────────────────────────────────────────────────────

def wilder_rsi(closes: list, period: int = 14) -> list:
    """Wilder's smoothed RSI — matches the bot's ta::rsi call."""
    n = len(closes)
    rsi_out = [float("nan")] * n
    if n < period + 1:
        return rsi_out

    gains = []
    losses = []
    for i in range(1, period + 1):
        diff = closes[i] - closes[i - 1]
        gains.append(max(diff, 0.0))
        losses.append(max(-diff, 0.0))

    avg_gain = sum(gains) / period
    avg_loss = sum(losses) / period

    for i in range(period, n):
        if i > period:
            diff = closes[i] - closes[i - 1]
            avg_gain = (avg_gain * (period - 1) + max(diff, 0.0)) / period
            avg_loss = (avg_loss * (period - 1) + max(-diff, 0.0)) / period
        rs = avg_gain / avg_loss if avg_loss > 1e-9 else 1e9
        rsi_out[i] = 100.0 - (100.0 / (1.0 + rs))

    return rsi_out


def bollinger_bands(closes: list, period: int = 20, k: float = 2.0):
    """Returns (upper, mid, lower) as parallel lists."""
    n = len(closes)
    upper = [float("nan")] * n
    mid   = [float("nan")] * n
    lower = [float("nan")] * n
    for i in range(period - 1, n):
        window = closes[i - period + 1: i + 1]
        mean = sum(window) / period
        std  = math.sqrt(sum((x - mean) ** 2 for x in window) / period)
        mid[i]   = mean
        upper[i] = mean + k * std
        lower[i] = mean - k * std
    return upper, mid, lower


def ema(closes: list, period: int) -> list:
    """Exponential moving average."""
    n = len(closes)
    out = [float("nan")] * n
    k = 2.0 / (period + 1)
    seed_idx = period - 1
    if seed_idx >= n:
        return out
    out[seed_idx] = sum(closes[:period]) / period
    for i in range(seed_idx + 1, n):
        out[i] = closes[i] * k + out[i - 1] * (1 - k)
    return out


def macd_histogram(closes: list, fast: int = 12, slow: int = 26, signal: int = 9) -> list:
    """MACD histogram (MACD line − signal line)."""
    ema_fast = ema(closes, fast)
    ema_slow = ema(closes, slow)
    n = len(closes)
    macd_line = [
        ema_fast[i] - ema_slow[i]
        if not math.isnan(ema_fast[i]) and not math.isnan(ema_slow[i])
        else float("nan")
        for i in range(n)
    ]

    # EMA of MACD line (signal)
    valid_start = next((i for i, v in enumerate(macd_line) if not math.isnan(v)), None)
    sig_line = [float("nan")] * n
    if valid_start is None or valid_start + signal > n:
        return [float("nan")] * n

    seed = valid_start + signal - 1
    sig_line[seed] = sum(macd_line[valid_start: valid_start + signal]) / signal
    k = 2.0 / (signal + 1)
    for i in range(seed + 1, n):
        sig_line[i] = macd_line[i] * k + sig_line[i - 1] * (1 - k)

    hist = [
        macd_line[i] - sig_line[i]
        if not math.isnan(macd_line[i]) and not math.isnan(sig_line[i])
        else float("nan")
        for i in range(n)
    ]
    return hist


def z_score(closes: list, period: int = 20) -> list:
    """Rolling z-score of close vs 20-bar mean."""
    n = len(closes)
    out = [float("nan")] * n
    for i in range(period - 1, n):
        window = closes[i - period + 1: i + 1]
        mean = sum(window) / period
        std  = math.sqrt(sum((x - mean) ** 2 for x in window) / period)
        out[i] = (closes[i] - mean) / std if std > 1e-9 else 0.0
    return out


def volume_ratio(volumes: list, period: int = 20) -> list:
    """Current volume / 20-bar mean volume."""
    n = len(volumes)
    out = [float("nan")] * n
    for i in range(period - 1, n):
        mean_vol = sum(volumes[i - period + 1: i + 1]) / period
        out[i] = volumes[i] / mean_vol if mean_vol > 1e-9 else 1.0
    return out


def trend_10bar(closes: list) -> list:
    """10-bar percentage change."""
    n = len(closes)
    out = [float("nan")] * n
    for i in range(10, n):
        out[i] = (closes[i] - closes[i - 10]) / closes[i - 10] * 100.0 \
                 if closes[i - 10] > 0 else 0.0
    return out


def true_range_series(bars: list) -> list:
    n = len(bars)
    tr = [float("nan")] * n
    for i in range(1, n):
        hl  = bars[i]["h"] - bars[i]["l"]
        hpc = abs(bars[i]["h"] - bars[i - 1]["c"])
        lpc = abs(bars[i]["l"] - bars[i - 1]["c"])
        tr[i] = max(hl, hpc, lpc)
    return tr


def atr_series(bars: list, period: int = 14) -> list:
    tr = true_range_series(bars)
    n = len(tr)
    out = [float("nan")] * n
    # seed
    seed = next((i for i in range(1, n) if not math.isnan(tr[i])), None)
    if seed is None or seed + period > n:
        return out
    out[seed + period - 1] = sum(tr[seed: seed + period]) / period
    for i in range(seed + period, n):
        out[i] = (out[i - 1] * (period - 1) + tr[i]) / period
    return out


def adx_series(bars: list, period: int = 14) -> list:
    """Wilder's ADX.  Returns ADX values (trend strength, 0–100)."""
    n = len(bars)
    adx_out = [float("nan")] * n
    if n < period * 2 + 1:
        return adx_out

    plus_dm  = [0.0] * n
    minus_dm = [0.0] * n
    tr_arr   = [0.0] * n

    for i in range(1, n):
        move_up   = bars[i]["h"] - bars[i - 1]["h"]
        move_down = bars[i - 1]["l"] - bars[i]["l"]
        plus_dm[i]  = move_up   if move_up   > move_down and move_up   > 0 else 0.0
        minus_dm[i] = move_down if move_down > move_up   and move_down > 0 else 0.0
        hl  = bars[i]["h"] - bars[i]["l"]
        hpc = abs(bars[i]["h"] - bars[i - 1]["c"])
        lpc = abs(bars[i]["l"] - bars[i - 1]["c"])
        tr_arr[i] = max(hl, hpc, lpc)

    # Wilder smooth
    def wilder_smooth(arr, p):
        out = [0.0] * n
        out[p] = sum(arr[1: p + 1])
        for i in range(p + 1, n):
            out[i] = out[i - 1] - out[i - 1] / p + arr[i]
        return out

    sm_tr    = wilder_smooth(tr_arr, period)
    sm_plus  = wilder_smooth(plus_dm, period)
    sm_minus = wilder_smooth(minus_dm, period)

    dx_arr = [float("nan")] * n
    for i in range(period, n):
        if sm_tr[i] < 1e-9:
            continue
        di_plus  = 100.0 * sm_plus[i]  / sm_tr[i]
        di_minus = 100.0 * sm_minus[i] / sm_tr[i]
        denom = di_plus + di_minus
        dx_arr[i] = 100.0 * abs(di_plus - di_minus) / denom if denom > 1e-9 else 0.0

    # Smooth DX into ADX
    first_dx = next((i for i in range(n) if not math.isnan(dx_arr[i])), None)
    if first_dx is None or first_dx + period > n:
        return adx_out
    adx_out[first_dx + period - 1] = sum(
        dx_arr[first_dx: first_dx + period]
    ) / period
    for i in range(first_dx + period, n):
        if not math.isnan(adx_out[i - 1]):
            adx_out[i] = (adx_out[i - 1] * (period - 1) + dx_arr[i]) / period
    return adx_out


# ─────────────────────────────────────────────────────────────────────────────
# Signal conditions  (mirror decision.rs signal logic)
# ─────────────────────────────────────────────────────────────────────────────

def _adx_regime(adx_val: float) -> str:
    """Map an ADX value to a regime string."""
    if math.isnan(adx_val):
        return "ranging"
    if adx_val > 27:
        return "trending"
    if adx_val > 19:
        return "neutral"
    return "ranging"


def compute_signals(bars: list) -> list:
    """
    Returns list of dicts, one per bar, with signal fields.
    Signal values:
      +1 = bullish, -1 = bearish, 0 = no signal / neutral
    """
    closes  = [b["c"] for b in bars]
    highs   = [b["h"] for b in bars]
    lows    = [b["l"] for b in bars]
    vols    = [b["v"] for b in bars]
    opens   = [b["o"] for b in bars]

    rsi_vals    = wilder_rsi(closes)
    bb_up, bb_mid, bb_low = bollinger_bands(closes)
    macd_hist   = macd_histogram(closes)
    ema8        = ema(closes, 8)
    ema21       = ema(closes, 21)
    z           = z_score(closes)
    vol_r       = volume_ratio(vols)
    trend10     = trend_10bar(closes)
    adx         = adx_series(bars)
    atr         = atr_series(bars)

    n = len(bars)
    result = []

    for i in range(n):
        rec = {
            "t":       bars[i]["t"],
            "close":   closes[i],
            "atr":     atr[i],
            "adx":     adx[i],
            "funding": bars[i].get("funding", 0.0),
            # regime
            "regime":  _adx_regime(adx[i]),
        }

        # ── 1. RSI ────────────────────────────────────────────────────────────
        r = rsi_vals[i]
        if math.isnan(r):
            rec["rsi"] = 0
        elif r <= 30:
            rec["rsi"] = 1    # oversold → bullish
        elif r >= 70:
            rec["rsi"] = -1   # overbought → bearish
        else:
            rec["rsi"] = 0

        # ── 2. Bollinger Bands ────────────────────────────────────────────────
        if math.isnan(bb_up[i]) or math.isnan(bb_low[i]):
            rec["bollinger"] = 0
        elif closes[i] < bb_low[i]:
            rec["bollinger"] = 1   # below lower band → bullish
        elif closes[i] > bb_up[i]:
            rec["bollinger"] = -1  # above upper band → bearish
        else:
            # Squeeze breakout: band width narrowing then expanding
            if i >= 1 and not math.isnan(bb_up[i-1]):
                bw_now  = (bb_up[i]   - bb_low[i])   / bb_mid[i]
                bw_prev = (bb_up[i-1] - bb_low[i-1]) / bb_mid[i-1]
                if bw_now > bw_prev * 1.05:   # expanding — follow price direction
                    rec["bollinger"] = 1 if closes[i] > bb_mid[i] else -1
                else:
                    rec["bollinger"] = 0
            else:
                rec["bollinger"] = 0

        # ── 3. MACD ───────────────────────────────────────────────────────────
        if math.isnan(macd_hist[i]):
            rec["macd"] = 0
        elif i >= 1 and not math.isnan(macd_hist[i - 1]):
            if macd_hist[i] > 0 and macd_hist[i] > macd_hist[i - 1]:
                rec["macd"] = 1    # positive and rising
            elif macd_hist[i] < 0 and macd_hist[i] < macd_hist[i - 1]:
                rec["macd"] = -1   # negative and falling
            else:
                rec["macd"] = 0
        else:
            rec["macd"] = 0

        # ── 4. EMA Cross ──────────────────────────────────────────────────────
        if math.isnan(ema8[i]) or math.isnan(ema21[i]):
            rec["ema_cross"] = 0
        elif i >= 1 and not math.isnan(ema8[i-1]) and not math.isnan(ema21[i-1]):
            # Crossover this bar
            was_above = ema8[i-1] > ema21[i-1]
            is_above  = ema8[i]   > ema21[i]
            if not was_above and is_above:
                rec["ema_cross"] = 1   # golden cross
            elif was_above and not is_above:
                rec["ema_cross"] = -1  # death cross
            else:
                # Trend continuation (no cross this bar, use direction)
                rec["ema_cross"] = 1 if is_above else -1
        else:
            rec["ema_cross"] = 0

        # ── 5. Z-Score ────────────────────────────────────────────────────────
        z_val = z[i]
        if math.isnan(z_val):
            rec["z_score"] = 0
        elif z_val < -2.0:
            rec["z_score"] = 1    # deeply oversold → bullish
        elif z_val > 2.0:
            rec["z_score"] = -1   # deeply overbought → bearish
        elif z_val < -1.5:
            rec["z_score"] = 1
        elif z_val > 1.5:
            rec["z_score"] = -1
        else:
            rec["z_score"] = 0

        # ── 6. Volume ─────────────────────────────────────────────────────────
        vr = vol_r[i]
        if math.isnan(vr):
            rec["volume"] = 0
        elif vr >= 1.5:
            # High volume — follow price direction
            rec["volume"] = 1 if closes[i] >= opens[i] else -1
        elif vr < 0.5:
            rec["volume"] = 0   # very low volume = no conviction
        else:
            rec["volume"] = 0

        # ── 7. Trend 10-bar ───────────────────────────────────────────────────
        t10 = trend10[i]
        if math.isnan(t10):
            rec["trend"] = 0
        elif t10 > 1.5:
            rec["trend"] = 1
        elif t10 < -1.5:
            rec["trend"] = -1
        else:
            rec["trend"] = 0

        # ── 8. Funding Rate ───────────────────────────────────────────────────
        fr = rec["funding"]
        if   fr >  0.0010: rec["funding_sig"] = -1   # extreme longs → bear
        elif fr >  0.0005: rec["funding_sig"] = -1
        elif fr >  0.0002: rec["funding_sig"] = -1
        elif fr < -0.0010: rec["funding_sig"] =  1   # extreme shorts → bull
        elif fr < -0.0005: rec["funding_sig"] =  1
        elif fr < -0.0002: rec["funding_sig"] =  1
        else:              rec["funding_sig"] =  0

        # ── 9. Order Flow proxy — candle body asymmetry ───────────────────────
        body  = closes[i] - opens[i]
        range_hl = highs[i] - lows[i]
        if range_hl < 1e-9:
            rec["order_flow"] = 0
        else:
            body_pct = body / range_hl
            if body_pct > 0.55:
                rec["order_flow"] = 1    # strong bull candle
            elif body_pct < -0.55:
                rec["order_flow"] = -1   # strong bear candle
            else:
                rec["order_flow"] = 0

        result.append(rec)

    return result

# ─────────────────────────────────────────────────────────────────────────────
# Forward returns
# ─────────────────────────────────────────────────────────────────────────────

def compute_forward_returns(signal_rows: list, horizons: dict) -> list:
    """Adds fwd_{label} field to each row (None at tail end)."""
    n = len(signal_rows)
    for label, nbars in horizons.items():
        for i in range(n):
            if i + nbars < n:
                cur   = signal_rows[i]["close"]
                fwd   = signal_rows[i + nbars]["close"]
                signal_rows[i][f"fwd_{label}"] = (fwd - cur) / cur if cur > 1e-9 else None
            else:
                signal_rows[i][f"fwd_{label}"] = None
    return signal_rows

# ─────────────────────────────────────────────────────────────────────────────
# Statistics helpers
# ─────────────────────────────────────────────────────────────────────────────

def pearson_ic(xs: list, ys: list) -> float:
    """Pearson correlation between xs and ys (both numeric, no NaN)."""
    n = len(xs)
    if n < 10:
        return float("nan")
    mx = sum(xs) / n
    my = sum(ys) / n
    num = sum((x - mx) * (y - my) for x, y in zip(xs, ys))
    dx  = math.sqrt(sum((x - mx) ** 2 for x in xs))
    dy  = math.sqrt(sum((y - my) ** 2 for y in ys))
    if dx < 1e-12 or dy < 1e-12:
        return float("nan")
    return num / (dx * dy)


def t_stat_hit_rate(hits: int, n: int) -> float:
    """One-sample t-test for hit_rate vs 50% baseline."""
    if n < 10:
        return float("nan")
    p = hits / n
    se = math.sqrt(p * (1 - p) / n)
    return (p - 0.5) / se if se > 1e-9 else float("nan")


def grade(ic: float, hit_rate: float, t: float, freq: float) -> str:
    """Composite letter grade."""
    if math.isnan(ic) or math.isnan(hit_rate) or math.isnan(t):
        return "?"

    # Penalty if fires < 2% or > 95% of bars (too rare or always-on)
    freq_ok  = 0.02 <= freq <= 0.95
    ic_score = max(0.0, min(1.0, (ic - (-0.05)) / (0.15 - (-0.05))))  # -0.05..+0.15 → 0..1
    hr_score = max(0.0, min(1.0, (hit_rate - 0.45) / (0.65 - 0.45)))  # 45%..65% → 0..1
    t_score  = max(0.0, min(1.0, (t - 0.5) / (3.5 - 0.5)))            # 0.5..3.5 → 0..1
    composite = ic_score * 0.40 + hr_score * 0.40 + t_score * 0.20
    if not freq_ok:
        composite *= 0.70   # penalise degenerate frequencies

    if composite >= 0.75: return "A"
    if composite >= 0.55: return "B"
    if composite >= 0.35: return "C"
    if composite >= 0.15: return "D"
    return "F"

# ─────────────────────────────────────────────────────────────────────────────
# Core evaluation loop
# ─────────────────────────────────────────────────────────────────────────────

SIGNAL_NAMES = [
    "rsi", "bollinger", "macd", "ema_cross",
    "z_score", "volume", "trend", "funding_sig", "order_flow",
]

SIGNAL_LABELS = {
    "rsi":         "RSI(14)",
    "bollinger":   "Bollinger",
    "macd":        "MACD Hist",
    "ema_cross":   "EMA Cross (8/21)",
    "z_score":     "Z-Score(20)",
    "volume":      "Volume Ratio",
    "trend":       "Trend 10-bar",
    "funding_sig": "Funding Rate",
    "order_flow":  "Order Flow Proxy",
}

REGIMES = ["trending", "neutral", "ranging", "all"]


def evaluate_signals(rows: list, horizon: str) -> dict:
    """
    Returns nested dict: results[signal][regime] = {ic, hit_rate, t, freq, n}
    Rows must have fwd_{horizon} field set.
    """
    fwd_key = f"fwd_{horizon}"
    results = {}

    for sig in SIGNAL_NAMES:
        results[sig] = {}

        for reg in REGIMES:
            # Collect aligned (signal != 0) rows matching regime
            xs, ys, hits, total_bars, fired_bars = [], [], 0, 0, 0

            for r in rows:
                fwd = r.get(fwd_key)
                if fwd is None:
                    continue
                row_regime = r.get("regime", "all")
                if reg != "all" and row_regime != reg:
                    continue

                total_bars += 1
                sig_val = r.get(sig, 0)
                if sig_val == 0:
                    continue

                fired_bars += 1
                xs.append(float(sig_val))
                ys.append(fwd)
                # Hit = signal direction matches return direction
                if (sig_val > 0 and fwd > 0) or (sig_val < 0 and fwd < 0):
                    hits += 1

            n      = len(xs)
            freq   = fired_bars / total_bars if total_bars > 0 else 0.0
            ic     = pearson_ic(xs, ys) if n >= 10 else float("nan")
            hr     = hits / n if n > 0 else float("nan")
            t      = t_stat_hit_rate(hits, n)
            g      = grade(ic, hr, t, freq)

            results[sig][reg] = {
                "ic":       ic,
                "hit_rate": hr,
                "t_stat":   t,
                "freq":     freq,
                "n":        n,
                "grade":    g,
            }

    return results

# ─────────────────────────────────────────────────────────────────────────────
# HTML report
# ─────────────────────────────────────────────────────────────────────────────

GRADE_COLOUR = {
    "A": "#22c55e",
    "B": "#86efac",
    "C": "#fbbf24",
    "D": "#f97316",
    "F": "#ef4444",
    "?": "#6b7280",
}


def fmt(v, decimals=3):
    if isinstance(v, float) and math.isnan(v):
        return "—"
    if isinstance(v, float):
        return f"{v:.{decimals}f}"
    return str(v)


def build_html(all_results: dict, meta: dict) -> str:
    """
    all_results: {symbol: {horizon: {signal: {regime: stats}}}}
    meta:        run info dict
    """
    # Aggregate across all symbols for a combined view
    combined = {}
    for sig in SIGNAL_NAMES:
        combined[sig] = {}
        for reg in REGIMES:
            agg_ic, agg_hr, agg_t, agg_n = [], [], [], 0
            for sym_res in all_results.values():
                for hz_res in sym_res.values():
                    s = hz_res.get(sig, {}).get(reg, {})
                    if s.get("n", 0) >= 10:
                        if not math.isnan(s.get("ic", float("nan"))):
                            agg_ic.append(s["ic"])
                        if not math.isnan(s.get("hit_rate", float("nan"))):
                            agg_hr.append(s["hit_rate"])
                        if not math.isnan(s.get("t_stat", float("nan"))):
                            agg_t.append(s["t_stat"])
                        agg_n += s["n"]
            combined[sig][reg] = {
                "ic":       sum(agg_ic) / len(agg_ic) if agg_ic else float("nan"),
                "hit_rate": sum(agg_hr) / len(agg_hr) if agg_hr else float("nan"),
                "t_stat":   sum(agg_t)  / len(agg_t)  if agg_t  else float("nan"),
                "freq":     float("nan"),
                "n":        agg_n,
            }
            g = grade(
                combined[sig][reg]["ic"],
                combined[sig][reg]["hit_rate"],
                combined[sig][reg]["t_stat"],
                0.15,  # assume reasonable freq for aggregate
            )
            combined[sig][reg]["grade"] = g

    # ── HTML head ─────────────────────────────────────────────────────────────
    html = """<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>TradingBots.fun Signal Quality Backtest</title>
<style>
  :root { --bg: #0f172a; --card: #1e293b; --border: #334155; --text: #e2e8f0; --muted: #94a3b8; }
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body { background: var(--bg); color: var(--text); font-family: 'Segoe UI', system-ui, sans-serif; padding: 24px; }
  h1 { font-size: 1.6rem; margin-bottom: 4px; }
  h2 { font-size: 1.15rem; margin: 28px 0 10px; color: #93c5fd; }
  h3 { font-size: 1rem; margin: 20px 0 8px; color: #94a3b8; }
  .meta { color: var(--muted); font-size: 0.85rem; margin-bottom: 24px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); gap: 12px; margin-bottom: 24px; }
  .card { background: var(--card); border: 1px solid var(--border); border-radius: 8px; padding: 16px; }
  .card-title { font-size: 0.85rem; color: var(--muted); margin-bottom: 6px; }
  .card-val { font-size: 1.8rem; font-weight: 700; }
  table { width: 100%; border-collapse: collapse; font-size: 0.82rem; }
  th { background: #1e293b; color: var(--muted); text-align: left; padding: 8px 10px; border-bottom: 1px solid var(--border); }
  td { padding: 7px 10px; border-bottom: 1px solid #1e293b; }
  tr:hover td { background: #1e293b; }
  .grade { display: inline-block; width: 24px; height: 24px; border-radius: 50%; text-align: center; line-height: 24px; font-weight: 700; font-size: 0.8rem; color: #000; }
  .tag { display: inline-block; padding: 2px 8px; border-radius: 4px; font-size: 0.75rem; font-weight: 600; }
  .good  { color: #22c55e; }
  .warn  { color: #fbbf24; }
  .bad   { color: #ef4444; }
  .section { background: var(--card); border: 1px solid var(--border); border-radius: 8px; padding: 16px; margin-bottom: 20px; }
  details summary { cursor: pointer; color: #93c5fd; font-size: 0.95rem; margin-bottom: 8px; }
  .rec { padding: 8px 12px; border-left: 3px solid; border-radius: 4px; margin: 6px 0; font-size: 0.84rem; }
  .rec-keep    { border-color: #22c55e; background: #052e16; }
  .rec-caution { border-color: #fbbf24; background: #1c1203; }
  .rec-remove  { border-color: #ef4444; background: #1c0204; }
</style>
</head>
<body>
"""

    ts = datetime.fromtimestamp(meta["run_time"], tz=timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    html += f"""
<h1>📊 TradingBots.fun Signal Quality Backtest</h1>
<div class="meta">
  Run: {ts} &nbsp;|&nbsp;
  Symbols: {meta['symbols']} &nbsp;|&nbsp;
  Interval: {meta['interval']} &nbsp;|&nbsp;
  Period: {meta['period']} &nbsp;|&nbsp;
  Total bars: {meta['total_bars']:,} &nbsp;|&nbsp;
  Primary horizon: {meta['primary_horizon']}
</div>
"""

    # ── Summary cards ──────────────────────────────────────────────────────────
    total_a = sum(1 for s in SIGNAL_NAMES
                  if combined[s].get("all", {}).get("grade","?") == "A")
    total_b = sum(1 for s in SIGNAL_NAMES
                  if combined[s].get("all", {}).get("grade","?") == "B")
    total_f = sum(1 for s in SIGNAL_NAMES
                  if combined[s].get("all", {}).get("grade","?") in ("F","D"))
    avg_ic  = [combined[s]["all"]["ic"] for s in SIGNAL_NAMES
               if not math.isnan(combined[s]["all"].get("ic", float("nan")))]
    avg_ic_val = sum(avg_ic) / len(avg_ic) if avg_ic else float("nan")

    html += f"""
<div class="grid">
  <div class="card">
    <div class="card-title">Signals graded A/B</div>
    <div class="card-val" style="color:#22c55e">{total_a + total_b} / {len(SIGNAL_NAMES)}</div>
  </div>
  <div class="card">
    <div class="card-title">Avg Information Coefficient</div>
    <div class="card-val" style="color:{'#22c55e' if not math.isnan(avg_ic_val) and avg_ic_val > 0.03 else '#fbbf24'}">{fmt(avg_ic_val, 3)}</div>
  </div>
  <div class="card">
    <div class="card-title">Weak signals (D/F)</div>
    <div class="card-val" style="color:{'#ef4444' if total_f > 2 else '#fbbf24'}">{total_f}</div>
  </div>
</div>
"""

    # ── Combined signal ranking table ─────────────────────────────────────────
    html += """
<h2>Overall Signal Rankings (all symbols, all regimes, 4h horizon)</h2>
<div class="section">
<table>
<thead>
<tr>
  <th>#</th><th>Signal</th><th>IC</th><th>Hit Rate</th><th>T-Stat</th>
  <th>N (bars)</th><th>Grade</th>
</tr>
</thead>
<tbody>
"""
    rows_sorted = sorted(
        SIGNAL_NAMES,
        key=lambda s: combined[s].get("all", {}).get("ic", -999.0),
        reverse=True,
    )
    for rank, sig in enumerate(rows_sorted, 1):
        st = combined[sig].get("all", {})
        g  = st.get("grade", "?")
        gc = GRADE_COLOUR.get(g, "#6b7280")
        _ic = st.get("ic", float("nan"))
        _hr = st.get("hit_rate", float("nan"))
        _ts = st.get("t_stat", float("nan"))
        if not math.isnan(_ic) and _ic > 0.03:
            ic_col = "good"
        elif not math.isnan(_ic) and _ic > 0.01:
            ic_col = "warn"
        else:
            ic_col = "bad"
        if not math.isnan(_hr) and _hr > 0.52:
            hr_col = "good"
        elif not math.isnan(_hr) and _hr > 0.50:
            hr_col = "warn"
        else:
            hr_col = "bad"
        if not math.isnan(_ts) and _ts > 2.0:
            t_col = "good"
        elif not math.isnan(_ts) and _ts > 1.0:
            t_col = "warn"
        else:
            t_col = "bad"
        html += f"""
<tr>
  <td>{rank}</td>
  <td><strong>{SIGNAL_LABELS.get(sig, sig)}</strong></td>
  <td class="{ic_col}">{fmt(st.get('ic', float('nan')),4)}</td>
  <td class="{hr_col}">{fmt(st.get('hit_rate', float('nan')),3) if not math.isnan(st.get('hit_rate', float('nan'))) else '—'}</td>
  <td class="{t_col}">{fmt(st.get('t_stat', float('nan')),2)}</td>
  <td>{st.get('n', 0):,}</td>
  <td><span class="grade" style="background:{gc}">{g}</span></td>
</tr>
"""
    html += "</tbody></table></div>\n"

    # ── Regime breakdown per signal ───────────────────────────────────────────
    html += "<h2>Regime Breakdown (combined, 4h horizon)</h2>\n"
    html += "<div class='section'><table>\n"
    html += """<thead><tr>
  <th>Signal</th>
  <th>Trending IC</th><th>Trending HR</th>
  <th>Neutral IC</th><th>Neutral HR</th>
  <th>Ranging IC</th><th>Ranging HR</th>
</tr></thead><tbody>\n"""
    for sig in rows_sorted:
        cells = ""
        for reg in ["trending", "neutral", "ranging"]:
            st = combined[sig].get(reg, {})
            ic = st.get("ic", float("nan"))
            hr = st.get("hit_rate", float("nan"))
            if not math.isnan(ic) and ic > 0.03:
                ic_col = "good"
            elif not math.isnan(ic) and ic > 0.01:
                ic_col = "warn"
            else:
                ic_col = "bad"
            if not math.isnan(hr) and hr > 0.52:
                hr_col = "good"
            elif not math.isnan(hr) and hr > 0.50:
                hr_col = "warn"
            else:
                hr_col = "bad"
            cells += f'<td class="{ic_col}">{fmt(ic,4)}</td>'
            cells += f'<td class="{hr_col}">{fmt(hr,3) if not math.isnan(hr) else "—"}</td>'
        html += f"<tr><td><strong>{SIGNAL_LABELS.get(sig, sig)}</strong></td>{cells}</tr>\n"
    html += "</tbody></table></div>\n"

    # ── Per-symbol details (collapsible) ──────────────────────────────────────
    html += "<h2>Per-Symbol Detail</h2>\n"
    for sym in sorted(all_results.keys()):
        sym_res = all_results[sym]
        html += f"<details><summary>{sym} — click to expand</summary>\n"
        for hz in ["1h", "4h", "1d"]:
            hz_res = sym_res.get(hz, {})
            html += f"<h3>{sym} — {hz} horizon</h3>\n"
            html += """<table><thead><tr>
  <th>Signal</th><th>IC</th><th>Hit Rate</th><th>T-Stat</th>
  <th>Freq</th><th>N</th><th>Grade</th>
</tr></thead><tbody>\n"""
            for sig in rows_sorted:
                st = hz_res.get(sig, {}).get("all", {})
                g  = st.get("grade", "?")
                gc = GRADE_COLOUR.get(g, "#6b7280")
                html += f"""<tr>
  <td>{SIGNAL_LABELS.get(sig, sig)}</td>
  <td>{fmt(st.get('ic', float('nan')),4)}</td>
  <td>{fmt(st.get('hit_rate', float('nan')),3) if not math.isnan(st.get('hit_rate', float('nan'))) else '—'}</td>
  <td>{fmt(st.get('t_stat', float('nan')),2)}</td>
  <td>{fmt(st.get('freq', float('nan')),3)}</td>
  <td>{st.get('n', 0):,}</td>
  <td><span class="grade" style="background:{gc}">{g}</span></td>
</tr>\n"""
            html += "</tbody></table>\n"
        html += "</details>\n"

    # ── Recommendations ───────────────────────────────────────────────────────
    html += "<h2>Recommendations</h2><div class='section'>\n"
    for sig in rows_sorted:
        st  = combined[sig].get("all", {})
        g   = st.get("grade", "?")
        ic  = st.get("ic",   float("nan"))
        hr  = st.get("hit_rate", float("nan"))
        lbl = SIGNAL_LABELS.get(sig, sig)

        if g in ("A", "B"):
            cls = "rec-keep"
            msg = f"✅ <strong>{lbl}</strong> — Keep. IC={fmt(ic,4)}, HR={fmt(hr,3)}, Grade={g}."
            if g == "A":
                msg += " Consider increasing default weight."
        elif g == "C":
            cls = "rec-caution"
            msg = f"⚠️ <strong>{lbl}</strong> — Marginal. IC={fmt(ic,4)}, HR={fmt(hr,3)}. Review entry thresholds."
        else:
            cls = "rec-remove"
            msg = f"❌ <strong>{lbl}</strong> — Weak (Grade={g}, IC={fmt(ic,4)}). Consider reducing weight or removing."

        html += f'<div class="rec {cls}">{msg}</div>\n'

    html += "</div>\n"

    # ── Footer ─────────────────────────────────────────────────────────────────
    html += f"""
<div class="meta" style="margin-top:32px; text-align:center;">
  TradingBots.fun • Signal Quality Backtest • Generated {ts}
</div>
</body></html>
"""
    return html

# ─────────────────────────────────────────────────────────────────────────────
# Main
# ─────────────────────────────────────────────────────────────────────────────

def main():
    end_ms   = int(time.time() * 1000)
    start_ms = end_ms - int(YEARS * 365.25 * 24 * 3600 * 1000)

    all_results   = {}
    total_bars    = 0
    run_time      = time.time()

    print(f"\n{'='*60}")
    print("  TradingBots.fun Signal Quality Backtest")
    print(f"  Symbols: {', '.join(SYMBOLS)}")
    print(f"  Interval: {INTERVAL}  |  Lookback: {YEARS} year(s)")
    print(f"{'='*60}\n")

    for sym in SYMBOLS:
        print(f"▶ {sym} — fetching klines …", end=" ", flush=True)
        try:
            bars = fetch_klines(sym, INTERVAL, start_ms, end_ms)
        except Exception as e:
            print(f"ERROR: {e}")
            continue

        if len(bars) < 200:
            print(f"insufficient data ({len(bars)} bars) — skip")
            continue
        print(f"{len(bars):,} bars", end="")

        # Funding history (futures symbols only — skip on error)
        try:
            fund_map = fetch_funding_history(sym, start_ms, end_ms)
            bars     = forward_fill_funding(bars, fund_map)
            if fund_map:
                print(f"  💰 {len(fund_map)} funding pts", end="")
        except Exception:
            for b in bars:
                b["funding"] = 0.0

        print()
        total_bars += len(bars)

        # Compute signals
        rows = compute_signals(bars)
        rows = compute_forward_returns(rows, FORWARD_HORIZONS)

        # Evaluate per horizon
        sym_hz_results = {}
        for hz in FORWARD_HORIZONS:
            sym_hz_results[hz] = evaluate_signals(rows, hz)

        all_results[sym] = sym_hz_results

        # Quick summary line
        prim = sym_hz_results.get(PRIMARY_HORIZON, {})
        grades_line = "  ".join(
            f"{SIGNAL_LABELS.get(s,s)[:8]}:{prim.get(s,{}).get('all',{}).get('grade','?')}"
            for s in SIGNAL_NAMES
        )
        print(f"  {grades_line}")

    if not all_results:
        print("No data fetched. Check your internet connection.")
        sys.exit(1)

    # Build report
    meta = {
        "symbols":        ", ".join(SYMBOLS),
        "interval":       INTERVAL,
        "period":         f"{YEARS}y",
        "total_bars":     total_bars,
        "primary_horizon": PRIMARY_HORIZON,
        "run_time":       run_time,
    }

    html = build_html(all_results, meta)
    out_path = "signal_quality_report.html"
    with open(out_path, "w", encoding="utf-8") as f:
        f.write(html)

    print(f"\n{'='*60}")
    print(f"  Report saved → {out_path}")
    print(f"  Open in browser: file://{os.path.abspath(out_path)}")
    print(f"{'='*60}\n")

    # Print summary table to console
    print(f"{'Signal':<22} {'IC':>7} {'HitRate':>8} {'T-Stat':>7} {'Grade':>6}")
    print("-" * 55)
    from itertools import chain  # noqa (stdlib)
    for sig in SIGNAL_NAMES:
        agg_ic, agg_hr, agg_t = [], [], []
        for sym_res in all_results.values():
            hz_res = sym_res.get(PRIMARY_HORIZON, {})
            s = hz_res.get(sig, {}).get("all", {})
            if s.get("n", 0) >= 10:
                for lst, key in [(agg_ic, "ic"), (agg_hr, "hit_rate"), (agg_t, "t_stat")]:
                    v = s.get(key, float("nan"))
                    if not math.isnan(v):
                        lst.append(v)
        ic = sum(agg_ic)/len(agg_ic) if agg_ic else float("nan")
        hr = sum(agg_hr)/len(agg_hr) if agg_hr else float("nan")
        t  = sum(agg_t) /len(agg_t)  if agg_t  else float("nan")
        g  = grade(ic, hr, t, 0.15)
        ic_str = f"{ic:.4f}" if not math.isnan(ic) else "  n/a "
        hr_str = f"{hr:.3f}" if not math.isnan(hr) else " n/a "
        t_str  = f"{t:.2f}"  if not math.isnan(t)  else " n/a "
        if g in ("A", "B"):
            flag = " ✓"
        elif g in ("D", "F"):
            flag = " ✗"
        else:
            flag = "  "
        print(f"{SIGNAL_LABELS.get(sig,sig):<22} {ic_str:>7} {hr_str:>8} {t_str:>7} {g:>5}{flag}")


if __name__ == "__main__":
    main()
