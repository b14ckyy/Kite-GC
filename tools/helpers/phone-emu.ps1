# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marc Hoffmann (b14ckyy)
#
# Phone-UI iteration loop: boot an Android emulator (or use a USB device) and run the app through
# `tauri android dev` — the UI comes from the Vite dev server with hot reload; no APK, no adb install.
#
#   .\tools\helpers\phone-emu.ps1                      # list the AVDs
#   .\tools\helpers\phone-emu.ps1 Kite_21x9            # boot that AVD + tauri android dev on it
#   .\tools\helpers\phone-emu.ps1 -Device              # skip the emulator: dev on the USB-attached device
#   .\tools\helpers\phone-emu.ps1 -Device -Beside      # ... as "Kite Dev" (com.kitegc.app.dev) BESIDE an
#                                                      # installed copy from Google Play, which a debug
#                                                      # build cannot replace (different signature)
#   .\tools\helpers\phone-emu.ps1 -Shot                # screenshot of the running app (see phone-devtools.mjs)
#
# The app is always landscape (sensorLandscape); -Landscape only pins the emulator's own orientation.
#
# AVDs (android-34 google_apis x86_64, host GPU):
#   Kite_5in_16x9   5.0"  1080×1920  16:9    (pixel profile)
#   Kite_6in_20x9   6.4"  1080×2400  20:9    (pixel_6 profile)
#   Kite_21x9       6.0"  1080×2520  21:9    (Xperia-style, custom lcd override)
#   Kite_AX12_A9    5.5"  1280×720   API 28  (RadioMaster AX12 stand-in: Android 9 + sideloaded WebView 138)
#   Kite_AX12       5.5"  1280×720   API 34  (same panel, current Android — geometry only)
# Kite_AX12_A9 MUST boot with -writable-system, or Android drops the sideloaded WebView and the dev build
# dies with "SyntaxError: Unexpected token ?". Restore: reinstall the WebView 138 APK and
# `adb shell cmd webviewupdate set-webview-implementation com.google.android.webview`.
# Data inside the emulator: no USB serial / BLE, but network — the host is 10.0.2.2 (SITL over
# TCP, MAVLink UDP), or import a .kflight and replay it.
# Stop: Ctrl+C in this terminal ends tauri dev + vite; close the emulator window separately.
# If a later start says "Port 1420 is already in use", a previous vite is still alive — kill it.

param(
  [string]$Avd = '',
  [switch]$Device,
  [switch]$Beside,
  [switch]$Landscape,
  [switch]$Shot,
  [string]$ShotPath = ''
)
$serial = ''

$sdk = Join-Path $env:LOCALAPPDATA 'Android\Sdk'
$emu = Join-Path $sdk 'emulator\emulator.exe'
$adb = Join-Path $sdk 'platform-tools\adb.exe'
$repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)   # tools/helpers → repo root

# Every adb call is pinned to ONE serial: with a phone on the cable next to the emulator, a bare
# `adb shell` answers "more than one device" and a boot-wait loop on it never ends.
function Get-Serial([bool]$wantEmulator) {
  $rows = (& $adb devices) | Where-Object { $_ -match "`tdevice$" } | ForEach-Object { ($_ -split "\s+")[0] }
  if ($wantEmulator) { return ($rows | Where-Object { $_ -like 'emulator-*' } | Select-Object -First 1) }
  return ($rows | Where-Object { $_ -notlike 'emulator-*' } | Select-Object -First 1)
}

function Wait-Boot {
  Write-Host 'waiting for the emulator to come up ...'
  do { Start-Sleep -Seconds 2; $script:serial = Get-Serial $true } while (-not $script:serial)
  Write-Host "waiting for Android ($script:serial) to finish booting ..."
  do {
    Start-Sleep -Seconds 2
    $booted = (& $adb -s $script:serial shell getprop sys.boot_completed 2>$null)
  } while ("$booted".Trim() -ne '1')
}

function Save-Shot {
  param([string]$Path)
  if (-not $Path) { $Path = Join-Path $repo ('phone-shot-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '.png') }
  # NOT `adb screencap`: Kite's window is transparent (native-video hole punch) and the emulator's
  # screencap returns black for it. The DevTools capture reads the WebView's own pixels.
  node (Join-Path $PSScriptRoot 'phone-devtools.mjs') shot $Path
}

if ($Shot) {
  Save-Shot $ShotPath
  exit 0
}

if (-not $Device) {
  if (-not $Avd) {
    Write-Host 'Available AVDs:'
    & $emu -list-avds
    Write-Host ''
    Write-Host 'usage: .\tools\helpers\phone-emu.ps1 <AVD> [-Landscape]   |   .\tools\helpers\phone-emu.ps1 -Device'
    exit 0
  }
  $running = (& $adb devices) -match '^emulator-'
  if (-not $running) {
    Write-Host "booting emulator $Avd ..."
    $emuArgs = @('-avd', $Avd, '-gpu', 'host', '-no-snapshot-load', '-no-boot-anim')
    # See the AVD list above: the AX12 stand-in keeps its sideloaded WebView only with the flag.
    if ($Avd -eq 'Kite_AX12_A9') { $emuArgs += '-writable-system' }
    Start-Process -FilePath $emu -ArgumentList $emuArgs
  } else {
    Write-Host 'an emulator is already running — using it'
  }
  Wait-Boot
  $rot = if ($Landscape) { 1 } else { 0 }
  & $adb -s $serial shell settings put system accelerometer_rotation 0
  & $adb -s $serial shell settings put system user_rotation $rot
} else {
  $serial = Get-Serial $false
  if (-not $serial) { Write-Host 'no USB device attached'; exit 1 }
}

# Dev server over the adb bridge, NOT the network: through the emulator's NAT, module requests stalled
# for minutes. `adb reverse` + `--host 127.0.0.1` points the WebView at the device's own 127.0.0.1:1420.
& $adb -s $serial reverse tcp:1420 tcp:1420 | Out-Null
Set-Location $repo
if ($Beside) {
  # The debug build gets its own application id, but the CLI starts the OTHER copy — a watcher starts
  # the dev copy once Gradle installed it. HMR does not reach it; reload with
  #   node tools/helpers/phone-devtools.mjs eval "location.reload()"
  $env:KITE_ANDROID_DEV_ID = '1'
  $before = "$((& $adb -s $serial shell dumpsys package com.kitegc.app.dev) -match 'lastUpdateTime' | Select-Object -First 1)"
  Start-Job -ArgumentList $adb, $serial, $before -ScriptBlock {
    param($adb, $serial, $before)
    for ($i = 0; $i -lt 600; $i++) {
      Start-Sleep -Seconds 3
      $now = "$((& $adb -s $serial shell dumpsys package com.kitegc.app.dev) -match 'lastUpdateTime' | Select-Object -First 1)"
      if ($now -and $now -ne $before) {
        Start-Sleep -Seconds 4   # let the CLI start the other copy first, then come to the front
        & $adb -s $serial shell am start -n com.kitegc.app.dev/com.kitegc.app.MainActivity | Out-Null
        break
      }
    }
  } | Out-Null
}
try {
  if ($Device) {
    npx tauri android dev --host 127.0.0.1
  } else {
    npx tauri android dev --host 127.0.0.1 $Avd
  }
} finally {
  # The script shares the caller's environment: without this, every later dev build from the same
  # terminal would silently become the .dev copy.
  if ($Beside) { Remove-Item Env:KITE_ANDROID_DEV_ID -ErrorAction SilentlyContinue }
}
