#!/bin/bash
# ============================================================
# Kite Ground Control — Linux Build Script
# Builds standalone Linux packages (.deb, .AppImage, .rpm)
#
# Recommended: Use "just build-linux" or "just build" instead
# ============================================================
# Prerequisites:
#   - Node.js (LTS)
#   - Rust (via rustup)
#   - System packages (Debian/Ubuntu example):
#       sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
#                        libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
# ============================================================

set -e

echo ""
echo "============================================"
echo " Kite Ground Control — Linux Release Build"
echo "============================================"
echo ""

# Check for just (new recommended way)
if command -v just &> /dev/null; then
    echo "[INFO] just is installed - recommended command: just build-linux"
    echo ""
fi

# Check prerequisites
if ! command -v node &> /dev/null; then
    echo "[ERROR] Node.js not found."
    echo "        Install via your package manager or https://nodejs.org/"
    exit 1
fi

if ! command -v cargo &> /dev/null; then
    echo "[ERROR] Rust/Cargo not found."
    echo "        Install from https://rustup.rs/"
    exit 1
fi

# Detect architecture
ARCH=$(uname -m)
echo "Building for architecture: $ARCH"
echo ""

echo "[1/4] Checking Node.js version..."
node -v

echo "[2/4] Installing npm dependencies..."
npm install

echo "[3/4] Building application with Tauri..."
npm run tauri build

echo "[4/4] Build completed successfully!"
echo ""
echo "Build output is in: src-tauri/target/release/bundle/"
echo ""
echo "You will find .deb, .AppImage and/or .rpm packages there."
echo ""
