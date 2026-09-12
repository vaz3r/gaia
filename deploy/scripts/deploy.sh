#!/usr/bin/env bash
set -euo pipefail

# Unified Deployment Script (Target-Based Architecture)
# Usage: ./deploy/scripts/deploy.sh <target-name> [commit-ref] [services...]
#
# Examples:
#   ./deploy/scripts/deploy.sh gaia-node
#   ./deploy/scripts/deploy.sh workspace-production HEAD
#   ./deploy/scripts/deploy.sh workspace-production HEAD "classifier-api classifier-worker"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

TARGET="${1:-}"
REF="${2:-HEAD}"
SERVICES="${3:-}"

if [ -z "$TARGET" ]; then
    echo "ERROR: Target name required."
    echo "Usage: $0 <target-name> [commit-ref] [services...]"
    echo "Available targets:"
    ls -1 "$REPO_ROOT/deploy/targets/"
    exit 1
fi

TARGET_DIR="$REPO_ROOT/deploy/targets/$TARGET"
if [ ! -d "$TARGET_DIR" ]; then
    echo "ERROR: Target directory not found: $TARGET_DIR"
    exit 1
fi

# ── Load target-specific env ──
set -a
source "$TARGET_DIR/.env"
set +a

: "${DEPLOY_HOST:?DEPLOY_HOST required in target .env}"
: "${DEPLOY_USER:?DEPLOY_USER required in target .env}"
: "${DEPLOY_REMOTE_GIT:?DEPLOY_REMOTE_GIT required in target .env}"
: "${DEPLOY_REMOTE_DATA:?DEPLOY_REMOTE_DATA required in target .env}"

# SSH Configuration
if [ -n "${DEPLOY_PASSWORD:-}" ]; then
    # Use sshpass for password authentication
    SSH="sshpass -p $DEPLOY_PASSWORD ssh -o StrictHostKeyChecking=no -o PreferredAuthentications=password -o PubkeyAuthentication=no $DEPLOY_USER@$DEPLOY_HOST"
    SCP="sshpass -p $DEPLOY_PASSWORD scp -o StrictHostKeyChecking=no -o PreferredAuthentications=password -o PubkeyAuthentication=no"
elif [ -n "${DEPLOY_SSH_KEY:-}" ]; then
    # Use SSH key authentication
    SSH="ssh -i $DEPLOY_SSH_KEY -o StrictHostKeyChecking=no $DEPLOY_USER@$DEPLOY_HOST"
    SCP="scp -i $DEPLOY_SSH_KEY -o StrictHostKeyChecking=no"
else
    # Fallback to interactive prompt
    SSH="ssh -o StrictHostKeyChecking=no $DEPLOY_USER@$DEPLOY_HOST"
    SCP="scp -o StrictHostKeyChecking=no"
fi

TAG=$(git rev-parse --short "$REF")
REMOTE_TARGET_DIR="$DEPLOY_REMOTE_GIT/deploy/targets/$TARGET"

echo "=== Deploying $TAG to $TARGET ($DEPLOY_HOST) ==="

# ── 1. Check GitHub auth on remote ──
echo "[1/4] Checking git access..."
if ! $SSH "cd $DEPLOY_REMOTE_GIT && git ls-remote --exit-code origin HEAD" >/dev/null 2>&1; then
    echo "ERROR: Cannot reach GitHub from $DEPLOY_HOST."
    exit 1
fi

# ── 2. Fetch + checkout on remote ──
echo "[2/4] Updating source to $TAG..."
$SSH "cd $DEPLOY_REMOTE_GIT && git fetch origin && git checkout $TAG"

# ── 3. Ensure data directories exist ──
echo "[3/4] Ensuring data directories..."
$SSH "echo '${DEPLOY_PASSWORD:-}' | sudo -S mkdir -p ${DEPLOY_REMOTE_DATA}/crawler ${DEPLOY_REMOTE_DATA}/postgres ${DEPLOY_REMOTE_DATA}/logs ${DEPLOY_REMOTE_DATA}/classifier/models ${DEPLOY_REMOTE_DATA}/anomalies/models ${DEPLOY_REMOTE_DATA}/anomalies/data /mnt/gaia/logs/crawler && echo '${DEPLOY_PASSWORD:-}' | sudo -S chown -R 10001:10001 ${DEPLOY_REMOTE_DATA}/crawler /mnt/gaia/logs && echo '${DEPLOY_PASSWORD:-}' | sudo -S chown -R $DEPLOY_USER:$DEPLOY_USER ${DEPLOY_REMOTE_DATA}/classifier ${DEPLOY_REMOTE_DATA}/anomalies && echo '${DEPLOY_PASSWORD:-}' | sudo -S chmod -R 777 ${DEPLOY_REMOTE_DATA}/classifier ${DEPLOY_REMOTE_DATA}/anomalies || true"

# Ensure active classifier model and metadata exist on remote host
ACTIVE_JSON="$REPO_ROOT/apps/classifier/models/active_model.json"
if [ -f "$ACTIVE_JSON" ]; then
    ACTIVE_MODEL_FILE=$(python3 -c "import json; print(json.load(open('$ACTIVE_JSON')).get('filename', ''))" 2>/dev/null || true)
    if [ -n "$ACTIVE_MODEL_FILE" ] && [ -f "$REPO_ROOT/apps/classifier/models/$ACTIVE_MODEL_FILE" ]; then
        if ! $SSH "[ -f ${DEPLOY_REMOTE_DATA}/classifier/models/$ACTIVE_MODEL_FILE ]" >/dev/null 2>&1; then
            echo "Syncing active classifier model ($ACTIVE_MODEL_FILE) to remote ${DEPLOY_HOST}..."
            $SCP "$REPO_ROOT/apps/classifier/models/$ACTIVE_MODEL_FILE" "$DEPLOY_USER@$DEPLOY_HOST:${DEPLOY_REMOTE_DATA}/classifier/models/"
        fi
        echo "Syncing active_model.json to remote ${DEPLOY_HOST}..."
        $SCP "$ACTIVE_JSON" "$DEPLOY_USER@$DEPLOY_HOST:${DEPLOY_REMOTE_DATA}/classifier/models/"
        $SSH "echo '${DEPLOY_PASSWORD:-}' | sudo -S chmod -R 777 ${DEPLOY_REMOTE_DATA}/classifier || true"
    fi
fi

# Ensure baseline anomaly models exist on remote host
if [ -f "$REPO_ROOT/apps/ml/anomalies/models_storage/isolation_forest.joblib" ]; then
    if ! $SSH "[ -f ${DEPLOY_REMOTE_DATA}/anomalies/models/isolation_forest.joblib ]" >/dev/null 2>&1; then
        echo "Syncing baseline anomaly models to remote ${DEPLOY_HOST}..."
        $SCP "$REPO_ROOT/apps/ml/anomalies/models_storage/isolation_forest.joblib" "$DEPLOY_USER@$DEPLOY_HOST:${DEPLOY_REMOTE_DATA}/anomalies/models/"
        $SCP "$REPO_ROOT/apps/ml/anomalies/models_storage/autoencoder.joblib" "$DEPLOY_USER@$DEPLOY_HOST:${DEPLOY_REMOTE_DATA}/anomalies/models/"
        $SCP "$REPO_ROOT/apps/ml/anomalies/models_storage/supervised_classifier.joblib" "$DEPLOY_USER@$DEPLOY_HOST:${DEPLOY_REMOTE_DATA}/anomalies/models/"
        $SSH "echo '${DEPLOY_PASSWORD:-}' | sudo -S chmod -R 777 ${DEPLOY_REMOTE_DATA}/anomalies || true"
    fi
fi


# ── 4. Build and deploy services ──
if [ -n "$SERVICES" ]; then
    echo "[4/4] Building and deploying services: $SERVICES (only target services updated via --no-deps)..."
    $SSH "cd $REMOTE_TARGET_DIR && GIT_COMMIT=$TAG docker compose --env-file .env up -d --no-deps --build $SERVICES"
else
    echo "[4/4] Building and deploying compose stack (ensuring existing containers are not recreated)..."
    $SSH "cd $REMOTE_TARGET_DIR && GIT_COMMIT=$TAG docker compose --env-file .env up -d --no-recreate --build"
fi

echo ""
echo "=== Deploy $TAG to $TARGET complete ==="
