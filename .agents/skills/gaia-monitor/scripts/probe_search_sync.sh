#!/usr/bin/env bash
# probe_search_sync.sh — Telemetry probe for Meilisearch & CDC Synchronizer on gaia-portal
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"

FORMAT="text"
if [[ "${1:-}" == "--json" ]]; then
    FORMAT="json"
fi

PORTAL_ENV="$REPO_ROOT/deploy/targets/gaia-portal/.env"
if [ ! -f "$PORTAL_ENV" ]; then
    echo "ERROR: $PORTAL_ENV not found" >&2
    exit 1
fi

set -a
# shellcheck disable=SC1090
source "$PORTAL_ENV"
set +a

# 1. Total Torrents in Master Postgres (for lag comparison)
PG_TOTAL_TORRENTS=$(docker exec gaia-postgres psql -U crawler -d craw -t -A -c "SELECT count(*) FROM torrents;" 2>/dev/null || echo "0")
PG_TOTAL_TORRENTS=${PG_TOTAL_TORRENTS:-0}

# 2. Remote execution on gaia-portal: query Meilisearch and sync logs in a single SSH session
REMOTE_OUTPUT=$(sshpass -p "$DEPLOY_PASSWORD" ssh -o StrictHostKeyChecking=no -o PreferredAuthentications=password -o PubkeyAuthentication=no -o ConnectTimeout=5 "$DEPLOY_USER@$DEPLOY_HOST" '
IP=$(docker inspect gaia-portal-meilisearch --format "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}" 2>/dev/null || echo "127.0.0.1")
KEY=$(docker inspect gaia-portal-meilisearch --format "{{range .Config.Env}}{{println .}}{{end}}" 2>/dev/null | grep MEILI_MASTER_KEY | cut -d= -f2- || echo "")

STATS=$(curl -s -m 3 -H "Authorization: Bearer $KEY" "http://$IP:7700/indexes/torrents/stats" 2>/dev/null || echo "{}")
HEALTH=$(curl -s -m 3 "http://$IP:7700/health" 2>/dev/null || echo "{}")
SYNC_LOGS=$(docker logs gaia-portal-sync --tail 50 2>&1 || true)

echo "=== MEILI_STATS ==="
echo "$STATS"
echo "=== MEILI_HEALTH ==="
echo "$HEALTH"
echo "=== SYNC_LOGS ==="
echo "$SYNC_LOGS"
' 2>/dev/null || true)

# 3. Parse with Python
PARSED_JSON=$(python3 -c "
import json, re, sys

raw = '''$REMOTE_OUTPUT'''
pg_total = int('$PG_TOTAL_TORRENTS')

stats_part = raw.split('=== MEILI_STATS ===')[1].split('=== MEILI_HEALTH ===')[0].strip() if '=== MEILI_STATS ===' in raw else '{}'
health_part = raw.split('=== MEILI_HEALTH ===')[1].split('=== SYNC_LOGS ===')[0].strip() if '=== MEILI_HEALTH ===' in raw else '{}'
logs_part = raw.split('=== SYNC_LOGS ===')[1].strip() if '=== SYNC_LOGS ===' in raw else ''

try:
    s = json.loads(stats_part)
    docs = int(s.get('numberOfDocuments', 0))
    is_indexing = bool(s.get('isIndexing', False))
except Exception:
    docs = 0
    is_indexing = False

try:
    h = json.loads(health_part)
    status = h.get('status', 'available' if docs > 0 else 'offline')
except Exception:
    status = 'available' if docs > 0 else 'offline'

lag = max(0, pg_total - docs)

# Sync log parsing
swap_match = re.findall(r'REBUILD & ATOMIC SWAP COMPLETE in ([\d\.]+)s!\s*\((\d+[\d,]*)\s*docs live\)', logs_part)
last_duration = f'{swap_match[-1][0]}s' if swap_match else 'N/A'
live_docs = swap_match[-1][1] if swap_match else 'N/A'

age_match = re.findall(r'last rebuild was (\d+)m ago', logs_part)
last_age = int(age_match[-1]) if age_match else -1

rate_match = re.findall(r'Streamed\s+[\d,]+\s+records\s+\((\d+)\s+docs/sec\)', logs_part)
last_rate = int(rate_match[-1]) if rate_match else 0

sync_status = 'HEALTHY'
if last_age > 150:
    sync_status = 'OVERDUE'
elif 'error' in logs_part.lower() or 'exception' in logs_part.lower():
    sync_status = 'ERROR'

res = {
    'meilisearch': {
        'status': status,
        'indexed_documents': docs,
        'is_indexing': is_indexing,
        'postgres_total_torrents': pg_total,
        'replication_lag_documents': lag
    },
    'sync_worker': {
        'status': sync_status,
        'last_rebuild_age_minutes': last_age,
        'last_rebuild_duration': last_duration,
        'live_docs_at_swap': live_docs,
        'streaming_rate_docs_per_sec': last_rate
    }
}
print(json.dumps(res, indent=2))
")

if [[ "$FORMAT" == "json" ]]; then
    echo "$PARSED_JSON"
else
    echo "=== MEILISEARCH & SEARCH REPLICATION TELEMETRY ==="
    MEILI_STATUS=$(echo "$PARSED_JSON" | python3 -c "import json, sys; print(json.load(sys.stdin)['meilisearch']['status'])")
    MEILI_DOCS=$(echo "$PARSED_JSON" | python3 -c "import json, sys; print(json.load(sys.stdin)['meilisearch']['indexed_documents'])")
    SYNC_LAG=$(echo "$PARSED_JSON" | python3 -c "import json, sys; print(json.load(sys.stdin)['meilisearch']['replication_lag_documents'])")
    IS_INDEXING=$(echo "$PARSED_JSON" | python3 -c "import json, sys; print(json.load(sys.stdin)['meilisearch']['is_indexing'])")
    
    SYNC_STATUS=$(echo "$PARSED_JSON" | python3 -c "import json, sys; print(json.load(sys.stdin)['sync_worker']['status'])")
    LAST_AGE=$(echo "$PARSED_JSON" | python3 -c "import json, sys; print(json.load(sys.stdin)['sync_worker']['last_rebuild_age_minutes'])")
    LAST_DUR=$(echo "$PARSED_JSON" | python3 -c "import json, sys; print(json.load(sys.stdin)['sync_worker']['last_rebuild_duration'])")
    LIVE_DOCS=$(echo "$PARSED_JSON" | python3 -c "import json, sys; print(json.load(sys.stdin)['sync_worker']['live_docs_at_swap'])")
    DOCS_SEC=$(echo "$PARSED_JSON" | python3 -c "import json, sys; print(json.load(sys.stdin)['sync_worker']['streaming_rate_docs_per_sec'])")

    echo "Meilisearch Engine Status: $MEILI_STATUS"
    echo "Indexed Documents in Meili: $MEILI_DOCS"
    echo "Master Postgres Torrents: $PG_TOTAL_TORRENTS"
    echo "Replication Sync Lag: $SYNC_LAG documents"
    echo "Index Is Building: $IS_INDEXING"
    echo ""
    echo "--- CDC Synchronizer Performance ---"
    echo "Sync Worker Status: $SYNC_STATUS"
    echo "Last Full Rebuild: $LAST_AGE mins ago (Target <= 120m)"
    echo "Last Swap Duration: $LAST_DUR (Live docs: $LIVE_DOCS)"
    echo "CDC Streaming Throughput: $DOCS_SEC docs/sec"
fi
