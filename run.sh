#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${BLUE}Document Finder — Building app${NC}"

# Check Rust
if ! command -v rustc &>/dev/null; then
  echo -e "${RED}✗ Rust not found. Install at: https://rustup.rs/${NC}"
  exit 1
fi
echo -e "${GREEN}✓ Rust $(rustc --version | awk '{print $2}')${NC}"

# Check Node.js
if ! command -v node &>/dev/null; then
  echo -e "${RED}✗ Node.js not found. Install at: https://nodejs.org/${NC}"
  exit 1
fi
echo -e "${GREEN}✓ Node $(node --version)${NC}"

# Check/install pnpm
if ! command -v pnpm &>/dev/null; then
  echo -e "${BLUE}→ Installing pnpm...${NC}"
  npm install -g pnpm
fi
echo -e "${GREEN}✓ pnpm $(pnpm --version)${NC}"

# Install Node dependencies if needed
if [ ! -d "node_modules" ]; then
  echo -e "${BLUE}→ Installing dependencies...${NC}"
  pnpm install
fi

echo -e "${BLUE}→ Building app (first build takes ~2 min while Rust compiles)...${NC}"
pnpm tauri build

# Locate the built .app bundle
APP_PATH="src-tauri/target/release/bundle/macos/Document Finder.app"
DMG_PATH=$(find src-tauri/target/release/bundle/dmg -name "*.dmg" 2>/dev/null | head -1 || true)

echo ""
echo -e "${GREEN}✓ Build complete${NC}"

if [ -d "$APP_PATH" ]; then
  echo -e "${BLUE}→ App bundle: ${APP_PATH}${NC}"

  # Offer to copy to /Applications
  echo ""
  echo -e "${YELLOW}Install to /Applications? [y/N]${NC} \c"
  read -r answer
  if [[ "$answer" =~ ^[Yy]$ ]]; then
    cp -r "$APP_PATH" /Applications/
    echo -e "${GREEN}✓ Installed to /Applications/Document Finder.app${NC}"
    open -a "Document Finder"
  else
    # Just open the bundle in Finder so the user can drag it
    open -R "$APP_PATH"
  fi
elif [ -n "$DMG_PATH" ]; then
  echo -e "${BLUE}→ DMG: ${DMG_PATH}${NC}"
  open "$DMG_PATH"
fi
