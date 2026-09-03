# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marc Hoffmann (b14ckyy)
#
# Phone-UI iteration loop: boot an Android emulator (or use a USB device) and run the app through
# `tauri android dev` — Rust is built once, the UI comes from the Vite dev server with hot reload,
# so a saved .svelte file shows up on the device about a second later. No APK, no adb install.
#
#   .\tools\phone-emu.ps1                      # list the AVDs
#   .\tools\phone-emu.ps1 Kite_21x9            # boot that AVD + tauri android dev on it
#   .\tools\phone-emu.ps1 -Device              # skip the emulator: dev on the USB-attached device
#   .\tools\phone-emu.ps1 -Shot                # screenshot of the running app (see phone-devtools.mjs)
#
# The activity is locked to sensorLandscape (AndroidManifest.xml), so the emulator's rotation
# setting does not matter — the app is always landscape; -Landscape only pins the emulator's own
# orientation so the launcher matches.
#
# AVDs (created 2026-09-03, android-34 google_apis x86_64, host GPU):
#   Kite_5in_16x9   5.0"  1080×1920  16:9    (pixel profile)
#   Kite_6in_20x9   6.4"  1080×2400  20:9    (pixel_6 profile)
#   Kite_21x9       6.0"  1080×2520  21:9    (Xperia-style, custom lcd override)
# Data inside the emulator: no USB serial / BLE, but network — the host is 10.0.2.2 (SITL over
# TCP, MAVLink UDP), or import a .kflight and replay it.
# Stop: Ctrl+C in this terminal ends tauri dev + vite; close the emulator window separately.
# If a later start says "Port 1420 is already in use", a previous vite is still alive — kill it.

param(
  [string]$Avd = '',
  [switch]$Device,
  [switch]$Landscape,
  [switch]$Shot,
  [string]$ShotPath = ''
)

$sdk = Join-Path $env:LOCALAPPDATA 'Android\Sdk'
$emu = Join-Path $sdk 'emulator\emulator.exe'
$adb = Join-Path $sdk 'platform-tools\adb.exe'
$repo = Split-Path -Parent $PSScriptRoot

function Wait-Boot {
  & $adb wait-for-device | Out-Null
  Write-Host 'waiting for Android to finish booting ...'
  do {
    Start-Sleep -Seconds 2
    $booted = (& $adb shell getprop sys.boot_completed 2>$null).Trim()
  } while ($booted -ne '1')
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
    Write-Host 'usage: .\tools\phone-emu.ps1 <AVD> [-Landscape]   |   .\tools\phone-emu.ps1 -Device'
    exit 0
  }
  $running = (& $adb devices) -match '^emulator-'
  if (-not $running) {
    Write-Host "booting emulator $Avd ..."
    Start-Process -FilePath $emu -ArgumentList @('-avd', $Avd, '-gpu', 'host', '-no-snapshot-load', '-no-boot-anim')
  } else {
    Write-Host 'an emulator is already running — using it'
  }
  Wait-Boot
  if ($Landscape) {
    & $adb shell settings put system accelerometer_rotation 0
    & $adb shell settings put system user_rotation 1
  } else {
    & $adb shell settings put system accelerometer_rotation 0
    & $adb shell settings put system user_rotation 0
  }
}

# Dev server over the adb bridge, NOT over the network: the CLI's default is the host's LAN IP,
# which the emulator reaches through its NAT — and there module requests stalled for minutes
# (grey screen). `adb reverse` maps the device's own 127.0.0.1:1420 to the host, `--host 127.0.0.1`
# makes Tauri point the WebView (via its dev proxy) at exactly that. Works for USB devices too, no
# Wi-Fi involved. The reverse mapping lives until the device/emulator disconnects.
& $adb reverse tcp:1420 tcp:1420 | Out-Null
Set-Location $repo
if ($Device) {
  npx tauri android dev --host 127.0.0.1
} else {
  npx tauri android dev --host 127.0.0.1 $Avd
}
