#!/bin/bash
# ============================================================
# Kite Ground Control — collect + rename build outputs (Linux + macOS)
# Renames Tauri's outputs to a unified scheme and drops them in <repo>/release/:
#
#     KiteGC_<OS>_<Arch>_<Version>_<Type>.<ext>
#     KiteGC_Android_<abi>_<Version>_installer.apk      (when an Android release build exists)
#     KiteGC_Android_<abi>_<Version>_playstore.aab      (`just build-android-aab`, for Google Play)
#
#   Type = installer   (.deb / .rpm / .dmg; the Android .apk)
#        | standalone  (macOS only: the .app as zip)
#        | portable     (the bare CLI binary, zipped with an empty `.portable` marker so the
#                        download keeps its data in a data/ folder next to the executable)
#        | update       (macOS only: the .app as .tar.gz — what the in-app updater downloads)
#
# Updater signatures (`<unified name>.sig`) are NOT produced here: the maintainer signs the collected
# artifacts locally with scripts/make-update-manifest.sh, so the signing key never leaves his machines.
#
# Shared by the local `just build*` recipes AND the GitHub release workflow, so the names are identical
# everywhere. release/ is git-ignored and refreshed on every run — `--keep` adds to it instead.
# `--android-only` (implies --keep) collects only the APKs and the Play bundle: the desktop outputs of an
# earlier build still sit in target/release and would be republished under the current version number.
# ============================================================
set -e

KEEP=0
ANDROID_ONLY=0
for arg in "$@"; do
    case "$arg" in
        --keep)         KEEP=1 ;;
        --android-only) ANDROID_ONLY=1; KEEP=1 ;;
    esac
done

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(grep '"version"' "$ROOT/package.json" | head -1 | sed -E 's/.*"version"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')"
TARGET="${CARGO_TARGET_DIR:-$ROOT/src-tauri/target}"
OUT="$ROOT/release"
APP="KiteGC"

case "$(uname -s)" in
    Darwin) OS="macOS" ;;
    Linux)  OS="Linux" ;;
    *)      OS="$(uname -s)" ;;
esac
if [ "$OS" = "macOS" ]; then
    # The macOS build is always universal (arm64 + x86_64 in one bundle).
    ARCH="universal"
else
    case "$(uname -m)" in
        x86_64 | amd64)  ARCH="x64" ;;
        aarch64 | arm64) ARCH="arm64" ;;
        *)               ARCH="$(uname -m)" ;;
    esac
fi

name() { printf '%s_%s_%s_%s_%s.%s' "$APP" "$OS" "$ARCH" "$VERSION" "$1" "$2"; }

[ "$KEEP" = 1 ] || rm -rf "$OUT"
mkdir -p "$OUT"
collected=()

# Copy the NEWEST file matching <glob> under the unified name <type>.<ext> — Tauri never prunes its
# bundle dir, and a first-match glob would ship a stale, lower version under the current name.
grab_file() { # <glob> <type> <ext>
    local newest dest
    newest="$(ls -1dt $1 2>/dev/null | head -n1)"
    [ -n "$newest" ] && [ -e "$newest" ] || return 0
    dest="$(name "$2" "$3")"
    cp -f "$newest" "$OUT/$dest"
    collected+=("$dest")
}

# Zip a bare binary + a generated empty `.portable` marker under the unified portable name.
zip_portable() { # <binary-path> <name-in-zip>
    [ -f "$1" ] || return 0
    local tmp dest
    tmp="$(mktemp -d)"
    cp -f "$1" "$tmp/$2"
    chmod +x "$tmp/$2" 2>/dev/null || true
    : > "$tmp/.portable"
    dest="$(name portable zip)"
    (cd "$tmp" && zip -q "$OUT/$dest" "$2" .portable)
    rm -rf "$tmp"
    collected+=("$dest")
}

if [ "$ANDROID_ONLY" = 1 ]; then
    : # desktop outputs are skipped — see --android-only in the header
elif [ "$OS" = "macOS" ]; then
    BUNDLE="$TARGET/universal-apple-darwin/release/bundle"
    grab_file "$BUNDLE/dmg/*.dmg" installer dmg
    # A bundle: zip with ditto to keep its structure/symlinks; the NEWEST .app (see grab_file).
    app="$(ls -1dt "$BUNDLE"/macos/*.app 2>/dev/null | head -n1)"
    if [ -n "$app" ] && [ -e "$app" ]; then
        dest="$(name standalone zip)"
        (cd "$(dirname "$app")" && ditto -c -k --keepParent "$(basename "$app")" "$OUT/$dest")
        collected+=("$dest")
        # The updater archive: the .app as .tar.gz with the bundle at the top level (the Tauri updater's
        # layout). Built here so a signed build needs no updater key; make-update-manifest.sh signs it.
        dest="$(name update tar.gz)"
        tar -czf "$OUT/$dest" -C "$(dirname "$app")" "$(basename "$app")"
        collected+=("$dest")
    fi
else
    REL="$TARGET/release"
    BUNDLE="$REL/bundle"
    grab_file "$BUNDLE/deb/*.deb" installer deb
    grab_file "$BUNDLE/rpm/*.rpm" installer rpm
    zip_portable "$REL/kite-gc" kite-gc
fi

# Delete the consumed raw bundle outputs (and Tauri's staging dirs) so stale artifacts cannot pile up
# and be mis-collected next time. $REL/kite-gc is a cargo output and stays.
if [ ${#collected[@]} -gt 0 ] && [ "$ANDROID_ONLY" != 1 ]; then
    if [ "$OS" = "macOS" ]; then
        rm -rf "$BUNDLE/dmg" "$BUNDLE/macos"
    else
        rm -rf "$BUNDLE/deb" "$BUNDLE/rpm"
    fi
fi

# Android: one release APK per ABI folder — newest per folder, raw output deleted afterwards.
APK_ROOT="$ROOT/src-tauri/gen/android/app/build/outputs/apk"
for abi_dir in "$APK_ROOT"/*/; do
    [ -d "$abi_dir" ] || continue
    apk="$(ls -1dt "$abi_dir"release/*.apk 2>/dev/null | head -n1)"
    [ -n "$apk" ] && [ -e "$apk" ] || continue
    abi="$(basename "$abi_dir")"
    case "$abi" in x86_64) abi="x64" ;; arm) abi="armv7" ;; esac
    dest="${APP}_Android_${abi}_${VERSION}_installer.apk"
    cp -f "$apk" "$OUT/$dest"
    rm -rf "${abi_dir}release"
    collected+=("$dest")
done

# Android App Bundle for Google Play: `tauri android build --aab` leaves it under
# outputs/bundle/<abi>Release/. Same handling as the APKs.
AAB_ROOT="$ROOT/src-tauri/gen/android/app/build/outputs/bundle"
for abi_dir in "$AAB_ROOT"/*Release/; do
    [ -d "$abi_dir" ] || continue
    aab="$(ls -1dt "$abi_dir"*.aab 2>/dev/null | head -n1)"
    [ -n "$aab" ] && [ -e "$aab" ] || continue
    abi="$(basename "$abi_dir")"; abi="${abi%Release}"
    case "$abi" in x86_64) abi="x64" ;; arm) abi="armv7" ;; esac
    dest="${APP}_Android_${abi}_${VERSION}_playstore.aab"
    cp -f "$aab" "$OUT/$dest"
    rm -rf "$abi_dir"
    collected+=("$dest")
done

echo ""
if [ ${#collected[@]} -eq 0 ]; then
    if [ "$ANDROID_ONLY" = 1 ]; then
        echo "[collect-release] No APK or AAB found under $(dirname "$APK_ROOT") — did the build succeed?"
    else
        echo "[collect-release] No build outputs found under $TARGET — did the build succeed?"
    fi
else
    echo "[collect-release] Collected into $OUT :"
    for c in "${collected[@]}"; do echo "  - $c"; done
fi
