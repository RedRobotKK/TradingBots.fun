#!/usr/bin/env bash
# apply_all_migrations.sh
#
# Runs every migration (001 → 017 → 0021 → 0022 → 0023) against a target
# DigitalOcean PostgreSQL database via SSH through the VPS.
#
# Safe to run on a blank database (idempotent — all CREATE TABLE use IF NOT EXISTS).
# Safe to re-run if partially applied (each migration is isolated; failures stop
# the run before the next file is applied).
#
# Usage (from repo root):
#   ./scripts/apply_all_migrations.sh                         – staging only
#   ./scripts/apply_all_migrations.sh --prod-only             – production only
#   ./scripts/apply_all_migrations.sh --prod-only --prod-ip=X.X.X.X
#
# Credential override (recommended):
#   DATABASE_URL='postgresql://doadmin:PASSWORD@host:25060/defaultdb?sslmode=require' \
#     ./scripts/apply_all_migrations.sh --prod-only --prod-ip=157.230.32.73

set -euo pipefail

BOLD=$'\033[1m'; GREEN=$'\033[32m'; RED=$'\033[31m'; YELLOW=$'\033[33m'; CYAN=$'\033[36m'; RESET=$'\033[0m'
ok()   { echo -e "${GREEN}✔${RESET}  $*"; }
err()  { echo -e "${RED}✖${RESET}  $*" >&2; }
warn() { echo -e "${YELLOW}⚠${RESET}  $*"; }
info() { echo -e "${CYAN}   $*${RESET}"; }

# ── Defaults ─────────────────────────────────────────────────────────────────
STAGING_VPS_IP="${VPS_IP:-165.232.160.43}"
STAGING_VPS_USER="${VPS_USER:-root}"
PROD_VPS_IP=""
PROD_ENV_FILE="$HOME/.tradingbots-prod.env"

RUN_STAGING=true
RUN_PROD=false

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"
MIGRATIONS_DIR="$REPO_ROOT/migrations"

# ── Ordered migration list ────────────────────────────────────────────────────
# 0021 alters `bot_sessions` which is an in-memory struct, NOT a DB table.
# It is wrapped in a DO/EXCEPTION block so it fails gracefully on a fresh DB.
MIGRATIONS=(
  "001_initial.sql"
  "002_funnel_events.sql"
  "003_referral_and_ltv.sql"
  "004_invite_leaderboard.sql"
  "005_trial_promo_email.sql"
  "006_add_email_to_tenants.sql"
  "007_trade_journal.sql"
  "008_hl_wallet.sql"
  "009_fix_claim_invite_code.sql"
  "010_investment_thesis.sql"
  "011_collective_learning.sql"
  "012_performance_indexes.sql"
  "013_bridge_requests.sql"
  "014_pattern_reports.sql"
  "015_price_oracle.sql"
  "016_scale_architecture.sql"
  "017_seed_system_tenant.sql"
  "0021_v021_session_fields.sql"
  "0022_v021_hyperliquid_trade_logs.sql"
  "0023_v021_latency_measurements.sql"
)

# ── Arg parsing ───────────────────────────────────────────────────────────────
for arg in "$@"; do
  case "$arg" in
    --prod)         RUN_STAGING=true;  RUN_PROD=true ;;
    --prod-only)    RUN_STAGING=false; RUN_PROD=true ;;
    --staging-only) RUN_STAGING=true;  RUN_PROD=false ;;
    --prod-ip=*)    PROD_VPS_IP="${arg#--prod-ip=}" ;;
    --staging-ip=*) STAGING_VPS_IP="${arg#--staging-ip=}" ;;
    *) warn "Unknown flag: $arg" ;;
  esac
done

# ── Resolve credentials ───────────────────────────────────────────────────────
resolve_db_url() {
  local label="$1"
  local vps_ip="$2"
  local vps_user="$3"

  # Caller-supplied DATABASE_URL wins
  if [[ -n "${DATABASE_URL:-}" ]]; then
    local display
    display=$(echo "$DATABASE_URL" | sed 's|://[^:]*:[^@]*@|://***:***@|')
    info "Using DATABASE_URL from environment: $display"
    echo "$DATABASE_URL"
    return
  fi

  # Fallback: read from env file on the VPS
  local url
  url=$(ssh -o StrictHostKeyChecking=no -o ConnectTimeout=10 \
    "${vps_user}@${vps_ip}" \
    "grep -rh '^DATABASE_URL=' /root/deploy/TradingBots.fun/.env /etc/environment ~/.env 2>/dev/null | head -1 | cut -d= -f2-" 2>/dev/null || true)

  if [[ -z "$url" ]] || echo "$url" | grep -q '\.\.\.'; then
    err "$label: Could not find a valid DATABASE_URL. Pass it via environment:"
    err "  DATABASE_URL='postgresql://doadmin:PASSWORD@host:25060/defaultdb?sslmode=require' \\"
    err "    ./scripts/apply_all_migrations.sh --prod-only --prod-ip=$vps_ip"
    return 1
  fi
  echo "$url"
}

# ── Run all migrations against one target ────────────────────────────────────
run_migrations() {
  local label="$1"
  local vps_ip="$2"
  local vps_user="$3"
  local db_url

  echo ""
  echo -e "${BOLD}── ${label} (${vps_ip}) ───────────────────────────────────────────${RESET}"
  info "Via SSH: ${vps_user}@${vps_ip}"

  # Check / install psql
  info "Checking psql on remote..."
  ssh -o StrictHostKeyChecking=no -o ConnectTimeout=10 "${vps_user}@${vps_ip}" \
    "which psql >/dev/null 2>&1 || (apt-get update -qq && apt-get install -y -qq postgresql-client)" \
    2>&1 | grep -v "^perl:" || true

  db_url=$(resolve_db_url "$label" "$vps_ip" "$vps_user") || return 1
  echo ""

  local failed=0
  local applied=0
  local skipped=0

  for migration in "${MIGRATIONS[@]}"; do
    local fpath="$MIGRATIONS_DIR/$migration"
    if [[ ! -f "$fpath" ]]; then
      warn "  SKIP (file not found): $migration"
      (( skipped++ )) || true
      continue
    fi

    echo -ne "  Applying ${CYAN}${migration}${RESET}... "

    # Special handling: 0021 touches bot_sessions which may not exist as a DB table.
    # Wrap it in a DO block so it is a no-op rather than a fatal error.
    local sql_content
    sql_content=$(cat "$fpath")

    if [[ "$migration" == "0021_v021_session_fields.sql" ]]; then
      sql_content=$(cat <<'WRAPPER_EOF'
DO $$
BEGIN
  ALTER TABLE bot_sessions
    ADD COLUMN IF NOT EXISTS venue                TEXT    NOT NULL DEFAULT 'internal',
    ADD COLUMN IF NOT EXISTS leverage_max         INTEGER,
    ADD COLUMN IF NOT EXISTS risk_mode            TEXT,
    ADD COLUMN IF NOT EXISTS symbols_whitelist    JSONB,
    ADD COLUMN IF NOT EXISTS max_drawdown_pct     DOUBLE PRECISION,
    ADD COLUMN IF NOT EXISTS performance_fee_pct  INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS hyperliquid_address  TEXT,
    ADD COLUMN IF NOT EXISTS webhook_url          TEXT,
    ADD COLUMN IF NOT EXISTS paused               BOOLEAN NOT NULL DEFAULT FALSE;
EXCEPTION
  WHEN undefined_table THEN
    RAISE NOTICE 'bot_sessions table does not exist — migration 0021 skipped (in-memory only)';
END;
$$;
WRAPPER_EOF
)
    fi

    # Upload SQL to a temp file on the VPS, run it, then delete it.
    # This avoids shell-escaping issues with piping $sql_content through ssh.
    local remote_tmp="/tmp/tb_migration_$$.sql"
    echo "$sql_content" | ssh -o StrictHostKeyChecking=no -o ConnectTimeout=10 \
      "${vps_user}@${vps_ip}" "cat > $remote_tmp" 2>/dev/null

    local result exit_code
    set +e
    result=$(ssh -o StrictHostKeyChecking=no -o ConnectTimeout=30 \
      "${vps_user}@${vps_ip}" \
      "PGSSLMODE=require psql \"$db_url\" -v ON_ERROR_STOP=1 -q -f $remote_tmp 2>&1; rm -f $remote_tmp")
    exit_code=$?
    set -e

    # Strip the harmless perl locale warnings — they hide the real error
    local clean_result
    clean_result=$(echo "$result" | grep -v "^perl:" | grep -v "^LC_" | grep -v "^LANGUAGE" | grep -v "are supported" | grep -v "Falling back")

    if [[ $exit_code -eq 0 ]]; then
      echo -e "${GREEN}✔${RESET}"
      [[ -n "$clean_result" ]] && echo "     $clean_result"  # show any NOTICEs
      (( applied++ )) || true
    else
      echo -e "${RED}✖${RESET}"
      echo "$clean_result" | sed 's/^/     /'
      err "  Migration failed: $migration"
      (( failed++ )) || true
      # Stop on first failure to avoid cascading errors
      break
    fi
  done

  echo ""
  if [[ $failed -gt 0 ]]; then
    err "$label: $failed migration(s) failed. Applied: $applied. Stopped early."
    return 1
  else
    ok "$label: All $applied migration(s) applied successfully. ($skipped skipped)"
  fi
}

# ── Main ─────────────────────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}TradingBots.fun — Full Schema Migration${RESET}"
echo -e "${BOLD}Migrations dir:${RESET} $MIGRATIONS_DIR"
echo -e "${BOLD}Total files:${RESET}    ${#MIGRATIONS[@]}"
echo ""

overall_fail=0

if [[ "$RUN_PROD" == "true" ]]; then
  # Resolve prod VPS IP
  if [[ -z "$PROD_VPS_IP" ]]; then
    if [[ -f "$PROD_ENV_FILE" ]]; then
      PROD_VPS_IP=$(grep -E '^PROD_DROPLET_IP=' "$PROD_ENV_FILE" | cut -d= -f2 | tr -d '"' | head -1)
    fi
  fi
  if [[ -z "$PROD_VPS_IP" ]]; then
    err "Production VPS IP not set. Use --prod-ip=X.X.X.X"
    exit 1
  fi

  echo -e "${YELLOW}${BOLD}"
  echo "  ⚠  About to apply ALL MIGRATIONS to PRODUCTION (${PROD_VPS_IP})  ⚠"
  echo -e "${RESET}"
  read -r -p "  This will create tables and seed data. Type 'yes' to continue: " confirm
  [[ "$confirm" == "yes" ]] || { warn "Aborted."; exit 0; }

  run_migrations "PRODUCTION" "$PROD_VPS_IP" "root" || (( overall_fail++ )) || true
fi

if [[ "$RUN_STAGING" == "true" ]]; then
  run_migrations "STAGING" "$STAGING_VPS_IP" "$STAGING_VPS_USER" || (( overall_fail++ )) || true
fi

echo ""
if [[ $overall_fail -gt 0 ]]; then
  err "One or more targets failed — review output above."
  exit 1
else
  ok "All done."
  echo ""
  echo -e "  Next steps:"
  echo -e "  1. ${YELLOW}Rotate your database password${RESET} in the DigitalOcean dashboard"
  echo -e "     (it was passed in plaintext via this script)"
  echo -e "  2. Update DATABASE_URL in the production VPS env file:"
  echo -e "     ${CYAN}ssh root@\$PROD_VPS_IP${RESET}"
  echo -e "     ${CYAN}echo DATABASE_URL='postgresql://doadmin:NEWPASSWORD@...' >> /root/deploy/TradingBots.fun/.env${RESET}"
  echo -e "  3. Restart the bot service: ${CYAN}systemctl restart tradingbots${RESET}"
fi
