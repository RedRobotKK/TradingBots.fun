#!/usr/bin/env bash
# apply_migration_017.sh
#
# Applies migration 017_seed_system_tenant.sql to the DigitalOcean managed
# Postgres databases. Runs the SQL via SSH through the VPS (which is already
# whitelisted on the DO database firewall), so it works from any machine
# including a local Mac dev laptop.
#
# Usage (from repo root):
#   ./scripts/apply_migration_017.sh                        – staging only (default)
#   ./scripts/apply_migration_017.sh --prod                 – staging + production
#   ./scripts/apply_migration_017.sh --prod-only            – production only
#   ./scripts/apply_migration_017.sh --prod --prod-ip=1.2.3.4  – explicit prod VPS IP
#
# Reads VPS connection details from the same defaults as deploy.sh.
# Override with env vars:  VPS_IP, VPS_USER, PROD_DROPLET_IP

set -euo pipefail

# ── Colours ──────────────────────────────────────────────────────────────────
BOLD=$'\033[1m'; GREEN=$'\033[32m'; RED=$'\033[31m'; YELLOW=$'\033[33m'; RESET=$'\033[0m'
ok()   { echo -e "${GREEN}✔${RESET}  $*"; }
err()  { echo -e "${RED}✖${RESET}  $*" >&2; }
warn() { echo -e "${YELLOW}⚠${RESET}  $*"; }

# ── Defaults (mirror deploy.sh) ───────────────────────────────────────────────
STAGING_VPS_IP="${VPS_IP:-165.232.160.43}"
STAGING_VPS_USER="${VPS_USER:-root}"

PROD_ENV_FILE="$HOME/.tradingbots-prod.env"
PROD_VPS_IP=""
PROD_VPS_USER="root"

# Load prod VPS IP if the env file exists
if [[ -f "$PROD_ENV_FILE" ]]; then
  # shellcheck disable=SC1090
  source "$PROD_ENV_FILE"
  PROD_VPS_IP="${PROD_DROPLET_IP:-}"
  PROD_VPS_USER="${VPS_USER:-root}"
fi

# ── Args ──────────────────────────────────────────────────────────────────────
DO_STAGING=true
DO_PROD=false

for arg in "$@"; do
  case "$arg" in
    --prod)           DO_PROD=true ;;
    --prod-only)      DO_STAGING=false; DO_PROD=true ;;
    --prod-ip=*)      PROD_VPS_IP="${arg#--prod-ip=}" ;;
    --staging-ip=*)   STAGING_VPS_IP="${arg#--staging-ip=}" ;;
    --help|-h)
      sed -n '2,14p' "$0" | sed 's/^# \?//'
      exit 0 ;;
    *) err "Unknown argument: $arg"; exit 1 ;;
  esac
done

# ── Locate migration SQL ──────────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
MIGRATION="$REPO_ROOT/migrations/017_seed_system_tenant.sql"

if [[ ! -f "$MIGRATION" ]]; then
  err "Migration file not found: $MIGRATION"
  exit 1
fi

SQL="$(cat "$MIGRATION")"

# ── SSH connectivity check ────────────────────────────────────────────────────
check_ssh() {
  local user="$1" ip="$2"
  if ! ssh -o ConnectTimeout=10 -o BatchMode=yes "${user}@${ip}" "echo ok" &>/dev/null; then
    err "Cannot reach ${user}@${ip} via SSH."
    err "Make sure your SSH key is added and the host is reachable."
    return 1
  fi
}

# ── Apply via SSH ─────────────────────────────────────────────────────────────
# The VPS has DATABASE_URL in /etc/environment and psql installed.
# We pipe the SQL through SSH so we never need local credentials or whitelisting.
apply_via_ssh() {
  local label="$1"
  local vps_user="$2"
  local vps_ip="$3"
  local db_env_var="$4"   # DATABASE_URL var name on the VPS

  echo ""
  echo -e "${BOLD}── $label ──────────────────────────────────────────${RESET}"
  echo "   Via SSH: ${vps_user}@${vps_ip}"
  echo ""

  check_ssh "$vps_user" "$vps_ip"

  # Ensure psql is available on the remote host — install if missing
  echo "   Checking psql on remote..."
  ssh -o ConnectTimeout=10 "${vps_user}@${vps_ip}" bash <<'INSTALL'
if command -v psql &>/dev/null; then
  echo "   psql already installed: $(psql --version)"
else
  echo "   Installing postgresql-client..."
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -qq
  apt-get install -y -q postgresql-client 2>&1 | tail -3
  if command -v psql &>/dev/null; then
    echo "   psql installed: $(psql --version)"
  else
    echo "ERROR: psql install failed" >&2
    exit 1
  fi
fi
INSTALL

  # Allow caller to override DATABASE_URL entirely via env var
  if [[ -n "${DATABASE_URL:-}" ]]; then
    echo "   Using DATABASE_URL from environment (caller-supplied)"
    remote_db_url="$DATABASE_URL"
  else

  # Locate DATABASE_URL on the remote — check every place deploy.sh might write it
  echo "   Locating DATABASE_URL on remote..."
  local remote_db_url
  remote_db_url=$(ssh -o ConnectTimeout=10 "${vps_user}@${vps_ip}" bash <<'FINDURL'
# Helper: extract DATABASE_URL from a file, skip placeholder values.
# Handles: DATABASE_URL=..., export DATABASE_URL=..., DATABASE_URL="...", etc.
extract_from_file() {
  local f="$1"
  [[ -f "$f" ]] || return 1
  local val
  val=$(grep -E "(^|^export )DATABASE_URL=" "$f" 2>/dev/null \
        | head -1 \
        | sed 's/^export //;s/^DATABASE_URL=//' \
        | tr -d '"'"'"' ')
  # Skip if value is a placeholder (contains … or is empty or has no @)
  if [[ -n "$val" && "$val" != *"…"* && "$val" == *"@"* ]]; then
    echo "$val"
    return 0
  fi
  return 1
}

# 1. EnvironmentFile referenced in the systemd service (most reliable on prod)
ENV_FILE=$(systemctl cat tradingbots 2>/dev/null \
  | grep "^EnvironmentFile=" | head -1 | cut -d= -f2- | tr -d ' ')
if [[ -n "$ENV_FILE" && -f "$ENV_FILE" ]]; then
  val=$(extract_from_file "$ENV_FILE") && { echo "$val"; exit 0; }
fi

# 2. Common .env file locations
for f in /root/deploy/TradingBots.fun/.env \
          /root/TradingBots.fun/.env \
          /opt/tradingbots/.env \
          /root/.env \
          /root/tradingbots.env; do
  [[ -f "$f" ]] || continue
  val=$(extract_from_file "$f") && { echo "$val"; exit 0; }
done

# 3. /etc/environment (staging path — real URL, not placeholder)
val=$(extract_from_file /etc/environment) && { echo "$val"; exit 0; }

# 4. Live process environment
PID=$(systemctl show tradingbots -p MainPID --value 2>/dev/null)
if [[ -n "$PID" && "$PID" != "0" ]]; then
  val=$(tr '\0' '\n' < /proc/$PID/environ 2>/dev/null \
        | grep "^DATABASE_URL=" | cut -d= -f2-)
  if [[ -n "$val" && "$val" != *"…"* && "$val" == *"@"* ]]; then
    echo "$val"; exit 0
  fi
fi

echo ""
FINDURL
)

  if [[ -z "$remote_db_url" ]]; then
    err "Could not locate DATABASE_URL on ${vps_ip}."
    err ""
    err "Checking .env file format for diagnosis:"
    ssh -o ConnectTimeout=10 "${vps_user}@${vps_ip}" \
      "grep -i 'database' /root/deploy/TradingBots.fun/.env 2>/dev/null | sed 's/:.*@/:HIDDEN@/g' || echo '  (file missing or no DATABASE_URL key)'"
    err ""
    err "Fix: pass the URL directly as an env var:"
    err "  DATABASE_URL='postgresql://user:pass@host:port/db?sslmode=require' \\"
    err "    ./apply_migration_017.sh --prod-only --prod-ip=${vps_ip}"
    return 1
  fi
  fi  # end: DATABASE_URL auto-detect block
  echo "   Found: ${remote_db_url%%\?*}  (credentials hidden)"

  # Run psql on the VPS, piping the SQL from local file over stdin
  # shellcheck disable=SC2087
  if ssh -o ConnectTimeout=10 "${vps_user}@${vps_ip}" \
      "psql '${remote_db_url}' -v ON_ERROR_STOP=1" \
      <<< "$SQL"; then
    ok "$label: migration applied"
  else
    err "$label: migration failed"
    return 1
  fi

  # Verify the row is present
  local count
  count=$(ssh -o ConnectTimeout=10 "${vps_user}@${vps_ip}" \
    "psql '${remote_db_url}' -tAc \
     \"SELECT count(*) FROM tenants WHERE id = '00000000-0000-0000-0000-000000000001';\"")

  if [[ "${count:-0}" -ge 1 ]]; then
    ok "$label: system tenant row confirmed present"
  else
    warn "$label: row not found after insert — check manually"
  fi
}

# ── Run ───────────────────────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}Applying 017_seed_system_tenant.sql${RESET}"
echo "Migration: $MIGRATION"

FAILED=0

if [[ "$DO_STAGING" == true ]]; then
  apply_via_ssh \
    "STAGING (tradingbots_staging)" \
    "$STAGING_VPS_USER" \
    "$STAGING_VPS_IP" \
    "DATABASE_URL" \
    || FAILED=1
fi

if [[ "$DO_PROD" == true ]]; then
  if [[ -z "$PROD_VPS_IP" ]]; then
    err "PROD_DROPLET_IP not set — run ./deploy.sh --provision-prod first"
    err "or set PROD_DROPLET_IP in $PROD_ENV_FILE"
    FAILED=1
  else
    echo ""
    echo -e "${YELLOW}${BOLD}  ⚠  About to write to PRODUCTION (${PROD_VPS_IP})  ⚠${RESET}"
    read -rp "  Type 'yes' to continue: " confirm
    if [[ "$confirm" != "yes" ]]; then
      warn "Production skipped."
    else
      apply_via_ssh \
        "PRODUCTION (tradingbots)" \
        "$PROD_VPS_USER" \
        "$PROD_VPS_IP" \
        "DATABASE_URL" \
        || FAILED=1
    fi
  fi
fi

echo ""
if [[ "$FAILED" -eq 0 ]]; then
  ok "Done. The equity_snapshot FK errors should stop immediately."
else
  err "One or more targets failed — review output above."
  exit 1
fi
