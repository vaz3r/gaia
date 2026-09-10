#!/usr/bin/env bash
# ==============================================================================
# Gaia MCP Server Launcher for OpenCode (macOS / Linux)
# Automatically connects over Tailscale to PostgreSQL and runs stdio transport.
# ==============================================================================

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# 1. Resolve Python 3.9+
PYTHON_BIN=""
for candidate in python3 /opt/homebrew/bin/python3 /usr/local/bin/python3 python3.12 python3.11 python3.10; do
    if command -v "$candidate" &>/dev/null; then
        VER=$("$candidate" -c 'import sys; print(f"{sys.version_info.major}.{sys.version_info.minor}")' 2>/dev/null || true)
        MAJOR=$(echo "$VER" | cut -d. -f1)
        MINOR=$(echo "$VER" | cut -d. -f2)
        if [ "$MAJOR" -eq 3 ] && [ "$MINOR" -ge 9 ]; then
            PYTHON_BIN="$candidate"
            break
        fi
    fi
done

if [ -z "$PYTHON_BIN" ]; then
    echo "[-] Error: Python 3.9+ is not found." >&2
    exit 1
fi

# 2. Virtual environment setup
if [ ! -d ".venv" ]; then
    "$PYTHON_BIN" -m venv .venv
fi
source .venv/bin/activate

# 3. Ensure dependencies are installed
if ! python3 -c "import fastmcp, psycopg2, pydantic" 2>/dev/null; then
    echo "[*] Installing required packages (fastmcp, psycopg2-binary, pydantic)..." >&2
    pip install --quiet --upgrade pip
    pip install --quiet "fastmcp>=0.4.0" "psycopg2-binary>=2.9.0" "pydantic>=2.0.0"
fi

# 4. Resolve Database Host
check_port() {
    local host="$1"
    local port="$2"
    if nc -z -G 2 "$host" "$port" 2>/dev/null; then
        return 0
    elif nc -z -w 2 "$host" "$port" 2>/dev/null; then
        return 0
    fi
    python3 -c "import socket; s = socket.socket(); s.settimeout(1.5); s.connect(('$host', int('$port'))); s.close()" 2>/dev/null
}

if [ -z "$DB_HOST" ]; then
    TEST_HOSTS=("workspace-production" "100.87.194.112" "127.0.0.1")
    for h in "${TEST_HOSTS[@]}"; do
        if check_port "$h" 5432; then
            export DB_HOST="$h"
            break
        fi
    done
    if [ -z "$DB_HOST" ]; then
        export DB_HOST="workspace-production"
    fi
fi

export DB_USER="${DB_USER:-crawler}"
export DB_NAME="${DB_NAME:-craw}"
export DB_PASSWORD="${DB_PASSWORD:-83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b}"
export DB_PORT="${DB_PORT:-5432}"

# 5. Exec MCP Server via stdio
exec python3 server.py
