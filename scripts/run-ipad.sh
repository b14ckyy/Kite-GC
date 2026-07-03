#!/bin/bash
# ============================================================
# Kite Ground Control — iPad / iOS Dev Run Script
# One command to build the whole stack (web + Rust core + Xcode
# wrapper) and launch it on a connected iPad with live reload.
#
# Runs the full Tauri iOS flow:
#   npm run build            (SvelteKit web UI -> build/)
#   cargo build ios target   (Rust core -> static lib)
#   xcodebuild in gen/apple/ (wrap + sign + install on device)
#
# Min deployment target (iOS/iPadOS) comes from
# src-tauri/tauri.conf.json -> bundle.iOS.minimumSystemVersion.
# ============================================================
# Prerequisites (one-time):
#   - Node.js (LTS)
#   - Rust (via rustup); this script adds the iOS targets
#   - Xcode (full app, not just CLI tools) + an Apple ID added in
#     Xcode > Settings > Accounts (free personal team is fine)
#   - iPad on iPadOS 18.6+, connected by USB-C, unlocked, "Trust"ed,
#     with Developer Mode enabled (Settings > Privacy & Security)
# ============================================================
# Usage:
#   scripts/run-ipad.sh                 # auto-pick the connected device
#   scripts/run-ipad.sh "My iPad"       # target a device by name
#   FORCE_INIT=1 scripts/run-ipad.sh    # regenerate gen/apple first
# ============================================================

set -e

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo ""
echo "============================================"
echo " Kite Ground Control — iPad Dev Run"
echo "============================================"
echo ""

if ! command -v node &> /dev/null; then
    echo "[ERROR] Node.js not found. Install from https://nodejs.org/"
    exit 1
fi

if ! command -v cargo &> /dev/null; then
    echo "[ERROR] Rust/Cargo not found. Install from https://rustup.rs/"
    exit 1
fi

if ! xcode-select -p &> /dev/null || ! command -v xcodebuild &> /dev/null; then
    echo "[ERROR] Xcode not found. Install the full Xcode from the App Store,"
    echo "        then run: sudo xcode-select -s /Applications/Xcode.app"
    exit 1
fi

echo "[1/4] Ensuring iOS Rust targets are installed..."
rustup target add aarch64-apple-ios aarch64-apple-ios-sim

echo "[2/4] Installing npm dependencies..."
npm install

# gen/apple is gitignored + regenerable. Create it on first run, or when
# FORCE_INIT=1 (e.g. after changing the min deployment target in the config).
if [ ! -d "gen/apple" ] || [ "${FORCE_INIT:-0}" = "1" ]; then
    echo "[3/4] Generating the Xcode project (tauri ios init)..."
    npm run tauri ios init
else
    echo "[3/4] gen/apple already present — skipping init (FORCE_INIT=1 to redo)."
fi

echo "[4/4] Building + launching on device (live reload)..."
echo ""
echo "  First launch on a new device needs two manual trust steps:"
echo "    - iPad: Settings > General > VPN & Device Management > trust your Apple ID"
echo "    - Allow the Local Network + Bluetooth permission prompts in-app"
echo ""

if [ -n "$1" ]; then
    npm run tauri ios dev -- "$1"
else
    npm run tauri ios dev
fi
