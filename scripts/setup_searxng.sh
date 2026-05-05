#!/bin/bash
# SearXNG Setup Script for Document Finder

echo "==> Checking for Docker..."
if ! command -v docker &> /dev/null; then
    echo "Error: Docker is not installed. Install Docker Desktop from https://www.docker.com/products/docker-desktop/"
    exit 1
fi

if ! docker info &> /dev/null; then
    echo "Error: Docker is not running. Please start Docker Desktop and try again."
    exit 1
fi

# If container already running, just report success
if docker inspect document-finder-searxng --format '{{.State.Running}}' 2>/dev/null | grep -q "^true$"; then
    echo "==> SearXNG is already running."
    echo "SEARXNG_URL=http://localhost:8080"
    exit 0
fi

echo "==> Pulling SearXNG image (may take a few minutes on first run)..."
docker pull searxng/searxng:latest

echo "==> Starting SearXNG container on port 8080..."
docker rm -f document-finder-searxng 2>/dev/null || true

docker run -d \
  --name document-finder-searxng \
  --restart unless-stopped \
  -p 8080:8080 \
  -e INSTANCE_NAME="Document Finder" \
  searxng/searxng:latest

echo "==> Waiting for SearXNG to become ready..."
for i in $(seq 1 15); do
    if curl -sf http://localhost:8080/healthz > /dev/null 2>&1 || \
       curl -sf http://localhost:8080/ > /dev/null 2>&1; then
        echo "==> SearXNG is ready."
        echo "SEARXNG_URL=http://localhost:8080"
        exit 0
    fi
    sleep 2
done

echo "==> SearXNG started but health check timed out. It may still be starting up."
echo "SEARXNG_URL=http://localhost:8080"
