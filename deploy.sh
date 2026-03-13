#!/usr/bin/env bash
# deploy.sh – push to GitHub, pull & build on VPS, restart via systemd
# Usage:
#   ./deploy.sh              – full deploy (git push + VPS pull/build/restart)
#   ./deploy.sh --vps-only   – skip git push, just pull/build/restart on VPS
#   ./deploy.sh --push-only  – push to GitHub only, don't touch VPS
#   ./deploy.sh --restart    – restart service on VPS without rebuilding
#   ./deploy.sh --test-only  – run CI quality gate on VPS without deploying
#
# CI Quality Gate (runs before every build):
#   1. cargo test --all        – all unit + integration tests must pass
#   2. cargo clippy -D warnings – no compiler warnings or lints
#   3. cargo audit             – no known CVEs in dependencies (RustSec DB)
#   Deploy is BLOCKED if any of the above fail.

set -euo pipefail

# ── Config ────────────────────────────────────────────────────────────────────
VPS_IP="${VPS_IP:-165.232.160.43}"
VPS_USER="${VPS_USER:-root}"
VPS_DIR="/RedRobot-HedgeBot"
SERVICE="hedgebot"
BRANCH="master"

# ── Colours ───────────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
CYAN='\033[0;36m'; BOLD='\033[1m'; RESET='\033[0m'

info()    { echo -e "${CYAN}▸ $*${RESET}"; }
success() { echo -e "${GREEN}✓ $*${RESET}"; }
warn()    { echo -e "${YELLOW}⚠ $*${RESET}"; }
error()   { echo -e "${RED}✗ $*${RESET}" >&2; exit 1; }
header()  { echo -e "\n${BOLD}── $* ──${RESET}"; }

# ── Argument parsing ──────────────────────────────────────────────────────────
DO_PUSH=true
DO_DEPLOY=true
DO_BUILD=true
DO_TEST=true      # CI gate runs by default on every build

for arg in "$@"; do
  case $arg in
    --vps-only)   DO_PUSH=false ;;
    --push-only)  DO_DEPLOY=false ;;
    --restart)    DO_BUILD=false; DO_TEST=false ;;
    --test-only)  DO_PUSH=false; DO_BUILD=false ;;
    --no-test)    DO_TEST=false ;;
    --help|-h)
      echo "Usage: $0 [--vps-only | --push-only | --restart | --test-only | --no-test]"
      echo "  (no flags)    full deploy: CI gate + push to GitHub + build + restart VPS"
      echo "  --vps-only    skip GitHub push, just CI gate + build & restart on VPS"
      echo "  --push-only   push to GitHub only, don't touch VPS"
      echo "  --restart     SSH restart the service without rebuilding or testing"
      echo "  --test-only   run CI quality gate on VPS only (no build/restart)"
      echo "  --no-test     skip CI gate (emergency deploys only)"
      exit 0 ;;
    *) error "Unknown argument: $arg" ;;
  esac
done

# ── 1. Git push ───────────────────────────────────────────────────────────────
if $DO_PUSH; then
  header "Git"

  # Warn about any uncommitted changes
  if ! git diff --quiet || ! git diff --cached --quiet; then
    warn "You have uncommitted changes — they will NOT be deployed."
    warn "Run 'git add . && git commit -m \"...\"' first if you want them included."
    echo ""
  fi

  LOCAL=$(git rev-parse HEAD)
  info "Pushing branch '${BRANCH}' to origin…"
  git push origin "$BRANCH" 2>&1 | sed 's/^/  /'
  success "GitHub up to date  ($(git rev-parse --short HEAD))"
fi

# ── 2. VPS deploy ─────────────────────────────────────────────────────────────
if $DO_DEPLOY; then
  SSH="ssh -o ConnectTimeout=10 -o BatchMode=yes ${VPS_USER}@${VPS_IP}"

  header "VPS  ${VPS_USER}@${VPS_IP}"

  # Connectivity check
  info "Checking SSH connectivity…"
  if ! $SSH "echo ok" &>/dev/null; then
    error "Cannot reach ${VPS_USER}@${VPS_IP}. Check SSH keys or IP."
  fi
  success "SSH OK"

  # ── 2a. Pull latest code ────────────────────────────────────────────────────
  header "Git pull on VPS"
  $SSH bash <<ENDSSH
    set -euo pipefail
    cd ${VPS_DIR}
    export PATH="\$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin:\$PATH"
    source "\$HOME/.cargo/env" 2>/dev/null || true

    echo "Current: \$(git rev-parse --short HEAD)  (\$(git log -1 --format='%s'))"
    git fetch origin
    git reset --hard origin/${BRANCH}
    echo "Updated: \$(git rev-parse --short HEAD)  (\$(git log -1 --format='%s'))"
ENDSSH
  success "Code pulled"

  # ── 2b. CI Quality Gate ─────────────────────────────────────────────────────
  # Runs on the VPS after pull, before build.
  # Blocks deploy if any check fails (matches Rust Book + RustSec best practice).
  if $DO_TEST; then
    header "CI Quality Gate"
    info "Step 1/3 — cargo test (unit + integration)…"
    info "Step 2/3 — cargo clippy -D warnings (lint gate)…"
    info "Step 3/3 — cargo audit (RustSec CVE scan)…"

    $SSH bash <<ENDSSH
      set -euo pipefail
      cd ${VPS_DIR}
      export PATH="\$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin:\$PATH"
      source "\$HOME/.cargo/env" 2>/dev/null || true

      # Ensure swap is active (needed on 1 GB RAM droplets)
      if ! swapon --show 2>/dev/null | grep -q .; then
        swapon /swapfile 2>/dev/null || true
      fi

      echo ""
      echo "── Step 1/3: Tests ─────────────────────────────────────────────"
      # Run all unit tests (src/#[cfg(test)]) + integration tests (tests/)
      # Reference: Rust Book §11 — cargo test is the standard gate
      cargo test --all 2>&1
      echo "✓ All tests passed"

      echo ""
      echo "── Step 2/3: Clippy lint gate ──────────────────────────────────"
      # -D warnings treats all warnings as errors — blocks deploy on any lint
      # Reference: https://doc.rust-lang.org/clippy/
      # For financial code: catches integer arithmetic issues, unwrap panics, etc.
      cargo clippy --all-targets -- -D warnings 2>&1
      echo "✓ Clippy clean"

      echo ""
      echo "── Step 3/3: Security audit (RustSec) ──────────────────────────"
      # Scans Cargo.lock against the RustSec advisory database
      # Reference: https://rustsec.org / cargo-audit
      # Install if missing (silent if already present)
      cargo install cargo-audit --quiet 2>/dev/null || true
      cargo audit 2>&1
      echo "✓ No known CVEs in dependencies"

      echo ""
      echo "══════════════════════════════════════════════════════════════"
      echo "CI GATE PASSED — safe to build and deploy"
      echo "══════════════════════════════════════════════════════════════"
ENDSSH
    success "CI quality gate passed (tests ✓  clippy ✓  audit ✓)"
  fi

  # ── 2c. Build ───────────────────────────────────────────────────────────────
  if $DO_BUILD; then
    header "cargo build --release"
    info "This takes ~2 min on the VPS…"

    $SSH bash <<ENDSSH
      set -euo pipefail
      cd ${VPS_DIR}
      export PATH="\$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin:\$PATH"
      source "\$HOME/.cargo/env" 2>/dev/null || true

      # Ensure swap is active (needed on 1 GB RAM droplets)
      if ! swapon --show 2>/dev/null | grep -q .; then
        swapon /swapfile 2>/dev/null || true
      fi

      echo "rustc: \$(rustc --version)"
      echo "cargo: \$(cargo --version)"
      echo ""

      cargo build --release 2>&1
      echo ""
      echo "Binary: \$(ls -lh target/release/redrobot-hedgebot | awk '{print \$5, \$9}')"
ENDSSH
    success "Build complete"
  fi

  # ── 2c. Restart service ─────────────────────────────────────────────────────
  header "Restart ${SERVICE}"

  $SSH bash <<ENDSSH
    set -euo pipefail

    if systemctl is-enabled ${SERVICE} &>/dev/null; then
      systemctl restart ${SERVICE}
      sleep 2
      STATUS=\$(systemctl is-active ${SERVICE})
      echo "Service status: \${STATUS}"
      if [ "\${STATUS}" != "active" ]; then
        echo ""
        echo "=== Journal (last 20 lines) ==="
        journalctl -u ${SERVICE} -n 20 --no-pager
        exit 1
      fi
    else
      echo "systemd service '${SERVICE}' not found — falling back to pkill+nohup"
      pkill -f redrobot-hedgebot 2>/dev/null || true
      sleep 2
      cd ${VPS_DIR}
      set -a
      [ -f /etc/environment ] && source /etc/environment 2>/dev/null || true
      set +a
      nohup env \
        ANTHROPIC_API_KEY="\${ANTHROPIC_API_KEY}" \
        LUNARCRUSH_API_KEY="\${LUNARCRUSH_API_KEY}" \
        PAPER_TRADING="\${PAPER_TRADING:-true}" \
        ./target/release/redrobot-hedgebot >> /var/log/hedgebot.log 2>&1 &
      echo "Bot PID: \$!"
    fi
ENDSSH
  success "Service restarted"

  # ── 2d. Tail logs ───────────────────────────────────────────────────────────
  header "Last 10 log lines"
  $SSH bash <<ENDSSH
    if systemctl is-enabled ${SERVICE} &>/dev/null; then
      journalctl -u ${SERVICE} -n 10 --no-pager
    else
      tail -10 /var/log/hedgebot.log 2>/dev/null || echo "(no log file yet)"
    fi
ENDSSH

fi

# ── Done ──────────────────────────────────────────────────────────────────────
echo ""
success "Deploy complete  🚀  Dashboard → http://${VPS_IP}:3000"
