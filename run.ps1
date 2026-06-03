#!/usr/bin/env pwsh
# Document Finder - Windows build + launch.
# Edit code, run this, see the changes. One step.
#
# Builds with the `release-fast` Cargo profile (parallelized codegen +
# thin LTO) so the build pins all CPU cores. For faster iteration while
# developing (~3-5s per Rust change, instant frontend hot reload), use
# `pnpm tauri dev` instead - it skips the bundle step and uses debug.
$ErrorActionPreference = "Stop"
Set-Location -LiteralPath $PSScriptRoot

function Write-Info($msg)    { Write-Host $msg -ForegroundColor Cyan }
function Write-OK($msg)      { Write-Host $msg -ForegroundColor Green }
function Write-Failure($msg) { Write-Host $msg -ForegroundColor Red }

Write-Info "Document Finder - building latest changes"

# Toolchain checks ----------------------------------------------------------
if (-not (Get-Command rustc -ErrorAction SilentlyContinue)) {
    Write-Failure "Rust not found. Install at: https://rustup.rs/"
    exit 1
}
$rustVersion = (& rustc --version) -split ' ' | Select-Object -Index 1
Write-OK "Rust $rustVersion"

if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
    Write-Failure "Node.js not found. Install at: https://nodejs.org/"
    exit 1
}
Write-OK "Node $(& node --version)"

if (-not (Get-Command pnpm -ErrorAction SilentlyContinue)) {
    Write-Info "Installing pnpm..."
    & npm install -g pnpm
    if ($LASTEXITCODE -ne 0) { throw "npm install -g pnpm failed" }
}
Write-OK "pnpm $(& pnpm --version)"

# Install Node dependencies if needed --------------------------------------
if (-not (Test-Path "node_modules")) {
    Write-Info "Installing dependencies..."
    & pnpm install
    if ($LASTEXITCODE -ne 0) { throw "pnpm install failed" }
}

# Stop any running instance BEFORE building. On Windows the linker cannot
# overwrite a currently-executing .exe (the OS locks it), so a build while the
# app from a prior run is still open fails with "Access is denied (os error 5)".
# Stopping first also means the launch below cleanly replaces the old instance.
Write-Info "Stopping any running Document Finder instance..."
$null = Get-Process -Name "document-finder","Document Finder" -ErrorAction SilentlyContinue |
    ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
Start-Sleep -Milliseconds 500

# Build --------------------------------------------------------------------
Write-Info "Building app (parallel codegen - should pin all CPU cores)..."
# Invoke the Tauri CLI's Node entry directly instead of via `pnpm exec`. pnpm
# eats one `--`, and how many survive depends on the shell — that mismatch made
# `--profile` reach cargo wrong and the build fail. Calling node directly means
# the single `--` separator reaches cargo intact. (PowerShell preserves `--`
# when calling a native exe; only pnpm consumed it.)
& node "node_modules\@tauri-apps\cli\tauri.js" build -- --profile release-fast
if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }

$exeCandidates = @(
    "src-tauri\target\release-fast\document-finder.exe",
    "src-tauri\target\release-fast\Document Finder.exe"
)
$exePath = $exeCandidates | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $exePath) {
    Write-Failure "Build finished but no document-finder.exe found under src-tauri\target\release-fast\"
    exit 1
}
Write-OK "Built $exePath"

# Launch -------------------------------------------------------------------
Write-Info "Launching..."
Start-Process -FilePath $exePath
Write-OK "Done"
