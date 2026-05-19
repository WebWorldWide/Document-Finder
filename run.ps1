# Document Finder — Windows build + install + launch.
# PowerShell equivalent of run.sh. Edit code, run this, see the changes.
# `release-fast` Cargo profile = thin LTO + parallel codegen (~30-40s warm).
# For sub-5s iteration during development run `pnpm tauri dev` instead.

$ErrorActionPreference = "Stop"
Set-Location -Path $PSScriptRoot

function Info($msg)  { Write-Host "→ $msg" -ForegroundColor Cyan }
function Ok($msg)    { Write-Host "✓ $msg" -ForegroundColor Green }
function Warn($msg)  { Write-Host "! $msg" -ForegroundColor Yellow }
function Fail($msg)  { Write-Host "✗ $msg" -ForegroundColor Red; exit 1 }

Write-Host "Document Finder — building latest changes (windows)" -ForegroundColor Cyan

# Toolchain checks ------------------------------------------------------
if (-not (Get-Command rustc -ErrorAction SilentlyContinue)) {
    Fail "Rust not found. Install at: https://rustup.rs/"
}
Ok ("Rust " + ((rustc --version) -split ' ')[1])

if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
    Fail "Node.js not found. Install at: https://nodejs.org/"
}
Ok ("Node " + (node --version))

if (-not (Get-Command pnpm -ErrorAction SilentlyContinue)) {
    Info "Installing pnpm..."
    npm install -g pnpm
}
Ok ("pnpm " + (pnpm --version))

if (-not (Test-Path "node_modules")) {
    Info "Installing dependencies..."
    pnpm install
}

# Stop any running instance ---------------------------------------------
# The .exe inside the bundle is "document-finder.exe" — match the bare
# process name (without .exe) for Stop-Process.
Info "Stopping any running Document Finder instance..."
Get-Process -Name "document-finder" -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 1

# Build ---------------------------------------------------------------
# Bundle as MSI by default (the standard Windows installer flavor). If
# you'd rather have NSIS, swap --bundles msi for --bundles nsis.
Info "Building Windows binary + .msi (parallel codegen)..."
pnpm tauri build --bundles msi -- --profile release-fast

$BinPath = "src-tauri\target\release-fast\document-finder.exe"
if (-not (Test-Path $BinPath)) {
    Fail "Build finished but '$BinPath' not found."
}

# Install ---------------------------------------------------------------
# On Windows we just copy the .exe + its dependencies somewhere stable
# (LocalAppData), since silently running an .msi installer requires
# elevation. Power users can double-click the .msi in
# src-tauri\target\release-fast\bundle\msi\ for a real Start-menu install.
$InstallDir = "$env:LOCALAPPDATA\Programs\Document Finder"
Info "Installing to $InstallDir..."
if (Test-Path $InstallDir) { Remove-Item -Recurse -Force $InstallDir }
New-Item -ItemType Directory -Path $InstallDir | Out-Null
Copy-Item -Path $BinPath -Destination $InstallDir
# Copy any sidecar DLLs Tauri emits next to the binary.
Get-ChildItem -Path "src-tauri\target\release-fast" -Filter *.dll -ErrorAction SilentlyContinue |
    ForEach-Object { Copy-Item -Path $_.FullName -Destination $InstallDir }
Ok "Installed $InstallDir\document-finder.exe"

# Launch ----------------------------------------------------------------
Info "Launching $InstallDir\document-finder.exe ..."
Start-Process -FilePath "$InstallDir\document-finder.exe" -WorkingDirectory $InstallDir
Ok "Done"
