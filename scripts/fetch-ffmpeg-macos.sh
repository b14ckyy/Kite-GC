#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marc Hoffmann (b14ckyy)
# ============================================================
# Kite Ground Control — fetch static macOS ffmpeg for bundling
#
# Downloads static ffmpeg binaries (arm64 + x86_64) and writes them as Tauri
# sidecars under src-tauri/binaries/ so `just build-macos` / CI bundle ffmpeg
# INSIDE the .app — macOS users never install ffmpeg by hand (unlike the
# Windows/Linux runtime-download model, there is no upstream static macOS build
# to fetch at runtime, so we ship it).
#
# Source: github.com/eugeneware/ffmpeg-static (the binaries behind the widely
# used `ffmpeg-static` npm package). These are GPL builds — compatible with this
# app's GPL-3.0-or-later license. ffmpeg source: https://ffmpeg.org/download.html
#
# Produces (git-ignored):
#   src-tauri/binaries/ffmpeg-aarch64-apple-darwin
#   src-tauri/binaries/ffmpeg-x86_64-apple-darwin
#   src-tauri/binaries/ffmpeg-universal-apple-darwin   (lipo of the two)
# Tauri picks the one matching the build target triple and bundles it as
# `ffmpeg` next to the app binary, where find_ffmpeg() already looks first.
# ============================================================

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN_DIR="$ROOT/src-tauri/binaries"
# Pinned to a specific release tag (not `latest`) so CI builds are reproducible and the bundled
# ffmpeg binary can't change out from under us — bump this deliberately when updating ffmpeg.
FFMPEG_STATIC_TAG="b6.1.1"
# Download release assets DIRECTLY rather than resolving them through the GitHub REST API: an
# unauthenticated api.github.com call is rate-limited to 60/h per IP and returns HTTP 403 on shared
# CI-runner IPs (intermittent "curl: (56) … error: 403" build failures). The releases/download/ path
# is CDN-backed and public, so it isn't subject to that limit — and the tag + asset names are pinned
# below anyway, so the API lookup added nothing but a failure mode.
BASE="https://github.com/eugeneware/ffmpeg-static/releases/download/${FFMPEG_STATIC_TAG}"

mkdir -p "$BIN_DIR"

fetch_arch() {
    local asset="$1" triple="$2"
    local out="$BIN_DIR/ffmpeg-$triple"
    echo "[fetch-ffmpeg] $asset -> $(basename "$out")"
    # The .gz asset is ~1/3 the size of the raw binary.
    curl -fsSL "$BASE/$asset" | gunzip -c > "$out"
    chmod +x "$out"
}

echo "[fetch-ffmpeg] Downloading static macOS ffmpeg (arm64 + x86_64)..."
fetch_arch "ffmpeg-darwin-arm64.gz" "aarch64-apple-darwin"
fetch_arch "ffmpeg-darwin-x64.gz" "x86_64-apple-darwin"

echo "[fetch-ffmpeg] Creating universal binary via lipo..."
lipo -create \
    "$BIN_DIR/ffmpeg-aarch64-apple-darwin" \
    "$BIN_DIR/ffmpeg-x86_64-apple-darwin" \
    -output "$BIN_DIR/ffmpeg-universal-apple-darwin"
chmod +x "$BIN_DIR/ffmpeg-universal-apple-darwin"

echo "[fetch-ffmpeg] Verifying..."
"$BIN_DIR/ffmpeg-universal-apple-darwin" -version | head -1
lipo -archs "$BIN_DIR/ffmpeg-universal-apple-darwin"
echo "[fetch-ffmpeg] Done → $BIN_DIR"
