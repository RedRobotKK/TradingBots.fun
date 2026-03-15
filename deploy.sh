#!/usr/bin/env bash
# deploy.sh – provision infrastructure, push to GitHub, build & restart on VPS
#
# Usage:
#   ./deploy.sh                        – full deploy (CI gate + push to GitHub + build + restart)
#   ./deploy.sh --vps-only             – skip git push, just pull/build/restart on VPS
#   ./deploy.sh --push-only            – push to GitHub only, don't touch VPS
#   ./deploy.sh --restart              – restart service on VPS without rebuilding
#   ./deploy.sh --test-only            – run CI quality gate on VPS without deploying
#   ./deploy.sh --no-test              – skip CI gate (emergency deploys only)
#   ./deploy.sh --no-log-push          – skip uploading CI log to GitHub
#   ./deploy.sh --provision            – (re)run trading-bot server provisioning only
#   ./deploy.sh --provision-ollama     – provision a SEPARATE Ollama droplet
#
# CI Quality Gate (runs before every build):
#   1. cargo test --all         – all unit + integration tests must pass
#   2. cargo clippy -D warnings – no compiler warnings or lints
#   3. cargo audit              – no known CVEs in dependencies (RustSec DB)
#   Deploy is BLOCKED if any of the above fail.
#
# Infrastructure provisioned by --provision (idempotent, safe to re-run):
#   PostgreSQL 16  – installed, tradingbots DB + user created, pg_hba patched
#   sqlx-cli       – schema migrations run automatically on every deploy
#   MCP server     – @modelcontextprotocol/server-postgres for Claude DB access
#   NOTE: Ollama is NOT installed here. It runs on a dedicated droplet to avoid
#         memory contention with the trading bot. Use --provision-ollama instead.
#
# Infrastructure provisioned by --provision-ollama (separate droplet):
#   Target: OLLAMA_IP env var (e.g. export OLLAMA_IP=167.71.x.x)
#   Ollama – installed as systemd service, llama3.2 model pulled
#   Firewall – port 11434 open only to the trading bot VPS (not the internet)
#   OLLAMA_BASE_URL is written into /etc/environment on the *trading bot* VPS
#
# Logs:
#   /var/log/tradingbots-ci.log      (CI gate output, persistent)
#   /var/log/tradingbots-deploy.log  (build + restart output)
#   CI logs also pushed to GitHub: logs/ci/

set -euo pipefail

# ── Config ────────────────────────────────────────────────────────────────────
VPS_IP="${VPS_IP:-165.232.160.43}"
VPS_USER="${VPS_USER:-root}"
VPS_DIR="/tradingbots-fun"
SERVICE="tradingbots"
BRANCH="master"
CI_LOG="/var/log/tradingbots-ci.log"
DEPLOY_LOG="/var/log/tradingbots-deploy.log"

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
DO_LOG_PUSH=true          # upload CI log to GitHub after each run
DO_PROVISION=false        # --provision: trading-bot VPS setup (no Ollama)
DO_PROVISION_OLLAMA=false # --provision-ollama: separate Ollama droplet setup

# IP of the dedicated Ollama droplet — set before calling --provision-ollama:
#   export OLLAMA_IP=167.71.x.x && ./deploy.sh --provision-ollama
OLLAMA_IP="${OLLAMA_IP:-}"
OLLAMA_USER="${OLLAMA_USER:-root}"

for arg in "$@"; do
  case $arg in
    --vps-only)         DO_PUSH=false ;;
    --push-only)        DO_DEPLOY=false ;;
    --restart)          DO_BUILD=false; DO_TEST=false ;;
    --test-only)        DO_PUSH=false; DO_BUILD=false ;;
    --no-test)          DO_TEST=false; warn "--no-test: skipping CI gate (emergency mode)" ;;
    --no-log-push)      DO_LOG_PUSH=false ;;
    --provision)        DO_PROVISION=true; DO_PUSH=false; DO_BUILD=false; DO_TEST=false ;;
    --provision-ollama) DO_PROVISION_OLLAMA=true; DO_PUSH=false; DO_BUILD=false; DO_TEST=false; DO_DEPLOY=false ;;
    --help|-h)
      echo "Usage: $0 [options]"
      echo "  (no flags)          full deploy: CI gate + push to GitHub + build & restart VPS"
      echo "  --vps-only          skip GitHub push, just CI gate + build & restart on VPS"
      echo "  --push-only         push to GitHub only, don't touch VPS"
      echo "  --restart           SSH restart the service without rebuilding or testing"
      echo "  --test-only         run CI quality gate on VPS only (no build/restart)"
      echo "  --no-test           skip CI gate (emergency deploys only)"
      echo "  --no-log-push       skip uploading CI log to GitHub"
      echo "  --provision         set up trading-bot VPS: PostgreSQL 16 + MCP (NO Ollama)"
      echo "  --provision-ollama  provision a SEPARATE dedicated Ollama droplet"
      echo "                      Requires: export OLLAMA_IP=<droplet-ip> first"
      echo ""
      echo "  Ollama runs on its own droplet — never on the trading-bot VPS —"
      echo "  so LLM memory usage cannot starve the bot's trade execution."
      echo "  Recommended Ollama droplet: 8 GB RAM, any region close to VPS."
      exit 0 ;;
    *) error "Unknown argument: $arg"; exit 1 ;;
  esac
done

# ── 0. Infrastructure provisioning (--provision or first-run check) ───────────
# This block is intentionally run BEFORE the git push so the database is ready
# when the newly built binary starts. Idempotent — safe to run on every deploy.
if $DO_PROVISION || $DO_DEPLOY; then
  SSH="ssh -o ConnectTimeout=10 -o BatchMode=yes ${VPS_USER}@${VPS_IP}"

  if $DO_PROVISION; then
    header "Server Provisioning  (PostgreSQL + Ollama + MCP)"
  fi

  $SSH bash <<'PROVISION'
    set -euo pipefail
    export DEBIAN_FRONTEND=noninteractive

    RED='\033[0;31m'; GREEN='\033[0;32m'; CYAN='\033[0;36m'; RESET='\033[0m'
    ok()  { echo -e "${GREEN}✓ $*${RESET}"; }
    inf() { echo -e "${CYAN}▸ $*${RESET}"; }

    # ── PostgreSQL 16 ──────────────────────────────────────────────────────────
    # We use the official PGDG repo to guarantee PostgreSQL 16 regardless of
    # the Ubuntu LTS version.  Ubuntu 22.04 ships with PG 14 by default.
    inf "PostgreSQL 16..."
    if ! command -v psql &>/dev/null || ! psql --version | grep -q "16\|17"; then
      inf "Installing PostgreSQL 16 from PGDG..."
      apt-get install -y curl ca-certificates gnupg lsb-release
      # Add PGDG signing key
      curl -fsSL https://www.postgresql.org/media/keys/ACCC4CF8.asc \
        | gpg --dearmor -o /usr/share/keyrings/postgresql-archive-keyring.gpg
      # Add PGDG repo
      CODENAME=$(lsb_release -cs)
      echo "deb [signed-by=/usr/share/keyrings/postgresql-archive-keyring.gpg] \
            https://apt.postgresql.org/pub/repos/apt ${CODENAME}-pgdg main" \
        > /etc/apt/sources.list.d/pgdg.list
      apt-get update -q
      apt-get install -y postgresql-16 postgresql-client-16
      ok "PostgreSQL 16 installed"
    else
      ok "PostgreSQL already installed: $(psql --version | head -1)"
    fi

    # Ensure PostgreSQL is running and enabled at boot
    systemctl enable postgresql --quiet
    systemctl start  postgresql
    sleep 1
    ok "PostgreSQL service running"

    # ── Database user & database (idempotent) ──────────────────────────────────
    # Generate a stable password from the server hostname so it's reproducible
    # on reprovisioning. In production, override DB_PASSWORD in /etc/environment.
    DB_PASSWORD="${DB_PASSWORD:-$(hostname | sha256sum | cut -c1-32)}"

    # Create role if it doesn't exist (DO $$ avoids error on re-run)
    sudo -u postgres psql -c "
      DO \$\$
      BEGIN
        IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'tradingbots') THEN
          CREATE ROLE tradingbots WITH LOGIN PASSWORD '${DB_PASSWORD}';
          RAISE NOTICE 'Created role tradingbots';
        ELSE
          ALTER ROLE tradingbots WITH PASSWORD '${DB_PASSWORD}';
          RAISE NOTICE 'Updated password for existing role tradingbots';
        END IF;
      END \$\$;
    " 2>&1 | grep -v "^$"

    # Create database if it doesn't exist
    # Note: \gexec only works in interactive psql, not -c. Use createdb instead.
    if sudo -u postgres psql -tc "SELECT 1 FROM pg_database WHERE datname='tradingbots'" \
        | grep -q 1; then
      ok "Database tradingbots already exists"
    else
      sudo -u postgres createdb -O tradingbots tradingbots
      ok "Database tradingbots created"
    fi

    ok "PostgreSQL: role=tradingbots database=tradingbots"

    # ── pg_hba.conf: allow password auth over localhost TCP ───────────────────
    # PostgreSQL defaults to 'peer' auth for local Unix sockets, which means
    # the 'root' OS user can't authenticate as 'tradingbots'.  We allow md5 auth
    # over the loopback interface so the Rust binary can connect normally.
    PG_HBA=$(sudo -u postgres psql -tAc "SHOW hba_file")
    if ! grep -q "tradingbots" "${PG_HBA}" 2>/dev/null; then
      # Prepend our rules (first match wins in pg_hba.conf)
      TMP=$(mktemp)
      {
        echo "# TradingBots — allow password auth over loopback"
        echo "host    tradingbots    tradingbots    127.0.0.1/32    md5"
        echo "host    tradingbots    tradingbots    ::1/128         md5"
        cat "${PG_HBA}"
      } > "${TMP}"
      cp "${TMP}" "${PG_HBA}"
      rm "${TMP}"
      sudo -u postgres pg_ctlcluster 16 main reload 2>/dev/null \
        || systemctl reload postgresql \
        || systemctl restart postgresql
      ok "pg_hba.conf: TCP md5 auth added for tradingbots"
    else
      ok "pg_hba.conf: tradingbots rules already present"
    fi

    # ── Write DATABASE_URL to /etc/environment ────────────────────────────────
    # /etc/environment is sourced by systemd EnvironmentFile= directive in the
    # tradingbots.service unit.  We only write if not already present.
    CORRECT_DB_URL="postgresql://tradingbots:${DB_PASSWORD}@127.0.0.1/tradingbots"
    if ! grep -q "^DATABASE_URL=" /etc/environment 2>/dev/null; then
      echo "DATABASE_URL=${CORRECT_DB_URL}" >> /etc/environment
      ok "DATABASE_URL written to /etc/environment"
    elif grep -q "DATABASE_URL=.*redrobot" /etc/environment 2>/dev/null; then
      # Overwrite stale redrobot URL with the new tradingbots URL
      sed -i "s|^DATABASE_URL=.*|DATABASE_URL=${CORRECT_DB_URL}|" /etc/environment
      ok "DATABASE_URL updated (redrobot → tradingbots) in /etc/environment"
    else
      ok "DATABASE_URL already in /etc/environment"
    fi

    # ── Verify connection ──────────────────────────────────────────────────────
    source /etc/environment
    if PGPASSWORD="${DB_PASSWORD}" psql \
        -h 127.0.0.1 -U tradingbots -d tradingbots -c "SELECT version();" &>/dev/null; then
      ok "PostgreSQL connection verified: tradingbots@127.0.0.1/tradingbots"
    else
      echo -e "${RED}✗ Cannot connect as tradingbots — check pg_hba.conf and password${RESET}"
      echo "  Run: PGPASSWORD='${DB_PASSWORD}' psql -h 127.0.0.1 -U tradingbots tradingbots"
    fi

    # ── sqlx-cli: run schema migrations ───────────────────────────────────────
    # sqlx migrate run reads ./migrations/*.sql and applies any not yet tracked
    # in the _sqlx_migrations table.  Idempotent.
    export PATH="$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin:$PATH"
    source "$HOME/.cargo/env" 2>/dev/null || true

    SQLX_BIN="$HOME/.cargo/bin/sqlx"
    if ! "${SQLX_BIN}" --version &>/dev/null; then
      inf "Installing sqlx-cli..."
      cargo install sqlx-cli --no-default-features --features postgres 2>&1 | tail -3
      # Explicitly use the full path — the shell's PATH hash is stale after install
      ok "sqlx-cli installed: $(${SQLX_BIN} --version 2>/dev/null || echo 'ok')"
    else
      ok "sqlx-cli: $(${SQLX_BIN} --version 2>/dev/null || echo 'installed')"
    fi

    # ── One-time directory migration: /RedRobot-HedgeBot → /tradingbots-fun ───
    if [ -d "/RedRobot-HedgeBot" ] && [ ! -e "/tradingbots-fun" ]; then
      mv /RedRobot-HedgeBot /tradingbots-fun
      ok "Migrated /RedRobot-HedgeBot → /tradingbots-fun"
    elif [ -d "/RedRobot-HedgeBot" ] && [ ! -L "/RedRobot-HedgeBot" ]; then
      # Both exist — create backwards-compat symlink so old paths still work
      ln -sf /tradingbots-fun /RedRobot-HedgeBot
      ok "Created symlink /RedRobot-HedgeBot → /tradingbots-fun"
    fi

    cd /tradingbots-fun
    source /etc/environment
    "${SQLX_BIN}" migrate run --database-url "${DATABASE_URL}" 2>&1 \
      && ok "Migrations applied" \
      || echo "⚠ Migration run failed — will be retried on bot startup"

    # ── Node.js: for MCP server ────────────────────────────────────────────────
    if ! command -v node &>/dev/null; then
      inf "Installing Node.js LTS..."
      curl -fsSL https://deb.nodesource.com/setup_lts.x | bash -
      apt-get install -y nodejs
      ok "Node.js $(node --version) installed"
    else
      ok "Node.js: $(node --version)"
    fi

    # ── PostgreSQL MCP server — gives Claude direct DB query access ───────────
    # The MCP server runs as a subprocess of Claude Desktop (not a daemon).
    # We install it globally so Claude can launch it on demand.
    if ! npm list -g @modelcontextprotocol/server-postgres &>/dev/null; then
      inf "Installing PostgreSQL MCP server..."
      npm install -g @modelcontextprotocol/server-postgres 2>&1 | tail -3
      ok "MCP server installed: $(npm list -g @modelcontextprotocol/server-postgres 2>/dev/null | grep server-postgres || echo 'ok')"
    else
      ok "PostgreSQL MCP server already installed"
    fi

    # ── Ollama: NOT installed on this VPS ─────────────────────────────────────
    # Ollama (llama3.2 = ~4-6 GB RAM) must NOT run on the trading-bot VPS.
    # LLM memory pressure would starve the bot and cause missed trades / slow
    # execution.  Provision a separate dedicated droplet instead:
    #
    #   export OLLAMA_IP=<new-droplet-ip>
    #   ./deploy.sh --provision-ollama
    #
    # Then set OLLAMA_BASE_URL in /etc/environment on THIS VPS:
    #   echo "OLLAMA_BASE_URL=http://<ollama-droplet-ip>:11434" >> /etc/environment
    if grep -q "^OLLAMA_BASE_URL=" /etc/environment 2>/dev/null; then
      ok "OLLAMA_BASE_URL already configured: $(grep OLLAMA_BASE_URL /etc/environment)"
    else
      echo -e "\033[1;33m⚠ OLLAMA_BASE_URL not set — run --provision-ollama on a separate droplet\033[0m"
      echo "  After provisioning the Ollama droplet, add to /etc/environment:"
      echo "  OLLAMA_BASE_URL=http://<ollama-droplet-ip>:11434"
    fi

    # ── Summary ────────────────────────────────────────────────────────────────
    echo ""
    echo "════════════════════════════════════════════════════════════════"
    echo "Trading-bot provisioning complete"
    echo "────────────────────────────────────────────────────────────────"
    echo "  PostgreSQL : $(psql --version | head -1)"
    echo "  Database   : tradingbots @ 127.0.0.1"
    echo "  MCP server : $(npm list -g @modelcontextprotocol/server-postgres 2>/dev/null | grep server-postgres || echo 'ok')"
    echo "────────────────────────────────────────────────────────────────"
    source /etc/environment 2>/dev/null || true
    echo "  DATABASE_URL    = ${DATABASE_URL:-NOT SET}"
    echo "  OLLAMA_BASE_URL = ${OLLAMA_BASE_URL:-⚠ not set — run --provision-ollama}"
    echo "════════════════════════════════════════════════════════════════"
    echo ""
    echo "Next steps:"
    echo "  1. Provision Ollama droplet:  export OLLAMA_IP=<ip> && ./deploy.sh --provision-ollama"
    echo "  2. Configure Claude MCP:      cat /tradingbots-fun/CLAUDE_MCP_SETUP.md"
PROVISION

  if $DO_PROVISION; then
    success "Provisioning complete — infrastructure ready"
    # If --provision was the only flag, don't proceed to the deploy steps.
    if ! $DO_DEPLOY; then
      exit 0
    fi
  fi
fi

# ── 0b. Ollama droplet provisioning (--provision-ollama) ─────────────────────
# Runs on a SEPARATE droplet — never on the trading-bot VPS.
# Memory budget for llama3.2: ~4-6 GB.  Recommended: 8 GB RAM droplet.
# The model is served over HTTP on port 11434, restricted to the bot VPS IP only.
if $DO_PROVISION_OLLAMA; then
  if [[ -z "${OLLAMA_IP}" ]]; then
    error "OLLAMA_IP is not set. Export it first:"
    error "  export OLLAMA_IP=<your-ollama-droplet-ip>"
    error "  ./deploy.sh --provision-ollama"
    exit 1
  fi

  header "Ollama Droplet Provisioning  (${OLLAMA_USER}@${OLLAMA_IP})"
  OLLAMA_SSH="ssh -o ConnectTimeout=10 -o BatchMode=yes ${OLLAMA_USER}@${OLLAMA_IP}"
  BOT_IP="${VPS_IP}"  # trading-bot IP — Ollama will only accept connections from here

  info "Checking SSH connectivity to Ollama droplet..."
  if ! $OLLAMA_SSH "echo ok" &>/dev/null; then
    error "Cannot reach ${OLLAMA_USER}@${OLLAMA_IP}. Check SSH keys or IP."
    exit 1
  fi

  $OLLAMA_SSH bash <<OLLAMA_PROVISION
    set -euo pipefail
    export DEBIAN_FRONTEND=noninteractive
    GREEN='\033[0;32m'; CYAN='\033[0;36m'; YELLOW='\033[1;33m'; RESET='\033[0m'
    ok()  { echo -e "\${GREEN}✓ \$*\${RESET}"; }
    inf() { echo -e "\${CYAN}▸ \$*\${RESET}"; }
    warn(){ echo -e "\${YELLOW}⚠ \$*\${RESET}"; }

    # ── Install Ollama ────────────────────────────────────────────────────────
    if ! command -v ollama &>/dev/null; then
      inf "Installing Ollama..."
      curl -fsSL https://ollama.com/install.sh | sh
      ok "Ollama installed"
    else
      ok "Ollama: \$(ollama --version 2>/dev/null || echo 'installed')"
    fi

    # ── Bind Ollama to all interfaces so the bot VPS can reach it ─────────────
    # Default is localhost-only. We override via systemd drop-in so it listens
    # on 0.0.0.0:11434, but the firewall (ufw) restricts access to bot IP only.
    mkdir -p /etc/systemd/system/ollama.service.d
    cat > /etc/systemd/system/ollama.service.d/listen.conf <<'EOF'
[Service]
Environment="OLLAMA_HOST=0.0.0.0:11434"
EOF
    systemctl daemon-reload
    systemctl enable ollama --quiet
    systemctl restart ollama
    sleep 3
    ok "Ollama service configured to listen on 0.0.0.0:11434"

    # ── Firewall: restrict port 11434 to bot VPS IP only ─────────────────────
    # This is the critical security step — Ollama has no built-in auth.
    # We allow only the trading-bot VPS to call the inference endpoint.
    if command -v ufw &>/dev/null; then
      ufw --force enable 2>/dev/null || true
      ufw default deny incoming  2>/dev/null || true
      ufw allow ssh              2>/dev/null || true   # keep SSH open
      ufw allow from ${BOT_IP} to any port 11434 proto tcp 2>/dev/null || true
      ufw deny 11434             2>/dev/null || true   # block all others
      ok "ufw: port 11434 open to bot VPS (${BOT_IP}) only"
    else
      warn "ufw not available — manually restrict port 11434 in your cloud firewall"
      warn "Allow: ${BOT_IP} → 11434/tcp"
      warn "Deny:  all others → 11434/tcp"
    fi

    # ── Pull default model ────────────────────────────────────────────────────
    # llama3.2 (3B params, ~2 GB on disk, ~4 GB RAM): good CPU inference speed.
    # For a GPU droplet, consider: mistral-7b, llama3.1-8b, or deepseek-r1:7b.
    if ollama list 2>/dev/null | grep -q "llama3.2"; then
      ok "Model llama3.2 already present"
    else
      inf "Pulling llama3.2 (~2 GB download, first run only)..."
      ollama pull llama3.2 2>&1 | tail -5 && ok "llama3.2 ready" \
        || warn "Pull failed — run manually: ollama pull llama3.2"
    fi

    # ── Verify inference endpoint ─────────────────────────────────────────────
    sleep 2
    if curl -sf http://localhost:11434/api/tags &>/dev/null; then
      ok "Ollama API responding at localhost:11434"
    else
      warn "Ollama API not yet responding — check: systemctl status ollama"
    fi

    echo ""
    echo "════════════════════════════════════════════════════════════════"
    echo "Ollama droplet provisioning complete"
    echo "────────────────────────────────────────────────────────────────"
    echo "  Ollama    : \$(ollama --version 2>/dev/null || echo 'ok')"
    echo "  Listening : 0.0.0.0:11434"
    echo "  Firewall  : 11434/tcp open to ${BOT_IP} only"
    echo "  Model     : llama3.2"
    echo "════════════════════════════════════════════════════════════════"
    echo ""
    echo "Add this to /etc/environment on the TRADING-BOT VPS:"
    echo "  OLLAMA_BASE_URL=http://${OLLAMA_IP}:11434"
OLLAMA_PROVISION

  # Write OLLAMA_BASE_URL into the trading-bot VPS /etc/environment
  BOT_SSH="ssh -o ConnectTimeout=10 -o BatchMode=yes ${VPS_USER}@${VPS_IP}"
  info "Writing OLLAMA_BASE_URL to trading-bot VPS /etc/environment..."
  $BOT_SSH bash <<BOTENV
    if grep -q "^OLLAMA_BASE_URL=" /etc/environment 2>/dev/null; then
      sed -i "s|^OLLAMA_BASE_URL=.*|OLLAMA_BASE_URL=http://${OLLAMA_IP}:11434|" /etc/environment
      echo "✓ Updated OLLAMA_BASE_URL in /etc/environment"
    else
      echo "OLLAMA_BASE_URL=http://${OLLAMA_IP}:11434" >> /etc/environment
      echo "✓ Wrote OLLAMA_BASE_URL to /etc/environment"
    fi
    systemctl restart tradingbots 2>/dev/null || true
    echo "✓ tradingbots restarted to pick up OLLAMA_BASE_URL"
BOTENV

  success "Ollama droplet ready — trading bot will use http://${OLLAMA_IP}:11434"
  exit 0
fi

# ── 1. Git push ───────────────────────────────────────────────────────────────
if $DO_PUSH; then
  header "Git"

  if ! git diff --quiet || ! git diff --cached --quiet; then
    warn "You have uncommitted changes — they will NOT be deployed."
    warn "Run 'git add . && git commit -m \"...\"' first if you want them included."
    echo ""
  fi

  # Update local remote if GitHub renamed the repo
  LOCAL_REMOTE=$(git remote get-url origin 2>/dev/null || true)
  if echo "${LOCAL_REMOTE}" | grep -q "RedRobot-HedgeBot"; then
    NEW_LOCAL_REMOTE=$(echo "${LOCAL_REMOTE}" | sed 's/RedRobot-HedgeBot/TradingBots.fun/g')
    git remote set-url origin "${NEW_LOCAL_REMOTE}"
    echo "✓ Updated local git remote → ${NEW_LOCAL_REMOTE}"
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
    # One-time directory rename migration
    if [ -d "/RedRobot-HedgeBot" ] && [ ! -e "/tradingbots-fun" ]; then
      mv /RedRobot-HedgeBot /tradingbots-fun
      echo "✓ Migrated /RedRobot-HedgeBot → /tradingbots-fun"
    elif [ -d "/RedRobot-HedgeBot" ] && [ ! -L "/RedRobot-HedgeBot" ]; then
      ln -sf /tradingbots-fun /RedRobot-HedgeBot
      echo "✓ Symlinked /RedRobot-HedgeBot → /tradingbots-fun"
    fi
    # Fix git safe.directory after mv (mv preserves files but git checks ownership)
    git config --global --add safe.directory /tradingbots-fun 2>/dev/null || true
    # Update git remote if still pointing at old repo name
    OLD_REMOTE=\$(git -C /tradingbots-fun remote get-url origin 2>/dev/null || true)
    if echo "\${OLD_REMOTE}" | grep -q "RedRobot-HedgeBot"; then
      NEW_REMOTE=\$(echo "\${OLD_REMOTE}" | sed 's/RedRobot-HedgeBot/TradingBots.fun/g')
      git -C /tradingbots-fun remote set-url origin "\${NEW_REMOTE}"
      echo "✓ Updated git remote → \${NEW_REMOTE}"
    fi
    # Update DATABASE_URL in /etc/environment to new tradingbots DB if still pointing at old redrobot DB
    if grep -q "DATABASE_URL=.*redrobot" /etc/environment 2>/dev/null; then
      source /etc/environment 2>/dev/null || true
      NEW_DB_URL="\$(echo \"\${DATABASE_URL}\" | sed 's|/redrobot\b|/tradingbots|g; s|redrobot:|tradingbots:|g')"
      sed -i "s|DATABASE_URL=.*|DATABASE_URL=\${NEW_DB_URL}|" /etc/environment
      echo "✓ Updated DATABASE_URL in /etc/environment → tradingbots"
    fi
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
  # The log persists on the VPS at /var/log/tradingbots-ci.log for post-mortem.
  if $DO_TEST; then
    header "CI Quality Gate"
    dim "  Full output → ${CI_LOG} on VPS"
    echo ""

    $SSH bash <<'ENDSSH'
      set -uo pipefail  # Note: NOT -e here — we capture each step's exit code manually

      VPS_DIR="/tradingbots-fun"
      CI_LOG="/var/log/tradingbots-ci.log"
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

      # RUSTSEC-2023-0071: rsa Marvin Attack — no fix available upstream.
      # Enters via sqlx-mysql (compile-time macro dep only, never used at runtime).
      # See audit.toml for full rationale.
      AUDIT_OUT=$(cargo audit --ignore RUSTSEC-2023-0071 2>&1)
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

  # ── 2c. Run pending migrations ───────────────────────────────────────────
  # Migrations run BEFORE the binary is built so the DB schema is always at
  # the right version even if the build fails and the old binary keeps running.
  $SSH bash <<'MIGR'
    set -euo pipefail
    export PATH="$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin:$PATH"
    source "$HOME/.cargo/env" 2>/dev/null || true
    source /etc/environment 2>/dev/null || true
    SQLX_BIN="$HOME/.cargo/bin/sqlx"

    if [ -z "${DATABASE_URL:-}" ]; then
      echo "⚠ DATABASE_URL not set — skipping migrations (run ./deploy.sh --provision first)"
    elif "${SQLX_BIN}" --version &>/dev/null; then
      cd /tradingbots-fun
      echo "▸ Running sqlx migrations…"
      "${SQLX_BIN}" migrate run --database-url "${DATABASE_URL}" \
        && echo "✓ Migrations applied" \
        || echo "⚠ Migration failed — check DB connection"
    else
      echo "⚠ sqlx-cli not installed — run ./deploy.sh --provision"
    fi
MIGR

  # ── 2d. Build ─────────────────────────────────────────────────────────────
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
      BIN_INFO=\$(ls -lh target/release/tradingbots-fun | awk '{print \$5, \$9}')
      echo "✓ Binary: \${BIN_INFO}" | tee -a ${DEPLOY_LOG}
ENDSSH
    success "Build complete"
  fi

  # ── 2c-2. Sync systemd unit file if present in repo ──────────────────────
  UNIT_FILE="${VPS_DIR}/tradingbots.service"
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
          ls -lh ${VPS_DIR}/target/release/tradingbots-fun 2>/dev/null || echo "Binary not found"
          echo ""
          echo "── Environment (sensitive values redacted) ─────────────────────"
          systemctl show ${SERVICE} --property=Environment --value 2>/dev/null \
            | tr ' ' '\n' | sed 's/=.*/=***/' | head -20
        } | tee -a ${DEPLOY_LOG}
        exit 1
      fi
    else
      # Stop old hedgebot service if still running under the old name
      if systemctl is-active hedgebot &>/dev/null; then
        systemctl stop hedgebot 2>/dev/null || true
        systemctl disable hedgebot 2>/dev/null || true
        echo "✓ Stopped old hedgebot service" | tee -a ${DEPLOY_LOG}
      fi
      # Try to install + enable new tradingbots service from repo
      if [ -f "${VPS_DIR}/tradingbots.service" ]; then
        cp "${VPS_DIR}/tradingbots.service" /etc/systemd/system/tradingbots.service
        systemctl daemon-reload
        systemctl enable tradingbots
        systemctl start tradingbots
        sleep 3
        STATUS=\$(systemctl is-active tradingbots)
        echo "✓ tradingbots service installed and started: \${STATUS}" | tee -a ${DEPLOY_LOG}
        exit 0
      fi
      echo "systemd service '${SERVICE}' not found — falling back to pkill+nohup" | tee -a ${DEPLOY_LOG}
      pkill -f tradingbots-fun 2>/dev/null || true
      sleep 2

      set -a
      [ -f /etc/environment ] && source /etc/environment 2>/dev/null || true
      set +a

      nohup env \
        ANTHROPIC_API_KEY="\${ANTHROPIC_API_KEY}" \
        LUNARCRUSH_API_KEY="\${LUNARCRUSH_API_KEY}" \
        PAPER_TRADING="\${PAPER_TRADING:-true}" \
        ${VPS_DIR}/target/release/tradingbots-fun >> ${DEPLOY_LOG} 2>&1 &
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
