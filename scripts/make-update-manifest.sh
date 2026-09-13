#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marc Hoffmann (b14ckyy)
# ============================================================
# Kite Ground Control — sign the collected artifacts + write the updater manifest (LOCAL ONLY)
#
# The in-app "Update and Restart" (src-tauri/src/commands/updater.rs) fetches
#     https://github.com/b14ckyy/Kite-GC/releases/download/<tag>/latest.json
# and installs the entry for its platform after checking the minisign signature against the public
# key in tauri.conf.json. Every entry therefore needs the artifact AND its `.sig`. Run this over the
# release/ folder once every platform's artifacts are in it, right before `gh release create`:
#
#     TAURI_SIGNING_PRIVATE_KEY_PATH=<Dev-Docs>/keys/kite-updater.key \
#         scripts/make-update-manifest.sh [tag]        # tag defaults to v<package.json version>
#     gh release create <tag> --notes-file … release/*
#
# What it does:
#   1. Signs every updater artifact that has no `.sig` beside it. Nothing is signed at build time —
#      not on CI, not on Sebastian's macOS machine — so the private key never leaves the
#      maintainer's machines (it lives in the private Dev-Docs repository).
#   2. Writes release/latest.json — one entry per platform key the plugin looks up
#      (`<os>-<arch>[-<installer>]`). Missing artifacts are reported and skipped, so a release with
#      fewer platforms still gets a valid manifest.
#
# Portable ZIPs are NOT in the manifest: the portable update is our own path (same module) and only
# needs the ZIP under its unified name.
# ============================================================
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/release"
VERSION="$(grep '"version"' "$ROOT/package.json" | head -1 | sed -E 's/.*"version"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')"
TAG="${1:-v$VERSION}"
BASE="https://github.com/b14ckyy/Kite-GC/releases/download/$TAG"

[ -d "$OUT" ] || { echo "[manifest] no release/ folder — collect the builds first" >&2; exit 1; }
if [ -z "${TAURI_SIGNING_PRIVATE_KEY_PATH:-}" ] && [ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
    echo "[manifest] set TAURI_SIGNING_PRIVATE_KEY_PATH to the updater key (Dev-Docs/keys/kite-updater.key)" >&2
    exit 1
fi

# <platform-key> <artifact file name>. Order = the order in the manifest.
PLATFORMS=(
    "windows-x86_64      KiteGC_Windows_x64_${VERSION}_installer.exe"
    "darwin-aarch64      KiteGC_macOS_universal_${VERSION}_update.tar.gz"
    "darwin-x86_64       KiteGC_macOS_universal_${VERSION}_update.tar.gz"
    "linux-x86_64-deb    KiteGC_Linux_x64_${VERSION}_installer.deb"
    "linux-x86_64-rpm    KiteGC_Linux_x64_${VERSION}_installer.rpm"
    "linux-x86_64        KiteGC_Linux_x64_${VERSION}_standalone.AppImage"
    "linux-aarch64-deb   KiteGC_Linux_arm64_${VERSION}_installer.deb"
    "linux-aarch64-rpm   KiteGC_Linux_arm64_${VERSION}_installer.rpm"
    "linux-aarch64       KiteGC_Linux_arm64_${VERSION}_standalone.AppImage"
)

# 1. Sign what is unsigned. `tauri signer sign` writes <file>.sig beside the file and reads the key
#    from the TAURI_SIGNING_PRIVATE_KEY(_PATH) environment. The key has no password, but the CLI still
#    prompts for one unless the password variable is set — an empty one — and stdin is closed.
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"
for entry in "${PLATFORMS[@]}"; do
    file="${entry#* }"; file="${file##* }"
    [ -f "$OUT/$file" ] || continue
    if [ ! -f "$OUT/$file.sig" ]; then
        echo "[manifest] signing $file"
        (cd "$ROOT" && npx tauri signer sign "$OUT/$file" </dev/null >/dev/null)
    fi
done

# 2. The manifest.
entries=()
for entry in "${PLATFORMS[@]}"; do
    key="${entry%% *}"
    file="${entry#* }"; file="${file##* }"
    if [ ! -f "$OUT/$file" ]; then
        echo "[manifest]   - $key: $file not in release/, skipped"
        continue
    fi
    if [ ! -f "$OUT/$file.sig" ]; then
        echo "[manifest]   - $key: $file has no .sig, skipped" >&2
        continue
    fi
    sig="$(tr -d '\r\n' < "$OUT/$file.sig")"
    entries+=("    \"$key\": { \"url\": \"$BASE/$file\", \"signature\": \"$sig\" }")
done
[ ${#entries[@]} -gt 0 ] || { echo "[manifest] nothing to publish" >&2; exit 1; }

{
    echo "{"
    echo "  \"version\": \"$VERSION\","
    echo "  \"notes\": \"\","
    echo "  \"pub_date\": \"$(date -u +%Y-%m-%dT%H:%M:%SZ)\","
    echo "  \"platforms\": {"
    for i in "${!entries[@]}"; do
        if [ "$i" -lt $((${#entries[@]} - 1)) ]; then echo "${entries[$i]},"; else echo "${entries[$i]}"; fi
    done
    echo "  }"
    echo "}"
} > "$OUT/latest.json"

echo "[manifest] wrote release/latest.json for $TAG (${#entries[@]} platform entries)"
