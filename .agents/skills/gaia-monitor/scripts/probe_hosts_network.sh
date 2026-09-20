#!/usr/bin/env bash
# probe_hosts_network.sh — Probes host resource metrics, tunnels, and OpSec compliance across GAIA targets
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"

FORMAT="text"
if [[ "${1:-}" == "--json" ]]; then
    FORMAT="json"
fi

HOSTS_TMP=$(mktemp)
NETWORK_TMP=$(mktemp)
trap 'rm -f "$HOSTS_TMP" "$NETWORK_TMP"' EXIT

# ── 1. Host Resources Probe ──
# Probes local host (workspace-production)
probe_local_host() {
    local target="workspace-production"
    local hostname=$(hostname)
    local uptime_str=$(uptime -p 2>/dev/null || uptime | sed 's/.*up \([^,]*\), .*/\1/')
    local load=$(cat /proc/loadavg | awk '{print $1" "$2" "$3}')
    local mem_raw=$(free -m | awk 'NR==2{print $2"\t"$3"\t"$7}')
    local swap_raw=$(free -m | awk 'NR==3{print $2"\t"$3}')
    local disk_root=$(df -hP / | awk 'NR==2{print $2"\t"$3"\t"$4"\t"$5}')
    local disk_data=$(df -hP /home/core/gaia-data 2>/dev/null | awk 'NR==2{print $2"\t"$3"\t"$4"\t"$5}' || echo "$disk_root")

    printf "%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n" \
        "$target" "$hostname" "$uptime_str" "$load" "$mem_raw" "$swap_raw" "$disk_root" "$disk_data" >> "$HOSTS_TMP"
}

# Probes remote targets
probe_remote_host() {
    local target="$1"
    local env_file="$REPO_ROOT/deploy/targets/$target/.env"
    [ -f "$env_file" ] || return 0

    local DEPLOY_HOST DEPLOY_USER DEPLOY_PASSWORD DEPLOY_SSH_KEY
    DEPLOY_HOST=$(grep -E '^DEPLOY_HOST=' "$env_file" | head -1 | cut -d= -f2- | tr -d '"'\'' ')
    DEPLOY_USER=$(grep -E '^DEPLOY_USER=' "$env_file" | head -1 | cut -d= -f2- | tr -d '"'\'' ')
    DEPLOY_PASSWORD=$(grep -E '^DEPLOY_PASSWORD=' "$env_file" | head -1 | cut -d= -f2- | tr -d '"'\'' ' || true)
    DEPLOY_SSH_KEY=$(grep -E '^DEPLOY_SSH_KEY=' "$env_file" | head -1 | cut -d= -f2- | tr -d '"'\'' ' || true)

    local SSH_CMD=""
    if [ -n "$DEPLOY_PASSWORD" ]; then
        SSH_CMD="sshpass -p $DEPLOY_PASSWORD ssh -o StrictHostKeyChecking=no -o PreferredAuthentications=password -o PubkeyAuthentication=no -o ConnectTimeout=5 $DEPLOY_USER@$DEPLOY_HOST"
    elif [ -n "$DEPLOY_SSH_KEY" ]; then
        SSH_CMD="ssh -i $DEPLOY_SSH_KEY -o StrictHostKeyChecking=no -o ConnectTimeout=5 $DEPLOY_USER@$DEPLOY_HOST"
    else
        SSH_CMD="ssh -o StrictHostKeyChecking=no -o ConnectTimeout=5 $DEPLOY_USER@$DEPLOY_HOST"
    fi

    local remote_script='
    h=$(hostname 2>/dev/null || echo "unknown")
    u=$(uptime -p 2>/dev/null || uptime | sed "s/.*up \([^,]*\), .*/\1/")
    l=$(cat /proc/loadavg 2>/dev/null | awk "{print \$1\" \"\$2\" \"\$3}" || echo "0 0 0")
    m=$(free -m 2>/dev/null | awk "NR==2{print \$2\"\t\"\$3\"\t\"\$7}" || echo "0\t0\t0")
    s=$(free -m 2>/dev/null | awk "NR==3{print \$2\"\t\"\$3}" || echo "0\t0")
    dr=$(df -hP / 2>/dev/null | awk "NR==2{print \$2\"\t\"\$3\"\t\"\$4\"\t\"\$5}" || echo "0\t0\t0\t0%")
    printf "%s\t%s\t%s\t%s\t%s\t%s\n" "$h" "$u" "$l" "$m" "$s" "$dr"
    '

    local out
    out=$($SSH_CMD "$remote_script" 2>/dev/null || true)
    if [ -n "$out" ]; then
        printf "%s\t%s\t%s\n" "$target" "$out" "$out" >> "$HOSTS_TMP"
    fi
}

probe_local_host
probe_remote_host "gaia-gateway"
probe_remote_host "gaia-portal"
probe_remote_host "gaia-node"

# ── 2. Network Tunnels & OpSec Probe ──
# Local WireGuard client handshake to gateway
WG_HANDSHAKE_TS=$(docker exec gaia-wireguard-client wg show wg0 latest-handshakes 2>/dev/null | awk '{print $2}' || echo "0")
NOW_TS=$(date +%s)
WG_HANDSHAKE_AGE=$(( NOW_TS - WG_HANDSHAKE_TS ))
if [ "$WG_HANDSHAKE_TS" -eq 0 ] || [ "$WG_HANDSHAKE_AGE" -lt 0 ]; then
    WG_HANDSHAKE_AGE=9999
fi

# Tunnel Ping to Gateway (10.99.0.1)
PING_RAW=$(docker exec gaia-wireguard-client ping -c 2 -W 2 10.99.0.1 2>/dev/null || true)
TUNNEL_LATENCY="N/A"
TUNNEL_PACKET_LOSS="100%"
if echo "$PING_RAW" | grep -q "min/avg/max"; then
    TUNNEL_LATENCY=$(echo "$PING_RAW" | grep "min/avg/max" | awk -F'/' '{print $5" ms"}')
    TUNNEL_PACKET_LOSS=$(echo "$PING_RAW" | grep -oE '[0-9]+% packet loss')
fi

# Portal WireGuard Tunnel Ping
PORTAL_ENV="$REPO_ROOT/deploy/targets/gaia-portal/.env"
PORTAL_TUNNEL_LATENCY="N/A"
if [ -f "$PORTAL_ENV" ]; then
    set -a; source "$PORTAL_ENV"; set +a
    PORTAL_PING=$(sshpass -p "$DEPLOY_PASSWORD" ssh -o StrictHostKeyChecking=no -o PreferredAuthentications=password -o PubkeyAuthentication=no -o ConnectTimeout=3 "$DEPLOY_USER@$DEPLOY_HOST" "docker exec gaia-portal-wireguard-client ping -c 2 -W 2 10.99.0.1" 2>/dev/null || true)
    if echo "$PORTAL_PING" | grep -q "min/avg/max"; then
        PORTAL_TUNNEL_LATENCY=$(echo "$PORTAL_PING" | grep "min/avg/max" | awk -F'/' '{print $5" ms"}')
    fi
fi

# OpSec Checks: DNS-over-TLS & Egress Killswitch
DOT_STATUS="DISABLED"
if resolvectl status 2>/dev/null | grep -F -q '+DNSOverTLS'; then
    DOT_STATUS="ACTIVE (+DNSOverTLS)"
fi

EGRESS_KILLSWITCH="ACTIVE"
# If outbound HTTP to 1.1.1.1 succeeds, killswitch is leaking
if curl -s -m 2 http://1.1.1.1 >/dev/null 2>&1; then
    EGRESS_KILLSWITCH="LEAK_DETECTED (Port 80 reachable)"
fi

if [[ "$FORMAT" == "json" ]]; then
    python3 -c "
import json

hosts = []
with open('$HOSTS_TMP') as f:
    for line in f:
        line = line.strip()
        if not line: continue
        p = line.split('\t')
        if len(p) >= 8:
            load_p = p[3].split()
            hosts.append({
                'target': p[0],
                'hostname': p[1],
                'uptime': p[2],
                'load_avg': {'1m': float(load_p[0]), '5m': float(load_p[1]), '15m': float(load_p[2])} if len(load_p)>=3 else {},
                'ram_mb': {'total': int(p[4]), 'used': int(p[5]), 'available': int(p[6])},
                'swap_mb': {'total': int(p[7]), 'used': int(p[8])} if len(p)>=9 else {},
                'disk_root': p[9] if len(p)>=10 else 'N/A'
            })

res = {
    'hosts': hosts,
    'network_tunnels': {
        'core_to_gateway_wg_age_sec': int('$WG_HANDSHAKE_AGE'),
        'core_tunnel_ping_latency': '$TUNNEL_LATENCY',
        'core_tunnel_packet_loss': '$TUNNEL_PACKET_LOSS',
        'portal_tunnel_ping_latency': '$PORTAL_TUNNEL_LATENCY'
    },
    'opsec_compliance': {
        'dns_over_tls': '$DOT_STATUS',
        'egress_killswitch': '$EGRESS_KILLSWITCH'
    }
}
print(json.dumps(res, indent=2))
"
else
    echo "=== HOST INFRASTRUCTURE & RESOURCE TELEMETRY ==="
    printf "%-22s | %-16s | %-18s | %-14s | %-14s | %-12s\n" "Target" "Hostname" "Uptime" "Load Avg (15m)" "RAM Avail" "Root Disk"
    printf "%s\n" "---------------------------------------------------------------------------------------------------------------"
    while IFS=$'\t' read -r target h u l m_tot m_used m_avail s_tot s_used d_sz d_used d_avail d_pct rest; do
        load15=$(echo "$l" | awk '{print $3}')
        printf "%-22s | %-16s | %-18s | %-14s | %-14s | %-12s\n" "$target" "$h" "$u" "${load15:-N/A}" "${m_avail:-0}MB / ${m_tot:-0}MB" "${d_pct:-N/A}"
    done < "$HOSTS_TMP"
    echo ""
    echo "--- Encrypted Tunnels & OpSec Compliance ---"
    echo "Core WireGuard Handshake Age: ${WG_HANDSHAKE_AGE}s (SLA <= 180s)"
    echo "Core Tunnel Latency to Gateway: $TUNNEL_LATENCY ($TUNNEL_PACKET_LOSS)"
    echo "Portal Tunnel Latency to Gateway: $PORTAL_TUNNEL_LATENCY"
    echo "DNS-over-TLS: $DOT_STATUS"
    echo "Kernel Egress Killswitch: $EGRESS_KILLSWITCH"
fi
