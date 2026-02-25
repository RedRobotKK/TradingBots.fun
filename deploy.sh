#!/bin/bash
# deploy.sh – sync local source to the droplet and rebuild

DROPLET_IP="${DROPLET_IP:-165.232.160.43}"
DROPLET_USER="${DROPLET_USER:-root}"
REMOTE_DIR="/RedRobot-HedgeBot"

echo "📦 Syncing source to ${DROPLET_USER}@${DROPLET_IP}:${REMOTE_DIR}"

# Sync only source files (not target/ which is huge)
rsync -avz --exclude 'target/' --exclude '.git/' \
    ./ "${DROPLET_USER}@${DROPLET_IP}:${REMOTE_DIR}/"

echo ""
echo "🔨 Building on droplet (this takes ~3 min with swap)..."
ssh "${DROPLET_USER}@${DROPLET_IP}" bash << 'ENDSSH'
  # Source Rust – installed by rustup into ~/.cargo/bin
  export PATH="$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin:$PATH"
  source "$HOME/.cargo/env" 2>/dev/null || true

  cd /RedRobot-HedgeBot

  # Ensure swap is active (needed to compile axum on 1 GB RAM)
  if ! swapon --show | grep -q swapfile; then
    echo "Enabling swap..."
    swapon /swapfile 2>/dev/null || true
  fi

  echo "Rust: $(rustc --version 2>/dev/null || echo 'NOT FOUND')"
  echo "Cargo: $(cargo --version 2>/dev/null || echo 'NOT FOUND')"

  cargo build --release 2>&1
  echo ""
  echo "Build exit code: $?"
ENDSSH

echo ""
echo "🚀 Restarting bot..."
ssh "${DROPLET_USER}@${DROPLET_IP}" bash << 'ENDSSH'
  pkill -f redrobot-hedgebot 2>/dev/null || true
  sleep 2
  cd /RedRobot-HedgeBot

  # Load env vars from /etc/environment (where ANTHROPIC_API_KEY etc. live)
  set -a
  [ -f /etc/environment ] && source /etc/environment 2>/dev/null || true
  set +a

  echo "ANTHROPIC_API_KEY set: $([ -n "$ANTHROPIC_API_KEY" ] && echo YES || echo NO – add to /etc/environment)"
  echo "LUNARCRUSH_API_KEY set: $([ -n "$LUNARCRUSH_API_KEY" ] && echo YES || echo USING HARDCODED FALLBACK)"

  nohup env \
    ANTHROPIC_API_KEY="${ANTHROPIC_API_KEY}" \
    LUNARCRUSH_API_KEY="${LUNARCRUSH_API_KEY}" \
    PAPER_TRADING="${PAPER_TRADING:-true}" \
    ./target/release/redrobot-hedgebot >> /var/log/hedgebot.log 2>&1 &
  echo "Bot PID: $!"
  sleep 3
  echo "=== Last 10 log lines ==="
  tail -10 /var/log/hedgebot.log
ENDSSH

echo ""
echo "✅ Done. Dashboard: http://${DROPLET_IP}:3000"
