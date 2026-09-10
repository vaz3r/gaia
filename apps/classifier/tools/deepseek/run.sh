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

# 1. Ensure Python 3.9+ is available (search PATH, Homebrew on Apple Silicon and Intel)
PYTHON_BIN=""
for candidate in python3 /opt/homebrew/bin/python3 /usr/local/bin/python3 python3.12 python3.11 python3.10; do
    if command -v "$candidate" &>/dev/null; then
        VER=$("$candidate" -c 'import sys; print(f"{sys.version_info.major}.{sys.version_info.minor}")' 2>/dev/null || true)
        MAJOR=$(echo "$VER" | cut -d. -f1)
        MINOR=$(echo "$VER" | cut -d. -f2)
        if [ "$MAJOR" -eq 3 ] && [ "$MINOR" -ge 9 ]; then
            PYTHON_BIN="$candidate"
            PY_VER="$VER"
            break
        fi
    fi
done

if [ -z "$PYTHON_BIN" ]; then
    echo "[-] Error: Python 3.9+ is not installed or not found."
    echo "    On macOS: brew install python@3.11"
    exit 1
fi

echo "[+] Using Python: $PYTHON_BIN ($PY_VER)"

# 2. Setup Virtual Environment
if [ ! -d ".venv" ]; then
    echo "[*] Creating virtual environment (.venv)..."
    "$PYTHON_BIN" -m venv .venv
fi

echo "[*] Activating virtual environment..."
source .venv/bin/activate

# 3. Upgrade pip and install requirements
echo "[*] Ensuring dependencies are up-to-date..."
pip install --quiet --upgrade pip
pip install --quiet -r requirements.txt

# 4. Ensure Playwright Chromium is installed
# Check macOS and Linux standard cache paths
PLAYWRIGHT_INSTALLED=0
if [ -d "$HOME/Library/Caches/ms-playwright" ] && [ -n "$(ls -A "$HOME/Library/Caches/ms-playwright" 2>/dev/null)" ]; then
    PLAYWRIGHT_INSTALLED=1
elif [ -d "$HOME/.cache/ms-playwright" ] && [ -n "$(ls -A "$HOME/.cache/ms-playwright" 2>/dev/null)" ]; then
    PLAYWRIGHT_INSTALLED=1
fi

if [ "$PLAYWRIGHT_INSTALLED" -eq 0 ]; then
    echo "[*] Installing Playwright Chromium browser binary..."
    playwright install chromium
fi

# 5. Database Host Resolution (Canonical: workspace-production)
check_port() {
    local host="$1"
    local port="$2"
    # macOS BSD netcat uses -G <timeout>, Linux GNU netcat uses -w <timeout>
    if nc -z -G 2 "$host" "$port" 2>/dev/null; then
        return 0
    elif nc -z -w 2 "$host" "$port" 2>/dev/null; then
        return 0
    fi
    # Fallback to python socket check if nc options differ
    python3 -c "import socket; s = socket.socket(); s.settimeout(2.0); s.connect(('$host', int('$port'))); s.close()" 2>/dev/null
}

if [ -z "$DB_HOST" ]; then
    echo "[*] Resolving PostgreSQL connection..."
    TEST_HOSTS=("workspace-production" "100.87.194.112" "127.0.0.1")
    FOUND_HOST=""

    for h in "${TEST_HOSTS[@]}"; do
        if check_port "$h" 5432; then
            FOUND_HOST="$h"
            break
        fi
    done

    if [ -n "$FOUND_HOST" ]; then
        export DB_HOST="$FOUND_HOST"
        echo "[+] Successfully connected to PostgreSQL at: $DB_HOST:5432"
    else
        echo "[!] Warning: Could not reach PostgreSQL at workspace-production or 100.87.194.112."
        echo "    Ensure Tailscale is running and connected on your Mac."
        echo "    Falling back to: workspace-production"
        export DB_HOST="workspace-production"
    fi
else
    echo "[+] Using explicit DB_HOST=$DB_HOST"
fi

export DB_USER="${DB_USER:-crawler}"
export DB_NAME="${DB_NAME:-craw}"
export DB_PASSWORD="${DB_PASSWORD:-83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b}"
export DB_PORT="${DB_PORT:-5432}"

# Display database metrics summary if reachable
python3 -c "
import psycopg2, os
try:
    conn = psycopg2.connect(
        host=os.environ.get('DB_HOST'),
        port=int(os.environ.get('DB_PORT', 5432)),
        user=os.environ.get('DB_USER'),
        dbname=os.environ.get('DB_NAME'),
        password=os.environ.get('DB_PASSWORD'),
        connect_timeout=3
    )
    with conn.cursor() as cur:
        cur.execute('SELECT count(*) FROM labeled_results;')
        labeled = cur.fetchone()[0]
        cur.execute('SELECT count(*) FROM torrents WHERE needs_review = true;')
        review = cur.fetchone()[0]
        print(f'[+] Remote Database Stats: {labeled:,} ground-truth labels | {review:,} in Review Queue')
    conn.close()
except Exception as e:
    print(f'[!] DB notice: {e}')
" 2>/dev/null || true

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
        echo "[*] Existing DeepSeek session is older than 6 hours or expired."
        NEEDS_LOGIN=1
    fi
fi

if [ "$NEEDS_LOGIN" -eq 1 ]; then
    echo "================================================================="
    echo " [!] DeepSeek Sign-in Required"
    echo " A Chromium window will open on your desktop."
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

# Ask for volume / loops
read -r -p "How many torrents to process? (e.g. 500, 1000, 2500) [default: 1000]: " USER_TOTAL
USER_TOTAL="${USER_TOTAL:-1000}"
BATCH=50
LOOPS=$(( (USER_TOTAL + BATCH - 1) / BATCH ))
DELAY=8.0

echo "[*] Configuration: Batch Size=$BATCH | Batches=$LOOPS | Target ~$(( BATCH * LOOPS )) torrents"
echo "================================================================="

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
