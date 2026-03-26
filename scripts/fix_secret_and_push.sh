#!/usr/bin/env bash
# fix_secret_and_push.sh
# 1. Commits all pending changes (clean compare_databases.sh, version bump, etc.)
# 2. Rewrites git history to remove the hardcoded DB password from commit 620a9e9
# 3. Force-pushes the clean history
# 4. Deploys to staging and production
set -euo pipefail
cd "$(dirname "$0")/.."

OFFENDING="620a9e90bb40152b38b0472473c23e7cb845c94f"

echo "── Committing any pending changes ──────────────────────────────────────"
git add scripts/compare_databases.sh scripts/fix_secret_and_push.sh Cargo.toml signal_weights.json 2>/dev/null || true
git add -u  # stage all other tracked modifications
if ! git diff --cached --quiet; then
  git commit -m "fix: remove hardcoded DB credentials from compare_databases.sh; pending bumps"
else
  echo "  (nothing to commit)"
fi

echo ""
echo "── Saving clean version of compare_databases.sh ────────────────────────"
CLEAN_FILE="$(mktemp)"
cp scripts/compare_databases.sh "$CLEAN_FILE"
chmod +x "$CLEAN_FILE"

echo ""
echo "── Rewriting history to scrub secret from ${OFFENDING:0:8} ─────────────"
PARENT=$(git rev-parse "${OFFENDING}^")
FILTER_BRANCH_SQUELCH_WARNING=1 \
  git filter-branch --force --tree-filter \
  "if [ -f scripts/compare_databases.sh ]; then cp '$CLEAN_FILE' scripts/compare_databases.sh && chmod +x scripts/compare_databases.sh; fi" \
  "${PARENT}..HEAD"

rm -f "$CLEAN_FILE"

echo ""
echo "── Verifying secret is gone ─────────────────────────────────────────────"
if git log -p "${PARENT}..HEAD" -- scripts/compare_databases.sh | grep -q 'AVNS_'; then
  echo "ERROR: secret still found in history — aborting push" >&2
  exit 1
fi
echo "  ✓ No credentials found in rewritten history"

echo ""
echo "── Force-pushing cleaned history ───────────────────────────────────────"
git push --force-with-lease origin master

echo ""
echo "── Deploying to staging ─────────────────────────────────────────────────"
./deploy.sh

echo ""
echo "── Deploying to production ──────────────────────────────────────────────"
./deploy.sh --prod

echo ""
echo "✓ Done. Secret removed, both environments deployed."
echo ""
echo "⚠  IMPORTANT: Rotate your database password in DigitalOcean."
echo "   The old password appeared in terminal history and earlier commits"
echo "   and should be considered compromised regardless of the rewrite."
