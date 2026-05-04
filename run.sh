#!/bin/bash

# Document Finder v2 - Build & Run Script
# This script handles dependency checks, installation, and launching the app.

set -e

# Colors for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo -e "${BLUE}==> Document Finder v2 Starting...${NC}"

# 1. Check for Rust
if ! command -v rustc &> /dev/null; then
    echo -e "${RED}Error: Rust is not installed. Please visit https://rustup.rs/${NC}"
    exit 1
fi

# 2. Check for pnpm
if ! command -v pnpm &> /dev/null; then
    echo -e "${BLUE}==> Installing pnpm...${NC}"
    npm install -g pnpm
fi

# 3. Install Node dependencies if node_modules is missing
if [ ! -d "node_modules" ]; then
    echo -e "${BLUE}==> Installing dependencies...${NC}"
    pnpm install
fi

# 4. Launch the app in dev mode
echo -e "${GREEN}==> Launching app (this may take a minute on first run)...${NC}"
pnpm tauri dev
