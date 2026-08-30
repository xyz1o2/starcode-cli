#!/bin/sh
# One-command deploy: build binary, generate site, push to Cloudflare Pages
# Usage: ./deploy.sh [--linux] [--windows]
set -e

ROOT="$(cd "$(dirname "$0")" && pwd)"

echo "=== StarCode CLI Deploy ==="

# Step 1 — Build binaries
if [ "$1" = "--windows" ]; then
  echo ">>> Windows build must run on Windows: build-dist-windows.ps1"
elif [ "$1" = "--skip-build" ]; then
  echo ">>> Skipping binary build"
else
  echo ">>> Building Linux binary..."
  "${ROOT}/build-dist-linux.sh"
fi

# Step 2 — Generate static site (postgenerate auto-copies dist/ into .output/public/)
echo ""
echo ">>> Generating static site..."
cd "${ROOT}/cloudflare-web"
npm run generate

# Step 3 — Deploy to Cloudflare Pages
echo ""
echo ">>> Deploying to Cloudflare Pages..."
npx wrangler pages deploy .output/public --project-name starcode-cli --branch main

echo ""
echo "=== Deploy complete ==="
echo "https://starcode.help"
