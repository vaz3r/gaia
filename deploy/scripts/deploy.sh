#!/usr/bin/env bash
set -euo pipefail

# Unified Deployment Script (Target-Based Architecture)
#
# Usage:
#   ./deploy/scripts/deploy.sh <target> [ref] [services] [flags...]
#   ./deploy/scripts/deploy.sh --stack "t1,t2,t3" [ref] [flags...]
#
# Flags:
#   --force-recreate   Recreate containers (for config/infra changes)
#   --verify           Run post-deploy health checks
#   --stack "a,b,c"    Deploy multiple targets in order (with --verify between each)
#
# Examples:
#   ./deploy/scripts/deploy.sh gaia-node
#   ./deploy/scripts/deploy.sh workspace-production HEAD
#   ./deploy/scripts/deploy.sh workspace-production HEAD "classifier-api classifier-worker"
#   ./deploy/scripts/deploy.sh gaia-gateway HEAD --force-recreate --verify
#   ./deploy/scripts/deploy.sh --stack "gaia-gateway,gaia-portal,workspace-production" HEAD --force-recreate --verify

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# ── Health check commands per target ──
health_check_cmd() {
    case "$1" in
        gaia-gateway)       echo "nc -z 127.0.0.1 8443 && curl -sk -o /dev/null https://127.0.0.1/" ;;
        gaia-portal)        echo "docker inspect gaia-portal-wstunnel-client --format '{{.State.Health.Status}}' 2>/dev/null | grep -q healthy" ;;
        workspace-production) echo "docker exec gaia-postgres pg_isready -U crawler -d craw 2>/dev/null" ;;
        gaia-node)          echo "docker inspect gaia-crawler --format '{{.State.Status}}' 2>/dev/null | grep -q running" ;;
        *)                  echo "" ;;
    esac
}

# ── Parse flags ──
FORCE_RECREATE=0
VERIFY=0
STACK_TARGETS=""
REF="HEAD"
POSITIONALS=()

while [ $# -gt 0 ]; do
    case "$1" in
        --force-recreate) FORCE_RECREATE=1; shift ;;
        --verify)         VERIFY=1; shift ;;
        --stack)          STACK_TARGETS="$2"; shift 2 ;;
        --stack=*)        STACK_TARGETS="${1#--stack=}"; shift ;;
        -*)               echo "ERROR: Unknown flag: $1"; exit 1 ;;
        *)                POSITIONALS+=("$1"); shift ;;
    esac
done

# ── Stack mode: deploy multiple targets in sequence ──
if [ -n "$STACK_TARGETS" ]; then
    IFS=',' read -ra STACK <<< "$STACK_TARGETS"
    REF="${POSITIONALS[0]:-HEAD}"
    TOTAL=${#STACK[@]}
    FAILED=0

    echo "=== Stack deploy: $STACK_TARGETS ($TOTAL targets), ref=$REF ==="
    echo ""

    for i in "${!STACK[@]}"; do
        TARGET="${STACK[$i]}"
        IDX=$((i + 1))
        echo "--- [$IDX/$TOTAL] $TARGET ---"

        if ! "$0" "$TARGET" "$REF" --force-recreate ${VERIFY:+--verify}; then
            echo ""
            echo "ERROR: Deployment failed at target: $TARGET"
            echo "Fix the issue, then re-run from the failed target:"
            echo "  $0 --stack \"$(IFS=','; echo "${STACK[*]:$i}"|tr ' ' ',')\" $REF --force-recreate --verify"
            exit 1
        fi
        echo ""
    done

    echo "=== Stack deploy complete: $STACK_TARGETS ==="
    exit 0
fi

# ── Single target mode ──
TARGET="${POSITIONALS[0]:-}"
REF="${POSITIONALS[1]:-HEAD}"
SERVICES="${POSITIONALS[2]:-}"

if [ -z "$TARGET" ]; then
    echo "ERROR: Target name required."
    echo ""
    echo "Usage:"
    echo "  $0 <target> [ref] [services] [--force-recreate] [--verify]"
    echo "  $0 --stack \"t1,t2,t3\" [ref] [--force-recreate] [--verify]"
    echo ""
    echo "Available targets:"
    ls -1 "$REPO_ROOT/deploy/targets/"
    echo ""
    echo "Examples:"
    echo "  $0 gaia-node"
    echo "  $0 workspace-production HEAD \"classifier-api classifier-worker\""
    echo "  $0 --stack \"gaia-gateway,gaia-portal,workspace-production\" HEAD --force-recreate --verify"
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
    SSH="sshpass -p $DEPLOY_PASSWORD ssh -o StrictHostKeyChecking=no -o PreferredAuthentications=password -o PubkeyAuthentication=no $DEPLOY_USER@$DEPLOY_HOST"
    SCP="sshpass -p $DEPLOY_PASSWORD scp -o StrictHostKeyChecking=no -o PreferredAuthentications=password -o PubkeyAuthentication=no"
elif [ -n "${DEPLOY_SSH_KEY:-}" ]; then
    SSH="ssh -i $DEPLOY_SSH_KEY -o StrictHostKeyChecking=no $DEPLOY_USER@$DEPLOY_HOST"
    SCP="scp -i $DEPLOY_SSH_KEY -o StrictHostKeyChecking=no"
else
    SSH="ssh -o StrictHostKeyChecking=no $DEPLOY_USER@$DEPLOY_HOST"
    SCP="scp -o StrictHostKeyChecking=no"
fi

TAG=$(git rev-parse --short "$REF")
REMOTE_TARGET_DIR="$DEPLOY_REMOTE_GIT/deploy/targets/$TARGET"

RECREATE_FLAG="--no-recreate"
[ "$FORCE_RECREATE" -eq 1 ] && RECREATE_FLAG="--force-recreate"

echo "=== Deploying $TAG to $TARGET ($DEPLOY_HOST) [recreate=$([ "$FORCE_RECREATE" -eq 1 ] && echo on || echo off)] ==="

# ── Suspend egress killswitch on remote during deploy if active ──
RESTORE_KILLSWITCH=0
if $SSH "command -v systemctl >/dev/null 2>&1 && systemctl is-active --quiet gaia-killswitch" 2>/dev/null; then
    echo "Temporarily suspending egress killswitch on $DEPLOY_HOST for deployment..."
    $SSH "sudo iptables -D OUTPUT -p tcp -m tcp --dport 443 -j REJECT --reject-with icmp-net-unreachable 2>/dev/null || true; sudo iptables -D OUTPUT -p tcp -m tcp --dport 80 -j REJECT --reject-with icmp-net-unreachable 2>/dev/null || true"
    RESTORE_KILLSWITCH=1
    trap 'if [ "$RESTORE_KILLSWITCH" -eq 1 ]; then echo "Restoring egress killswitch on $DEPLOY_HOST..."; $SSH "sudo systemctl restart gaia-killswitch" 2>/dev/null || true; fi' EXIT
fi

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
    echo "[4/4] Building and deploying services: $SERVICES..."
    for svc in $SERVICES; do
        echo "--> Building and deploying $svc..."
        $SSH "cd $REMOTE_TARGET_DIR && GIT_COMMIT=$TAG docker compose --env-file .env up -d --no-deps $RECREATE_FLAG --build $svc"
    done
else
    echo "[4/4] Building and deploying compose stack ($RECREATE_FLAG)..."
    $SSH "cd $REMOTE_TARGET_DIR && GIT_COMMIT=$TAG docker compose --env-file .env up -d $RECREATE_FLAG --build"
fi

echo ""
echo "=== Deploy $TAG to $TARGET complete ==="

# ── 5. Post-deploy health check ──
if [ "$VERIFY" -eq 1 ]; then
    echo ""
    echo "--- Health check: $TARGET ---"
    CMD="$(health_check_cmd "$TARGET")"
    if [ -z "$CMD" ]; then
        echo "  No health check defined for $TARGET (skipping)"
    else
        sleep 3
        if $SSH "$CMD" 2>/dev/null; then
            echo "  PASS: $TARGET"
        else
            echo "  FAIL: $TARGET — health check failed"
            echo "  Command: $CMD"
            exit 1
        fi
    fi
fi
