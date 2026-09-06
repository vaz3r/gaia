#!/bin/bash
# Start the MCP server for Torrent Classifier (Gemini Spark)
#
# Usage:
#   ./start_mcp.sh              # Streamable HTTP on port 9000 (default)
#   ./start_mcp.sh 9001         # Custom port
#   ./start_mcp.sh stdio        # Stdio mode (local dev)
#
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
VENV_DIR="$SCRIPT_DIR/venv"

# Activate venv
if [ -d "$VENV_DIR" ]; then
    source "$VENV_DIR/bin/activate"
fi

# Check deps
python -c "import fastmcp, psycopg2" 2>/dev/null || {
    echo "Installing MCP dependencies..."
    pip install -r "$SCRIPT_DIR/labeling/requirements-mcp.txt"
}

MODE="${1:-http}"
PORT="${2:-9000}"

if [ "$MODE" = "stdio" ]; then
    echo "Starting MCP server (stdio)..."
    python "$SCRIPT_DIR/labeling/mcp_server.py"
else
    echo "Starting MCP server on http://0.0.0.0:$PORT/mcp"
    python "$SCRIPT_DIR/labeling/mcp_server.py" http "$PORT"
fi
