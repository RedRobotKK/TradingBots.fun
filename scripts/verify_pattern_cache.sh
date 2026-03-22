#!/bin/bash
set -euo pipefail

# Run the reporter, then double-check the cached insights to confirm the guardrail combos
# are refreshed and to emit a friendly alert when they change.
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

cargo run --bin reporter
python3 scripts/pattern_cache_alert.py
