#!/bin/bash
set -eo pipefail

MODE="${1:-wireguard}"
shift || true

cleanup() {
    echo "Shutting down tunnel..."
    if ip link show wg0 >/dev/null 2>&1; then
        wg-quick down wg0 || true
    fi
    exit 0
}
trap cleanup SIGTERM SIGINT

case "$MODE" in
    wstunnel-server)
        exec /usr/local/bin/wstunnel server "$@"
        ;;
    wstunnel-client)
        exec /usr/local/bin/wstunnel client "$@"
        ;;
    wireguard)
        CONFIG="${1:-/etc/wireguard/wg0.conf}"
        echo "Starting WireGuard interface using $CONFIG..."
        wg-quick up "$CONFIG"
        # Keep container alive and monitor interface
        while true; do
            if ! ip link show wg0 >/dev/null 2>&1; then
                echo "ERROR: wg0 interface disappeared!"
                exit 1
            fi
            sleep 5
        done
        ;;
    *)
        exec "$MODE" "$@"
        ;;
esac
