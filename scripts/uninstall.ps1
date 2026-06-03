#!/usr/bin/env pwsh
# Document Finder - clean uninstall (Windows).
#
# Removes the per-user data Document Finder writes to disk. The Windows
# installer (Settings > Apps > Document Finder > Uninstall) removes the program
# itself and offers a "Delete application data" checkbox; this script clears
# leftovers, or does a full wipe including your downloaded document library.
#
# It deletes ONLY Document-Finder-owned locations (derived from the app
# identifier + the known log dir). A CUSTOM library root set in Settings is NOT
# auto-discovered here - use the in-app "Erase app data" (Settings > Danger
# zone), which knows the custom location, or delete that folder manually.

$ErrorActionPreference = 'Stop'
$id = 'com.webworldwide.documentfinder'

function Remove-IfExists($path) {
  if ($path -and (Test-Path -LiteralPath $path)) {
    try {
      Remove-Item -LiteralPath $path -Recurse -Force
      Write-Host "removed  $path"
    } catch {
      Write-Warning "could not remove $path : $_"
    }
  } else {
    Write-Host "skip     $path (not present)"
  }
}

Write-Host "Document Finder - removing app data..."
Remove-IfExists (Join-Path $env:APPDATA $id)                          # AI models + fastembed cache + config
Remove-IfExists (Join-Path $env:LOCALAPPDATA $id)                     # WebView2 (EBWebView) storage: localStorage/IndexedDB/cache
Remove-IfExists (Join-Path $env:LOCALAPPDATA 'Document Finder\Logs')  # run log

# The document library is YOUR content - delete only on explicit confirmation.
$defaultLib = Join-Path ([Environment]::GetFolderPath('MyDocuments')) 'Document Finder'
if (Test-Path -LiteralPath $defaultLib) {
  $reply = Read-Host "Also delete your document library at '$defaultLib'? (downloaded PDFs/EPUBs + databases) [y/N]"
  if ($reply -match '^[Yy]') {
    Remove-IfExists $defaultLib
  } else {
    Write-Host "kept     $defaultLib"
  }
}

Write-Host ""
Write-Host "Done. To remove the program itself: Settings > Apps > Document Finder > Uninstall."
Write-Host "If you set a CUSTOM library folder in Settings, delete it manually or use in-app 'Erase app data' first."
