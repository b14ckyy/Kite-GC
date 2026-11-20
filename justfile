# Kite Ground Control task runner — install just: https://github.com/casey/just#installation
# On Windows the recipes run in PowerShell; Git Bash / sh is NOT required.

set windows-shell := ["powershell.exe", "-NoProfile", "-Command"]

# Default recipe → shows all available commands
default:
    @just --list

# Start development mode with hot-reload
dev:
    npm run tauri dev

# Build for the current platform (then collect outputs into release/)
[windows]
build:
    npm run tauri build
    @powershell -ExecutionPolicy Bypass -File scripts/collect-release.ps1

[unix]
build:
    npm run tauri build
    @bash scripts/collect-release.sh

# Explicit Windows release build
build-windows:
    @powershell -ExecutionPolicy Bypass -File scripts/build-windows.ps1

# Explicit Linux release build (only works on Linux)
build-linux:
    @bash scripts/build-linux.sh

# Universal macOS release build — arm64 + x86_64 .app/.dmg (only works on macOS, UNSIGNED)
build-macos:
    @bash scripts/build-macos.sh

# Needs an Apple Developer account — reads creds from your env / keychain profile (see scripts/notarize-macos.sh)
# Sign + notarize the built macOS bundle for distribution (macOS only)
notarize-macos:
    @bash scripts/notarize-macos.sh

# Maintainer only; needs TAURI_SIGNING_PRIVATE_KEY_PATH → Dev-Docs/keys/kite-updater.key. Run before `gh release create`.
# Sign the collected release artifacts + write release/latest.json for the in-app updater
update-manifest tag="":
    @bash scripts/make-update-manifest.sh {{tag}}

# Needs ANDROID_HOME + NDK_HOME (see the Build Guide). One APK per ABI lands in release/ as
# KiteGC_Android_<arm64|armv7>_<version>_installer.apk.
# `-AndroidOnly` / `--android-only` collects ONLY the Android files: a stale desktop binary still in
# target/release would otherwise be republished under the current version number.
# Android APKs, arm64 and 32-bit ARM
[windows]
build-android:
    npm run tauri android build -- --apk --split-per-abi --target aarch64 armv7
    @powershell -ExecutionPolicy Bypass -File scripts/collect-release.ps1 -AndroidOnly

[unix]
build-android:
    npm run tauri android build -- --apk --split-per-abi --target aarch64 armv7
    @bash scripts/collect-release.sh --android-only

# Local only: needs the release key (`src-tauri/gen/android/app/key.properties`, see the Build Guide), else
# the bundle is debug-signed and Play refuses it. ONE bundle with both ABIs (Play splits it per device), in
# release/ as KiteGC_Android_universal_<version>_playstore.aab.
# versionCode = `playVersionCode` in app/build.gradle.kts; KITE_ANDROID_BUILD=1..9 re-uploads the same version.
# The bundle is for the Play Console only: take it out of release/ before `gh release create … release/*`.
# Android App Bundle for Google Play, arm64 + 32-bit ARM (maintainer)
[windows]
build-android-aab:
    npm run tauri android build -- --aab --target aarch64 armv7
    @powershell -ExecutionPolicy Bypass -File scripts/collect-release.ps1 -AndroidOnly

[unix]
build-android-aab:
    npm run tauri android build -- --aab --target aarch64 armv7
    @bash scripts/collect-release.sh --android-only

# Android dev build on a connected device / emulator, with hot reload
dev-android:
    npm run tauri android dev

# Run frontend + backend static checks (Windows)
[windows]
check:
    @powershell -Command "Write-Host '→ Running svelte-check...' -ForegroundColor Cyan"
    npm run check
    @powershell -Command "Write-Host '→ Running cargo check...' -ForegroundColor Cyan"
    cargo check --manifest-path src-tauri/Cargo.toml --quiet

# Run frontend + backend static checks (Linux / macOS)
[unix]
check:
    @echo '→ Running svelte-check...'
    npm run check
    @echo '→ Running cargo check...'
    cargo check --manifest-path src-tauri/Cargo.toml --quiet

# Needs `rustup target add aarch64-linux-android`, plus the NDK's compiler pointed out explicitly — cc-rs
# looks for an `aarch64-linux-android-clang` the NDK does not ship (its compilers are API-versioned).
# `tauri android build` does this itself; only the bare check needs it.
# Type-check the Rust backend for Android in seconds, without a Gradle build (catches mobile cfg mistakes)
check-android:
    #!/usr/bin/env bash
    set -euo pipefail
    BIN="${NDK_HOME:?set NDK_HOME to your NDK, e.g. $ANDROID_HOME/ndk/27.2.12479018}/toolchains/llvm/prebuilt/linux-x86_64/bin"
    export CC_aarch64_linux_android="$BIN/aarch64-linux-android24-clang"
    export AR_aarch64_linux_android="$BIN/llvm-ar"
    export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CC_aarch64_linux_android"
    cargo check --manifest-path src-tauri/Cargo.toml --target aarch64-linux-android --lib

# Frontend check in watch mode
check-watch:
    npm run check:watch

# Install npm dependencies
install:
    npm install

# Clean build artifacts (Windows)
[windows]
clean:
    @powershell -Command "Write-Host 'Cleaning...' -ForegroundColor Cyan"
    @powershell -Command "Remove-Item -Recurse -Force -ErrorAction SilentlyContinue 'build', '.svelte-kit'"
    cargo clean --manifest-path src-tauri/Cargo.toml

# Clean build artifacts (Linux / macOS)
[unix]
clean:
    @echo 'Cleaning...'
    rm -rf build .svelte-kit
    cargo clean --manifest-path src-tauri/Cargo.toml