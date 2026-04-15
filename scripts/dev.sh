#!/bin/bash
# ============================================================
# INAV GCS — Development Server
# Starts the app in development mode with hot-reload
# ============================================================

set -e

echo "============================================"
echo " INAV GCS — Development Mode"
echo "============================================"
echo ""

# Install deps if node_modules is missing
if [ ! -d "node_modules" ]; then
    echo "Installing dependencies..."
    npm install
fi

echo "Starting development server..."
npm run tauri dev
