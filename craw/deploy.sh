#!/usr/bin/env bash
set -euo pipefail

# Deploy crawler + dashboard to remote host via git.
# Usage: ./deploy.sh [host] [commit-ref]
#
# Examples:
#   ./deploy.sh                    # deploy HEAD to zerone
#   ./deploy.sh zerone abc1234     # deploy specific commit
#   ./deploy.sh zerone HEAD~3      # rollback to 3 commits ago

HOST="${1:-zerone}"
REF="${2:-HEAD}"
REMOTE_GIT="/home/ubuntu/craw-stack"
REMOTE_DIR="$REMOTE_GIT/craw"
SSH_KEY="$(dirname "$0")/zerone"
SSH="ssh -i $SSH_KEY -o StrictHostKeyChecking=no ubuntu@$HOST"

# ── Load env ──
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
if [ -f "$SCRIPT_DIR/.env" ]; then
    set -a
    # shellcheck disable=SC1091
    source "$SCRIPT_DIR/.env"
    set +a
fi

: "${PG_PASSWORD:?PG_PASSWORD required — set in .env}"
: "${DB_HOST:?DB_HOST required — set in .env}"

TAG=$(git rev-parse --short "$REF")
REMOTE_DB="postgres://${POSTGRES_USER:-crawler}:${PG_PASSWORD}@${DB_HOST}:${PG_PORT:-5432}/${POSTGRES_DB:-craw}?sslmode=disable"

echo "=== Deploying $TAG to $HOST ==="
echo "    DB: ${DB_HOST}:${PG_PORT:-5432}"

# ── 1. Check GitHub auth on remote ──
echo "[1/6] Checking git access..."
if ! $SSH "cd $REMOTE_GIT && git ls-remote --exit-code origin HEAD" >/dev/null 2>&1; then
    echo "ERROR: Cannot reach GitHub from $HOST."
    echo "  Test: ssh $HOST 'cd $REMOTE_GIT && git fetch origin'"
    exit 1
fi

# ── 2. Fetch + checkout on remote ──
echo "[2/6] Updating source to $TAG..."
$SSH "cd $REMOTE_GIT && git fetch origin && git checkout $TAG"
$SSH "cd $REMOTE_GIT && git log --oneline -1"

# ── 3. Verify Docker Compose file exists ──
if ! $SSH "test -f $REMOTE_DIR/docker-compose.yml"; then
    echo "ERROR: docker-compose.yml not found at $REMOTE_DIR"
    exit 1
fi

# ── 4. Build images ──
echo "[3/6] Building images..."
$SSH "cd $REMOTE_DIR && GIT_COMMIT=$TAG docker compose build crawler dashboard"

# ── 5. Recreate containers ──
echo "[4/6] Recreating services..."
$SSH "cd $REMOTE_DIR && GIT_COMMIT=$TAG docker compose up -d --force-recreate crawler dashboard"

# ── 6. Run db-init.sql (idempotent) ──
echo "[5/6] Ensuring dashboard indexes..."
scp -i "$SSH_KEY" "$SCRIPT_DIR/dashboard/db-init.sql" "ubuntu@$HOST:/tmp/db-init.sql"
$SSH "docker run --rm --network host postgres:16 psql '$REMOTE_DB' -f /tmp/db-init.sql"

# ── 7. Verify ──
echo "[6/6] Verifying..."
sleep 5

echo "--- Services ---"
$SSH "docker ps --filter name=craw --format 'table {{.Names}}\t{{.Status}}'"

echo "--- Health ---"
HTTP_CODE=$($SSH "curl -s -o /dev/null -w '%{http_code}' http://localhost:${DASH_PORT:-3000}/api/health 2>/dev/null" || echo "000")
if [ "$HTTP_CODE" = "200" ]; then
    echo "Dashboard health: OK"
else
    echo "Dashboard health: FAILED (HTTP $HTTP_CODE)"
fi

echo "--- Crawler logs ---"
$SSH "docker logs --tail 5 craw-crawler 2>&1"

echo ""
echo "=== Deploy $TAG complete ==="
