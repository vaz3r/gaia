#!/usr/bin/env bash
# A/B test: capture routing_nodes, unique/hr, verified/hr over a window
# to compare 8-instance vs 1-instance configs.
#
# Usage:
#   benchmark/instances-ab.sh [seconds] [compose-dir]
#
# Defaults: 3600s (1h), ./crawler compose dir.
# Samples every 30s (matching crawler's stats interval).
# Outputs a CSV time-series and a summary to stdout.
set -euo pipefail

WINDOW="${1:-3600}"
COMPOSE_DIR="${2:-$(cd "$(dirname "$0")/.." && pwd)/crawler}"
SAMPLE_INTERVAL=30

cd "$COMPOSE_DIR"

# Strip ANSI escape codes (tracing-subscriber color output)
strip_ansi() { sed 's/\x1b\[[0-9;]*m//g'; }

OUTFILE="/tmp/instances-ab-$(date +%Y%m%d-%H%M%S).csv"
echo "time_elapsed_s,routing_nodes,unique_per_hr,metadata_verified,instance_nodes" > "$OUTFILE"

START=$(date +%s)
ELAPSED=0
LAST_VERIFIED=0

echo "Sampling every ${SAMPLE_INTERVAL}s for ${WINDOW}s window..." >&2

while [ "$ELAPSED" -lt "$WINDOW" ]; do
    # Grab the most recent crawl stats line, strip ANSI codes
    STATS_LINE=$(docker compose logs crawler --since 30s 2>&1 \
        | strip_ansi | grep "crawl stats" | tail -1 || true)

    if [ -n "$STATS_LINE" ]; then
        ROUTING=$(echo "$STATS_LINE" | grep -oP 'routing_nodes\s*=\s*\K[0-9]+' || echo "0")
        UNIQUE_HR=$(echo "$STATS_LINE" | grep -oP 'unique_per_hr\s*=\s*"\K[0-9.]+' || echo "0")
        VERIFIED=$(echo "$STATS_LINE" | grep -oP 'metadata_verified\s*=\s*\K[0-9]+' || echo "0")
        INSTANCE_NODES=$(echo "$STATS_LINE" | grep -oP 'instance_nodes\s*=\s*"\K[^"]+' || echo "")
    else
        ROUTING=0
        UNIQUE_HR=0
        VERIFIED=$LAST_VERIFIED
        INSTANCE_NODES=""
    fi

    LAST_VERIFIED=$VERIFIED
    echo "${ELAPSED},${ROUTING},${UNIQUE_HR},${VERIFIED},${INSTANCE_NODES}" >> "$OUTFILE"

    # Print progress to stderr
    printf "\r[%3ds/%ds] routing=%-5s unique_hr=%-8s verified=%-6s" \
        "$ELAPSED" "$WINDOW" "$ROUTING" "$UNIQUE_HR" "$VERIFIED" >&2

    sleep "$SAMPLE_INTERVAL"
    ELAPSED=$(( $(date +%s) - START ))
done

echo "" >&2

# --- Summary ---
echo "" >&2
echo "=== SUMMARY ===" >&2

# Compute routing_nodes steady-state (last 20% of samples, skip first 80% warmup)
TOTAL_SAMPLES=$(tail -n +2 "$OUTFILE" | wc -l)
WARMUP_SAMPLES=$(( TOTAL_SAMPLES * 8 / 10 ))
if [ "$WARMUP_SAMPLES" -lt 1 ]; then WARMUP_SAMPLES=1; fi

# routing_nodes: peak and steady-state (last 20%)
echo "routing_nodes:" >&2
tail -n +2 "$OUTFILE" | tail -n +"$WARMUP_SAMPLES" | cut -d, -f2 | sort -n | awk '
    NR==1{min=$1} {sum+=$1; count++} END{
        printf "  steady-state: min=%d avg=%d (last %d samples)\n", min, sum/count, count
    }' >&2

# routing_nodes: peak across all samples
PEAK_ROUTING=$(tail -n +2 "$OUTFILE" | cut -d, -f2 | sort -n | tail -1)
echo "  peak: ${PEAK_ROUTING}" >&2

# verified/hr: take last sample's metadata_verified, compute delta from first
FIRST_VERIFIED=$(tail -n +2 "$OUTFILE" | head -1 | cut -d, -f4)
LAST_VERIFIED=$(tail -n +2 "$OUTFILE" | tail -1 | cut -d, -f4)
VERIFIED_DELTA=$(( LAST_VERIFIED - FIRST_VERIFIED ))
VERIFIED_PER_HR=$(awk -v d="$VERIFIED_DELTA" -v t="$WINDOW" 'BEGIN{printf "%.1f", d/t*3600}')
echo "verified/hr: ${VERIFIED_PER_HR} (${VERIFIED_DELTA} over ${WINDOW}s)" >&2

# unique/hr: average of last 20%
echo "unique/hr (steady-state):" >&2
tail -n +2 "$OUTFILE" | tail -n +"$WARMUP_SAMPLES" | cut -d, -f3 | awk '
    {sum+=$1; count++} END{
        if(count>0) printf "  avg=%.1f (last %d samples)\n", sum/count, count
        else print "  no data"
    }' >&2

# instance_nodes: last sample
LAST_INST=$(tail -n +2 "$OUTFILE" | tail -1 | cut -d, -f5)
echo "instance_nodes (last): ${LAST_INST}" >&2

echo "" >&2
echo "CSV saved: ${OUTFILE}" >&2
echo "=== END SUMMARY ===" >&2

# Also print summary to stdout for piping
cat <<EOF
routing_nodes_peak=${PEAK_ROUTING}
routing_nodes_steady_state=$(tail -n +2 "$OUTFILE" | tail -n +"$WARMUP_SAMPLES" | cut -d, -f2 | sort -n | awk 'NR==1{min=$1}{sum+=$1;c++}END{printf "%d", sum/c}')
verified_per_hr=${VERIFIED_PER_HR}
verified_delta=${VERIFIED_DELTA}
instance_nodes_last=${LAST_INST}
csv=${OUTFILE}
EOF
