#!/usr/bin/env bash
# deploy.sh – push to GitHub, pull & build on VPS, restart via systemd
# Usage:
#   ./deploy.sh              – full deploy (CI gate + push to GitHub + build + restart)
#   ./deploy.sh --vps-only   – skip git push, just pull/build/restart on VPS
#   ./deploy.sh --push-only  – push to GitHub only, don't touch VPS
#   ./deploy.sh --restart    – restart service on VPS without rebuilding
#   ./deploy.sh --test-only  – run CI quality gate on VPS without deploying
#   ./deploy.sh --no-test    – skip CI gate (emergency deploys only)
#   ./deploy.sh --no-log-push – skip uploading CI log to GitHub
#
# CI Quality Gate (runs before every build):
#   1. cargo test --all        – all unit + integration tests must pass
#   2. cargo clippy -D warnings – no compiler warnings or lints
#   3. cargo audit             – no known CVEs in dependencies (RustSec DB)
#   Deploy is BLOCKED if any of the above fail.
#
# Logs:
#   Every CI run appends a full timestamped report to:
#     /var/log/hedgebot-ci.log      (on VPS — CI gate output, persistent)
#     /var/log/hedgebot-deploy.log  (on VPS — build + restart output)
#   After each run the CI log for that run is also:
#     SCP'd back locally → committed to logs/ci/ → pushed to GitHub
#     Browse at: https://github.com/<org>/<repo>/tree/master/logs/ci/
#   On failure, the last 60 lines of the CI log are printed automatically.

set -euo pipefail

# ── Config ────────────────────────────────────────────────────────────────────
VPS_IP="${VPS_IP:-165.232.160.43}"
VPS_USER="${VPS_USER:-root}"
VPS_DIR="/RedRobot-HedgeBot"
SERVICE="hedgebot"
BRANCH="master"
CI_LOG="/var/log/hedgebot-ci.log"
DEPLOY_LOG="/var/log/hedgebot-deploy.log"

# ── Colours ───────────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
CYAN='\033[0;36m'; BOLD='\033[1m'; RESET='\033[0m'
DIM='\033[2m'

info()    { echo -e "${CYAN}▸ $*${RESET}"; }
success() { echo -e "${GREEN}✓ $*${RESET}"; }
warn()    { echo -e "${YELLOW}⚠ $*${RESET}"; }
error()   { echo -e "${RED}✗ $*${RESET}" >&2; }
header()  { echo -e "\n${BOLD}── $* ──${RESET}"; }
dim()     { echo -e "${DIM}$*${RESET}"; }

# ── Argument parsing ──────────────────────────────────────────────────────────
DO_PUSH=true
DO_DEPLOY=true
DO_BUILD=true
DO_TEST=true
DO_LOG_PUSH=true   # upload CI log to GitHub after each run

for arg in "$@"; do
  case $arg in
    --vps-only)    DO_PUSH=false ;;
    --push-only)   DO_DEPLOY=false ;;
    --restart)     DO_BUILD=false; DO_TEST=false ;;
    --test-only)   DO_PUSH=false; DO_BUILD=false ;;
    --no-test)     DO_TEST=false; warn "--no-test: skipping CI gate (emergency mode)" ;;
    --no-log-push) DO_LOG_PUSH=false ;;
    --help|-h)
      echo "Usage: $0 [--vps-only | --push-only | --restart | --test-only | --no-test | --no-log-push]"
      echo "  (no flags)     full deploy: CI gate + push to GitHub + build + restart VPS"
      echo "  --vps-only     skip GitHub push, just CI gate + build & restart on VPS"
      echo "  --push-only    push to GitHub only, don't touch VPS"
      echo "  --restart      SSH restart the service without rebuilding or testing"
      echo "  --test-only    run CI quality gate on VPS only (no build/restart)"
      echo "  --no-test      skip CI gate (emergency deploys only)"
      echo "  --no-log-push  skip uploading CI log to GitHub"
      exit 0 ;;
    *) error "Unknown argument: $arg"; exit 1 ;;
  esac
done

# ── 1. Git push ───────────────────────────────────────────────────────────────
if $DO_PUSH; then
  header "Git"

  if ! git diff --quiet || ! git diff --cached --quiet; then
    warn "You have uncommitted changes — they will NOT be deployed."
    warn "Run 'git add . && git commit -m \"...\"' first if you want them included."
    echo ""
  fi

  info "Pushing branch '${BRANCH}' to origin…"
  git push origin "$BRANCH" 2>&1 | sed 's/^/  /'
  success "GitHub up to date  ($(git rev-parse --short HEAD))"
fi

# ── 2. VPS deploy ─────────────────────────────────────────────────────────────
if $DO_DEPLOY; then
  SSH="ssh -o ConnectTimeout=10 -o BatchMode=yes ${VPS_USER}@${VPS_IP}"

  header "VPS  ${VPS_USER}@${VPS_IP}"

  info "Checking SSH connectivity…"
  if ! $SSH "echo ok" &>/dev/null; then
    error "Cannot reach ${VPS_USER}@${VPS_IP}. Check SSH keys or IP."
    exit 1
  fi
  success "SSH OK"

  # ── 2a. Pull latest code ──────────────────────────────────────────────────
  header "Git pull on VPS"
  $SSH bash <<ENDSSH
    set -euo pipefail
    cd ${VPS_DIR}
    export PATH="\$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin:\$PATH"
    source "\$HOME/.cargo/env" 2>/dev/null || true

    echo "Before: \$(git rev-parse --short HEAD)  (\$(git log -1 --format='%s'))"
    git fetch origin 2>&1
    git reset --hard origin/${BRANCH} 2>&1
    echo "After:  \$(git rev-parse --short HEAD)  (\$(git log -1 --format='%s'))"
    echo ""
    echo "Files changed vs previous:"
    git diff --stat HEAD@{1} HEAD 2>/dev/null || echo "  (first pull — no previous ref)"
ENDSSH
  success "Code pulled"

  # ── 2b. CI Quality Gate ───────────────────────────────────────────────────
  # Each step is run independently with full error capture.
  # On any failure: full diagnostics are printed AND appended to CI_LOG.
  # The log persists on the VPS at /var/log/hedgebot-ci.log for post-mortem.
  if $DO_TEST; then
    header "CI Quality Gate"
    dim "  Full output → ${CI_LOG} on VPS"
    echo ""

    $SSH bash <<'ENDSSH'
      set -uo pipefail  # Note: NOT -e here — we capture each step's exit code manually

      VPS_DIR="/RedRobot-HedgeBot"
      CI_LOG="/var/log/hedgebot-ci.log"
      export PATH="$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin:$PATH"
      source "$HOME/.cargo/env" 2>/dev/null || true
      cd "$VPS_DIR"

      # Ensure swap active on low-RAM VPS
      if ! swapon --show 2>/dev/null | grep -q .; then
        swapon /swapfile 2>/dev/null || true
      fi

      COMMIT=$(git rev-parse --short HEAD)
      RUN_AT=$(date '+%Y-%m-%d %H:%M:%S %Z')
      PASS=0; FAIL=0
      STEP1="⏳"; STEP2="⏳"; STEP3="⏳"

      # ── Log header ─────────────────────────────────────────────────────────
      {
        echo "════════════════════════════════════════════════════════════════"
        echo "CI RUN  ${RUN_AT}  commit=${COMMIT}"
        echo "════════════════════════════════════════════════════════════════"
      } | tee -a "$CI_LOG"

      # ── Step 1: Tests ───────────────────────────────────────────────────────
      {
        echo ""
        echo "── Step 1/3: cargo test --all ──────────────────────────────────"
        echo "   Rust Book §11: unit (#[cfg(test)]) + integration (tests/) gate"
        echo "   Started: $(date '+%H:%M:%S')"
      } | tee -a "$CI_LOG"

      TEST_OUT=$(cargo test --all 2>&1)
      TEST_EXIT=$?
      echo "$TEST_OUT" | tee -a "$CI_LOG"

      if [ $TEST_EXIT -eq 0 ]; then
        # Extract test count from output line like "test result: ok. 42 passed; 0 failed"
        PASSED=$(echo "$TEST_OUT" | grep -oP '\d+ passed' | awk '{s+=$1} END{print s}')
        FAILED=$(echo "$TEST_OUT" | grep -oP '\d+ failed' | awk '{s+=$1} END{print s}')
        PASSED=${PASSED:-0}; FAILED=${FAILED:-0}
        echo "✓ PASS  ${PASSED} passed, ${FAILED} failed  ($(date '+%H:%M:%S'))" | tee -a "$CI_LOG"
        STEP1="✅ PASS (${PASSED} tests)"
        PASS=$((PASS+1))
      else
        {
          echo ""
          echo "✗ FAIL  cargo test exited with code ${TEST_EXIT}"
          echo ""
          echo "── FAILING TESTS ──────────────────────────────────────────────"
          # Show only the FAILED lines and their immediate context
          echo "$TEST_OUT" | grep -A 20 "^failures:" || true
          echo ""
          echo "── FULL TEST OUTPUT (last 80 lines) ───────────────────────────"
          echo "$TEST_OUT" | tail -80
        } | tee -a "$CI_LOG"
        STEP1="❌ FAIL"
        FAIL=$((FAIL+1))
      fi

      # ── Step 2: Clippy ──────────────────────────────────────────────────────
      {
        echo ""
        echo "── Step 2/3: cargo clippy --all-targets -D warnings ────────────"
        echo "   Ref: https://doc.rust-lang.org/clippy/"
        echo "   Started: $(date '+%H:%M:%S')"
      } | tee -a "$CI_LOG"

      CLIPPY_OUT=$(cargo clippy --all-targets -- -D warnings 2>&1)
      CLIPPY_EXIT=$?
      echo "$CLIPPY_OUT" | tee -a "$CI_LOG"

      if [ $CLIPPY_EXIT -eq 0 ]; then
        WARN_COUNT=$(echo "$CLIPPY_OUT" | grep -c "^warning:" || true)
        echo "✓ PASS  0 errors, ${WARN_COUNT} suppressed warnings  ($(date '+%H:%M:%S'))" | tee -a "$CI_LOG"
        STEP2="✅ PASS"
        PASS=$((PASS+1))
      else
        {
          echo ""
          echo "✗ FAIL  clippy exited with code ${CLIPPY_EXIT}"
          echo ""
          echo "── LINT ERRORS ────────────────────────────────────────────────"
          echo "$CLIPPY_OUT" | grep -E "^error|^ --> |^  \|" | head -60 || true
          echo ""
          echo "── FILES WITH ERRORS ──────────────────────────────────────────"
          echo "$CLIPPY_OUT" | grep -oP '(?<= --> ).*(?=:\d+:\d+)' | sort -u || true
        } | tee -a "$CI_LOG"
        STEP2="❌ FAIL"
        FAIL=$((FAIL+1))
      fi

      # ── Step 3: cargo audit ─────────────────────────────────────────────────
      {
        echo ""
        echo "── Step 3/3: cargo audit (RustSec CVE scan) ────────────────────"
        echo "   Ref: https://rustsec.org — scans Cargo.lock"
        echo "   Started: $(date '+%H:%M:%S')"
      } | tee -a "$CI_LOG"

      # Install cargo-audit if not present (idempotent, quiet)
      if ! command -v cargo-audit &>/dev/null; then
        echo "  Installing cargo-audit…" | tee -a "$CI_LOG"
        cargo install cargo-audit 2>&1 | tail -3 | tee -a "$CI_LOG"
      fi

      AUDIT_OUT=$(cargo audit 2>&1)
      AUDIT_EXIT=$?
      echo "$AUDIT_OUT" | tee -a "$CI_LOG"

      if [ $AUDIT_EXIT -eq 0 ]; then
        VULN_COUNT=$(echo "$AUDIT_OUT" | grep -c "Vulnerability found" || true)
        echo "✓ PASS  0 vulnerabilities  ($(date '+%H:%M:%S'))" | tee -a "$CI_LOG"
        STEP3="✅ PASS"
        PASS=$((PASS+1))
      else
        {
          echo ""
          echo "✗ FAIL  cargo audit found vulnerabilities (exit ${AUDIT_EXIT})"
          echo ""
          echo "── VULNERABILITIES FOUND ──────────────────────────────────────"
          echo "$AUDIT_OUT" | grep -A 8 "Vulnerability\|Advisory\|error\[" || true
        } | tee -a "$CI_LOG"
        STEP3="⚠️  ADVISORY"
        FAIL=$((FAIL+1))
      fi

      # ── Summary ─────────────────────────────────────────────────────────────
      {
        echo ""
        echo "════════════════════════════════════════════════════════════════"
        echo "CI SUMMARY  commit=${COMMIT}  ${RUN_AT}"
        echo "────────────────────────────────────────────────────────────────"
        echo "  cargo test   │ ${STEP1}"
        echo "  cargo clippy │ ${STEP2}"
        echo "  cargo audit  │ ${STEP3}"
        echo "────────────────────────────────────────────────────────────────"
        echo "  Passed: ${PASS}/3   Failed: ${FAIL}/3"
        echo "════════════════════════════════════════════════════════════════"
      } | tee -a "$CI_LOG"

      # Exit non-zero if any step failed — this blocks the deploy on the host
      if [ $FAIL -gt 0 ]; then
        echo ""
        echo "CI GATE FAILED — deploy blocked. Fix the issues above and retry."
        echo "Full log: ${CI_LOG}"
        exit 1
      else
        echo ""
        echo "CI GATE PASSED — safe to build and deploy."
      fi
ENDSSH

    CI_EXIT=$?
    if [ $CI_EXIT -ne 0 ]; then
      echo ""
      error "CI gate failed — fetching last 80 lines of ${CI_LOG} from VPS…"
      echo ""
      $SSH "tail -80 ${CI_LOG} 2>/dev/null || echo '(no CI log found)'"
      echo ""
      error "Deploy aborted. Fix the failures above, then re-run ./deploy.sh"
      exit 1
    fi
    success "CI quality gate passed (tests ✓  clippy ✓  audit ✓)"
  fi

  # ── 2c. Build ─────────────────────────────────────────────────────────────
  if $DO_BUILD; then
    header "cargo build --release"
    info "This takes ~2 min on the VPS…"
    dim "  Output also appended to ${DEPLOY_LOG}"
    echo ""

    $SSH bash <<ENDSSH
      set -euo pipefail
      cd ${VPS_DIR}
      export PATH="\$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin:\$PATH"
      source "\$HOME/.cargo/env" 2>/dev/null || true

      if ! swapon --show 2>/dev/null | grep -q .; then
        swapon /swapfile 2>/dev/null || true
      fi

      {
        echo ""
        echo "── Build $(date '+%Y-%m-%d %H:%M:%S')  commit=\$(git rev-parse --short HEAD) ──"
        echo "   rustc: \$(rustc --version)"
        echo "   cargo: \$(cargo --version)"
        echo "   host:  \$(uname -m)  \$(free -h | awk '/^Mem:/{print \$2}') RAM  swap=\$(swapon --show --noheadings | awk '{print \$3}' | head -1)"
        echo ""
      } | tee -a ${DEPLOY_LOG}

      cargo build --release 2>&1 | tee -a ${DEPLOY_LOG}
      BUILD_EXIT=\${PIPESTATUS[0]}

      if [ \$BUILD_EXIT -ne 0 ]; then
        {
          echo ""
          echo "✗ BUILD FAILED (exit \${BUILD_EXIT})"
          echo "── Last 30 lines of build output ──────────────────────────────"
          tail -30 ${DEPLOY_LOG}
        }
        exit 1
      fi

      echo "" | tee -a ${DEPLOY_LOG}
      BIN_INFO=\$(ls -lh target/release/redrobot-hedgebot | awk '{print \$5, \$9}')
      echo "✓ Binary: \${BIN_INFO}" | tee -a ${DEPLOY_LOG}
ENDSSH
    success "Build complete"
  fi

  # ── 2c-2. Sync systemd unit file if present in repo ──────────────────────
  UNIT_FILE="${VPS_DIR}/hedgebot.service"
  if $SSH test -f "${UNIT_FILE}" 2>/dev/null; then
    $SSH bash <<ENDSSH
      set -euo pipefail
      DEST=/etc/systemd/system/${SERVICE}.service
      if ! diff -q "${UNIT_FILE}" "\${DEST}" &>/dev/null; then
        echo "▸ Updating systemd unit file…"
        cp "${UNIT_FILE}" "\${DEST}"
        systemctl daemon-reload
        echo "✓ Unit file updated & daemon reloaded"
      else
        echo "✓ Unit file unchanged"
      fi
ENDSSH
  fi

  # ── 2d. Restart service ───────────────────────────────────────────────────
  header "Restart ${SERVICE}"

  $SSH bash <<ENDSSH
    set -euo pipefail
    cd ${VPS_DIR}

    {
      echo ""
      echo "── Restart \$(date '+%Y-%m-%d %H:%M:%S')  commit=\$(git rev-parse --short HEAD) ──"
    } | tee -a ${DEPLOY_LOG}

    if systemctl is-enabled ${SERVICE} &>/dev/null; then
      systemctl restart ${SERVICE}
      sleep 3

      STATUS=\$(systemctl is-active ${SERVICE})
      UPTIME=\$(systemctl show ${SERVICE} --property=ActiveEnterTimestamp --value 2>/dev/null || echo "unknown")

      echo "Service status : \${STATUS}" | tee -a ${DEPLOY_LOG}
      echo "Active since   : \${UPTIME}"  | tee -a ${DEPLOY_LOG}

      if [ "\${STATUS}" != "active" ]; then
        {
          echo ""
          echo "✗ SERVICE FAILED TO START"
          echo ""
          echo "── systemctl status ────────────────────────────────────────────"
          systemctl status ${SERVICE} --no-pager -l
          echo ""
          echo "── Journal (last 60 lines) ─────────────────────────────────────"
          journalctl -u ${SERVICE} -n 60 --no-pager
          echo ""
          echo "── Binary info ─────────────────────────────────────────────────"
          ls -lh ${VPS_DIR}/target/release/redrobot-hedgebot 2>/dev/null || echo "Binary not found"
          echo ""
          echo "── Environment (sensitive values redacted) ─────────────────────"
          systemctl show ${SERVICE} --property=Environment --value 2>/dev/null \
            | tr ' ' '\n' | sed 's/=.*/=***/' | head -20
        } | tee -a ${DEPLOY_LOG}
        exit 1
      fi
    else
      echo "systemd service '${SERVICE}' not found — falling back to pkill+nohup" | tee -a ${DEPLOY_LOG}
      pkill -f redrobot-hedgebot 2>/dev/null || true
      sleep 2

      set -a
      [ -f /etc/environment ] && source /etc/environment 2>/dev/null || true
      set +a

      nohup env \
        ANTHROPIC_API_KEY="\${ANTHROPIC_API_KEY}" \
        LUNARCRUSH_API_KEY="\${LUNARCRUSH_API_KEY}" \
        PAPER_TRADING="\${PAPER_TRADING:-true}" \
        ${VPS_DIR}/target/release/redrobot-hedgebot >> ${DEPLOY_LOG} 2>&1 &
      BOT_PID=\$!
      echo "Bot PID: \${BOT_PID}" | tee -a ${DEPLOY_LOG}
      sleep 3
      if ! kill -0 \${BOT_PID} 2>/dev/null; then
        echo "✗ Process \${BOT_PID} exited immediately — check ${DEPLOY_LOG}" | tee -a ${DEPLOY_LOG}
        tail -30 ${DEPLOY_LOG}
        exit 1
      fi
      echo "✓ Bot running (PID \${BOT_PID})" | tee -a ${DEPLOY_LOG}
    fi
ENDSSH
  success "Service restarted"

  # ── 2e. Post-deploy log tail ───────────────────────────────────────────────
  header "Post-deploy health check"
  $SSH bash <<ENDSSH
    echo "── Service status ──────────────────────────────────────────────────"
    if systemctl is-enabled ${SERVICE} &>/dev/null; then
      systemctl status ${SERVICE} --no-pager -l | head -20
      echo ""
      echo "── Journal (last 30 lines) ─────────────────────────────────────────"
      journalctl -u ${SERVICE} -n 30 --no-pager
    else
      echo "── Deploy log (last 30 lines) ──────────────────────────────────────"
      tail -30 ${DEPLOY_LOG} 2>/dev/null || echo "(no deploy log yet)"
    fi
    echo ""
    echo "── CI log tail (last 20 lines) ─────────────────────────────────────"
    tail -20 ${CI_LOG} 2>/dev/null || echo "(no CI log yet)"
    echo ""
    echo "── Disk & memory ───────────────────────────────────────────────────"
    df -h ${VPS_DIR} | tail -1
    free -h | grep Mem
ENDSSH

fi

# ── 3. Upload CI log to GitHub ────────────────────────────────────────────────
# SCP the latest CI run out of the VPS, commit it to logs/ci/ in the repo,
# and push to GitHub. Gives a permanent browsable audit trail per deploy/test run.
# Skipped automatically if: not connected to VPS, --no-log-push, or --push-only.
if $DO_DEPLOY && $DO_LOG_PUSH; then
  header "Upload CI log → GitHub"

  # Determine a meaningful result tag from the CI run (PASS / FAIL / SKIPPED)
  if $DO_TEST; then
    CI_RESULT_TAG=$( $SSH "tail -20 ${CI_LOG} 2>/dev/null | grep -oP '(PASSED|FAILED)' | tail -1" 2>/dev/null || echo "UNKNOWN" )
    CI_RESULT_TAG="${CI_RESULT_TAG:-UNKNOWN}"
  else
    CI_RESULT_TAG="SKIPPED"
  fi

  # Fetch the commit hash from the VPS (what was actually deployed)
  DEPLOYED_COMMIT=$( $SSH "git -C ${VPS_DIR} rev-parse --short HEAD 2>/dev/null" || git rev-parse --short HEAD )

  # Filename: logs/ci/YYYY-MM-DD_HHMMSS_<commit>_<result>.log
  TIMESTAMP=$(date '+%Y-%m-%d_%H%M%S')
  LOG_FILENAME="logs/ci/${TIMESTAMP}_${DEPLOYED_COMMIT}_${CI_RESULT_TAG}.log"
  LOG_DIR="$(git rev-parse --show-toplevel)/logs/ci"

  mkdir -p "$LOG_DIR"

  if $DO_TEST; then
    info "Downloading CI log from VPS…"
    # Extract only the latest CI run from the cumulative log (since last separator)
    $SSH "awk '/^════/{buf=\"\"} {buf=buf\$0\"\n\"} END{printf \"%s\", buf}' ${CI_LOG} 2>/dev/null" \
      > "${LOG_DIR}/$(basename ${LOG_FILENAME})" 2>/dev/null \
      || $SSH "tail -120 ${CI_LOG} 2>/dev/null" > "${LOG_DIR}/$(basename ${LOG_FILENAME})"
  else
    # No CI run — write a brief marker file so the push still records the deploy
    {
      echo "CI gate skipped (--no-test or --restart) at ${TIMESTAMP}"
      echo "Deployed commit: ${DEPLOYED_COMMIT}"
      echo "Operator: $(git config user.name 2>/dev/null || echo unknown)"
    } > "${LOG_DIR}/$(basename ${LOG_FILENAME})"
  fi

  # Commit and push (non-fatal — a log push failure must not break a successful deploy)
  (
    cd "$(git rev-parse --show-toplevel)"
    git add "logs/ci/"
    EMOJI=$([ "$CI_RESULT_TAG" = "PASSED" ] && echo "✅" || [ "$CI_RESULT_TAG" = "SKIPPED" ] && echo "⏭️" || echo "❌")
    git commit -m "ci-log: ${TIMESTAMP} ${DEPLOYED_COMMIT} ${EMOJI} ${CI_RESULT_TAG}" \
      --no-verify 2>/dev/null
    git push origin "${BRANCH}" 2>&1 | sed 's/^/  /'
    success "CI log pushed → logs/ci/$(basename ${LOG_FILENAME})"
  ) || warn "Log push failed (non-fatal) — deploy still succeeded. Push manually if needed."
fi

# ── Done ──────────────────────────────────────────────────────────────────────
echo ""
success "Deploy complete  🚀  Dashboard → http://${VPS_IP}:3000"
echo ""
dim "  Logs on VPS:"
dim "    CI gate  : ${CI_LOG}"
dim "    Deploy   : ${DEPLOY_LOG}"
dim "    Service  : journalctl -u ${SERVICE} -f"
dim "  Logs on GitHub:"
dim "    Browse   : https://github.com/$(git remote get-url origin 2>/dev/null | sed 's/.*github.com[:/]//' | sed 's/\.git$//')/tree/${BRANCH}/logs/ci"
