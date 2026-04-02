#!/usr/bin/env python3
import json
import os
import pathlib
import sys
import urllib.error
import urllib.request
from datetime import datetime
from typing import Dict, Optional

ROOT = pathlib.Path(__file__).resolve().parent
REPORTS = ROOT / ".." / "reports"
CACHE_FILE = REPORTS / "pattern_cache.json"
LAST_MARKER = REPORTS / ".pattern_cache_last"
ALERT_JSON = REPORTS / "pattern_cache_alert.json"
LOG_FILE = REPORTS / "pattern_cache_alert.log"
HL_STATS_FILE = REPORTS / "hyperliquid_stats.json"
HL_LAST = REPORTS / ".hyperliquid_last"
HL_LOG_FILE = REPORTS / "hyperliquid_alert.log"


def load_cache():
    if not CACHE_FILE.exists():
        print(f"⚠️ Missing cache file: {CACHE_FILE}", file=sys.stderr)
        sys.exit(1)
    return json.loads(CACHE_FILE.read_text())


def parse_timestamp(value: str) -> datetime:
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def load_stats() -> Optional[Dict]:
    if not HL_STATS_FILE.exists():
        return None
    try:
        return json.loads(HL_STATS_FILE.read_text())
    except json.JSONDecodeError:
        return None


def load_last_hits() -> int:
    if not HL_LAST.exists():
        return 0
    try:
        data = json.loads(HL_LAST.read_text())
        return data.get("rate_limit_hits", 0)
    except json.JSONDecodeError:
        return 0


def save_last_hits(count: int):
    try:
        HL_LAST.write_text(json.dumps({"rate_limit_hits": count}))
    except OSError:
        pass


def append_hyperliquid_alert_log(entry: Dict):
    try:
        HL_LOG_FILE.parent.mkdir(parents=True, exist_ok=True)
        log_entry = {"logged_at": datetime.utcnow().isoformat() + "Z", **entry}
        with HL_LOG_FILE.open("a") as fh:
            fh.write(json.dumps(log_entry) + "\n")
    except OSError as exc:
        print(f"⚠️ Failed to append Hyperliquid log: {exc}", file=sys.stderr)


def dispatch_hyperliquid_alert(alert: Dict):
    url = os.getenv("HYPERLIQUID_ALERT_WEBHOOK_URL")
    if not url:
        return
    method = os.getenv("HYPERLIQUID_ALERT_WEBHOOK_METHOD", "POST").upper()
    headers = {"Content-Type": "application/json"}
    token = os.getenv("HYPERLIQUID_ALERT_WEBHOOK_TOKEN")
    if token:
        headers["Authorization"] = token if token.lower().startswith(("bearer ", "token ")) else f"Bearer {token}"
    extra_headers = os.getenv("HYPERLIQUID_ALERT_WEBHOOK_HEADERS")
    if extra_headers:
        try:
            parsed = json.loads(extra_headers)
            if isinstance(parsed, dict):
                for key, value in parsed.items():
                    headers[key] = str(value)
        except json.JSONDecodeError as exc:
            print(f"⚠️ Invalid webhook headers JSON: {exc}", file=sys.stderr)
    data = json.dumps(alert).encode("utf-8")
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            print(f"→ {method} Hyperliquid webhook {url} ({resp.status})")
    except urllib.error.URLError as exc:
        print(f"⚠️ Hyperliquid webhook dispatch failed: {exc}", file=sys.stderr)


def alert_if_changed(cache: dict):
    # Read-only access to the JSON cache dict — not an HTTP GET route
    cache_ts: Optional[str] = cache.get("updated_at")
    if not cache_ts:
        print("⚠️ `updated_at` missing in pattern_cache.json", file=sys.stderr)
        sys.exit(1)
    last = LAST_MARKER.read_text().strip() if LAST_MARKER.exists() else ""
    insights = cache.get("insights") or {}
    date = insights.get("date", "<unknown>")
    winner = insights.get("report_summary", {}).get("daily_winner")
    loser = insights.get("report_summary", {}).get("daily_loser")
    combos = insights.get("top_win_combos", [])
    top_combo = combos[0] if combos else {}
    combo_breakdown = top_combo.get("breakdown", "n/a")
    combo_context = top_combo.get("context", "n/a")
    combo_rate = top_combo.get("win_rate", 0.0) * 100.0
    if cache_ts == last:
        print(f"Pattern cache unchanged ({cache_ts}).")
    else:
        print(f"✔️ Pattern cache refreshed at {cache_ts} for {date}.")
        if winner:
            print(f"   Daily winner: {winner[0]} (${winner[1]:.2f})")
        if loser:
            print(f"   Daily loser: {loser[0]} (${loser[1]:.2f})")
        print(
            f"   Top win combo: {combo_breakdown} @ {combo_context} ({combo_rate:.0f}% win rate)"
        )
    LAST_MARKER.write_text(cache_ts)
    payload = {
        "updated_at": cache_ts,
        "date": date,
        "winner": winner,
        "loser": loser,
        "top_combo": {
            "breakdown": combo_breakdown,
            "context": combo_context,
            "win_rate": combo_rate,
        },
        "hyperliquid_stats": None,
        "hyperliquid_alert": None,
    }
    stats = load_stats()
    if stats:
        payload["hyperliquid_stats"] = stats
        current_hits = stats.get("rate_limit_hits", 0)
        last_hits = load_last_hits()
        delta = current_hits - last_hits
        save_last_hits(current_hits)
        if delta > 0:
            hl_alert = {
                "delta": delta,
                "rate_limit_hits": current_hits,
                "last_rate_limit_at": stats.get("last_rate_limit_at"),
            }
            payload["hyperliquid_alert"] = hl_alert
            append_hyperliquid_alert_log(hl_alert)
            dispatch_hyperliquid_alert(hl_alert)
    else:
        save_last_hits(load_last_hits())
    ALERT_JSON.write_text(json.dumps(payload, indent=2))
    append_alert_log(payload)
    dispatch_webhook(payload)


def main():
    REPORTS.mkdir(parents=True, exist_ok=True)
    cache = load_cache()
    alert_if_changed(cache)


def append_alert_log(payload: dict):
    try:
        LOG_FILE.parent.mkdir(parents=True, exist_ok=True)
        entry = {
            "logged_at": datetime.utcnow().isoformat() + "Z",
            **payload,
        }
        with LOG_FILE.open("a") as fh:
            fh.write(json.dumps(entry) + "\n")
    except OSError as exc:
        print(f"⚠️ Failed to append alert log: {exc}", file=sys.stderr)


if __name__ == "__main__":
    main()


def dispatch_webhook(payload: dict):
    url = os.getenv("PATTERN_CACHE_WEBHOOK_URL")
    if not url:
        return
    method = os.getenv("PATTERN_CACHE_WEBHOOK_METHOD", "POST").upper()
    headers = {"Content-Type": "application/json"}
    token = os.getenv("PATTERN_CACHE_WEBHOOK_TOKEN")
    if token:
        headers["Authorization"] = token if token.lower().startswith(("bearer ", "token ")) else f"Bearer {token}"
    extra_headers = os.getenv("PATTERN_CACHE_WEBHOOK_HEADERS")
    if extra_headers:
        try:
            parsed = json.loads(extra_headers)
            if isinstance(parsed, dict):
                for key, value in parsed.items():
                    headers[key] = str(value)
        except json.JSONDecodeError as exc:
            print(f"⚠️ Invalid webhook headers JSON: {exc}", file=sys.stderr)
    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            print(f"→ {method} webhook {url} ({resp.status})")
    except urllib.error.URLError as exc:
        print(f"⚠️ Webhook dispatch failed: {exc}", file=sys.stderr)
