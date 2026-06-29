#!/bin/bash
# ============================================================
# Kite Ground Control — collect build outputs (Linux)
# Gathers the standalone binary + packages (.deb / .AppImage / .rpm) into
# <repo>/release/ so they don't have to be hunted down across
# target/release/bundle/*. Called at the end of `just build` / `just build-linux`.
# The release/ folder is git-ignored (local per developer).
# ============================================================
set -e

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="${CARGO_TARGET_DIR:-$ROOT/src-tauri/target}"
REL="$TARGET/release"
BUNDLE="$REL/bundle"
OUT="$ROOT/release"

# Fresh folder so it only ever holds the latest build.
rm -rf "$OUT"
mkdir -p "$OUT"

collected=()
grab() {
    for f in $1; do
        [ -e "$f" ] || continue
        cp -f "$f" "$OUT/"
        collected+=("$(basename "$f")")
    done
}

# Standalone binary + packages.
if [ -f "$REL/kite-gc" ]; then cp -f "$REL/kite-gc" "$OUT/"; collected+=("kite-gc"); fi
grab "$BUNDLE/deb/*.deb"
grab "$BUNDLE/appimage/*.AppImage"
grab "$BUNDLE/rpm/*.rpm"

echo ""
if [ ${#collected[@]} -eq 0 ]; then
    echo "[collect-release] No build outputs found under $REL — did the build succeed?"
else
    echo "[collect-release] Collected into $OUT :"
    for c in "${collected[@]}"; do echo "  - $c"; done
fi
