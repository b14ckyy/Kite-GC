# ============================================================
# Kite Ground Control — Windows Build Script (PowerShell)
# Builds a standalone Windows executable + NSIS installer.
#
# Recommended: use "just build-windows" (or "just build").
# ============================================================
# Prerequisites:
#   - Node.js (LTS, e.g. winget install OpenJS.NodeJS.LTS)
#   - Rust (via rustup)
#   - Visual Studio Build Tools 2022 (MSVC linker)
#   - WebView2 Runtime (included in Win10/11)
# ============================================================

Write-Host ''
Write-Host '============================================' -ForegroundColor Cyan
Write-Host ' Kite Ground Control - Windows Release Build' -ForegroundColor Cyan
Write-Host '============================================' -ForegroundColor Cyan
Write-Host ''

# OneDrive workaround: cargo dislikes spaces / live-sync in the target dir.
# Redirect the build output to a local path when the project lives in OneDrive.
if (-not $env:CARGO_TARGET_DIR) {
    $projectRoot = (Resolve-Path "$PSScriptRoot\..").Path
    if ($projectRoot -match 'OneDrive') {
        $env:CARGO_TARGET_DIR = 'D:\cargo-target\kite-gc'
        Write-Host "[INFO] Project is in a OneDrive path - setting CARGO_TARGET_DIR to $env:CARGO_TARGET_DIR" -ForegroundColor Yellow
    }
}

# === Prerequisite checks ===
if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
    Write-Host '[ERROR] Node.js not found. Install from https://nodejs.org/ or: winget install OpenJS.NodeJS.LTS' -ForegroundColor Red
    exit 1
}
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host '[ERROR] Rust/Cargo not found. Install from https://rustup.rs/' -ForegroundColor Red
    exit 1
}

Write-Host "[1/4] Node.js version: $(node -v)"

Write-Host '[2/4] Installing npm dependencies...'
npm install
if ($LASTEXITCODE -ne 0) { Write-Host '[ERROR] npm install failed.' -ForegroundColor Red; exit 1 }

Write-Host '[3/4] Building application with Tauri...'
npm run tauri build
if ($LASTEXITCODE -ne 0) { Write-Host '[ERROR] Tauri build failed.' -ForegroundColor Red; exit 1 }

Write-Host ''
Write-Host '[4/4] Build completed successfully!' -ForegroundColor Green
Write-Host ''
Write-Host 'Output location:'
if ($env:CARGO_TARGET_DIR) {
    Write-Host "  $env:CARGO_TARGET_DIR\release\bundle\"
} else {
    Write-Host '  src-tauri\target\release\bundle\'
}
Write-Host '  (NSIS installer .exe + portable build)'
