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