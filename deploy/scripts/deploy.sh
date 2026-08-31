#!/usr/bin/env bash
set -euo pipefail

# Unified Deployment Script (Target-Based Architecture)
# Usage: ./deploy/scripts/deploy.sh <target-name> [commit-ref]
#
# Examples:
#   ./deploy/scripts/deploy.sh gaia-node
#   ./deploy/scripts/deploy.sh workspace-production HEAD~1

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

TARGET="${1:-}"
REF="${2:-HEAD}"

if [ -z "$TARGET" ]; then
    echo "ERROR: Target name required."
    echo "Usage: $0 <target-name> [commit-ref]"
    echo "Available targets:"
    ls -1 "$REPO_ROOT/deploy/targets/"
    exit 1
fi

TARGET_DIR="$REPO_ROOT/deploy/targets/$TARGET"
if [ ! -d "$TARGET_DIR" ]; then
    echo "ERROR: Target directory not found: $TARGET_DIR"
    exit 1
fi

# ── Load target-specific env ──
set -a
source "$TARGET_DIR/.env"
set +a

: "${DEPLOY_HOST:?DEPLOY_HOST required in target .env}"
: "${DEPLOY_USER:?DEPLOY_USER required in target .env}"
: "${DEPLOY_REMOTE_GIT:?DEPLOY_REMOTE_GIT required in target .env}"
: "${DEPLOY_REMOTE_DATA:?DEPLOY_REMOTE_DATA required in target .env}"

# SSH Configuration
if [ -n "${DEPLOY_PASSWORD:-}" ]; then
    # Use sshpass for password authentication
    SSH="sshpass -p $DEPLOY_PASSWORD ssh -o StrictHostKeyChecking=no $DEPLOY_USER@$DEPLOY_HOST"
elif [ -n "${DEPLOY_SSH_KEY:-}" ]; then
    # Use SSH key authentication
    SSH="ssh -i $DEPLOY_SSH_KEY -o StrictHostKeyChecking=no $DEPLOY_USER@$DEPLOY_HOST"
else
    # Fallback to interactive prompt
    SSH="ssh -o StrictHostKeyChecking=no $DEPLOY_USER@$DEPLOY_HOST"
fi

TAG=$(git rev-parse --short "$REF")
REMOTE_TARGET_DIR="$DEPLOY_REMOTE_GIT/deploy/targets/$TARGET"

echo "=== Deploying $TAG to $TARGET ($DEPLOY_HOST) ==="

# ── 1. Check GitHub auth on remote ──
echo "[1/4] Checking git access..."
if ! $SSH "cd $DEPLOY_REMOTE_GIT && git ls-remote --exit-code origin HEAD" >/dev/null 2>&1; then
    echo "ERROR: Cannot reach GitHub from $DEPLOY_HOST."
    exit 1
fi

# ── 2. Fetch + checkout on remote ──
echo "[2/4] Updating source to $TAG..."
$SSH "cd $DEPLOY_REMOTE_GIT && git fetch origin && git checkout $TAG"

# ── 3. Ensure data directories exist ──
echo "[3/4] Ensuring data directories..."
$SSH "echo '${DEPLOY_PASSWORD:-}' | sudo -S mkdir -p ${DEPLOY_REMOTE_DATA}/crawler ${DEPLOY_REMOTE_DATA}/postgres ${DEPLOY_REMOTE_DATA}/logs /mnt/gaia/logs/crawler && echo '${DEPLOY_PASSWORD:-}' | sudo -S chown -R 10001:10001 ${DEPLOY_REMOTE_DATA} /mnt/gaia/logs || true"

# ── 4. Build and deploy services ──
echo "[4/4] Building and deploying compose stack..."
$SSH "cd $REMOTE_TARGET_DIR && GIT_COMMIT=$TAG docker compose --env-file .env up -d --build --force-recreate"

echo ""
echo "=== Deploy $TAG to $TARGET complete ==="
