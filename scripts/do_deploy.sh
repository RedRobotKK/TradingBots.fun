#!/usr/bin/env bash
# do_deploy.sh — commit all pending fixes, set prod env, deploy staging then prod
set -euo pipefail

cd "$(dirname "$0")/.."

echo "── Step 1: commit all pending fixes and improvements ────────────────────"
git add \
  src/bridge.rs \
  src/config.rs \
  src/exchange.rs \
  src/main.rs \
  src/risk.rs \
  src/web_dashboard.rs \
  migrations/017_seed_system_tenant.sql \
  scripts/apply_all_migrations.sh \
  scripts/apply_migration_017.sh \
  scripts/compare_databases.sh \
  scripts/do_deploy.sh

git commit -m "fix+feat: compile fixes, small-wallet sizing, dashboard metrics, env-driven config

Compile fixes:
- main.rs: declare \`mod latency;\` so crate::latency resolves in binary target
- main.rs: add missing venue field to PaperPosition struct initializers
- web_dashboard.rs: remove broken bare \`use crate::latency\` import; restore
  full \`crate::latency::LatencyTracker\` paths in struct field and type annotation
- bridge.rs: pass sentinel args (f64::MAX, 0.0) to get_account() calls

Config / risk improvements:
- config.rs: MAX_POSITION_PCT, MIN_POSITION_PCT, DAILY_LOSS_LIMIT, MIN_HEALTH_FACTOR
  now read from env vars so each wallet can tune its own limits
- exchange.rs: get_account() takes (daily_loss_limit, min_health_factor) from config
- risk.rs: Account struct carries min_health_factor; is_healthy() uses it

Small wallet algo improvements (main.rs):
- Wallets ≤\$100: min position 15%, max 35% (was 5%/12%)
- Wallets \$101-300: min 10%, max 20%
- Position cap ≤\$100: 3 slots (was 6) — fewer, higher-conviction trades
- Position cap \$101-300: 5 slots; \$301-500: 10 slots
- First tranche exit trigger: 1.0R → 0.75R

Dashboard metrics (web_dashboard.rs):
- Replace Open/Closed (maxes out) with Slots Used (N/cap) + Avg Open R
- Slots card colour-codes green/amber/red by utilisation
- Avg Open R card shows mean R-multiple across all live positions
- Status bar: N/cap slots · avg R · portfolio heat % · Sharpe

New scripts:
- scripts/apply_all_migrations.sh: idempotent full-schema migration runner
- scripts/apply_migration_017.sh: seed system tenant (migration 017)
- scripts/compare_databases.sh: side-by-side staging vs prod DB comparison"

echo ""
echo "── Step 2: set prod deploy path ────────────────────────────────────────"
grep -q 'VPS_DIR' ~/.tradingbots-prod.env 2>/dev/null \
  || echo 'VPS_DIR=/root/deploy/TradingBots.fun' >> ~/.tradingbots-prod.env
echo "  ~/.tradingbots-prod.env updated"

echo ""
echo "── Step 3: deploy to staging ───────────────────────────────────────────"
./deploy.sh

echo ""
echo "── Step 4: deploy to production ────────────────────────────────────────"
./deploy.sh --prod
