#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marc Hoffmann (b14ckyy)
# ============================================================
# Kite Ground Control — macOS sign + notarize (LOCAL ONLY)
# Signs the built universal .app + .dmg with your Developer ID, submits the
# .dmg to Apple's notary service, and staples the ticket so it opens with no
# Gatekeeper warning on any Mac.
#
# Recommended: run AFTER "just build-macos", via "just notarize-macos".
#
# CREDENTIALS ARE NEVER STORED IN THE REPO. This runs on the maintainer's
# machine only — GitHub CI builds unsigned (see .github/workflows/release.yml).
# It reads (never prints) the following from your environment:
#
#   APPLE_SIGNING_IDENTITY   e.g. "Developer ID Application: Your Name (TEAMID)"
#                            (list yours: `security find-identity -v -p codesigning`)
#
#   Notarization credentials — EITHER (recommended) a keychain profile:
#     NOTARY_PROFILE         name you gave `xcrun notarytool store-credentials`
#   OR the three raw values (used only if NOTARY_PROFILE is unset):
#     APPLE_ID               your Apple ID email
#     APPLE_TEAM_ID          your 10-char team id
#     APPLE_APP_PASSWORD     an app-specific password (appleid.apple.com)
#
# One-time keychain-profile setup (keeps the password out of your shell/env):
#   xcrun notarytool store-credentials kite-notary \
#     --apple-id you@example.com --team-id TEAMID --password <app-specific-pw>
#   export NOTARY_PROFILE=kite-notary APPLE_SIGNING_IDENTITY="Developer ID Application: ... (TEAMID)"
# ============================================================

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/release"

: "${APPLE_SIGNING_IDENTITY:?Set APPLE_SIGNING_IDENTITY (see header)}"

APP="$(ls -d "$OUT"/*.app 2>/dev/null | head -1 || true)"
DMG="$(ls "$OUT"/*.dmg 2>/dev/null | head -1 || true)"
if [ -z "$APP" ] || [ -z "$DMG" ]; then
    echo "[notarize] Missing .app or .dmg in $OUT — run 'just build-macos' first."
    exit 1
fi

echo "[1/5] Code-signing the app bundle (hardened runtime)..."
# --options runtime is required for notarization; --deep signs nested frameworks/helpers.
# --entitlements is NOT optional here: codesign only embeds the entitlements it is handed, and
# --force replaces whatever signature the bundler left behind. Signing without it produces a
# hardened-runtime binary with an EMPTY entitlement set, and the hardened runtime then denies the
# resources Kite declares in Entitlements.plist: the BLE transport cannot open a CoreBluetooth
# central, the "Camera (device)" video source gets no device, and the GCS marker gets no location.
# None of that reproduces in an unsigned local build, so it only ever breaks the released .dmg.
codesign --force --deep --options runtime --timestamp \
    --entitlements "$ROOT/src-tauri/Entitlements.plist" \
    --sign "$APPLE_SIGNING_IDENTITY" "$APP"

echo "[2/5] Signing the .dmg..."
codesign --force --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$DMG"

echo "[3/5] Submitting the .dmg to Apple notary service (this can take a few minutes)..."
if [ -n "${NOTARY_PROFILE:-}" ]; then
    # Preferred: credentials live in the login keychain, nothing secret touches the process args.
    xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
else
    : "${APPLE_ID:?Set NOTARY_PROFILE, or APPLE_ID/APPLE_TEAM_ID/APPLE_APP_PASSWORD (see header)}"
    : "${APPLE_TEAM_ID:?Set APPLE_TEAM_ID}"
    : "${APPLE_APP_PASSWORD:?Set APPLE_APP_PASSWORD}"
    # notarytool reads the password from the env var name, not the value on the command line.
    xcrun notarytool submit "$DMG" \
        --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" \
        --password "$APPLE_APP_PASSWORD" --wait
fi

echo "[4/5] Stapling the notarization ticket to the .dmg..."
xcrun stapler staple "$DMG"

echo "[5/5] Verifying..."
spctl --assess --type open --context context:primary-signature -v "$DMG" || true
codesign --verify --deep --strict --verbose=2 "$APP"

echo ""
echo "[notarize] Done. Distributable: $DMG"
