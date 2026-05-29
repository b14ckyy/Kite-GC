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

# Build for the current platform
build:
    npm run tauri build

# Explicit Windows release build
build-windows:
    @powershell -ExecutionPolicy Bypass -File scripts/build-windows.ps1

# Explicit Linux release build (only works on Linux)
build-linux:
    @bash scripts/build-linux.sh

# =============================================================================
# Quality Checks
# =============================================================================

# Run frontend + backend static checks
check:
    @powershell -Command "Write-Host '→ Running svelte-check...' -ForegroundColor Cyan"
    npm run check
    @powershell -Command "Write-Host '→ Running cargo check...' -ForegroundColor Cyan"
    cargo check --manifest-path src-tauri/Cargo.toml --quiet

# Frontend check in watch mode
check-watch:
    npm run check:watch

# =============================================================================
# Maintenance
# =============================================================================

# Install npm dependencies
install:
    npm install

# Clean build artifacts
clean:
    @powershell -Command "Write-Host 'Cleaning...' -ForegroundColor Cyan"
    powershell -Command "Remove-Item -Recurse -Force -ErrorAction SilentlyContinue 'build', '.svelte-kit'" 2>$null || true
    rm -rf build .svelte-kit 2>/dev/null || true
    cargo clean --manifest-path src-tauri/Cargo.toml