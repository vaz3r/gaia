#!/usr/bin/env bash
set -euo pipefail

# Deploy crawler + dashboard to remote host via git.
# Usage: ./deploy.sh [host] [commit-ref]
#
# Examples:
#   ./deploy.sh                    # deploy HEAD to configured host
#   ./deploy.sh gaia abc1234       # deploy specific commit
#   ./deploy.sh gaia HEAD~3        # rollback to 3 commits ago

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
REF="${2:-HEAD}"
REMOTE_GIT="${DEPLOY_REMOTE_GIT:?DEPLOY_REMOTE_GIT required}"
REMOTE_COMPOSE="$REMOTE_GIT/deploy/compose"
SSH_KEY="${DEPLOY_SSH_KEY:?DEPLOY_SSH_KEY required}"
SSH_USER="${DEPLOY_USER:?DEPLOY_USER required}"
SSH="ssh -i $SSH_KEY -o StrictHostKeyChecking=no $SSH_USER@$HOST"

# ── Load env ──
if [ -f "$REPO_ROOT/.env" ]; then
    set -a
    # shellcheck disable=SC1091
    source "$REPO_ROOT/.env"
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

# ── 2b. Ensure data directories exist with correct ownership ──
REMOTE_DATA="${DEPLOY_REMOTE_DATA:?DEPLOY_REMOTE_DATA required}"
$SSH "sudo mkdir -p ${REMOTE_DATA}/crawler ${REMOTE_DATA}/logs && sudo chown -R 10001:10001 ${REMOTE_DATA}"

# Services to deploy (e.g. "crawler" alone on crawler-only hosts, or
# "crawler dashboard" where the dashboard also runs).
DEPLOY_SERVICES="${DEPLOY_SERVICES:-crawler dashboard log-shipper}"

# ── 3. Verify Docker Compose file exists ──
if ! $SSH "test -f $REMOTE_COMPOSE/docker-compose.yml"; then
    echo "ERROR: docker-compose.yml not found at $REMOTE_COMPOSE"
    exit 1
fi

# ── 4. Build images ──
echo "[3/6] Building images..."
$SSH "cd $REMOTE_COMPOSE && GIT_COMMIT=$TAG docker compose --env-file $REMOTE_GIT/.env build $DEPLOY_SERVICES"

# ── 5. Recreate containers ──
echo "[4/6] Recreating services..."
$SSH "cd $REMOTE_COMPOSE && GIT_COMMIT=$TAG docker compose --env-file $REMOTE_GIT/.env up -d --force-recreate $DEPLOY_SERVICES"

# ── 6. Run db-init.sql (idempotent, only when dashboard is deployed) ──
if [[ " $DEPLOY_SERVICES " == *" dashboard "* ]]; then
    echo "[5/6] Ensuring dashboard indexes..."
    scp -i "$SSH_KEY" "$REPO_ROOT/apps/dashboard/db-init.sql" "$SSH_USER@$HOST:/tmp/db-init.sql"
    $SSH "docker run --rm --network host -v /tmp/db-init.sql:/tmp/db-init.sql:ro postgres:16 psql '$REMOTE_DB' -f /tmp/db-init.sql"
else
    echo "[5/6] Skipping dashboard indexes (dashboard not in DEPLOY_SERVICES)..."
fi

# ── 7. Verify ──
echo "[6/6] Verifying..."
sleep 5

echo "--- Services ---"
$SSH "docker ps --filter name=gaia --format 'table {{.Names}}\t{{.Status}}'"

echo "--- Health ---"
HTTP_CODE=$($SSH "curl -s -o /dev/null -w '%{http_code}' http://localhost:${DASH_PORT:-3000}/api/health 2>/dev/null" || echo "000")
if [ "$HTTP_CODE" = "200" ]; then
    echo "Dashboard health: OK"
else
    echo "Dashboard health: FAILED (HTTP $HTTP_CODE)"
fi

echo "--- Crawler logs ---"
$SSH "docker logs --tail 5 gaia-crawler 2>&1"

echo ""
echo "=== Deploy $TAG complete ==="
