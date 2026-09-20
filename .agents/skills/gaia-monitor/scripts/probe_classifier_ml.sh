#!/usr/bin/env bash
# probe_classifier_ml.sh — Telemetry probe for Classification & ML Pipelines
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

# 1. Database Classification Stats
DB_STATS=$(run_psql "
SELECT 
    count(*) FILTER (WHERE category IS NULL) AS unclassified,
    count(*) FILTER (WHERE category IS NOT NULL) AS classified,
    count(*) FILTER (WHERE health_score IS NOT NULL) AS scored
FROM torrents;
")

UNCLASSIFIED=$(echo "$DB_STATS" | awk -F'\t' '{print $1}')
CLASSIFIED=$(echo "$DB_STATS" | awk -F'\t' '{print $2}')
SCORED=$(echo "$DB_STATS" | awk -F'\t' '{print $3}')

UNCLASSIFIED=${UNCLASSIFIED:-0}
CLASSIFIED=${CLASSIFIED:-0}
SCORED=${SCORED:-0}

# Category Breakdown
CATEGORY_BREAKDOWN=$(run_psql "
SELECT coalesce(category, 'UNCLASSIFIED') as cat, count(*), round(avg(category_confidence)::numeric, 3) 
FROM torrents 
GROUP BY 1 
ORDER BY count(*) DESC 
LIMIT 10;
")

# 2. Extract Classifier Timing & Batch Metrics from logs
# Example log: [10:58:44] Batch: 2000 | Rate:      2 rec/s | Accepted: 1227 ( 61.4%) | Flagged:  773 ( 38.6%) | Timing: fetch 147.67s, infer 1115.14s, db 31.04s
CLASSIFIER_LOG_LINE=$(docker logs gaia-classifier --tail 100 2>/dev/null | grep "Timing:" | tail -n 1 || true)

BATCH_SIZE=0
RATE_REC_SEC=0
ACCEPTED_PCT="0.0"
FLAGGED_PCT="0.0"
TIMING_FETCH="0.0"
TIMING_INFER="0.0"
TIMING_DB="0.0"
DB_TIME_RATIO="0.0"
INFER_TIME_RATIO="0.0"
CLASSIFIER_BOTTLENECK="NONE"

if [ -n "$CLASSIFIER_LOG_LINE" ]; then
    # Extract metrics using python regex
    PARSED_METRICS=$(python3 -c "
import re, sys
line = '''$CLASSIFIER_LOG_LINE'''
batch = re.search(r'Batch:\s*(\d+)', line)
rate = re.search(r'Rate:\s*(\d+)\s*rec/s', line)
acc = re.search(r'Accepted:\s*\d+\s*\(\s*([\d\.]+)%\)', line)
flg = re.search(r'Flagged:\s*\d+\s*\(\s*([\d\.]+)%\)', line)
fetch = re.search(r'fetch\s+([\d\.]+)s', line)
infer = re.search(r'infer\s+([\d\.]+)s', line)
db = re.search(r'db\s+([\d\.]+)s', line)

b = batch.group(1) if batch else '0'
r = rate.group(1) if rate else '0'
ap = acc.group(1) if acc else '0.0'
fp = flg.group(1) if flg else '0.0'
f = float(fetch.group(1)) if fetch else 0.0
i = float(infer.group(1)) if infer else 0.0
d = float(db.group(1)) if db else 0.0

total = f + i + d
db_ratio = (d / total * 100.0) if total > 0 else 0.0
infer_ratio = (i / total * 100.0) if total > 0 else 0.0

bottleneck = 'NONE'
if total > 0:
    if db_ratio > 50.0:
        bottleneck = 'DB_WRITE_LOCK_CONTENTION'
    elif infer_ratio > 70.0 and float(r) < 10.0:
        bottleneck = 'CPU_INFERENCE_SATURATION'
    elif f > 30.0:
        bottleneck = 'DB_FETCH_LATENCY'

print(f'{b}\t{r}\t{ap}\t{fp}\t{f:.2f}\t{i:.2f}\t{d:.2f}\t{db_ratio:.1f}\t{infer_ratio:.1f}\t{bottleneck}')
")
    IFS=$'\t' read -r BATCH_SIZE RATE_REC_SEC ACCEPTED_PCT FLAGGED_PCT TIMING_FETCH TIMING_INFER TIMING_DB DB_TIME_RATIO INFER_TIME_RATIO CLASSIFIER_BOTTLENECK <<< "$PARSED_METRICS"
fi

# 3. Extract ML Anomaly & Scoring Metrics
# Example: [anomalies] 2026-09-20 11:00:42,597 [INFO] Window: 2026-09-20 10:00:00+00:00 | Score: 0.244 | Severity: NORMAL | Incident: NORMAL
# Example: [scoring] INFO:scoring-worker:Scored 500 torrents (ALLOW=488, REVIEW/DOWN=12, SUPPRESS=0)
ML_LOGS=$(docker logs gaia-ml --tail 100 2>/dev/null || true)

ML_ANOMALY_WINDOW=""
ML_ANOMALY_SCORE="0.000"
ML_ANOMALY_SEVERITY="NORMAL"
ML_RECENT_SCORED=0
ML_RECENT_REFRESHED=0

ML_PARSED=$(python3 -c "
import re, sys
logs = '''$ML_LOGS'''

# Find latest anomaly log
anomaly_matches = re.findall(r'Window:\s*([^\|]+)\|\s*Score:\s*([\d\.]+)\s*\|\s*Severity:\s*([^\|]+)\|\s*Incident:\s*(\w+)', logs)
window = ''
score = '0.000'
sev = 'UNKNOWN'
if anomaly_matches:
    last = anomaly_matches[-1]
    window = last[0].strip()
    score = last[1].strip()
    sev = last[2].strip()

# Find latest scoring log
scored = re.findall(r'Scored\s+(\d+)\s+torrents', logs)
refreshed = re.findall(r'Refreshed\s+(\d+)\s+torrents', logs)
s_cnt = sum(int(x) for x in scored[-5:]) if scored else 0
r_cnt = sum(int(x) for x in refreshed[-5:]) if refreshed else 0

print(f'{window}\t{score}\t{sev}\t{s_cnt}\t{r_cnt}')
")
IFS=$'\t' read -r ML_ANOMALY_WINDOW ML_ANOMALY_SCORE ML_ANOMALY_SEVERITY ML_RECENT_SCORED ML_RECENT_REFRESHED <<< "$ML_PARSED"

if [[ "$FORMAT" == "json" ]]; then
    python3 -c "
import json
cats = []
for line in '''$CATEGORY_BREAKDOWN'''.strip().split('\n'):
    if not line: continue
    parts = line.split('\t')
    if len(parts) >= 3:
        cats.append({'category': parts[0], 'count': int(parts[1]), 'avg_confidence': float(parts[2]) if parts[2] else None})

res = {
    'unclassified_backlog': int('$UNCLASSIFIED'),
    'classified_total': int('$CLASSIFIED'),
    'scored_total': int('$SCORED'),
    'classifier_batch': {
        'size': int('$BATCH_SIZE'),
        'rate_rec_sec': int('$RATE_REC_SEC'),
        'accepted_pct': float('$ACCEPTED_PCT'),
        'flagged_pct': float('$FLAGGED_PCT'),
        'timing_fetch_sec': float('$TIMING_FETCH'),
        'timing_infer_sec': float('$TIMING_INFER'),
        'timing_db_sec': float('$TIMING_DB'),
        'db_time_ratio_pct': float('$DB_TIME_RATIO'),
        'infer_time_ratio_pct': float('$INFER_TIME_RATIO'),
        'bottleneck': '$CLASSIFIER_BOTTLENECK'
    },
    'ml_intelligence': {
        'anomaly_window': '$ML_ANOMALY_WINDOW',
        'anomaly_score': float('$ML_ANOMALY_SCORE'),
        'anomaly_severity': '$ML_ANOMALY_SEVERITY',
        'recent_scored_batch_total': int('$ML_RECENT_SCORED'),
        'recent_refreshed_batch_total': int('$ML_RECENT_REFRESHED')
    },
    'categories': cats
}
print(json.dumps(res, indent=2))
"
else
    echo "=== CLASSIFIER & ML INTELLIGENCE TELEMETRY ==="
    echo "Unclassified Backlog Queue: $UNCLASSIFIED torrents"
    echo "Classified Torrents Total: $CLASSIFIED"
    echo "Risk/Quality Scored Total: $SCORED"
    echo ""
    echo "--- Classifier Worker Performance ---"
    echo "Latest Batch: $BATCH_SIZE records | Rate: $RATE_REC_SEC rec/s"
    echo "Batch Verdict: Accepted $ACCEPTED_PCT% | Flagged $FLAGGED_PCT%"
    echo "Timing Breakdown: Fetch ${TIMING_FETCH}s | Infer ${TIMING_INFER}s | DB Commit ${TIMING_DB}s"
    echo "Timing Proportions: DB Commit ${DB_TIME_RATIO}% | Infer ${INFER_TIME_RATIO}%"
    echo "Classifier Bottleneck: $CLASSIFIER_BOTTLENECK"
    echo ""
    echo "--- ML Worker & Anomaly Intelligence ---"
    echo "Anomaly Evaluation Window: $ML_ANOMALY_WINDOW"
    echo "Anomaly Score: $ML_ANOMALY_SCORE | Severity: $ML_ANOMALY_SEVERITY"
    echo "Scoring Activity (last 5 batches): Scored $ML_RECENT_SCORED | Refreshed $ML_RECENT_REFRESHED"
    echo ""
    echo "--- Top Categories in System ---"
    printf "%-20s | %-12s | %-10s\n" "Category" "Count" "Avg Conf"
    printf "%s\n" "-----------------------------------------------"
    while IFS=$'\t' read -r c cnt conf; do
        [ -n "$c" ] && printf "%-20s | %-12s | %-10s\n" "$c" "$cnt" "${conf:-N/A}"
    done <<< "$CATEGORY_BREAKDOWN"
fi
