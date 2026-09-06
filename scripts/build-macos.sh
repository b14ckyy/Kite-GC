#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marc Hoffmann (b14ckyy)
# ============================================================
# Kite Ground Control — macOS Build Script
# Builds a UNIVERSAL (arm64 + x86_64) .app and .dmg that run on both
# Apple Silicon and Intel Macs.
#
# Recommended: Use "just build-macos" instead.
#
# This produces an UNSIGNED bundle. To sign + notarize for distribution
# (no Gatekeeper warning on other Macs), run "just notarize-macos"
# afterwards — that step needs an Apple Developer account and reads your
# credentials from the environment / a keychain profile (never committed).
# ============================================================
# Prerequisites:
#   - Node.js (LTS)
#   - Rust (via rustup) + both mac targets (this script adds them)
#   - Xcode Command Line Tools (xcode-select --install)
# ============================================================

set -e

echo ""
echo "============================================"
echo " Kite Ground Control — macOS Release Build"
echo "============================================"
echo ""

if command -v just &> /dev/null; then
    echo "[INFO] just is installed - recommended command: just build-macos"
    echo ""
fi

if ! command -v node &> /dev/null; then
    echo "[ERROR] Node.js not found. Install from https://nodejs.org/"
    exit 1
fi

if ! command -v cargo &> /dev/null; then
    echo "[ERROR] Rust/Cargo not found. Install from https://rustup.rs/"
    exit 1
fi

echo "[1/4] Ensuring both macOS Rust targets are installed..."
rustup target add aarch64-apple-darwin x86_64-apple-darwin

echo "[2/4] Installing npm dependencies + fetching bundled ffmpeg..."
# `npm ci`, not `npm install`: install rewrites package-lock.json (it drops "peer" markers, among
# other churn), which makes the tree dirty and stamps the About dialog "<hash>-dirty" for a build
# that changed nothing. ci installs exactly what the lockfile pins, never writes it, and fails loudly
# if the lockfile and package.json disagree, all of which is what a release build wants. It is also
# what the CI workflows already use.
npm ci
bash "$(dirname "$0")/fetch-ffmpeg-macos.sh"

# Anything still modified or untracked at this point is the maintainer's own, and it will be stamped
# into the About dialog as "-dirty". Say so now rather than letting it ship unnoticed: a released
# build carrying that suffix cannot be tied back to a commit anyone else can check out.
if [ -n "$(git status --porcelain 2>/dev/null)" ]; then
    echo ""
    echo "[WARN] The working tree is not clean, so this build will be stamped <hash>-dirty:"
    git status --short | sed 's/^/       /'
    echo "       Stash or commit these first if this is a build you intend to distribute."
    echo ""
fi

echo "[3/4] Building universal application with Tauri..."
# The bundler signs the .app itself (hardened runtime + the entitlements from tauri.conf.json) when
# it finds APPLE_SIGNING_IDENTITY, and only then wraps it in the .dmg. That order is what makes
# notarization possible at all: Apple inspects the app inside the image, so signing has to happen
# here, before the .dmg exists, not afterwards. Say which kind of build this is, because the two
# look identical on disk and an unsigned one only reveals itself on someone else's Mac.
if [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
    echo "[INFO] Signing identity found in the environment: the bundle will be SIGNED."
    echo "       Run 'just notarize-macos' afterwards to notarize + staple it."
else
    echo "[INFO] No APPLE_SIGNING_IDENTITY set: the bundle will be UNSIGNED (fine for local use)."
    echo "       For a distributable build, export it BEFORE this script and re-run:"
    echo "         export APPLE_SIGNING_IDENTITY=\"Developer ID Application: Your Name (TEAMID)\""
fi
npm run tauri build -- --target universal-apple-darwin --bundles app dmg

echo "[4/4] Collecting outputs into release/ (unified naming) ..."
bash "$(dirname "$0")/collect-release.sh"
