@echo off
REM ============================================================
REM Kite Ground Control — Development Server (Windows)
REM Starts the app in development mode with hot-reload
REM
REM Recommended: Use "just dev" instead (see justfile in project root)
REM ============================================================

echo.
echo ============================================
echo  Kite Ground Control — Development Mode
echo ============================================
echo.

REM Check for just (new recommended way)
where just >nul 2>&1
if %errorlevel% equ 0 (
    echo [INFO] just is installed - you can also use: just dev
    echo.
)

if not exist "node_modules" (
    echo [1/2] Installing npm dependencies...
    call npm install
    if %errorlevel% neq 0 (
        echo [ERROR] npm install failed.
        exit /b 1
    )
)

echo [2/2] Starting Tauri development server...
echo.

call npm run tauri dev
