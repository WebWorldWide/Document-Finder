#!/bin/bash
# SearXNG Setup Script for Document Finder

echo "==> Checking for Docker..."
if ! command -v docker &> /dev/null; then
    echo "Error: Docker is not installed. Please install Docker Desktop first."
    exit 1
fi

echo "==> Pulling SearXNG image..."
docker pull searxng/searxng:latest

echo "==> Starting SearXNG container on port 8080..."
# If it already exists, remove it first
docker rm -f document-finder-searxng 2>/dev/null || true

docker run -d \
  --name document-finder-searxng \
  -p 8080:8080 \
  -v "$(pwd)/searxng:/etc/searxng" \
  searxng/searxng:latest

echo "==> SearXNG is now running at http://localhost:8080"
echo "==> You can now use 'http://localhost:8080' as a search source."
