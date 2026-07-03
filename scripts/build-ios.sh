#!/bin/bash
# ============================================================
# Kite Ground Control — iOS / iPad Release Build Script
# Builds a signed .ipa for install/distribution (not live-reload).
# Use scripts/run-ipad.sh for day-to-day dev runs instead.
#
# Runs the full Tauri iOS release flow:
#   npm run build            (SvelteKit web UI -> build/)
#   cargo build ios target   (Rust core -> static lib, release)
#   xcodebuild archive       (wrap + sign -> .ipa)
#
# Min deployment target comes from
# src-tauri/tauri.conf.json -> bundle.iOS.minimumSystemVersion.
# ============================================================
# Prerequisites (one-time):
#   - Node.js (LTS), Rust (via rustup), full Xcode
#   - An Apple ID / signing team configured in the Xcode project
#     (open gen/apple/*.xcodeproj once: target > Signing & Capabilities
#     > Automatically manage signing > pick your Team). Distribution
#     builds need an Apple Developer account.
# ============================================================
# Usage:
#   scripts/build-ios.sh                # release .ipa
#   FORCE_INIT=1 scripts/build-ios.sh   # regenerate gen/apple first
# ============================================================

set -e

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo ""
echo "============================================"
echo " Kite Ground Control — iOS Release Build"
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

echo "[1/4] Ensuring iOS Rust target is installed..."
rustup target add aarch64-apple-ios

echo "[2/4] Installing npm dependencies..."
npm install

if [ ! -d "gen/apple" ] || [ "${FORCE_INIT:-0}" = "1" ]; then
    echo "[3/4] Generating the Xcode project (tauri ios init)..."
    npm run tauri ios init
else
    echo "[3/4] gen/apple already present — skipping init (FORCE_INIT=1 to redo)."
fi

echo "[4/4] Building signed release .ipa with Tauri..."
npm run tauri ios build

echo ""
echo "[build-ios] Done. Look for the .ipa under:"
echo "  gen/apple/build/arm64/  (and the Xcode archive/export path Tauri prints above)"
