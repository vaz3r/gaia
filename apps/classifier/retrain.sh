#!/usr/bin/env bash
# ==============================================================================
# Gaia Classifier - Universal Retrain Runner (macOS M-series / Linux)
# Retrains model on PostgreSQL ground truth and validates Quality Gate.
# ==============================================================================

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "================================================================="
echo " Gaia Classifier: Retraining Runner"
echo " Working directory: $SCRIPT_DIR"
echo " System: $(uname -s) ($(uname -m))"
echo "================================================================="

# 1. Ensure Python 3.9+ is available
if ! command -v python3 &>/dev/null; then
    echo "[-] Error: python3 is not installed or not in PATH."
    exit 1
fi

PY_VER=$(python3 -c 'import sys; print(f"{sys.version_info.major}.{sys.version_info.minor}")')
echo "[+] Python version: $PY_VER"

# 2. Setup Virtual Environment
VENV_PATH=""
if [ -d ".venv" ]; then
    VENV_PATH=".venv"
elif [ -d "venv" ]; then
    VENV_PATH="venv"
elif [ -d "tools/deepseek/.venv" ]; then
    VENV_PATH="tools/deepseek/.venv"
else
    echo "[*] Creating virtual environment (.venv)..."
    python3 -m venv .venv
    VENV_PATH=".venv"
fi

echo "[*] Activating virtual environment ($VENV_PATH)..."
source "$VENV_PATH/bin/activate"

# 3. Ensure core ML dependencies are installed
echo "[*] Checking ML dependencies (scikit-learn, psycopg2, scipy)..."
python3 -c "import sklearn, psycopg2, scipy, joblib" 2>/dev/null || {
    echo "[*] Installing requirements from requirements.txt..."
    pip install --quiet --upgrade pip
    pip install -r requirements.txt
}

# 4. Database Host Resolution (Canonical: workspace-production)
if [ -z "$DB_HOST" ]; then
    echo "[*] Resolving PostgreSQL connection..."
    TEST_HOSTS=("workspace-production" "100.87.194.112" "127.0.0.1")
    FOUND_HOST=""

    for h in "${TEST_HOSTS[@]}"; do
        if nc -z -w 2 "$h" 5432 2>/dev/null; then
            FOUND_HOST="$h"
            break
        fi
    done

    if [ -n "$FOUND_HOST" ]; then
        export DB_HOST="$FOUND_HOST"
        echo "[+] Successfully resolved PostgreSQL at: $DB_HOST:5432"
    else
        echo "[!] Warning: Could not reach PostgreSQL on workspace-production or 100.87.194.112."
        echo "    Ensure Tailscale is connected. Falling back to DB_HOST=workspace-production."
        export DB_HOST="workspace-production"
    fi
else
    echo "[+] Using configured DB_HOST: $DB_HOST"
fi

export POSTGRES_USER="${POSTGRES_USER:-crawler}"
export POSTGRES_DB="${POSTGRES_DB:-craw}"
export PG_PASSWORD="${PG_PASSWORD:-83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b}"

echo "================================================================="
echo " Starting Model Retraining & Hyperparameter Optimization..."
echo " Command: python3 -u scripts/retrain.py $@"
echo "================================================================="
python3 -u scripts/retrain.py "$@"
