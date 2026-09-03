#!/usr/bin/env bash
set -e

HOST="100.87.194.112"
PORT="3000"

echo "=== DASHBOARD STATS ==="
curl -s http://$HOST:$PORT/api/stats | jq '.'

echo "=== DASHBOARD METRICS (CURRENT) ==="
curl -s http://$HOST:$PORT/api/metrics/current | jq '.rates'
