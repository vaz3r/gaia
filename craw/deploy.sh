#!/usr/bin/env bash
set -euo pipefail

# Deploy crawler to zerone (ARM aarch64)
# Usage: ./deploy.sh [zerone-host]
#
# This script:
# 1. Cross-compiles for aarch64 (or builds Docker image on target)
# 2. Deploys the binary to the target host
# 3. Restarts the container
#
# For Docker-based deployment (recommended):
#   docker buildx build --platform linux/arm64 -t dht-crawler .
#   docker compose up -d

HOST="${1:-zerone}"
BUILD_DIR="target/aarch64-unknown-linux-gnu/release"

echo "=== Deploying crawler to $HOST ==="

# Step 1: Cross-compile for aarch64
echo "[1/4] Building for aarch64..."
cd "$(dirname "$0")"
cargo build --release --target aarch64-unknown-linux-gnu

# Step 2: Verify binary architecture
echo "[2/4] Verifying binary..."
file "$BUILD_DIR/craw"

# Step 3: Deploy to remote
echo "[3/4] Copying to $HOST..."
scp "$BUILD_DIR/craw" "$HOST:/tmp/craw-new"
ssh "$HOST" "sudo cp /tmp/craw-new /home/ubuntu/craw-stack/craw && sudo chmod 755 /home/ubuntu/craw-stack/craw"

# Step 4: Restart with docker compose (so .env changes take effect)
echo "[4/4] Restarting container..."
ssh "$HOST" "cd /home/ubuntu/craw-stack && docker compose up -d crawler"

# Step 5: Verify
sleep 5
echo "=== Verifying deployment ==="
ssh "$HOST" "docker logs --tail 3 craw-crawler 2>&1"

echo ""
echo "=== Deployment complete ==="
echo "Binary: aarch64-unknown-linux-gnu"
echo "Note: For full Docker build, run on target:"
echo "  docker buildx build --platform linux/arm64 -t dht-crawler ."
echo "  docker compose up -d"
