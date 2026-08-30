# Kite Ground Control - Task Runner (just)
#
# Recommended task runner for this project.
# Install just: https://github.com/casey/just#installation
#
# On Windows this justfile is configured to use PowerShell.
# Make sure Git Bash / sh is NOT required.

set windows-shell := ["powershell.exe", "-NoProfile", "-Command"]

# Default recipe → shows all available commands
default:
    @just --list

# =============================================================================
# Development
# =============================================================================

# Start development mode with hot-reload
dev:
    npm run tauri dev

# =============================================================================
# Building
# =============================================================================

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

# Sign + notarize the built macOS bundle for distribution (macOS only; needs an Apple
# Developer account — reads creds from your env / keychain profile, see scripts/notarize-macos.sh)
notarize-macos:
    @bash scripts/notarize-macos.sh

# Android APK, arm64 (EXPERIMENTAL — needs ANDROID_HOME + NDK_HOME; see the Build Guide).
# Prefer the "Android" GitHub Actions workflow, which needs nothing installed locally.
build-android:
    npm run tauri android build -- --apk --target aarch64

# Android dev build on a connected device / emulator, with hot reload
dev-android:
    npm run tauri android dev

# =============================================================================
# Quality Checks
# =============================================================================

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

# Type-check the Rust backend for Android without a full Gradle build. Catches cfg mistakes in the
# mobile-specific code (transport/serial_android.rs, android/jvm.rs) in seconds.
#
# Needs `rustup target add aarch64-linux-android`, plus the NDK's compiler pointed out explicitly —
# cc-rs looks for an `aarch64-linux-android-clang` that the NDK does not ship (its compilers are
# API-versioned), and without it the C dependencies fail to configure. `tauri android build` does this
# itself, which is why only the bare check needs it.
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

# =============================================================================
# Maintenance
# =============================================================================

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