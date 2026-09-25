#!/usr/bin/env bash
set -euo pipefail

# ─────────────────────────────────────────────────────────────────────────────
# GAIA Crawler Node Kernel & Network Performance Provisioner
#
# Configures:
#   1. Kernel sysctl tuning for high-throughput Mainline DHT crawling:
#      - Increases nf_conntrack_max to 262,144 (eliminates kernel table full drops)
#      - Reduces nf_conntrack UDP timeouts (10s normal, 30s stream)
#      - Optimizes socket backlogs (somaxconn 8192, netdev_max_backlog 16384)
#      - Optimizes UDP buffer minimums and socket memory ceilings
#   2. Stateless DHT UDP NOTRACK rules (iptables raw table):
#      - Bypasses conntrack for stateless DHT query/response packets on port 6882
#   3. Persistent systemd service (gaia-crawler-notrack.service):
#      - Re-applies raw NOTRACK rules automatically on reboot
#   4. Restarts gaia-crawler to establish clean sockets
#
# Usage:
#   ./deploy/scripts/setup-crawler-kernel.sh [target_name]      # Remote via SSH (default: gaia-node)
#   sudo ./deploy/scripts/setup-crawler-kernel.sh --local [port] # Local host execution (default port: 6882)
# ─────────────────────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

DEFAULT_DHT_PORT=6882

# ── Local Execution (runs directly on the crawler host with root/sudo) ──
apply_crawler_kernel_local() {
    local dht_port="${1:-$DEFAULT_DHT_PORT}"
    echo "=== [1/4] Applying Kernel Sysctl Optimizations for GAIA Crawler ==="

    cat << 'EOF' > /etc/sysctl.d/99-gaia-crawler.conf
# GAIA Crawler High-Throughput DHT Kernel Tuning
# Prevents connection tracking table exhaustion and UDP buffer drops

# Connection Tracking Capacity & Aggressive Timeout Pruning
net.netfilter.nf_conntrack_max = 262144
net.netfilter.nf_conntrack_udp_timeout = 10
net.netfilter.nf_conntrack_udp_timeout_stream = 30

# Socket Listen & Packet Backlogs
net.core.somaxconn = 8192
net.core.netdev_max_backlog = 16384

# Socket Memory Limits (16MB max, 16KB min for UDP)
net.core.rmem_max = 16777216
net.core.wmem_max = 16777216
net.ipv4.udp_rmem_min = 16384
net.ipv4.udp_wmem_min = 16384

# File Descriptor Ceilings
fs.file-max = 2097152
EOF

    # Apply sysctl settings immediately
    sysctl --system >/dev/null 2>&1 || sysctl -p /etc/sysctl.d/99-gaia-crawler.conf
    echo "  [PASS] /etc/sysctl.d/99-gaia-crawler.conf created and applied."

    echo ""
    echo "=== [2/4] Configuring Stateless DHT UDP NOTRACK Rules (Port $dht_port) ==="

    # Ensure iptables raw NOTRACK rules exist idempotently
    iptables -t raw -C PREROUTING -p udp --dport "$dht_port" -j NOTRACK 2>/dev/null || \
        iptables -t raw -I PREROUTING -p udp --dport "$dht_port" -j NOTRACK

    iptables -t raw -C OUTPUT -p udp --sport "$dht_port" -j NOTRACK 2>/dev/null || \
        iptables -t raw -I OUTPUT -p udp --sport "$dht_port" -j NOTRACK

    echo "  [PASS] Raw NOTRACK rules active for UDP port $dht_port."

    echo ""
    echo "=== [3/4] Establishing Reboot Persistence via Systemd Service ==="

    cat << EOF > /etc/systemd/system/gaia-crawler-notrack.service
[Unit]
Description=GAIA Crawler Stateless DHT NOTRACK Rules
After=network.target network-online.target

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/usr/bin/bash -c "iptables -t raw -C PREROUTING -p udp --dport $dht_port -j NOTRACK 2>/dev/null || iptables -t raw -I PREROUTING -p udp --dport $dht_port -j NOTRACK; iptables -t raw -C OUTPUT -p udp --sport $dht_port -j NOTRACK 2>/dev/null || iptables -t raw -I OUTPUT -p udp --sport $dht_port -j NOTRACK"

[Install]
WantedBy=multi-user.target
EOF

    systemctl daemon-reload
    systemctl enable --now gaia-crawler-notrack.service >/dev/null 2>&1
    echo "  [PASS] gaia-crawler-notrack.service enabled and started."

    echo ""
    echo "=== [4/4] Restarting gaia-crawler Container & Verifying Telemetry ==="

    if command -v docker >/dev/null 2>&1 && docker inspect gaia-crawler >/dev/null 2>&1; then
        echo "Restarting gaia-crawler to establish clean sockets..."
        docker restart gaia-crawler >/dev/null
        echo "  [PASS] gaia-crawler restarted."
    fi

    # Flush stale conntrack entries if conntrack tool is present
    if command -v conntrack >/dev/null 2>&1; then
        conntrack -F >/dev/null 2>&1 || true
    fi

    echo ""
    echo "--- Verification Metrics ---"
    local count max
    count=$(sysctl -n net.netfilter.nf_conntrack_count 2>/dev/null || echo "N/A")
    max=$(sysctl -n net.netfilter.nf_conntrack_max 2>/dev/null || echo "N/A")
    echo "Conntrack Table Usage: $count / $max"
    
    echo "Active Raw Table NOTRACK Rules:"
    iptables -t raw -L -n -v | grep -E "NOTRACK|Chain" || true

    echo ""
    echo "=== Crawler Node Kernel & Network Tuning Complete ==="
}

# ── Dispatch: Local Execution ──
if [ "${1:-}" = "--local" ]; then
    PORT="${2:-$DEFAULT_DHT_PORT}"
    apply_crawler_kernel_local "$PORT"
    exit 0
fi

# ── Dispatch: Remote Target Execution via SSH ──
TARGET="${1:-gaia-node}"
TARGET_DIR="$REPO_ROOT/deploy/targets/$TARGET"

if [ ! -d "$TARGET_DIR" ]; then
    echo "ERROR: Target directory not found: $TARGET_DIR"
    echo "Available targets:"
    ls -1 "$REPO_ROOT/deploy/targets/"
    exit 1
fi

# Load target SSH env
set -a
source "$TARGET_DIR/.env"
set +a

: "${DEPLOY_HOST:?DEPLOY_HOST required in target .env}"
: "${DEPLOY_USER:?DEPLOY_USER required in target .env}"

if [ -n "${DEPLOY_PASSWORD:-}" ]; then
    SSH="sshpass -p $DEPLOY_PASSWORD ssh -o StrictHostKeyChecking=no -o PreferredAuthentications=password -o PubkeyAuthentication=no $DEPLOY_USER@$DEPLOY_HOST"
    SCP="sshpass -p $DEPLOY_PASSWORD scp -o StrictHostKeyChecking=no -o PreferredAuthentications=password -o PubkeyAuthentication=no"
elif [ -n "${DEPLOY_SSH_KEY:-}" ]; then
    SSH="ssh -i $DEPLOY_SSH_KEY -o StrictHostKeyChecking=no $DEPLOY_USER@$DEPLOY_HOST"
    SCP="scp -i $DEPLOY_SSH_KEY -o StrictHostKeyChecking=no"
else
    SSH="ssh -o StrictHostKeyChecking=no $DEPLOY_USER@$DEPLOY_HOST"
    SCP="scp -o StrictHostKeyChecking=no"
fi

echo "=== Deploying Crawler Kernel Tuning to $TARGET ($DEPLOY_HOST) ==="

# Transfer this script to remote target and execute with sudo
$SCP "$0" "$DEPLOY_USER@$DEPLOY_HOST:/tmp/setup-crawler-kernel.sh"

if [ "$DEPLOY_USER" = "root" ]; then
    $SSH "chmod +x /tmp/setup-crawler-kernel.sh && /tmp/setup-crawler-kernel.sh --local $DEFAULT_DHT_PORT && rm -f /tmp/setup-crawler-kernel.sh"
else
    $SSH "chmod +x /tmp/setup-crawler-kernel.sh && echo '${DEPLOY_PASSWORD:-}' | sudo -S /tmp/setup-crawler-kernel.sh --local $DEFAULT_DHT_PORT && rm -f /tmp/setup-crawler-kernel.sh"
fi

echo "=== Crawler Kernel Tuning Successfully Deployed to $TARGET ==="
