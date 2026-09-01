#!/usr/bin/env bash
set -euo pipefail

# StarCode CLI - One-click install script
# Usage: curl -fsSL https://raw.githubusercontent.com/your-org/starcode-cli/main/install-npm.sh | bash

BOLD="\033[1m"
GREEN="\033[32m"
YELLOW="\033[33m"
RED="\033[31m"
RESET="\033[0m"

info()  { echo -e "${GREEN}[info]${RESET} $*"; }
warn()  { echo -e "${YELLOW}[warn]${RESET} $*"; }
error() { echo -e "${RED}[error]${RESET} $*" >&2; }

# ── Check prerequisites ──────────────────────────
check_node() {
  if ! command -v node &>/dev/null; then
    error "Node.js is required but not installed."
    echo "  Install Node.js 18+ via: https://nodejs.org/"
    echo "  Or use nvm: curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.0/install.sh | bash"
    exit 1
  fi

  local node_major
  node_major=$(node -e "console.log(process.versions.node.split('.')[0])")
  if [ "$node_major" -lt 18 ]; then
    error "Node.js 18+ required, found $(node -v). Please upgrade."
    exit 1
  fi
}

check_npm() {
  if ! command -v npm &>/dev/null; then
    error "npm is required but not installed."
    echo "  It should come with Node.js. Try reinstalling Node.js."
    exit 1
  fi
}

# ── Install ───────────────────────────────────────
install_global() {
  info "Installing starcode-cli via npm..."
  npm install -g starcode-cli@latest 2>&1
}

# ── Verify ────────────────────────────────────────
verify() {
  if command -v starcode-cli &>/dev/null; then
    info "starcode-cli installed successfully!"
    echo ""
    echo -e "  Run ${BOLD}starcode-cli --help${RESET} to get started."
    echo ""
  else
    warn "starcode-cli was installed but 'starcode-cli' is not in your PATH."
    echo "  You may need to restart your terminal or add npm global bin to PATH:"
    echo ""
    echo "    export PATH=\"\$(npm config get prefix)/bin:\$PATH\""
    echo ""
    echo "  Add the above line to your ~/.bashrc or ~/.zshrc to make it permanent."
    echo ""
    echo "  Or run directly with npx:"
    echo "    npx starcode-cli --help"
  fi
}

# ── Main ──────────────────────────────────────────
main() {
  echo ""
  echo -e "${BOLD}StarCode CLI Installer${RESET}"
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
  echo ""

  check_node
  check_npm
  install_global
  verify
}

main "$@"
