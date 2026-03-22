#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT=$(git rev-parse --show-toplevel)
cd "$REPO_ROOT"

BRANCH=${1:-master}
GITHUB_REMOTE=${GITHUB_REMOTE:-origin}
STAGING_REMOTE=${STAGING_REMOTE:-staging}
PRODUCTION_REMOTE=${PRODUCTION_REMOTE:-production}

function require_remote() {
    git remote get-url "$1" >/dev/null 2>&1 || {
        echo "Remote '$1' is not configured. Please add it before continuing." >&2
        exit 1
    }
}

function push_branch() {
    local remote=$1
    echo "➜ Pushing ${BRANCH} to ${remote}"
    git push "$remote" "${BRANCH:?}"
}

require_remote "$GITHUB_REMOTE"
require_remote "$STAGING_REMOTE"
require_remote "$PRODUCTION_REMOTE"

echo "Promoting '${BRANCH}' → GitHub → staging → production"
git fetch "$GITHUB_REMOTE"
push_branch "$GITHUB_REMOTE"
push_branch "$STAGING_REMOTE"
push_branch "$PRODUCTION_REMOTE"

echo "Promotion complete. Review the hooks/logs on each host if deployment automation is configured."
