# ============================================================
# Kite Ground Control — collect build outputs (Windows)
# Gathers the final installers + standalone executable into <repo>/release/
# so they don't have to be hunted down across target/release/bundle/*.
# Called automatically at the end of `just build` / `just build-windows`.
# The release/ folder is git-ignored (local per developer).
# ============================================================
$ErrorActionPreference = 'Stop'

$root = (Resolve-Path "$PSScriptRoot\..").Path
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'src-tauri\target' }
$rel = Join-Path $target 'release'
$bundle = Join-Path $rel 'bundle'
$out = Join-Path $root 'release'

# Fresh folder so it only ever holds the latest build.
if (Test-Path $out) { Remove-Item $out -Recurse -Force }
New-Item -ItemType Directory -Path $out | Out-Null

$collected = @()
function Grab($pattern) {
    if (Test-Path $pattern) {
        Get-ChildItem $pattern | ForEach-Object {
            Copy-Item $_.FullName -Destination $out -Force
            $script:collected += $_.Name
        }
    }
}

# Standalone executable + NSIS installer.
$exe = Join-Path $rel 'kite-gc.exe'
if (Test-Path $exe) { Copy-Item $exe -Destination $out -Force; $collected += 'kite-gc.exe' }
Grab (Join-Path $bundle 'nsis\*-setup.exe')

Write-Host ''
if ($collected.Count -eq 0) {
    Write-Host "[collect-release] No build outputs found under $rel - did the build succeed?" -ForegroundColor Yellow
} else {
    Write-Host "[collect-release] Collected into $out :" -ForegroundColor Green
    $collected | ForEach-Object { Write-Host "  - $_" }
}
