#!/bin/bash
# ============================================================
# Kite Ground Control — Development Server
# Starts the app in development mode with hot-reload
#
# Recommended: Use "just dev" instead (see justfile in project root)
# ============================================================

set -e

echo ""
echo "============================================"
echo " Kite Ground Control — Development Mode"
echo "============================================"
echo ""

# Check for just (new recommended way)
if command -v just &> /dev/null; then
    echo "[INFO] just is installed - you can also use: just dev"
    echo ""
fi

# Install deps if node_modules is missing
if [ ! -d "node_modules" ]; then
    echo "[1/2] Installing npm dependencies..."
    npm install
fi

echo "[2/2] Starting Tauri development server..."
echo ""

npm run tauri dev
