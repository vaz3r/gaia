#!/usr/bin/env bash
# ==============================================================================
# Gaia DeepSeek Classifier - Universal Runner (macOS / Linux)
# Works seamlessly over Tailscale, local LAN, or local machine.
# ==============================================================================

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "================================================================="
echo " Gaia DeepSeek Classifier: Universal Runner"
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
if [ ! -d ".venv" ]; then
    echo "[*] Creating virtual environment (.venv)..."
    python3 -m venv .venv
fi

echo "[*] Activating virtual environment..."
source .venv/bin/activate

# 3. Upgrade pip and install requirements
echo "[*] Ensuring dependencies are up-to-date..."
pip install --quiet --upgrade pip
pip install --quiet -r requirements.txt

# 4. Ensure Playwright Chromium is installed
if [ ! -d "$HOME/Library/Caches/ms-playwright" ] && [ ! -d "$HOME/.cache/ms-playwright" ]; then
    echo "[*] Installing Playwright Chromium browser binary..."
    playwright install chromium
fi

# 5. Database Host Resolution (Canonical: workspace-production)
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
        echo "[+] Successfully connected to PostgreSQL at: $DB_HOST:5432"
    else
        echo "[!] Notice: Could not ping PostgreSQL at workspace-production / 100.87.194.112."
        echo "    Using workspace-production by default. Ensure Tailscale is connected."
        export DB_HOST="workspace-production"
    fi
else
    echo "[+] Using explicit DB_HOST=$DB_HOST"
fi

export DB_USER="${DB_USER:-crawler}"
export DB_NAME="${DB_NAME:-craw}"
export DB_PASSWORD="${DB_PASSWORD:-83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b}"
export DB_PORT="${DB_PORT:-5432}"

# 6. DeepSeek Authentication Check
SESSION_FILE="$SCRIPT_DIR/session/session.json"
NEEDS_LOGIN=0

if [ ! -f "$SESSION_FILE" ]; then
    NEEDS_LOGIN=1
else
    # Check if session is older than 6 hours
    if ! python3 -c "
import json, time, sys
try:
    with open('$SESSION_FILE') as f:
        d = json.load(f)
    age = time.time() - d.get('captured_at', 0)
    if age > 6 * 3600 or not d.get('token'):
        sys.exit(1)
except Exception:
    sys.exit(1)
" 2>/dev/null; then
        echo "[*] Existing DeepSeek session is older than 6 hours or invalid."
        NEEDS_LOGIN=1
    fi
fi

if [ "$NEEDS_LOGIN" -eq 1 ]; then
    echo "================================================================="
    echo " [!] DeepSeek Sign-in Required"
    echo " A visible browser window will now open on your desktop."
    echo " Please sign in to your free chat.deepseek.com account (and solve"
    echo " the human verification if prompted)."
    echo " Once signed in, the window will close and classification will start."
    echo "================================================================="
    python3 -m deepseek.auth
fi

# 7. Menu / Execution Mode
echo ""
echo "================================================================="
echo " Ready to classify! Select execution mode:"
echo " 1) Drain Review Queue (Flagged torrents: needs_review=true) [Recommended]"
echo " 2) Target Underrepresented: Documentaries"
echo " 3) Target Underrepresented: Audiobooks"
echo " 4) Target Underrepresented: Other (ambiguous dumps/tools)"
echo " 5) General Unclassified Pool"
echo " 6) Custom command arguments"
echo "================================================================="
read -r -p "Enter choice [1-6, default=1]: " CHOICE
CHOICE="${CHOICE:-1}"

LOOPS=50
BATCH=50
DELAY=8.0

case "$CHOICE" in
    1)
        echo "[+] Running in Review Queue mode (needs_review=true)..."
        exec python3 classify.py --mode review_queue --batch "$BATCH" --loops "$LOOPS" --delay "$DELAY"
        ;;
    2)
        echo "[+] Targeting Documentaries..."
        exec python3 classify.py --target Documentaries --batch "$BATCH" --loops "$LOOPS" --delay "$DELAY"
        ;;
    3)
        echo "[+] Targeting Audiobooks..."
        exec python3 classify.py --target Audiobooks --batch "$BATCH" --loops "$LOOPS" --delay "$DELAY"
        ;;
    4)
        echo "[+] Targeting Other..."
        exec python3 classify.py --target Other --batch "$BATCH" --loops "$LOOPS" --delay "$DELAY"
        ;;
    5)
        echo "[+] Processing general unclassified pool..."
        exec python3 classify.py --batch "$BATCH" --loops "$LOOPS" --delay "$DELAY"
        ;;
    6)
        read -r -p "Enter custom classify.py arguments: " CUSTOM_ARGS
        exec python3 classify.py $CUSTOM_ARGS
        ;;
    *)
        echo "[+] Defaulting to Review Queue mode..."
        exec python3 classify.py --mode review_queue --batch "$BATCH" --loops "$LOOPS" --delay "$DELAY"
        ;;
esac
