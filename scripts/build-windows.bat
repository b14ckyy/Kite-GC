@echo off
REM ============================================================
REM INAV GCS — Windows Build Script
REM Builds a standalone Windows executable (.exe / .msi)
REM ============================================================
REM Prerequisites: Node.js, Rust, WebView2 Runtime (included in Win10/11)
REM ============================================================

echo ============================================
echo  INAV GCS — Windows Build
echo ============================================
echo.

REM Check prerequisites
where node >nul 2>&1
if %errorlevel% neq 0 (
    echo ERROR: Node.js not found. Install from https://nodejs.org/
    exit /b 1
)

where cargo >nul 2>&1
if %errorlevel% neq 0 (
    echo ERROR: Rust/Cargo not found. Install from https://rustup.rs/
    exit /b 1
)

echo [1/3] Installing dependencies...
call npm install
if %errorlevel% neq 0 (
    echo ERROR: npm install failed
    exit /b 1
)

echo [2/3] Building application...
call npm run tauri build
if %errorlevel% neq 0 (
    echo ERROR: Build failed
    exit /b 1
)

echo [3/3] Done!
echo.
echo Build output is in: src-tauri\target\release\bundle\
echo.
pause
