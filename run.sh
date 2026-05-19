#!/usr/bin/env bash
# Document Finder — build + install + launch.
# Edit code, run this, see the changes. One step.
#
# This builds with the `release-fast` Cargo profile (parallelized codegen +
# thin LTO) so the build pins all CPU cores instead of crawling on one. For
# a *much* faster iteration loop while developing (~3–5s per Rust change,
# instant frontend hot reload), use `pnpm tauri dev` instead — it skips the
# bundle step and uses the debug profile.
set -euo pipefail
cd "$(dirname "$0")"

GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m'

echo -e "${BLUE}Document Finder — building latest changes${NC}"

# Toolchain checks ----------------------------------------------------------
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

# Install Node dependencies if needed --------------------------------------
if [ ! -d "node_modules" ]; then
  echo -e "${BLUE}→ Installing dependencies...${NC}"
  pnpm install
fi

# Build --------------------------------------------------------------------
# `-- --profile release-fast` passes through to cargo. The custom profile
# parallelizes codegen so this finishes in ~30–40s on a warm cache instead
# of the ~2 min the strict `release` profile takes.
echo -e "${BLUE}→ Building app (parallel codegen — should pin all CPU cores)...${NC}"
pnpm tauri build -- --profile release-fast

APP_PATH="src-tauri/target/release-fast/bundle/macos/Document Finder.app"
if [ ! -d "$APP_PATH" ]; then
  echo -e "${RED}✗ Build finished but '$APP_PATH' not found.${NC}"
  exit 1
fi

# Stop any running instance so the new build replaces it cleanly. The
# binary inside the bundle is named `document-finder` (lowercase,
# hyphenated — see CFBundleExecutable in Info.plist), so we have to match
# *that*, not the "Document Finder" display name. The old `pkill -x
# "Document Finder"` silently never matched anything, which is how stale
# instances kept running after a rebuild.
echo -e "${BLUE}→ Stopping any running Document Finder instance...${NC}"
pkill -x "document-finder" 2>/dev/null || true
sleep 1

# Eject any stray Document Finder DMG that might still be mounted from a
# prior install. If a DMG copy of the app is mounted, macOS Launch
# Services will happily prefer it over /Applications/, and `open -a`
# brings the DMG-mounted (stale) app to the foreground instead of the
# freshly built one. Silent failure is fine — usually nothing is mounted.
for vol in /Volumes/dmg.* /Volumes/Document*Finder*; do
  [ -d "$vol/Document Finder.app" ] || continue
  echo -e "${BLUE}→ Ejecting stale DMG at $vol...${NC}"
  hdiutil detach "$vol" >/dev/null 2>&1 || true
done

# Install to /Applications so the new build *replaces* the old one. Without
# this, double-clicking the previously installed copy would still launch
# stale code and the changes wouldn't appear to "take effect".
echo -e "${BLUE}→ Installing to /Applications...${NC}"
rm -rf "/Applications/Document Finder.app"
cp -R "$APP_PATH" /Applications/
echo -e "${GREEN}✓ Installed /Applications/Document Finder.app${NC}"

# Launch -------------------------------------------------------------------
# Use the explicit -a path (not the bundle ID) so Launch Services can't
# re-resolve to a different "Document Finder.app" cached from a DMG or
# Downloads folder.
echo -e "${BLUE}→ Launching /Applications/Document Finder.app ...${NC}"
open "/Applications/Document Finder.app"
echo -e "${GREEN}✓ Done${NC}"
