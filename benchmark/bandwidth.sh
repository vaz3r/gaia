#!/usr/bin/env bash
# Measure tunnel bandwidth through the Gluetun WireGuard interface (tun0).
#
# Usage:
#   benchmark/bandwidth.sh [seconds] [docker-compose-dir]
#
# Defaults: 600s window, ./crawler compose dir. Prints MB/s (in/out),
# GB/day and GB/month projections. Assumes `gluetun` is a running service.
set -euo pipefail

WINDOW="${1:-600}"
COMPOSE_DIR="${2:-$(cd "$(dirname "$0")/.." && pwd)/crawler}"
CONTAINER="gluetun"

cd "$COMPOSE_DIR"

read_tun() {
    docker exec "$CONTAINER" sh -c 'cat /proc/net/dev | grep tun0' 2>/dev/null \
        || { echo "ERROR: cannot read tun0 from $CONTAINER (is the stack up?)" >&2; exit 1; }
}

snapshot() {
    # /proc/net/dev fields: 1=face, 2=RX-bytes, 10=TX-bytes
    local line
    line="$(read_tun)"
    local rx_bytes tx_bytes
    rx_bytes="$(echo "$line" | tr -s ' ' | awk '{print $2}')"
    tx_bytes="$(echo "$line" | tr -s ' ' | awk '{print $10}')"
    echo "$rx_bytes $tx_bytes"
}

pre=( $(snapshot) )
echo "Sampling tunnel bandwidth for ${WINDOW}s..."
sleep "$WINDOW"
post=( $(snapshot) )

rx_delta=$(( post[0] - pre[0] ))
tx_delta=$(( post[1] - pre[1] ))

rx_mbps=$(awk -v b="$rx_delta" -v t="$WINDOW" 'BEGIN{printf "%.3f", b/t/1e6}')
tx_mbps=$(awk -v b="$tx_delta" -v t="$WINDOW" 'BEGIN{printf "%.3f", b/t/1e6}')
tot_mbps=$(awk -v a="$rx_mbps" -v b="$tx_mbps" 'BEGIN{printf "%.3f", a+b}')

GB=1073741824
rx_day=$(awk -v b="$rx_delta" -v t="$WINDOW" -v g="$GB" 'BEGIN{printf "%.1f", b/t*86400/g}')
tx_day=$(awk -v b="$tx_delta" -v t="$WINDOW" -v g="$GB" 'BEGIN{printf "%.1f", b/t*86400/g}')
rx_month=$(awk -v b="$rx_delta" -v t="$WINDOW" -v g="$GB" 'BEGIN{printf "%.1f", b/t*86400*30.4/g}')
tx_month=$(awk -v b="$tx_delta" -v t="$WINDOW" -v g="$GB" 'BEGIN{printf "%.1f", b/t*86400*30.4/g}')

cat <<EOF
=== Tunnel bandwidth ($WINDOW s window) ===
  Download (inbound / Oracle ingress): ${rx_mbps} MB/s
  Upload   (outbound / Oracle egress): ${tx_mbps} MB/s
  Total                               : ${tot_mbps} MB/s

=== Projections ===
                per day    per month (30.4d)
  Download:     ${rx_day} GB    ${rx_month} GB
  Upload:       ${tx_day} GB    ${tx_month} GB

Oracle Always Free: outbound 10 TB/month free (we use a few %); inbound free.
EOF
