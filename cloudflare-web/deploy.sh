#!/usr/bin/env bash
set -euo pipefail

# StarCode CLI Website - Cloudflare Pages Deploy Script
# Usage: ./deploy.sh [--prod]

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

BOLD="\033[1m"
GREEN="\033[32m"
YELLOW="\033[33m"
RED="\033[31m"
RESET="\033[0m"

info()  { echo -e "${GREEN}[info]${RESET} $*"; }
warn()  { echo -e "${YELLOW}[warn]${RESET} $*"; }
error() { echo -e "${RED}[error]${RESET} $*" >&2; }

DEPLOY_ENV="preview"
for arg in "$@"; do
  case "$arg" in
    --prod) DEPLOY_ENV="production" ;;
  esac
done

PROJECT_NAME="ai-web"

# ── Check prerequisites ──────────────────────────
check_deps() {
  if ! command -v node &>/dev/null; then
    error "Node.js is required. Install from https://nodejs.org/"
    exit 1
  fi

  if ! command -v npx &>/dev/null; then
    error "npx is required (comes with Node.js)."
    exit 1
  fi
}

# ── Install dependencies ─────────────────────────
install_deps() {
  if [ ! -d "node_modules" ] || [ "package.json" -nt "node_modules/.package-lock.json" ] 2>/dev/null; then
    info "Installing dependencies..."
    npm install
  else
    info "Dependencies up to date."
  fi
}

# ── Build static site ────────────────────────────
build() {
  info "Generating static site..."
  rm -rf .output/dist
  npx nuxi generate 2>&1

  # Ensure output is in the right place
  if [ -d "dist" ] && [ ! -d ".output/public/dist" ]; then
    mkdir -p .output/public/dist
    cp -r dist/* .output/public/dist/ 2>/dev/null || true
  fi

  if [ ! -d ".output/public" ]; then
    error "Build failed: .output/public not found"
    exit 1
  fi

  local file_count
  file_count=$(find .output/public -type f | wc -l)
  info "Build complete: $file_count files in .output/public/"
}

# ── Deploy to Cloudflare Pages ───────────────────
deploy() {
  info "Deploying to Cloudflare Pages ($DEPLOY_ENV)..."

  if [ "$DEPLOY_ENV" = "production" ]; then
    npx wrangler pages deploy .output/public \
      --project-name="$PROJECT_NAME" \
      --branch=main \
      --commit-dirty=true \
      2>&1
  else
    npx wrangler pages deploy .output/public \
      --project-name="$PROJECT_NAME" \
      2>&1
  fi
}

# ── Main ─────────────────────────────────────────
main() {
  echo ""
  echo -e "${BOLD}StarCode CLI Website - Deploy${RESET}"
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
  echo -e "  Environment: ${BOLD}$DEPLOY_ENV${RESET}"
  echo ""

  check_deps
  install_deps
  build
  deploy

  echo ""
  info "Deploy complete!"
  if [ "$DEPLOY_ENV" = "production" ]; then
    echo -e "  ${BOLD}https://starcode.help${RESET}"
  else
    echo -e "  Preview URL shown in the output above."
  fi
  echo ""
}

main
