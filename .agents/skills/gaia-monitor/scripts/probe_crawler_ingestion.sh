#!/usr/bin/env bash
# probe_crawler_ingestion.sh — Telemetry probe for crawler ingestion and torrent rates
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"

FORMAT="text"
if [[ "${1:-}" == "--json" ]]; then
    FORMAT="json"
fi

# Execute query against PostgreSQL
run_psql() {
    docker exec gaia-postgres psql -U crawler -d craw -t -A -F $'\t' -c "$1" 2>/dev/null || true
}

# 1. Total & Window Verification Counts
STATS_RAW=$(run_psql "
SELECT 
    count(*) AS total,
    count(*) FILTER (WHERE verified_at > now() - interval '1 hour') AS v1h,
    count(*) FILTER (WHERE verified_at > now() - interval '24 hours') AS v24h
FROM torrents;
")

TOTAL_TORRENTS=$(echo "$STATS_RAW" | awk -F'\t' '{print $1}')
VERIFIED_1H=$(echo "$STATS_RAW" | awk -F'\t' '{print $2}')
VERIFIED_24H=$(echo "$STATS_RAW" | awk -F'\t' '{print $3}')

TOTAL_TORRENTS=${TOTAL_TORRENTS:-0}
VERIFIED_1H=${VERIFIED_1H:-0}
VERIFIED_24H=${VERIFIED_24H:-0}

# Calculate 24h moving average per hour
if [ "$VERIFIED_24H" -gt 0 ]; then
    HOURLY_MOVING_AVG=$(( VERIFIED_24H / 24 ))
else
    HOURLY_MOVING_AVG=0
fi

# 2. Hourly breakdown for last 12 hours
HOURLY_HISTORY=$(run_psql "
SELECT 
    to_char(date_trunc('hour', verified_at), 'YYYY-MM-DD HH24:00') as hour_slot, 
    count(*) as count,
    round(count(*) / 60.0, 1) as avg_min
FROM torrents 
WHERE verified_at > now() - interval '12 hours' 
GROUP BY 1 
ORDER BY 1 DESC;
")

# 3. DHT & Wire metrics from metrics table
METRICS_RAW=$(run_psql "
WITH latest_ts AS (SELECT max(ts) AS mts FROM metrics)
SELECT metric_name, metric_value 
FROM metrics, latest_ts 
WHERE ts = latest_ts.mts 
  AND metric_name IN (
    'routing_table_len', 'unique_infohashes', 'fetch_attempts', 
    'outbound_queries', 'outbound_timeouts', 'inbound_get_peers',
    'inbound_announce_peer', 'send_dropped'
  );
")

ROUTING_LEN=0
FETCH_ATTEMPTS=0
UNIQUE_HASHES=0
OUTBOUND_QUERIES=0
OUTBOUND_TIMEOUTS=0
INBOUND_GET_PEERS=0
INBOUND_ANNOUNCE=0
SEND_DROPPED=0

while IFS=$'\t' read -r name val; do
    case "$name" in
        routing_table_len) ROUTING_LEN="$val" ;;
        fetch_attempts) FETCH_ATTEMPTS="$val" ;;
        unique_infohashes) UNIQUE_HASHES="$val" ;;
        outbound_queries) OUTBOUND_QUERIES="$val" ;;
        outbound_timeouts) OUTBOUND_TIMEOUTS="$val" ;;
        inbound_get_peers) INBOUND_GET_PEERS="$val" ;;
        inbound_announce_peer) INBOUND_ANNOUNCE="$val" ;;
        send_dropped) SEND_DROPPED="$val" ;;
    esac
done <<< "$METRICS_RAW"

# Compute BEP-9 Wire Metadata Success Rate
WIRE_SUCCESS_RATE="0.0"
if [ "$FETCH_ATTEMPTS" -gt 0 ] && [ "$UNIQUE_HASHES" -gt 0 ]; then
    WIRE_SUCCESS_RATE=$(awk -v h="$UNIQUE_HASHES" -v a="$FETCH_ATTEMPTS" 'BEGIN { printf "%.1f", (h / a) * 100 }')
fi

# Drop or anomaly indicator
INGESTION_ANOMALY="NONE"
if [ "$HOURLY_MOVING_AVG" -gt 0 ]; then
    # Compare verified_1h to moving average
    DIFF_PCT=$(awk -v v1="$VERIFIED_1H" -v ma="$HOURLY_MOVING_AVG" 'BEGIN { printf "%.1f", ((v1 - ma) / ma) * 100 }')
    if [ "$VERIFIED_1H" -eq 0 ]; then
        INGESTION_ANOMALY="CRITICAL_ZERO_INGESTION"
    else
        INGESTION_ANOMALY=$(awk -v diff="$DIFF_PCT" 'BEGIN {
            if (diff < -50.0) print "SEVERE_DROP";
            else if (diff < -30.0) print "MODERATE_DROP";
            else print "NONE";
        }')
    fi
else
    DIFF_PCT="0.0"
fi

if [[ "$FORMAT" == "json" ]]; then
    python3 -c "
import json, sys

hourly_rows = []
for line in '''$HOURLY_HISTORY'''.strip().split('\n'):
    if not line: continue
    parts = line.split('\t')
    if len(parts) >= 3:
        hourly_rows.append({'hour': parts[0], 'verified': int(parts[1]), 'avg_per_min': float(parts[2])})

res = {
    'total_torrents': int('$TOTAL_TORRENTS'),
    'verified_last_1h': int('$VERIFIED_1H'),
    'verified_last_24h': int('$VERIFIED_24H'),
    'hourly_moving_avg': int('$HOURLY_MOVING_AVG'),
    'rate_delta_pct': float('$DIFF_PCT'),
    'ingestion_anomaly': '$INGESTION_ANOMALY',
    'routing_table_len': int('$ROUTING_LEN'),
    'bep9_wire_success_rate': float('$WIRE_SUCCESS_RATE'),
    'fetch_attempts': int('$FETCH_ATTEMPTS'),
    'unique_infohashes': int('$UNIQUE_HASHES'),
    'outbound_queries': int('$OUTBOUND_QUERIES'),
    'outbound_timeouts': int('$OUTBOUND_TIMEOUTS'),
    'inbound_get_peers': int('$INBOUND_GET_PEERS'),
    'inbound_announce_peer': int('$INBOUND_ANNOUNCE'),
    'hourly_history': hourly_rows
}
print(json.dumps(res, indent=2))
"
else
    echo "=== CRAWLER INGESTION TELEMETRY ==="
    echo "Total Torrents: $TOTAL_TORRENTS"
    echo "Verified Last 1h: $VERIFIED_1H torrents/hr"
    echo "Verified Last 24h: $VERIFIED_24H torrents"
    echo "24h Hourly Moving Avg: $HOURLY_MOVING_AVG torrents/hr"
    echo "Rate Delta vs Baseline: $DIFF_PCT%"
    echo "Anomaly Status: $INGESTION_ANOMALY"
    echo "Routing Table Size: $ROUTING_LEN nodes"
    echo "BEP-9 Wire Success Rate: $WIRE_SUCCESS_RATE%"
    echo "Fetch Attempts: $FETCH_ATTEMPTS | Unique Infohashes: $UNIQUE_HASHES"
    echo "Inbound Queries: get_peers=$INBOUND_GET_PEERS, announce=$INBOUND_ANNOUNCE"
    echo ""
    echo "--- Last 12 Hours Throughput ---"
    printf "%-18s | %-12s | %-12s\n" "Hour Slot" "Verified" "Avg/Min"
    printf "%s\n" "------------------------------------------------"
    while IFS=$'\t' read -r h c m; do
        [ -n "$h" ] && printf "%-18s | %-12s | %-12s\n" "$h" "$c" "$m"
    done <<< "$HOURLY_HISTORY"
fi
