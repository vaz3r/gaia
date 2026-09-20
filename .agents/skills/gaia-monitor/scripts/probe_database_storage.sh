#!/usr/bin/env bash
# probe_database_storage.sh — Telemetry probe for PostgreSQL, PgBouncer, Redis, and Backup
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"

FORMAT="text"
if [[ "${1:-}" == "--json" ]]; then
    FORMAT="json"
fi

run_psql() {
    docker exec gaia-postgres psql -U crawler -d craw -t -A -F $'\t' -c "$1" 2>/dev/null || true
}

# 1. PostgreSQL DB Health & Performance Metrics
PG_METRICS=$(run_psql "
WITH cache_stat AS (
    SELECT 
        round(sum(blks_hit) * 100.0 / greatest(sum(blks_hit + blks_read), 1), 2) AS cache_hit_pct,
        sum(xact_commit) AS commits,
        sum(xact_rollback) AS rollbacks
    FROM pg_stat_database 
    WHERE datname = 'craw'
),
conn_stat AS (
    SELECT 
        count(*) AS total_conns,
        count(*) FILTER (WHERE state = 'active') AS active_conns,
        count(*) FILTER (WHERE state = 'idle') AS idle_conns,
        count(*) FILTER (WHERE state = 'idle in transaction') AS idle_in_tx
    FROM pg_stat_activity
),
max_conn AS (
    SELECT setting::int AS max_conns FROM pg_settings WHERE name = 'max_connections'
),
db_sz AS (
    SELECT pg_size_pretty(pg_database_size('craw')) AS db_size, pg_database_size('craw') AS db_size_bytes
),
locks AS (
    SELECT count(*) AS waiting_locks FROM pg_locks WHERE NOT granted
)
SELECT 
    cache_hit_pct, commits, rollbacks, 
    total_conns, active_conns, idle_conns, idle_in_tx, 
    max_conns, db_size, db_size_bytes, waiting_locks
FROM cache_stat, conn_stat, max_conn, db_sz, locks;
")

CACHE_HIT_PCT="0.0"
COMMITS=0
ROLLBACKS=0
TOTAL_CONNS=0
ACTIVE_CONNS=0
IDLE_CONNS=0
IDLE_IN_TX=0
MAX_CONNS=100
DB_SIZE="0 MB"
DB_SIZE_BYTES=0
WAITING_LOCKS=0

if [ -n "$PG_METRICS" ]; then
    IFS=$'\t' read -r CACHE_HIT_PCT COMMITS ROLLBACKS TOTAL_CONNS ACTIVE_CONNS IDLE_CONNS IDLE_IN_TX MAX_CONNS DB_SIZE DB_SIZE_BYTES WAITING_LOCKS <<< "$PG_METRICS"
fi

# Slow queries (> 5 seconds)
SLOW_QUERIES_COUNT=$(run_psql "
SELECT count(*) 
FROM pg_stat_activity 
WHERE state = 'active' 
  AND pid != pg_backend_pid() 
  AND (now() - query_start) > interval '5 seconds';
")
SLOW_QUERIES_COUNT=${SLOW_QUERIES_COUNT:-0}

# Top 5 Tables by Disk Size
TOP_TABLES=$(run_psql "
SELECT 
    relname AS table_name,
    pg_size_pretty(pg_total_relation_size(c.oid)) AS total_size,
    reltuples::bigint AS approx_rows
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE relkind = 'r' AND nspname = 'public'
ORDER BY pg_total_relation_size(c.oid) DESC
LIMIT 5;
")

# 2. Redis Metrics
REDIS_INFO_MEM=$(docker exec gaia-redis redis-cli info memory 2>/dev/null || true)
REDIS_INFO_STATS=$(docker exec gaia-redis redis-cli info stats 2>/dev/null || true)
REDIS_USED_MEM=$(echo "$REDIS_INFO_MEM" | grep "used_memory_human:" | cut -d: -f2 | tr -d '\r')
REDIS_MAX_MEM=$(echo "$REDIS_INFO_MEM" | grep "maxmemory_human:" | cut -d: -f2 | tr -d '\r')
REDIS_OPS_SEC=$(echo "$REDIS_INFO_STATS" | grep "instantaneous_ops_per_sec:" | cut -d: -f2 | tr -d '\r')
REDIS_HITS=$(echo "$REDIS_INFO_STATS" | grep "keyspace_hits:" | cut -d: -f2 | tr -d '\r')
REDIS_MISSES=$(echo "$REDIS_INFO_STATS" | grep "keyspace_misses:" | cut -d: -f2 | tr -d '\r')
REDIS_EVICTED=$(echo "$REDIS_INFO_STATS" | grep "evicted_keys:" | cut -d: -f2 | tr -d '\r')

REDIS_USED_MEM=${REDIS_USED_MEM:-"N/A"}
REDIS_MAX_MEM=${REDIS_MAX_MEM:-"N/A"}
REDIS_OPS_SEC=${REDIS_OPS_SEC:-0}
REDIS_EVICTED=${REDIS_EVICTED:-0}
REDIS_HITS=${REDIS_HITS:-0}
REDIS_MISSES=${REDIS_MISSES:-0}

REDIS_HIT_RATE="0.0"
TOTAL_KEYS_OPS=$(( REDIS_HITS + REDIS_MISSES ))
if [ "$TOTAL_KEYS_OPS" -gt 0 ]; then
    REDIS_HIT_RATE=$(awk -v h="$REDIS_HITS" -v t="$TOTAL_KEYS_OPS" 'BEGIN { printf "%.1f", (h / t) * 100 }')
fi

# 3. Backup Status
BACKUP_LOGS=$(docker logs gaia-backup --tail 50 2>/dev/null || true)
LAST_BACKUP_STATUS="UNKNOWN"
LAST_BACKUP_TIME="NONE"
LAST_BACKUP_DURATION="N/A"

if echo "$BACKUP_LOGS" | grep -q "Backup process finished successfully"; then
    LAST_BACKUP_STATUS="SUCCESS"
    LAST_BACKUP_TIME=$(echo "$BACKUP_LOGS" | grep "Starting database backup" | tail -n 1 | grep -oE '\[[^]]+\]' | tr -d '[]' || echo "N/A")
fi

if [[ "$FORMAT" == "json" ]]; then
    python3 -c "
import json
tables = []
for line in '''$TOP_TABLES'''.strip().split('\n'):
    if not line: continue
    parts = line.split('\t')
    if len(parts) >= 3:
        tables.append({'table': parts[0], 'size': parts[1], 'rows': int(parts[2])})

res = {
    'postgresql': {
        'database_size': '$DB_SIZE',
        'database_size_bytes': int('$DB_SIZE_BYTES'),
        'buffer_cache_hit_ratio_pct': float('$CACHE_HIT_PCT'),
        'total_connections': int('$TOTAL_CONNS'),
        'active_connections': int('$ACTIVE_CONNS'),
        'idle_connections': int('$IDLE_CONNS'),
        'idle_in_transaction': int('$IDLE_IN_TX'),
        'max_connections': int('$MAX_CONNS'),
        'commits': int('$COMMITS'),
        'rollbacks': int('$ROLLBACKS'),
        'waiting_locks': int('$WAITING_LOCKS'),
        'slow_queries_active': int('$SLOW_QUERIES_COUNT'),
        'top_tables': tables
    },
    'redis': {
        'used_memory': '$REDIS_USED_MEM',
        'max_memory': '$REDIS_MAX_MEM',
        'ops_per_sec': int('$REDIS_OPS_SEC'),
        'hit_rate_pct': float('$REDIS_HIT_RATE'),
        'evicted_keys': int('$REDIS_EVICTED')
    },
    'backup': {
        'status': '$LAST_BACKUP_STATUS',
        'last_run_timestamp': '$LAST_BACKUP_TIME'
    }
}
print(json.dumps(res, indent=2))
"
else
    echo "=== DATABASE & STORAGE TELEMETRY ==="
    echo "PostgreSQL Master Size: $DB_SIZE"
    echo "Buffer Cache Hit Ratio: $CACHE_HIT_PCT% (SLA >= 99.0%)"
    echo "Connections: Active=$ACTIVE_CONNS | Idle=$IDLE_CONNS | InTX=$IDLE_IN_TX | Total=$TOTAL_CONNS/$MAX_CONNS"
    echo "Transactions: Commits=$COMMITS | Rollbacks=$ROLLBACKS"
    echo "Contention: Waiting Locks=$WAITING_LOCKS | Slow Queries (>5s)=$SLOW_QUERIES_COUNT"
    echo ""
    echo "--- Top 5 PostgreSQL Tables ---"
    printf "%-25s | %-12s | %-14s\n" "Table Name" "Disk Size" "Approx Rows"
    printf "%s\n" "--------------------------------------------------------"
    while IFS=$'\t' read -r t sz rows; do
        [ -n "$t" ] && printf "%-25s | %-12s | %-14s\n" "$t" "$sz" "$rows"
    done <<< "$TOP_TABLES"
    echo ""
    echo "--- Redis Cache Status ---"
    echo "Used Memory: $REDIS_USED_MEM / $REDIS_MAX_MEM | Ops/sec: $REDIS_OPS_SEC"
    echo "Hit Rate: $REDIS_HIT_RATE% | Evicted Keys: $REDIS_EVICTED"
    echo ""
    echo "--- Disaster Recovery Backup ---"
    echo "Last Backup Status: $LAST_BACKUP_STATUS"
    echo "Last Backup Timestamp: $LAST_BACKUP_TIME"
fi
