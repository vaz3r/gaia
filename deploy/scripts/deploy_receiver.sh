#!/usr/bin/env bash
set -euo pipefail

# Deploy log-receiver to remote host via git.
# Usage: ./deploy_receiver.sh [host] [commit-ref]

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

if [ -f "$SCRIPT_DIR/../config.env" ]; then
    set -a
    source "$SCRIPT_DIR/../config.env"
    set +a
fi

HOST="${1:-100.87.194.112}"
REF="${2:-HEAD}"
REMOTE_GIT="${DEPLOY_REMOTE_GIT:-/home/ubuntu/gaia}"
REMOTE_COMPOSE="$REMOTE_GIT/deploy/compose"
SSH_KEY="${DEPLOY_SSH_KEY:-~/.ssh/zerone}"
SSH_USER="${DEPLOY_USER:-ubuntu}"
SSH="ssh -i $SSH_KEY -o StrictHostKeyChecking=no $SSH_USER@$HOST"

TAG=$(git rev-parse --short "$REF")

echo "=== Deploying log-receiver ($TAG) to $HOST ==="

echo "[1/4] Checking git access..."
if ! $SSH "cd $REMOTE_GIT && git ls-remote --exit-code origin HEAD" >/dev/null 2>&1; then
    echo "ERROR: Cannot reach GitHub from $HOST. Make sure repository is cloned at $REMOTE_GIT."
    exit 1
fi

echo "[2/4] Updating source to $TAG..."
$SSH "cd $REMOTE_GIT && git fetch origin && git checkout $TAG"

echo "[3/4] Ensuring log directories exist..."
$SSH "sudo mkdir -p /mnt/gaia/logs/crawler && sudo chown -R 10001:10001 /mnt/gaia/logs"

echo "[4/4] Building and deploying log-receiver..."
$SSH "cd $REMOTE_COMPOSE && docker compose -f docker-compose-receiver.yml --env-file $REMOTE_GIT/.env up -d --build --force-recreate"

echo ""
echo "=== Deploy log-receiver $TAG complete ==="
