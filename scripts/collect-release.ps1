# ============================================================
# Kite Ground Control — collect + rename build outputs (Windows)
# Renames Tauri's outputs to a unified scheme and drops them in <repo>/release/:
#
#     KiteGC_Windows_x64_<Version>_<Type>.<ext>
#     KiteGC_Android_<abi>_<Version>_installer.apk      (when an Android release build exists)
#
#   Type = installer  (NSIS -setup.exe; the Android .apk)
#        | portable   (kite-gc.exe, zipped with an empty `.portable` marker so the download keeps its
#                      data in a data/ folder next to the executable)
#
# One naming source shared by local builds (`just build` / `just build-windows` / `just build-android`)
# AND the GitHub release workflow, so the filenames are identical everywhere. The release/ folder is
# git-ignored and refreshed on every run — `-Keep` adds to it instead.
#
# `-AndroidOnly` collects the APKs alone (implies -Keep). `just build-android` uses it: an Android
# build leaves the DESKTOP outputs of an earlier build untouched in target/release, and collecting
# them would republish a stale binary under the current version number — a 1.0.0-rc2 kite-gc.exe
# came out as KiteGC_Windows_x64_1.1.0-dev_portable.zip (found 2026-09-04).
# ============================================================
param([switch]$Keep, [switch]$AndroidOnly)
if ($AndroidOnly) { $Keep = $true }
$ErrorActionPreference = 'Stop'

$root = (Resolve-Path "$PSScriptRoot\..").Path
$version = (Get-Content (Join-Path $root 'package.json') -Raw | ConvertFrom-Json).version
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'src-tauri\target' }
$rel = Join-Path $target 'release'
$bundle = Join-Path $rel 'bundle'
$out = Join-Path $root 'release'

$app = 'KiteGC'; $os = 'Windows'; $arch = 'x64'
function Get-Name($type, $ext) { "${app}_${os}_${arch}_${version}_${type}.${ext}" }

if (-not $Keep -and (Test-Path $out)) { Remove-Item $out -Recurse -Force }
if (-not (Test-Path $out)) { New-Item -ItemType Directory -Path $out | Out-Null }

$collected = @()

if (-not $AndroidOnly) {

# NSIS installer. Pick the NEWEST match, not the first: Tauri never prunes its bundle dir, so
# stale installers from earlier builds pile up beside the fresh one and a first-match (which is
# alphabetical, so a lower version wins) would ship an old build under the current name. The
# cleanup at the end then removes the raw outputs so the pile-up can't recur.
Get-ChildItem (Join-Path $bundle 'nsis\*-setup.exe') -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1 | ForEach-Object {
    $dest = Get-Name 'installer' 'exe'
    Copy-Item $_.FullName (Join-Path $out $dest) -Force
    $script:collected += $dest
}

# Portable executable -> zip (+ a generated empty `.portable` marker).
$exe = Join-Path $rel 'kite-gc.exe'
if (Test-Path $exe) {
    $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ('kitegc-portable-' + [guid]::NewGuid())
    New-Item -ItemType Directory -Path $tmp | Out-Null
    Copy-Item $exe (Join-Path $tmp 'kite-gc.exe') -Force
    New-Item -ItemType File -Path (Join-Path $tmp '.portable') | Out-Null
    $dest = Get-Name 'portable' 'zip'
    Compress-Archive -Path (Join-Path $tmp 'kite-gc.exe'), (Join-Path $tmp '.portable') -DestinationPath (Join-Path $out $dest) -Force
    Remove-Item $tmp -Recurse -Force
    $script:collected += $dest
}

# Delete the raw bundle output we just consumed. Tauri never prunes it, and leaving it is exactly
# what lets stale, wrongly-versioned installers accumulate and get mis-collected next time. The
# portable .exe comes straight from $rel\kite-gc.exe (a cargo output, overwritten each build), so
# there's nothing to prune there.
if ($collected.Count -gt 0) {
    Remove-Item (Join-Path $bundle 'nsis') -Recurse -Force -ErrorAction SilentlyContinue
}

} # -not $AndroidOnly

# Android: `tauri android build --apk` leaves one release APK per ABI folder under the Gradle
# outputs (universal / arm64 / armv7 / x86_64 / x86). Same rules as above: newest file per folder,
# then the raw output is deleted so a stale APK can't be re-collected under a newer version.
$apkRoot = Join-Path $root 'src-tauri\gen\android\app\build\outputs\apk'
$abiNames = @{ 'arm64' = 'arm64'; 'armv7' = 'armv7'; 'x86_64' = 'x64'; 'x86' = 'x86'; 'universal' = 'universal' }
Get-ChildItem $apkRoot -Directory -ErrorAction SilentlyContinue | ForEach-Object {
    $abiDir = $_
    $apk = Get-ChildItem (Join-Path $abiDir.FullName 'release\*.apk') -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if ($apk) {
        $abi = if ($abiNames.ContainsKey($abiDir.Name)) { $abiNames[$abiDir.Name] } else { $abiDir.Name }
        $dest = "${app}_Android_${abi}_${version}_installer.apk"
        Copy-Item $apk.FullName (Join-Path $out $dest) -Force
        Remove-Item (Join-Path $abiDir.FullName 'release') -Recurse -Force -ErrorAction SilentlyContinue
        $script:collected += $dest
    }
}

Write-Host ''
if ($collected.Count -eq 0) {
    $where = if ($AndroidOnly) { $apkRoot } else { $rel }
    Write-Host "[collect-release] No build outputs found under $where - did the build succeed?" -ForegroundColor Yellow
} else {
    Write-Host "[collect-release] Collected into $out :" -ForegroundColor Green
    $collected | ForEach-Object { Write-Host "  - $_" }
}
