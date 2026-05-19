#!/usr/bin/env bash
# Document Finder — build + install + launch.
# Edit code, run this, see the changes. One step. Cross-platform:
#   - macOS:  builds the .app, installs to /Applications, launches.
#   - Linux:  builds, launches the bare release-fast binary
#             (Tauri also produces .AppImage + .deb if you want a system
#              install — see `pnpm tauri build --bundles appimage,deb`).
#   - Windows: not supported by this script — use `run.ps1` instead.
#
# `release-fast` Cargo profile = thin LTO + parallel codegen so the build
# pins every core (~30–40s warm). For sub-5s iteration during development
# use `pnpm tauri dev` instead — it skips the bundle step entirely.
set -euo pipefail
cd "$(dirname "$0")"

GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
YELLOW='\033[0;33m'
NC='\033[0m'

# OS detection ----------------------------------------------------------
case "$(uname -s 2>/dev/null || echo unknown)" in
  Darwin*) DF_OS=mac ;;
  Linux*)  DF_OS=linux ;;
  MINGW*|MSYS*|CYGWIN*)
    echo -e "${RED}✗ Windows detected.${NC} Use ${YELLOW}run.ps1${NC} in PowerShell instead — bash here can't drive the .msi/.exe install flow."
    exit 1
    ;;
  *)
    echo -e "${RED}✗ Unsupported OS:${NC} $(uname -s)"
    exit 1
    ;;
esac
echo -e "${BLUE}Document Finder — building latest changes ($DF_OS)${NC}"

# Toolchain checks ------------------------------------------------------
if ! command -v rustc &>/dev/null; then
  echo -e "${RED}✗ Rust not found. Install at: https://rustup.rs/${NC}"
  exit 1
fi
echo -e "${GREEN}✓ Rust $(rustc --version | awk '{print $2}')${NC}"

if ! command -v node &>/dev/null; then
  echo -e "${RED}✗ Node.js not found. Install at: https://nodejs.org/${NC}"
  exit 1
fi
echo -e "${GREEN}✓ Node $(node --version)${NC}"

if ! command -v pnpm &>/dev/null; then
  echo -e "${BLUE}→ Installing pnpm...${NC}"
  npm install -g pnpm
fi
echo -e "${GREEN}✓ pnpm $(pnpm --version)${NC}"

if [ ! -d "node_modules" ]; then
  echo -e "${BLUE}→ Installing dependencies...${NC}"
  pnpm install
fi

# Stop any running instance ---------------------------------------------
# The binary inside the bundle is named `document-finder` (lowercase,
# hyphenated — see CFBundleExecutable on macOS, .desktop file on Linux);
# match that, not the "Document Finder" display name.
echo -e "${BLUE}→ Stopping any running Document Finder instance...${NC}"
pkill -x "document-finder" 2>/dev/null || true
sleep 1

# Build -----------------------------------------------------------------
# On macOS we skip the DMG bundle — see the comment block in the
# pre-launch section for the LS-poisoning saga. On Linux we let Tauri
# produce its default bundles (AppImage + deb).
case "$DF_OS" in
  mac)
    echo -e "${BLUE}→ Building macOS .app (parallel codegen)...${NC}"
    pnpm tauri build --bundles app -- --profile release-fast
    APP_PATH="src-tauri/target/release-fast/bundle/macos/Document Finder.app"
    if [ ! -d "$APP_PATH" ]; then
      echo -e "${RED}✗ Build finished but '$APP_PATH' not found.${NC}"
      exit 1
    fi
    ;;
  linux)
    echo -e "${BLUE}→ Building Linux binary (parallel codegen)...${NC}"
    # Skip bundle step for the dev loop — the bare binary at
    # target/release-fast/document-finder is fully functional. If you
    # want .AppImage / .deb, run `pnpm tauri build` without the
    # --bundles flag.
    pnpm tauri build --bundles app -- --profile release-fast || true
    BIN_PATH="src-tauri/target/release-fast/document-finder"
    if [ ! -x "$BIN_PATH" ]; then
      echo -e "${RED}✗ Build finished but '$BIN_PATH' not found or not executable.${NC}"
      exit 1
    fi
    ;;
esac

# macOS-specific install + LS cleanup -----------------------------------
if [ "$DF_OS" = "mac" ]; then
  # Eject stray Document Finder DMGs that might still be mounted, AND
  # unregister any Launch Services records pointing at them. Without
  # this, `open -a` resolves to a random mounted-or-missing DMG copy
  # instead of /Applications.
  shopt -s nullglob 2>/dev/null || setopt null_glob 2>/dev/null || true
  for vol in /Volumes/dmg.* /Volumes/Document*Finder*; do
    [ -d "$vol/Document Finder.app" ] || continue
    echo -e "${BLUE}→ Ejecting stale DMG at $vol...${NC}"
    hdiutil detach "$vol" >/dev/null 2>&1 || true
  done

  LSREG=/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister
  if [ -x "$LSREG" ]; then
    "$LSREG" -dump 2>/dev/null \
      | grep "path:" \
      | grep -i "document.finder" \
      | sed 's/^[[:space:]]*path:[[:space:]]*//;s/ (0x[0-9a-f]*)$//' \
      | while IFS= read -r p; do
          case "$p" in
            "/Applications/Document Finder.app") ;;
            *) "$LSREG" -u "$p" >/dev/null 2>&1 ;;
          esac
        done
  fi

  echo -e "${BLUE}→ Installing to /Applications...${NC}"
  rm -rf "/Applications/Document Finder.app"
  cp -R "$APP_PATH" /Applications/
  echo -e "${GREEN}✓ Installed /Applications/Document Finder.app${NC}"

  echo -e "${BLUE}→ Launching /Applications/Document Finder.app ...${NC}"
  open "/Applications/Document Finder.app"
  echo -e "${GREEN}✓ Done${NC}"
fi

# Linux launch ----------------------------------------------------------
if [ "$DF_OS" = "linux" ]; then
  # Run the bare release-fast binary. Detached with setsid so the script
  # returns immediately and the app keeps running after the terminal
  # closes. stderr/stdout go to /tmp/document-finder.log so users have
  # somewhere to look if it crashes on launch (frontend logs are still
  # available via Settings → Logs).
  LOG=/tmp/document-finder.log
  echo -e "${BLUE}→ Launching $BIN_PATH (log at $LOG) ...${NC}"
  setsid "./$BIN_PATH" </dev/null >"$LOG" 2>&1 &
  disown
  echo -e "${GREEN}✓ Launched (pid $!)${NC}"
fi
