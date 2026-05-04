#!/bin/bash

# Document Finder v2 - Production Build Script
# This script compiles the app into a native macOS installer (.app/.dmg).

set -e

GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}==> Starting Production Build...${NC}"

# Ensure dependencies are current
pnpm install

# Build the native bundle
pnpm tauri build

echo -e "${GREEN}==> Build Complete!${NC}"
echo -e "${GREEN}==> Finder is opening the release folder...${NC}"

# Open the folder containing the finished app
open src-tauri/target/release/bundle/macos/
