#!/usr/bin/env bash
# Full benchmark: capture crawl stats, tunnel bandwidth, torrent discovery
# rate, and torrents-per-GB efficiency in one window.
#
# Usage:
#   benchmark/bench.sh [seconds] [docker-compose-dir]
#
# Defaults: 600s window, ./crawler compose dir. Requires the stack to be
# up (gluetun + crawler + redis). Prints a report suitable for comparing
# crawler configs against the original baseline (~27.8k torrents/GB).
set -euo pipefail

WINDOW="${1:-600}"
COMPOSE_DIR="${2:-$(cd "$(dirname "$0")/.." && pwd)/crawler}"
BENCH_DIR="$(cd "$(dirname "$0")" && pwd)"

cd "$COMPOSE_DIR"

# --- Pre-window snapshots ---
echo "Capturing baseline (${WINDOW}s window)..."
docker cp crawler:/data/crawler.sqlite /tmp/bench_pre.sqlite >/dev/null 2>&1
pre_rx=$(docker exec gluetun sh -c 'cat /proc/net/dev | grep tun0' | tr -s ' ' | awk '{print $2}')
pre_tx=$(docker exec gluetun sh -c 'cat /proc/net/dev | grep tun0' | tr -s ' ' | awk '{print $10}')
pre_count=$(python3 -c "import sqlite3; print(sqlite3.connect('file:/tmp/bench_pre.sqlite?mode=ro',uri=True).execute('SELECT COUNT(*) FROM torrents').fetchone()[0])")
pre_stats=$(docker compose logs crawler --since 1m 2>&1 | grep "crawl stats" | tail -1)

sleep "$WINDOW"

# --- Post-window snapshots ---
docker cp crawler:/data/crawler.sqlite /tmp/bench_post.sqlite >/dev/null 2>&1
post_rx=$(docker exec gluetun sh -c 'cat /proc/net/dev | grep tun0' | tr -s ' ' | awk '{print $2}')
post_tx=$(docker exec gluetun sh -c 'cat /proc/net/dev | grep tun0' | tr -s ' ' | awk '{print $10}')
post_count=$(python3 -c "import sqlite3; print(sqlite3.connect('file:/tmp/bench_post.sqlite?mode=ro',uri=True).execute('SELECT COUNT(*) FROM torrents').fetchone()[0])")
post_stats=$(docker compose logs crawler --since 1m 2>&1 | grep "crawl stats" | tail -1)

# --- Compute ---
GB=1073741824
torrents=$(( post_count - pre_count ))
per_hr=$(awk -v d="$torrents" -v t="$WINDOW" 'BEGIN{printf "%.1f", d/t*3600}')
rx_mbps=$(awk -v b=$((post_rx - pre_rx)) -v t="$WINDOW" 'BEGIN{printf "%.3f", b/t/1e6}')
tx_mbps=$(awk -v b=$((post_tx - pre_tx)) -v t="$WINDOW" 'BEGIN{printf "%.3f", b/t/1e6}')
tot_mbps=$(awk -v a="$rx_mbps" -v b="$tx_mbps" 'BEGIN{printf "%.3f", a+b}')
gb_in=$(awk -v b=$((post_rx - pre_rx)) -v g="$GB" 'BEGIN{printf "%.3f", b/g}')
per_gb=$(awk -v d="$torrents" -v g="$gb_in" 'BEGIN{ if (g>0) printf "%.0f", d/g; else print "n/a" }')

cat <<EOF

======================== BENCHMARK REPORT ($WINDOW s) =========================

  Torrents found  : $torrents  (${per_hr}/hr = $(awk -v h="$per_hr" 'BEGIN{printf "%.0f", h*24}')/day)
  Bandwidth       : ${rx_mbps} MB/s down, ${tx_mbps} MB/s up (${tot_mbps} total)
  Efficiency      : ${per_gb} torrents/GB  [baseline ~27.8k]

  ---- pre-window stats ----
  $pre_stats
  ---- post-window stats ----
  $post_stats

=============================== END REPORT ====================================
EOF
