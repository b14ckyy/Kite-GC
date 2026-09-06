#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marc Hoffmann (b14ckyy)
# ============================================================
# Kite Ground Control — macOS notarize + staple (LOCAL ONLY)
# Submits the collected .dmg to Apple's notary service and staples the ticket,
# so it opens with no Gatekeeper warning on any Mac.
#
# THE APP IS NOT SIGNED HERE. That happens during the build, because the .dmg
# has to wrap an ALREADY-SIGNED .app: Apple inspects the app inside the image,
# so signing a loose .app after the .dmg was built changes nothing the notary
# service looks at. The Tauri bundler signs the app for us (hardened runtime,
# --entitlements from tauri.conf.json) whenever it finds a signing identity in
# the environment. This script only signs the disk image itself, if the bundler
# did not, since that wrapper signature is independent of the payload. Order:
#
#   export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
#   just build-macos      # bundler signs the .app, then builds the .dmg around it
#   just notarize-macos   # this script: notarize + staple
#
# List your identities with: security find-identity -v -p codesigning
# A "Developer ID Application" certificate is required. An "Apple Development"
# certificate cannot notarize, and neither can an ad-hoc signature.
#
# CREDENTIALS ARE NEVER STORED IN THE REPO. This runs on the maintainer's
# machine only — GitHub CI builds unsigned (see .github/workflows/release.yml).
# It reads (never prints) EITHER (recommended) a keychain profile:
#     NOTARY_PROFILE         name you gave `xcrun notarytool store-credentials`
#   OR the three raw values (used only if NOTARY_PROFILE is unset):
#     APPLE_ID               your Apple ID email
#     APPLE_TEAM_ID          your 10-char team id
#     APPLE_APP_PASSWORD     an app-specific password (appleid.apple.com)
#
# One-time keychain-profile setup (keeps the password out of your shell/env):
#   xcrun notarytool store-credentials kite-notary \
#     --apple-id you@example.com --team-id TEAMID --password <app-specific-pw>
#   export NOTARY_PROFILE=kite-notary
# ============================================================

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/release"

DMG="$(ls "$OUT"/*.dmg 2>/dev/null | head -1 || true)"
if [ -z "$DMG" ]; then
    echo "[notarize] No .dmg in $OUT — run 'just build-macos' first."
    exit 1
fi

# Refuse early rather than burning a notary submission on a payload Apple will reject.
# The signature lives on the .app INSIDE the image, so mount it read-only and check there.
echo "[1/5] Checking the app inside the .dmg is signed for distribution..."
MNT="$(mktemp -d)"
cleanup() { hdiutil detach "$MNT" -quiet 2>/dev/null || true; rmdir "$MNT" 2>/dev/null || true; }
trap cleanup EXIT
hdiutil attach "$DMG" -mountpoint "$MNT" -nobrowse -readonly -quiet

APP="$(ls -d "$MNT"/*.app 2>/dev/null | head -1 || true)"
if [ -z "$APP" ]; then
    echo "[notarize] No .app inside $DMG. The image is not a Kite bundle."
    exit 1
fi

AUTHORITY="$(codesign -dvv "$APP" 2>&1 | grep '^Authority=' | head -1 | cut -d= -f2- || true)"
if ! printf '%s' "$AUTHORITY" | grep -q '^Developer ID Application'; then
    echo ""
    echo "[notarize] The app in the .dmg is NOT signed with a Developer ID certificate."
    echo "           Signing authority: ${AUTHORITY:-<none: unsigned or ad-hoc>}"
    echo ""
    echo "           Apple notarizes only Developer ID signed payloads, and the .dmg has to"
    echo "           be built AROUND the signed app, so re-signing now would not help."
    echo "           Rebuild with the identity exported, then run this again:"
    echo ""
    echo "             export APPLE_SIGNING_IDENTITY=\"Developer ID Application: Your Name (TEAMID)\""
    echo "             just build-macos"
    echo ""
    exit 1
fi
echo "       OK: $AUTHORITY"

# The hardened runtime is a notarization requirement, and the bundler only applies it while
# signing. Catch a bundle signed by hand without it before Apple does.
if ! codesign -d --verbose=4 "$APP" 2>&1 | grep -q 'flags=.*runtime'; then
    echo "[notarize] The app is signed but WITHOUT the hardened runtime, which notarization requires."
    echo "           Rebuild via 'just build-macos' so the bundler signs it (--options runtime)."
    exit 1
fi

cleanup
trap - EXIT

# The disk image itself should carry a signature too, separate from the app inside it. Whether the
# bundler already signed it depends on the Tauri version, so check rather than assume: signing the
# wrapper is idempotent and never touches the payload, unlike re-signing the app.
echo "[2/5] Checking the .dmg wrapper signature..."
if codesign -dvv "$DMG" 2>&1 | grep -q '^Authority=Developer ID Application'; then
    echo "       Already signed by the bundler, leaving it alone."
elif [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
    echo "       Unsigned, signing the image with APPLE_SIGNING_IDENTITY..."
    codesign --force --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$DMG"
else
    echo "[notarize] The .dmg wrapper is unsigned and APPLE_SIGNING_IDENTITY is not set."
    echo "           Export it and re-run; the app inside is already signed, so this is"
    echo "           the only step missing."
    exit 1
fi

echo "[3/5] Submitting the .dmg to Apple notary service (this can take a few minutes)..."
if [ -n "${NOTARY_PROFILE:-}" ]; then
    # Preferred: credentials live in the login keychain, nothing secret touches the process args.
    xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
else
    : "${APPLE_ID:?Set NOTARY_PROFILE, or APPLE_ID/APPLE_TEAM_ID/APPLE_APP_PASSWORD (see header)}"
    : "${APPLE_TEAM_ID:?Set APPLE_TEAM_ID}"
    : "${APPLE_APP_PASSWORD:?Set APPLE_APP_PASSWORD}"
    xcrun notarytool submit "$DMG" \
        --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" \
        --password "$APPLE_APP_PASSWORD" --wait
fi

echo "[4/5] Stapling the notarization ticket to the .dmg..."
xcrun stapler staple "$DMG"

echo "[5/5] Verifying..."
# What a user's Mac actually evaluates on first open.
spctl --assess --type open --context context:primary-signature -v "$DMG"

echo ""
echo "[notarize] Done. Distributable: $DMG"
