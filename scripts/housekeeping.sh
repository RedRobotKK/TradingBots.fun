#!/usr/bin/env bash
# housekeeping.sh — daily VPS disk hygiene for tradingbots.
#
# Installed as a cron job: runs at 03:00 UTC every day.
# Safe to run manually at any time.
#
# What it does:
#   1. Compresses trading .jsonl log files older than 72 hours.
#      (72h gap ensures the service has fully closed and rotated the file.)
#   2. Deletes compressed log files older than 30 days.
#   3. Reports disk usage so the log provides a trend over time.
#
# Logs to: /tradingbots-fun/housekeeping.log (appended, kept small by this script)

set -uo pipefail

LOGS_DIR="/tradingbots-fun/logs"
HOUSEKEEPING_LOG="/tradingbots-fun/housekeeping.log"
COMPRESS_AFTER_HOURS=72
RETAIN_COMPRESSED_DAYS=30
MAX_HOUSEKEEPING_LOG_LINES=500

log() {
    echo "[$(date -u '+%Y-%m-%dT%H:%M:%SZ')] $*" | tee -a "$HOUSEKEEPING_LOG"
}

# Trim the housekeeping log itself so it doesn't grow unbounded.
trim_own_log() {
    if [ -f "$HOUSEKEEPING_LOG" ]; then
        local lines
        lines=$(wc -l < "$HOUSEKEEPING_LOG")
        if [ "$lines" -gt "$MAX_HOUSEKEEPING_LOG_LINES" ]; then
            tail -n "$MAX_HOUSEKEEPING_LOG_LINES" "$HOUSEKEEPING_LOG" > "${HOUSEKEEPING_LOG}.tmp"
            mv "${HOUSEKEEPING_LOG}.tmp" "$HOUSEKEEPING_LOG"
        fi
    fi
}

log "=== Housekeeping start ==="
log "Disk before: $(df -h /dev/vda1 | awk 'NR==2{print $3"/"$2" ("$5" used)"}')"

# ── Step 1: Compress uncompressed .jsonl files older than 72 hours ────────────
# find: -mmin +N means modified more than N minutes ago
COMPRESS_MINUTES=$(( COMPRESS_AFTER_HOURS * 60 ))
compressed_count=0
while IFS= read -r -d '' f; do
    gzip "$f" && log "Compressed: $(basename "$f")" && (( compressed_count++ )) || log "WARNING: failed to compress $f"
done < <(find "$LOGS_DIR" -maxdepth 1 -name "*.jsonl" -not -name "*.gz" -mmin "+${COMPRESS_MINUTES}" -print0 2>/dev/null)

if [ "$compressed_count" -eq 0 ]; then
    log "Compress: nothing to compress (no .jsonl files older than ${COMPRESS_AFTER_HOURS}h)"
else
    log "Compress: ${compressed_count} file(s) compressed"
fi

# ── Step 2: Delete compressed logs older than 30 days ─────────────────────────
deleted_count=0
while IFS= read -r -d '' f; do
    rm -f "$f" && log "Deleted old log: $(basename "$f")" && (( deleted_count++ )) || log "WARNING: failed to delete $f"
done < <(find "$LOGS_DIR" -maxdepth 1 \( -name "*.jsonl.gz" -o -name "*.jsonl.1.gz" \) -mtime "+${RETAIN_COMPRESSED_DAYS}" -print0 2>/dev/null)

if [ "$deleted_count" -eq 0 ]; then
    log "Prune: nothing to delete (no compressed logs older than ${RETAIN_COMPRESSED_DAYS}d)"
else
    log "Prune: ${deleted_count} file(s) deleted"
fi

# ── Step 3: Report final disk state ───────────────────────────────────────────
DISK_PCT=$(df /tradingbots-fun --output=pcent 2>/dev/null | tail -1 | tr -d '% ' || echo "0")
log "Disk after:  $(df -h /dev/vda1 | awk 'NR==2{print $3"/"$2" ("$5" used)"}')"

# Warn if still above 80% after cleanup — could mean something else is growing.
if [ "${DISK_PCT:-0}" -gt 80 ]; then
    log "WARNING: disk still at ${DISK_PCT}% after housekeeping — investigate /tradingbots-fun"
fi

log "=== Housekeeping complete ==="
trim_own_log
