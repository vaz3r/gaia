#!/usr/bin/env bash
set -euo pipefail

# Restart only the crawler using its RUNNING image tag, without rebuilding.
# For config-only changes (bind-mounted config): git pull, then run this.
#
# Usage: ./config-restart.sh [host]
# Example: ./config-restart.sh gaia

# ── Load deployment config ──
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

if [ -f "$SCRIPT_DIR/../config.env" ]; then
    set -a
    # shellcheck disable=SC1091
    source "$SCRIPT_DIR/../config.env"
    set +a
fi

HOST="${1:-${DEPLOY_HOST:?DEPLOY_HOST required — set in deploy/config.env}}"
REMOTE_GIT="${DEPLOY_REMOTE_GIT:?DEPLOY_REMOTE_GIT required}"
REMOTE_COMPOSE="$REMOTE_GIT/deploy/compose"
SSH_KEY="${DEPLOY_SSH_KEY:?DEPLOY_SSH_KEY required}"
SSH_USER="${DEPLOY_USER:?DEPLOY_USER required}"
SSH="ssh -i $SSH_KEY -o StrictHostKeyChecking=no $SSH_USER@$HOST"

echo "=== Detecting running crawler image on $HOST ==="
IMAGE=$($SSH "docker inspect gaia-crawler --format '{{.Config.Image}}'")

if [ -z "$IMAGE" ] || [ "$IMAGE" = "null" ]; then
    echo "ERROR: gaia-crawler is not running. Use deploy.sh to deploy a new tag."
    exit 1
fi

TAG="${IMAGE##*:}"
if [ -z "$TAG" ] || [ "$TAG" = "$IMAGE" ]; then
    echo "ERROR: could not parse image tag from '$IMAGE' (expected name:tag)"
    exit 1
fi

echo "=== Restarting crawler with existing image tag '$TAG' (no build) ==="
echo "    (config is bind-mounted from $REMOTE_GIT/apps/crawler/config)"
$SSH "cd $REMOTE_COMPOSE && \
    GIT_COMMIT=$TAG docker compose --env-file $REMOTE_GIT/.env up -d --force-recreate --no-build crawler"

echo "=== Verifying ==="
sleep 8
$SSH "docker ps --filter name=gaia-crawler --format '{{.Names}}\t{{.Status}}'"
$SSH "docker inspect gaia-crawler --format 'Restarts={{.RestartCount}}'"

echo ""
echo "=== $TAG restart complete ==="
echo "    Confirm the change took effect:"
echo "    grep 'effective config' \$(ls -t ${DEPLOY_REMOTE_DATA}/logs/*.jsonl 2>/dev/null | head -1) || \\"
echo "    ssh $HOST 'ls -t ${DEPLOY_REMOTE_DATA}/logs/*.jsonl | head -1 | xargs grep \"effective config\"'"
