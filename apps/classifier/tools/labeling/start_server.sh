#!/bin/bash
# Start the MCP server for Torrent Classifier
# Usage:
#   ./labeling/start_server.sh              # Stdio (local dev)
#   ./labeling/start_server.sh http         # Streamable HTTP (Gemini Spark)
#   ./labeling/start_server.sh http 9000    # Custom port

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
VENV_DIR="$SCRIPT_DIR/../venv"

# Activate venv
if [ -d "$VENV_DIR" ]; then
    source "$VENV_DIR/bin/activate"
else
    echo "Warning: venv not found at $VENV_DIR, using system Python"
fi

# Initialize database schema
echo "Initializing database schema..."
python "$SCRIPT_DIR/mcp_server.py" --init-only 2>/dev/null || true

# Start server
TRANSPORT="${1:-stdio}"
PORT="${2:-9000}"

if [ "$TRANSPORT" = "stdio" ]; then
    echo "Starting MCP server (stdio)..."
    python "$SCRIPT_DIR/mcp_server.py"
else
    echo "Starting MCP server (Streamable HTTP on port $PORT)..."
    python "$SCRIPT_DIR/mcp_server.py" "$TRANSPORT" "$PORT"
fi
