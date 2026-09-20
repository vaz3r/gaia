#!/usr/bin/env bash
# monitor_gaia.sh — Master CLI orchestrator for the GAIA Health & Performance Monitoring Skill
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"

MODE="full"
FORMAT="markdown"
TARGET="all"

usage() {
    cat <<EOF
Usage: monitor_gaia.sh [options]

Options:
  --full          Run full ecosystem telemetry audit and bottleneck diagnosis (default)
  --quick         Run fast (< 5 sec) local host, tunnel, and container status check
  --json          Output machine-readable JSON telemetry and diagnostic findings
  --target <node> Filter inspection to target: workspace-production | gaia-gateway | gaia-portal | gaia-node
  --help          Display this help message
EOF
    exit 0
}

while [ $# -gt 0 ]; do
    case "$1" in
        --quick) MODE="quick" ;;
        --full) MODE="full" ;;
        --json) FORMAT="json" ;;
        --target) TARGET="$2"; shift ;;
        --help|-h) usage ;;
        *) echo "Unknown option: $1" >&2; usage ;;
    esac
    shift
done

if [ "$MODE" == "quick" ]; then
    echo "=== GAIA QUICK HEALTH PROBE ==="
    echo "--- 1. Tunnel & Ping ---"
    docker exec gaia-wireguard-client ping -c 1 -W 2 10.99.0.1 2>/dev/null && echo "✅ Gateway tunnel (10.99.0.1): REACHABLE" || echo "❌ Gateway tunnel: DOWN"
    
    echo "--- 2. Database & Redis ---"
    docker exec gaia-postgres pg_isready -U crawler -d craw 2>/dev/null && echo "✅ PostgreSQL: READY" || echo "❌ PostgreSQL: UNREADY"
    docker exec gaia-redis redis-cli ping 2>/dev/null && echo "✅ Redis: READY" || echo "❌ Redis: DOWN"

    echo "--- 3. Container Fleet ---"
    "$SCRIPT_DIR/probe_containers_fleet.sh" --target "$TARGET"
    exit 0
fi

# Full mode: run diagnostic engine and formatter
if [ "$FORMAT" == "json" ]; then
    python3 "$SCRIPT_DIR/diagnose_bottlenecks.py"
else
    python3 "$SCRIPT_DIR/diagnose_bottlenecks.py" | python3 "$SCRIPT_DIR/format_report.py"
fi
