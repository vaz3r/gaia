#!/usr/bin/env bash
set -euo pipefail

# Restart only the crawler using its RUNNING image tag, without rebuilding.
# For config-only changes (bind-mounted config): git pull, then run this.
#
# Usage: ./config-restart.sh [host]
# Example: ./config-restart.sh zerone

HOST="${1:-zerone}"
REMOTE_GIT="/home/ubuntu/gaia"
REMOTE_COMPOSE="$REMOTE_GIT/deploy/compose"
SSH_KEY="${HOME}/.ssh/zerone"
SSH="ssh -i $SSH_KEY -o StrictHostKeyChecking=no ubuntu@$HOST"

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
echo "    grep 'effective config' \$(ls -t $REMOTE_GIT/../gaia-data/logs/*.jsonl 2>/dev/null | head -1) || \\"
echo "    ssh $HOST 'ls -t /home/ubuntu/gaia-data/logs/*.jsonl | head -1 | xargs grep \"effective config\"'"