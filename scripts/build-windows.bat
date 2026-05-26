@echo off
REM ============================================================
REM Kite Ground Control — Windows Build Script
REM Builds a standalone Windows executable + NSIS installer
REM
REM Recommended: Use "just build-windows" or "just build" instead
REM ============================================================
REM Prerequisites:
REM   - Node.js (LTS, preferably via winget)
REM   - Rust (via rustup)
REM   - Visual Studio Build Tools 2022 (MSVC linker)
REM   - WebView2 Runtime (included in Win10/11)
REM ============================================================

echo.
echo ============================================
echo  Kite Ground Control — Windows Release Build
echo ============================================
echo.

REM Check for just (new recommended way)
where just >nul 2>&1
if %errorlevel% equ 0 (
    echo [INFO] just is installed - recommended command: just build-windows
    echo.
)

REM === Prerequisite checks ===
where node >nul 2>&1
if %errorlevel% neq 0 (
    echo [ERROR] Node.js not found.
    echo         Please install from https://nodejs.org/ or via: winget install OpenJS.NodeJS.LTS
    exit /b 1
)

where cargo >nul 2>&1
if %errorlevel% neq 0 (
    echo [ERROR] Rust/Cargo not found.
    echo         Please install from https://rustup.rs/
    exit /b 1
)

echo [1/4] Checking Node.js version...
for /f "tokens=*" %%i in ('node -v') do set NODE_VERSION=%%i
echo       Node.js %NODE_VERSION%

echo [2/4] Installing npm dependencies...
call npm install
if %errorlevel% neq 0 (
    echo [ERROR] npm install failed.
    exit /b 1
)

echo [3/4] Building application with Tauri...
call npm run tauri build
if %errorlevel% neq 0 (
    echo [ERROR] Tauri build failed.
    exit /b 1
)

echo [4/4] Build completed successfully!
echo.
echo Output location:
echo   src-tauri\target\release\bundle\
echo.
echo You will find the NSIS installer (.exe) and portable version there.
echo.
pause
