#!/usr/bin/env pwsh
# Document Finder - clean recompile + launch (Windows).
#
# Like run.ps1, but wipes build artifacts first so you get a from-scratch
# compile. Two clean depths:
#
#   .\rebuild.ps1          Clean just our crate, keep the cached llama.cpp +
#                          ONNX Runtime native builds. Recompiles ALL app code
#                          fresh in ~3-6 min. This is what you want 95% of the time.
#   .\rebuild.ps1 -Full    Nuke the whole target dir, forcing a llama.cpp + ONNX
#                          Runtime rebuild from source (~15-25 min). Use only when
#                          you suspect a stale native dependency.
#
# Other switches:
#   -Dev        After cleaning, run `pnpm tauri dev` (debug + hot reload) instead
#               of a release build. Best for iterating on UI / Rust logic.
#   -NoLaunch   Build only; don't start the app.
param(
    [switch]$Full,
    [switch]$Dev,
    [switch]$NoLaunch
)

$ErrorActionPreference = "Stop"
Set-Location -LiteralPath $PSScriptRoot

function Write-Info($msg)    { Write-Host $msg -ForegroundColor Cyan }
function Write-OK($msg)      { Write-Host $msg -ForegroundColor Green }
function Write-Failure($msg) { Write-Host $msg -ForegroundColor Red }

# 1) Clean ------------------------------------------------------------------
# Stop any running instance FIRST so `cargo clean` can delete the locked
# document-finder.exe. Without this, a clean while the app is open fails with
# "Access is denied. (os error 5)". (run.ps1 kills before building for the same
# reason.)
Write-Info "Stopping any running Document Finder instance..."
Get-Process -Name "document-finder", "Document Finder" -ErrorAction SilentlyContinue |
    ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
Start-Sleep -Milliseconds 500

if ($Full) {
    Write-Info "Full clean - this forces a llama.cpp + ONNX Runtime rebuild (~15-25 min)..."
    cargo clean --manifest-path src-tauri\Cargo.toml
} else {
    Write-Info "Cleaning the document-finder crate (keeping cached native deps)..."
    cargo clean -p document-finder --manifest-path src-tauri\Cargo.toml
}
if ($LASTEXITCODE -ne 0) { throw "cargo clean failed" }
if (Test-Path dist) { Remove-Item -Recurse -Force dist }

# 2) Frontend deps (only if missing) ----------------------------------------
if (-not (Test-Path node_modules)) {
    Write-Info "Installing frontend dependencies..."
    pnpm install --frozen-lockfile --ignore-scripts
    if ($LASTEXITCODE -ne 0) { throw "pnpm install failed" }
}

# 3) Build or run -----------------------------------------------------------
if ($Dev) {
    Write-Info "Launching dev build (hot reload; Ctrl+C to stop)..."
    pnpm tauri dev
    exit $LASTEXITCODE
}

Write-Info "Building (release-fast profile; pins all CPU cores)..."
# Invoke the Tauri CLI's Node entry directly instead of via `pnpm exec`. pnpm
# eats one `--`, and how many survive depends on the shell — that mismatch made
# `--profile` reach cargo wrong and the build fail. Calling node directly means
# the single `--` separator reaches cargo intact. (PowerShell preserves `--`
# when calling a native exe; only pnpm consumed it.)
& node "node_modules\@tauri-apps\cli\tauri.js" build -- --profile release-fast
if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }

$exe = @(
    "src-tauri\target\release-fast\document-finder.exe",
    "src-tauri\target\release-fast\Document Finder.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $exe) {
    Write-Failure "Build succeeded but no exe found under src-tauri\target\release-fast\"
    exit 1
}
Write-OK "Built $exe"

# 4) Launch -----------------------------------------------------------------
if (-not $NoLaunch) {
    Write-Info "Stopping any running instance..."
    Get-Process -Name "document-finder", "Document Finder" -ErrorAction SilentlyContinue |
        ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 500
    Write-Info "Launching..."
    Start-Process -FilePath $exe
    Write-OK "Done"
}
