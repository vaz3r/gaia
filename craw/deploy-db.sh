#!/usr/bin/env bash
set -euo pipefail

# Deploy PostgreSQL config to workspace-production.
# Usage: ./deploy-db.sh [commit-ref]
#
# This script:
#   1. Backs up the database (pg_dump)
#   2. Copies postgresql.production.conf to workspace-production
#   3. Updates docker-compose.yml to mount the config (removes -c flags)
#   4. Recreates the craw-db container with new config
#   5. Verifies PG is healthy with correct settings

REMOTE_HOST="workspace-production"
SSH_USER="core"
REMOTE_DIR="/home/${SSH_USER}/craw-stack"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SSH="ssh -o StrictHostKeyChecking=no ${SSH_USER}@${REMOTE_HOST}"
REF="${1:-HEAD}"

echo "=== Deploying PostgreSQL config to $REMOTE_HOST ==="

# ── 1. Ensure remote directories exist ──
echo "[1/7] Setting up remote directories..."
$SSH "mkdir -p ${REMOTE_DIR}/pg"

# ── 2. Copy config file ──
echo "[2/7] Copying postgresql.production.conf..."
scp -o StrictHostKeyChecking=no "$SCRIPT_DIR/pg/postgresql.production.conf" \
    "${SSH_USER}@${REMOTE_HOST}:${REMOTE_DIR}/pg/postgresql.production.conf"

# ── 3. Backup current database ──
echo "[3/7] Backing up database (pg_dump)..."
BACKUP_FILE="backup-$(date +%Y%m%d-%H%M).sql"
$SSH "docker exec craw-db pg_dump -U crawler craw" > "$SCRIPT_DIR/$BACKUP_FILE"
BACKUP_SIZE=$(wc -c < "$SCRIPT_DIR/$BACKUP_FILE" | tr -d ' ')
echo "    Backup saved: $BACKUP_FILE ($BACKUP_SIZE bytes)"

# ── 4. Update docker-compose.yml on remote ──
echo "[4/7] Updating docker-compose.yml..."
$SSH "cat > ${REMOTE_DIR}/docker-compose.yml" <<'COMPOSE'
services:
  db:
    image: postgres:16
    container_name: craw-db
    shm_size: 1g
    environment:
      POSTGRES_USER: crawler
      POSTGRES_PASSWORD: ${PG_PASSWORD}
      POSTGRES_DB: craw
    ports:
      - "0.0.0.0:5432:5432"
    volumes:
      - pg-data:/var/lib/postgresql/data
      - ./pg/postgresql.production.conf:/etc/postgresql/production.conf
    restart: unless-stopped
    command: postgres -c config_file=/etc/postgresql/production.conf

volumes:
  pg-data:
COMPOSE

# ── 5. Ensure .env exists ──
echo "[5/7] Verifying .env..."
if ! $SSH "test -f ${REMOTE_DIR}/.env"; then
    echo "    ERROR: .env not found at ${REMOTE_DIR}/.env"
    exit 1
fi
echo "    .env OK"

# ── 6. Recreate container ──
echo "[6/7] Recreating craw-db container..."
$SSH "cd ${REMOTE_DIR} && docker compose up -d --force-recreate db"

echo "    Waiting for PostgreSQL to be ready..."
for i in $(seq 1 30); do
    if $SSH "docker exec craw-db pg_isready -U crawler -d craw" >/dev/null 2>&1; then
        echo "    PostgreSQL ready after ${i}s"
        break
    fi
    if [ "$i" -eq 30 ]; then
        echo "    ERROR: PostgreSQL did not become ready in 30s"
        $SSH "docker logs --tail 20 craw-db"
        exit 1
    fi
    sleep 1
done

# ── 7. Verify ──
echo "[7/7] Verifying config..."
echo ""
echo "--- Key Settings ---"
$SSH "docker exec craw-db psql -U crawler -d craw -c \"
SELECT name, setting, unit
FROM pg_settings
WHERE name IN (
    'shared_buffers', 'effective_cache_size', 'work_mem',
    'maintenance_work_mem', 'max_connections', 'wal_buffers',
    'max_wal_size', 'synchronous_commit', 'listen_addresses',
    'logging_collector', 'checkpoint_timeout'
)
ORDER BY name;\""

echo ""
echo "--- Database Size ---"
$SSH "docker exec craw-db psql -U crawler -d craw -t -c \"SELECT pg_size_pretty(pg_database_size('craw'));\""

echo ""
echo "--- Connection Count ---"
$SSH "docker exec craw-db psql -U crawler -d craw -t -c \"SELECT count(*) FROM pg_stat_activity;\""

echo ""
echo "--- Container Status ---"
$SSH "docker ps --filter name=craw-db --format 'table {{.Names}}\t{{.Status}}\t{{.Ports}}'"

echo ""
echo "=== PostgreSQL config deploy complete ==="
echo "    Backup: $SCRIPT_DIR/$BACKUP_FILE"
