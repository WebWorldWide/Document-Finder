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
# Invoke the Tauri CLI's Node entry directly rather than `pnpm tauri` so the
# single `--` separator reaches cargo intact (pnpm eats one `--`, and how many
# survive is shell-dependent). The `release-fast` profile parallelizes codegen
# so this finishes in ~30–40s on a warm cache vs ~2 min for strict `release`.
echo -e "${BLUE}→ Building app (parallel codegen — should pin all CPU cores)...${NC}"
node node_modules/@tauri-apps/cli/tauri.js build -- --profile release-fast

# Locate the build output and (re)launch — per-OS, since Tauri emits a
# .app on macOS but a bare ELF binary on Linux (no /Applications, no `open`).
# `pkill` returns 1 when nothing matched — fine, just means the app wasn't
# running — so mute its exit code without disabling errexit globally.
case "$(uname -s)" in
  Darwin)
    APP_PATH="src-tauri/target/release-fast/bundle/macos/Document Finder.app"
    if [ ! -d "$APP_PATH" ]; then
      echo -e "${RED}✗ Build finished but '$APP_PATH' not found.${NC}"
      exit 1
    fi
    echo -e "${BLUE}→ Stopping any running Document Finder instance...${NC}"
    pkill -x "Document Finder" 2>/dev/null || true
    # Give the process a beat to release the bundle before we overwrite it.
    sleep 1
    # Install to /Applications so the new build *replaces* the old one. Without
    # this, double-clicking the previously installed copy would still launch
    # stale code and the changes wouldn't appear to "take effect".
    echo -e "${BLUE}→ Installing to /Applications...${NC}"
    rm -rf "/Applications/Document Finder.app"
    cp -R "$APP_PATH" /Applications/
    echo -e "${GREEN}✓ Installed /Applications/Document Finder.app${NC}"
    echo -e "${BLUE}→ Launching...${NC}"
    open -a "Document Finder"
    ;;
  *)
    # Linux: run the freshly built binary directly. The process name is
    # `document-finder` (not "Document Finder", which is macOS-only).
    BIN="src-tauri/target/release-fast/document-finder"
    if [ ! -x "$BIN" ]; then
      echo -e "${RED}✗ Build finished but '$BIN' not found.${NC}"
      exit 1
    fi
    echo -e "${BLUE}→ Stopping any running Document Finder instance...${NC}"
    pkill -x "document-finder" 2>/dev/null || true
    sleep 1
    echo -e "${BLUE}→ Launching...${NC}"
    "$BIN" &
    ;;
esac
echo -e "${GREEN}✓ Done${NC}"
