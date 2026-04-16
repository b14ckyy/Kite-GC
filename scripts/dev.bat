@echo off
REM ============================================================
REM Kite Ground Control — Development Server (Windows)
REM Starts the app in development mode with hot-reload
REM ============================================================

echo ============================================
echo  Kite Ground Control — Development Mode
echo ============================================
echo.

if not exist "node_modules" (
    echo Installing dependencies...
    call npm install
)

echo Starting development server...
call npm run tauri dev
