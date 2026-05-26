# Kite Ground Control - Task Runner (just)
#
# This is the recommended way to develop and build the project.
# Install just: https://github.com/casey/just#installation
#
# Usage:
#   just dev           → Development mode (hot reload)
#   just build         → Build for current platform
#   just check         → Run svelte-check + cargo check
#   just build-windows
#   just build-linux

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

# Explicit Windows release build (uses the improved script)
build-windows:
    @powershell -ExecutionPolicy Bypass -File scripts/build-windows.bat

# Explicit Linux release build
build-linux:
    @bash scripts/build-linux.sh

# =============================================================================
# Quality Checks (important for this project)
# =============================================================================

# Run frontend + backend checks
check:
    @echo "→ Running svelte-check..."
    npm run check
    @echo "→ Running cargo check..."
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
    @echo "Cleaning..."
    powershell -Command "Remove-Item -Recurse -Force -ErrorAction SilentlyContinue 'build', '.svelte-kit'" 2>$null || true
    rm -rf build .svelte-kit 2>/dev/null || true
    cargo clean --manifest-path src-tauri/Cargo.toml