#!/usr/bin/env bash
# vps_ci_gate.sh — runs on the VPS via SSH, outputs structured text the GHA runner parses.
# Each section is delimited by === markers so parse_ci.py can extract them unambiguously.
set -uo pipefail

cd /tradingbots-fun
export PATH="$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin:$PATH"
source "$HOME/.cargo/env" 2>/dev/null || true
swapon /swapfile 2>/dev/null || true

# ── Pull latest code so CI tests what was just pushed ─────────────────────────
git fetch origin master 2>&1
git reset --hard origin/master 2>&1
echo "VPS code now at: $(git rev-parse --short HEAD)"

# ── Metadata ──────────────────────────────────────────────────────────────────
COMMIT=$(git rev-parse --short HEAD)
COMMIT_FULL=$(git rev-parse HEAD)
COMMIT_MSG=$(git log -1 --format='%s')
BRANCH=$(git rev-parse --abbrev-ref HEAD)
RUN_AT=$(date -u '+%Y-%m-%dT%H:%M:%SZ')
RUSTC_VER=$(rustc --version 2>/dev/null || echo "unknown")
CARGO_VER=$(cargo --version 2>/dev/null || echo "unknown")
OS_INFO=$(uname -r)
ARCH=$(uname -m)
RAM=$(free -h | awk '/^Mem:/{print $2}')
SWAP_ACTIVE=$(swapon --show --noheadings 2>/dev/null | grep -q . && echo "true" || echo "false")

echo "=== META ==="
echo "commit=${COMMIT}"
echo "commit_full=${COMMIT_FULL}"
echo "commit_msg=${COMMIT_MSG}"
echo "branch=${BRANCH}"
echo "run_at=${RUN_AT}"
echo "rustc=${RUSTC_VER}"
echo "cargo_ver=${CARGO_VER}"
echo "os=${OS_INFO}"
echo "arch=${ARCH}"
echo "ram=${RAM}"
echo "swap_active=${SWAP_ACTIVE}"
echo "=== META END ==="

# ── Step 1: Tests ─────────────────────────────────────────────────────────────
echo ""
echo "=== STEP tests ==="
T_START=$(date +%s)
CARGO_BUILD_JOBS=1 cargo test --all -- --test-threads=1 2>&1
TEST_EXIT=$?
T_END=$(date +%s)
echo "=== STEP tests exit=${TEST_EXIT} duration=$((T_END - T_START))s ==="

# ── Step 2: Clippy ────────────────────────────────────────────────────────────
echo ""
echo "=== STEP clippy ==="
C_START=$(date +%s)
cargo clippy --all-targets -- -D warnings 2>&1
CLIPPY_EXIT=$?
C_END=$(date +%s)
echo "=== STEP clippy exit=${CLIPPY_EXIT} duration=$((C_END - C_START))s ==="

# ── Step 3: Audit ─────────────────────────────────────────────────────────────
echo ""
echo "=== STEP audit ==="
A_START=$(date +%s)
if ! command -v cargo-audit &>/dev/null; then
  echo "Installing cargo-audit..."
  cargo install cargo-audit 2>&1 | tail -5
fi
cargo audit 2>&1
AUDIT_EXIT=$?
A_END=$(date +%s)
echo "=== STEP audit exit=${AUDIT_EXIT} duration=$((A_END - A_START))s ==="

# ── Service health ─────────────────────────────────────────────────────────────
echo ""
echo "=== STEP service ==="
systemctl is-active tradingbots 2>/dev/null || echo "unknown"
systemctl show tradingbots --property=ActiveEnterTimestamp --value 2>/dev/null || echo ""
journalctl -u tradingbots -n 5 --no-pager --output=short 2>/dev/null || echo "(no journal)"
echo "=== STEP service ==="

echo ""
echo "=== DONE ==="
