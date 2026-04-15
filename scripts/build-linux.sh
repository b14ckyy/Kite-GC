#!/bin/bash
# ============================================================
# INAV GCS — Linux Build Script
# Builds standalone Linux packages (.deb, .AppImage, .rpm)
# ============================================================
# Prerequisites:
#   - Node.js (LTS)
#   - Rust (via rustup)
#   - System packages: libwebkit2gtk-4.1-dev build-essential curl wget
#     file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
# ============================================================

set -e

echo "============================================"
echo " INAV GCS — Linux Build"
echo "============================================"
echo ""

# Check prerequisites
if ! command -v node &> /dev/null; then
    echo "ERROR: Node.js not found. Install via your package manager or https://nodejs.org/"
    exit 1
fi

if ! command -v cargo &> /dev/null; then
    echo "ERROR: Rust/Cargo not found. Install from https://rustup.rs/"
    exit 1
fi

# Detect architecture
ARCH=$(uname -m)
echo "Building for architecture: $ARCH"
echo ""

echo "[1/3] Installing dependencies..."
npm install

echo "[2/3] Building application..."
npm run tauri build

echo "[3/3] Done!"
echo ""
echo "Build output is in: src-tauri/target/release/bundle/"
echo ""
