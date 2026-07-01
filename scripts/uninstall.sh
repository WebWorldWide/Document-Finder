#!/usr/bin/env bash
# Document Finder - clean uninstall (macOS + Linux).
#
# Removes the per-user data Document Finder writes to disk. Run this after
# removing the app itself (see the per-OS notes printed at the end). It deletes
# ONLY Document-Finder-owned locations. A CUSTOM library root set in Settings is
# NOT auto-discovered here - use the in-app "Erase app data" (Settings > Danger
# zone), which knows the custom location, or delete that folder manually.
set -u

# Sync: must match tauri.conf.json's "identifier" field exactly. Cross-checked
# by src-tauri/tests/linux_identifiers_in_sync.rs.
id="com.webworldwide.documentfinder"

rm_if() {
  if [ -e "$1" ]; then
    if rm -rf -- "$1"; then echo "removed  $1"; else echo "WARN     could not remove $1"; fi
  else
    echo "skip     $1 (not present)"
  fi
}

ask_library() {
  lib="$1"
  if [ -d "$lib" ]; then
    printf "Also delete your document library at '%s'? (downloaded PDFs/EPUBs + databases) [y/N] " "$lib"
    read -r reply
    case "$reply" in
      [Yy]*) rm_if "$lib" ;;
      *) echo "kept     $lib" ;;
    esac
  fi
}

echo "Document Finder - removing app data..."

case "$(uname -s)" in
  Darwin)
    rm_if "$HOME/Library/Application Support/$id"   # AI models + fastembed cache + config
    rm_if "$HOME/Library/Caches/$id"                # webview/app caches
    rm_if "$HOME/Library/WebKit/$id"                # webview storage
    rm_if "$HOME/Library/Saved Application State/$id.savedState"
    rm_if "$HOME/Library/Preferences/$id.plist"
    rm_if "$HOME/Library/Logs/Document Finder"      # run log
    defaults delete "$id" 2>/dev/null || true
    ask_library "$HOME/Documents/Document Finder"
    echo ""
    echo "Done. To remove the app: quit it, then drag /Applications/Document Finder.app to the Trash."
    ;;
  *)
    # Linux (XDG base dirs)
    rm_if "${XDG_DATA_HOME:-$HOME/.local/share}/$id"             # models + fastembed + webview website data
    rm_if "${XDG_CONFIG_HOME:-$HOME/.config}/$id"               # config
    rm_if "${XDG_CACHE_HOME:-$HOME/.cache}/$id"                 # caches
    # Sync: "document-finder" must match src-tauri/src/engine/runlog.rs's Linux
    # state_dir() join. Cross-checked by tests/linux_identifiers_in_sync.rs.
    rm_if "${XDG_STATE_HOME:-$HOME/.local/state}/document-finder"  # run log
    # Flatpak redirects all per-app data into its sandbox under ~/.var/app using
    # the reverse-DNS *app id* (mixed case — distinct from the lowercase runtime
    # identifier above). Without this, a Flatpak user's downloaded AI model
    # weights survive an uninstall.sh that reports it removed everything.
    # Must match the `id:` field in packaging/flatpak/com.webworldwide.DocumentFinder.yml
    # — cross-checked by tests/linux_identifiers_in_sync.rs.
    rm_if "$HOME/.var/app/com.webworldwide.DocumentFinder"        # Flatpak sandbox data (models/config/cache)
    # Resolve the REAL Documents dir — localized / XDG-customized desktops put it
    # somewhere other than ~/Documents, so a non-English user must still be offered
    # the library for removal. Mirrors the app's
    # dirs::document_dir().or_else(dirs::home_dir) exactly: xdg-user-dir requires
    # the same xdg-user-dirs package whose absence makes document_dir() return
    # None, so the no-xdg-user-dirs fallback here must be bare $HOME (NOT
    # $HOME/Documents) to match where the app actually creates the library on
    # that exact configuration.
    docs="$(xdg-user-dir DOCUMENTS 2>/dev/null || true)"
    [ -z "$docs" ] && docs="$HOME"
    ask_library "$docs/Document Finder"
    echo ""
    echo "Done. To remove the app, use whichever you installed:"
    echo "  .deb:     sudo apt purge document-finder"
    echo "  .rpm:     sudo dnf remove document-finder"
    echo "  AppImage: delete the .AppImage file (and ~/.local/share/applications/document-finder*.desktop)"
    echo "  Flatpak:  flatpak uninstall --delete-data com.webworldwide.DocumentFinder"
    ;;
esac

echo "If you set a CUSTOM library folder in Settings, delete it manually or use in-app 'Erase app data' first."
