# ============================================================
# Kite Ground Control — collect + rename build outputs (Windows)
# Renames Tauri's outputs to a unified scheme and drops them in <repo>/release/:
#
#     KiteGC_Windows_x64_<Version>_<Type>.<ext>
#     KiteGC_Android_<abi>_<Version>_installer.apk      (when an Android release build exists)
#     KiteGC_Android_<abi>_<Version>_playstore.aab      (`just build-android-aab`, for Google Play)
#
#   Type = installer  (NSIS -setup.exe; the Android .apk)
#        | portable   (kite-gc.exe, zipped with an empty `.portable` marker so the download keeps its
#                      data in a data/ folder next to the executable)
#
# Shared by the local `just build*` recipes AND the GitHub release workflow, so the names are identical
# everywhere. release/ is git-ignored and refreshed on every run — `-Keep` adds to it instead.
# `-AndroidOnly` (implies -Keep) collects only the APKs and the Play bundle: the desktop outputs of an
# earlier build still sit in target/release and would be republished under the current version number.
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

# NSIS installer: the NEWEST match, not the first — Tauri never prunes its bundle dir, and the
# alphabetical first match would ship a stale, lower version under the current name.
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

# Delete the consumed raw bundle output, so stale installers cannot pile up and be mis-collected.
if ($collected.Count -gt 0) {
    Remove-Item (Join-Path $bundle 'nsis') -Recurse -Force -ErrorAction SilentlyContinue
}

} # -not $AndroidOnly

# Android: one release APK per ABI folder — newest per folder, raw output deleted afterwards.
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

# Android App Bundle for Google Play: `tauri android build --aab` leaves it under
# outputs\bundle\<abi>Release\. Same handling as the APKs.
$aabRoot = Join-Path $root 'src-tauri\gen\android\app\build\outputs\bundle'
Get-ChildItem $aabRoot -Directory -Filter '*Release' -ErrorAction SilentlyContinue | ForEach-Object {
    $abiDir = $_
    $aab = Get-ChildItem (Join-Path $abiDir.FullName '*.aab') -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if ($aab) {
        $name = $abiDir.Name -replace 'Release$', ''
        $abi = if ($abiNames.ContainsKey($name)) { $abiNames[$name] } else { $name }
        $dest = "${app}_Android_${abi}_${version}_playstore.aab"
        Copy-Item $aab.FullName (Join-Path $out $dest) -Force
        Remove-Item $abiDir.FullName -Recurse -Force -ErrorAction SilentlyContinue
        $script:collected += $dest
    }
}

Write-Host ''
if ($collected.Count -eq 0) {
    $where = if ($AndroidOnly) { Split-Path $apkRoot } else { $rel }
    Write-Host "[collect-release] No build outputs found under $where - did the build succeed?" -ForegroundColor Yellow
} else {
    Write-Host "[collect-release] Collected into $out :" -ForegroundColor Green
    $collected | ForEach-Object { Write-Host "  - $_" }
}
