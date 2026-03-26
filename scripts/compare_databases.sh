#!/usr/bin/env bash
# compare_databases.sh
# Shows tables, row counts, and key data differences between staging and prod.

STAGING_VPS="root@165.232.160.43"
PROD_VPS="${PROD_DROPLET_IP:-root@157.230.32.73}"
PROD_VPS="root@${PROD_DROPLET_IP:-157.230.32.73}"

# URLs are read from each VPS's own .env so no credentials live in this script.
# Staging: DATABASE_URL is set in /etc/environment on the staging droplet.
# Prod:    DATABASE_URL is set in /root/deploy/TradingBots.fun/.env on the prod droplet.
_read_staging_url() {
  ssh -o StrictHostKeyChecking=no -o ConnectTimeout=10 "$STAGING_VPS" \
    'source /etc/environment 2>/dev/null; echo "${DATABASE_URL:-}"' 2>/dev/null
}
_read_prod_url() {
  ssh -o StrictHostKeyChecking=no -o ConnectTimeout=10 "$PROD_VPS" \
    'grep "^DATABASE_URL=" /root/deploy/TradingBots.fun/.env 2>/dev/null | head -1 | cut -d= -f2-' 2>/dev/null
}

STAGING_URL="${STAGING_DATABASE_URL:-$(_read_staging_url)}"
PROD_URL="${PROD_DATABASE_URL:-$(_read_prod_url)}"

if [[ -z "$STAGING_URL" ]]; then
  echo "ERROR: Could not read staging DATABASE_URL. Set STAGING_DATABASE_URL env var or ensure SSH access to $STAGING_VPS." >&2
  exit 1
fi
if [[ -z "$PROD_URL" ]]; then
  echo "ERROR: Could not read prod DATABASE_URL. Set PROD_DATABASE_URL env var or ensure SSH access to $PROD_VPS." >&2
  exit 1
fi

BOLD=$'\033[1m'; CYAN=$'\033[36m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; RESET=$'\033[0m'

run_query() {
  local vps="$1" url="$2" sql="$3"
  ssh -o StrictHostKeyChecking=no -o ConnectTimeout=10 "$vps" \
    "psql '$url' -t -A -F'|' -c \"$sql\" 2>/dev/null" 2>/dev/null || echo "ERROR"
}

echo ""
echo -e "${BOLD}TradingBots.fun — Database Comparison${RESET}"
echo -e "Staging  : tradingbots_staging @ db.tradingbots.fun"
echo -e "Production: defaultdb @ tradingbots-db-do-user-*.h.db.ondigitalocean.com"
echo ""

# ── 1. Tables in each DB ──────────────────────────────────────────────────────
echo -e "${BOLD}── Tables ───────────────────────────────────────────────────────────────${RESET}"

STAGING_TABLES=$(run_query "$STAGING_VPS" "$STAGING_URL" \
  "SELECT table_name FROM information_schema.tables WHERE table_schema='public' ORDER BY table_name")
PROD_TABLES=$(run_query "$PROD_VPS" "$PROD_URL" \
  "SELECT table_name FROM information_schema.tables WHERE table_schema='public' ORDER BY table_name")

printf "%-40s %-40s\n" "${CYAN}STAGING${RESET}" "${GREEN}PRODUCTION${RESET}"
printf "%-40s %-40s\n" "-------" "----------"

# Merge both lists
ALL_TABLES=$(echo -e "$STAGING_TABLES\n$PROD_TABLES" | sort -u | grep -v '^$')
while IFS= read -r tbl; do
  in_staging=$(echo "$STAGING_TABLES" | grep -c "^${tbl}$" || true)
  in_prod=$(echo "$PROD_TABLES"    | grep -c "^${tbl}$" || true)
  s_mark=$([ "$in_staging" -gt 0 ] && echo "✔" || echo "${YELLOW}✘ MISSING${RESET}")
  p_mark=$([ "$in_prod"    -gt 0 ] && echo "✔" || echo "${YELLOW}✘ MISSING${RESET}")
  printf "  %-38s %-38s  %s\n" "$tbl $s_mark" "$tbl $p_mark" ""
done <<< "$ALL_TABLES"

# ── 2. Row counts for key tables ─────────────────────────────────────────────
echo ""
echo -e "${BOLD}── Row Counts (key tables) ──────────────────────────────────────────────${RESET}"

KEY_TABLES=("tenants" "trades" "equity_snapshots" "aum_snapshots" "funnel_events"
            "referrals" "invite_codes" "trade_journal" "pattern_reports"
            "symbol_signals" "execution_queue" "tenant_activity")

printf "%-30s %12s %12s\n" "Table" "Staging" "Production"
printf "%-30s %12s %12s\n" "-----" "-------" "----------"

for tbl in "${KEY_TABLES[@]}"; do
  s_count=$(run_query "$STAGING_VPS" "$STAGING_URL" "SELECT COUNT(*) FROM $tbl" 2>/dev/null | tr -d '[:space:]')
  p_count=$(run_query "$PROD_VPS"    "$PROD_URL"    "SELECT COUNT(*) FROM $tbl" 2>/dev/null | tr -d '[:space:]')
  [[ -z "$s_count" || "$s_count" == "ERROR" ]] && s_count="${YELLOW}N/A${RESET}"
  [[ -z "$p_count" || "$p_count" == "ERROR" ]] && p_count="${YELLOW}N/A${RESET}"
  printf "  %-28s %10s   %10s\n" "$tbl" "$s_count" "$p_count"
done

# ── 3. System tenant present? ─────────────────────────────────────────────────
echo ""
echo -e "${BOLD}── System Tenant (migration 017) ────────────────────────────────────────${RESET}"

SYSTEM_UUID="00000000-0000-0000-0000-000000000001"
s_tenant=$(run_query "$STAGING_VPS" "$STAGING_URL" \
  "SELECT display_name, tier FROM tenants WHERE id='$SYSTEM_UUID'")
p_tenant=$(run_query "$PROD_VPS" "$PROD_URL" \
  "SELECT display_name, tier FROM tenants WHERE id='$SYSTEM_UUID'")

echo "  Staging    : ${s_tenant:-${YELLOW}NOT FOUND${RESET}}"
echo "  Production : ${p_tenant:-${YELLOW}NOT FOUND${RESET}}"

# ── 4. sqlx migrations table ─────────────────────────────────────────────────
echo ""
echo -e "${BOLD}── sqlx Migrations Applied ──────────────────────────────────────────────${RESET}"

s_migrations=$(run_query "$STAGING_VPS" "$STAGING_URL" \
  "SELECT version, description FROM _sqlx_migrations ORDER BY version" 2>/dev/null)
p_migrations=$(run_query "$PROD_VPS" "$PROD_URL" \
  "SELECT version, description FROM _sqlx_migrations ORDER BY version" 2>/dev/null)

echo "  Staging applied migrations:"
echo "$s_migrations" | grep -v '^$' | sed 's/^/    /' || echo "    (none or table missing)"
echo ""
echo "  Production applied migrations:"
echo "$p_migrations" | grep -v '^$' | sed 's/^/    /' || echo "    (none or table missing)"

echo ""
echo -e "${BOLD}Done.${RESET}"
