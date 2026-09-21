#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
GATEWAY_IP="192.168.10.111"
PORTAL_IP="192.168.10.139"

echo "================================================================="
echo "  GAIA PORTAL GATEWAY CAPACITY & LOAD BENCHMARK RUNNER"
echo "================================================================="
echo "Target: https://gaia-gateway/ (Gateway IP: $GATEWAY_IP)"
echo "Date:   $(date -u +"%Y-%m-%dT%H:%M:%SZ")"
echo ""

# 1. Quick probe to confirm gateway availability
echo "[1/4] Probing gateway connectivity..."
if ! curl -k -s -o /dev/null -m 5 --resolve gaia-gateway:443:$GATEWAY_IP "https://gaia-gateway/api/stats"; then
    echo "ERROR: Cannot reach https://gaia-gateway/api/stats via $GATEWAY_IP"
    exit 1
fi
echo "      ✓ Gateway connection verified."

# 2. Host health check before load test
echo "[2/4] Recording baseline host metrics..."
echo "      Gateway ($GATEWAY_IP):"
sshpass -p "rosrtdz@1995" ssh -o StrictHostKeyChecking=no -o ConnectTimeout=4 core@$GATEWAY_IP "uptime; free -m | grep Mem" 2>/dev/null || true
echo "      Portal ($PORTAL_IP):"
sshpass -p "rosrtdz@1995" ssh -o StrictHostKeyChecking=no -o ConnectTimeout=4 root@$PORTAL_IP "uptime; free -m | grep Mem" 2>/dev/null || true

# 3. Execute k6 benchmark via Docker
echo ""
echo "[3/4] Launching k6 Load Test Suite..."
docker run --rm -i \
  --add-host gaia-gateway:$GATEWAY_IP \
  -v "$REPO_ROOT/tests/load:/scripts" \
  -e TARGET_URL="https://gaia-gateway" \
  grafana/k6:latest run /scripts/k6_portal_gateway.js

# 4. Host health check after load test
echo ""
echo "[4/4] Recording post-load host metrics..."
echo "      Gateway ($GATEWAY_IP):"
sshpass -p "rosrtdz@1995" ssh -o StrictHostKeyChecking=no -o ConnectTimeout=4 core@$GATEWAY_IP "uptime" 2>/dev/null || true
echo "      Portal ($PORTAL_IP):"
sshpass -p "rosrtdz@1995" ssh -o StrictHostKeyChecking=no -o ConnectTimeout=4 root@$PORTAL_IP "uptime" 2>/dev/null || true

echo ""
echo "================================================================="
echo "  Benchmark execution complete."
echo "================================================================="
