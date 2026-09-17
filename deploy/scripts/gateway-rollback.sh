#!/usr/bin/env bash
set -euo pipefail

# Gateway & Tunnel Migration / Rollback Manager
# Usage: ./deploy/scripts/gateway-rollback.sh [to-systemd | to-docker | status]

ACTION="${1:-status}"
RANCH_PASS="rosrtdz@1995"
SSH_RANCH="sshpass -p ${RANCH_PASS} ssh -o StrictHostKeyChecking=no root@ranch"


case "$ACTION" in
    to-systemd)
        echo "=== [ROLLBACK] Reverting Gateway and Tunnels to Host Systemd Services ==="
        echo "1. Stopping Docker Compose tunnel services..."
        $SSH_RANCH "pct exec 116 -- bash -c 'cd /root/gaia/deploy/targets/gaia-gateway && docker compose down' || true"
        $SSH_RANCH "pct exec 110 -- bash -c 'cd /home/core/gaia/deploy/targets/workspace-production && docker compose stop wstunnel-client wireguard-client' || true"
        $SSH_RANCH "pct exec 116 -- ip link del wg0 2>/dev/null || true"
        $SSH_RANCH "pct exec 110 -- ip link del wg0 2>/dev/null || true"

        echo "2. Enabling and starting host systemd services on CT 116..."
        $SSH_RANCH "pct exec 116 -- systemctl enable --now nginx wstunnel-server wg-quick@wg0"

        echo "3. Enabling and starting host systemd services on CT 110..."
        $SSH_RANCH "pct exec 110 -- systemctl enable --now wstunnel-client wg-quick@wg0"

        echo "4. Verifying host service status..."
        sleep 3
        $SSH_RANCH "pct exec 110 -- ping -c 2 10.99.0.1"
        $SSH_RANCH "pct exec 116 -- curl -s https://127.0.0.1/api/stats"
        echo "=== Rollback to Systemd Complete & Verified ==="
        ;;

    to-docker)
        echo "=== [CUTOVER] Switching Gateway and Tunnels to Docker Compose ==="
        echo "1. Stopping and disabling host systemd services..."
        $SSH_RANCH "pct exec 116 -- systemctl disable --now nginx wstunnel-server wg-quick@wg0 || true"
        $SSH_RANCH "pct exec 110 -- systemctl disable --now wstunnel-client wg-quick@wg0 || true"
        $SSH_RANCH "pct exec 116 -- ip link del wg0 2>/dev/null || true"
        $SSH_RANCH "pct exec 110 -- ip link del wg0 2>/dev/null || true"

        echo "2. Starting Docker Compose on CT 116 (gaia-gateway)..."
        $SSH_RANCH "pct exec 116 -- bash -c 'cd /root/gaia/deploy/targets/gaia-gateway && docker compose up -d'"

        echo "3. Starting Docker Compose on CT 110 (workspace-production tunnel)..."
        $SSH_RANCH "pct exec 110 -- bash -c 'cd /home/core/gaia/deploy/targets/workspace-production && docker compose up -d wstunnel-client wireguard-client'"

        echo "4. Verifying container health and tunnel connectivity..."
        sleep 5
        $SSH_RANCH "pct exec 110 -- ping -c 2 10.99.0.1"
        $SSH_RANCH "pct exec 116 -- curl -s https://127.0.0.1/api/stats"
        echo "=== Cutover to Docker Compose Complete & Verified ==="
        ;;

    status)
        echo "=== Systemd Service Status ==="
        $SSH_RANCH "pct exec 116 -- systemctl is-active nginx wstunnel-server wg-quick@wg0 || true"
        $SSH_RANCH "pct exec 110 -- systemctl is-active wstunnel-client wg-quick@wg0 || true"
        echo ""
        echo "=== Docker Container Status ==="
        $SSH_RANCH "pct exec 116 -- docker ps --filter name=gaia-gateway"
        $SSH_RANCH "pct exec 110 -- docker ps --filter name=gaia-wstunnel --filter name=gaia-wireguard"
        ;;

    *)
        echo "Usage: $0 [to-systemd | to-docker | status]"
        exit 1
        ;;
esac
