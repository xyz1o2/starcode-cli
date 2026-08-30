#!/usr/bin/env bash
set -euo pipefail

# StarCode CLI - Publish all npm packages
# Usage: ./publish-npm.sh [version]
#
# Prerequisites:
#   - npm login (npm login)
#   - Rust toolchain installed
#   - musl cross-compiler (auto-downloaded if missing)

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
NPM_DIR="$SCRIPT_DIR/npm"
VERSION="${1:-}"

BOLD="\033[1m"
GREEN="\033[32m"
YELLOW="\033[33m"
RED="\033[31m"
RESET="\033[0m"

info()  { echo -e "${GREEN}[info]${RESET} $*"; }
warn()  { echo -e "${YELLOW}[warn]${RESET} $*"; }
error() { echo -e "${RED}[error]${RESET} $*" >&2; }

# ── Validate version ──────────────────────────────
if [ -z "$VERSION" ]; then
  # Read from main package.json
  VERSION=$(node -e "console.log(require('$NPM_DIR/starcode-cli/package.json').version)")
  info "No version arg, using package.json version: $VERSION"
fi

# Validate semver
if ! echo "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$'; then
  error "Invalid version: $VERSION (expected semver like 1.2.3)"
  exit 1
fi

info "Publishing starcode-cli v$VERSION"

# ── Check npm auth ────────────────────────────────
if ! npm whoami &>/dev/null; then
  error "Not logged in to npm. Run 'npm login' first."
  exit 1
fi
info "Logged in as: $(npm whoami)"

# ── Build musl binary ─────────────────────────────
MUSL_CC="$SCRIPT_DIR/.musl-cross/bin/x86_64-linux-musl-gcc"

ensure_musl() {
  if [ -f "$MUSL_CC" ]; then
    return
  fi
  info "Downloading musl cross-compiler..."
  mkdir -p "$SCRIPT_DIR/.musl-cross"
  curl -sL https://musl.cc/x86_64-linux-musl-cross.tgz \
    | tar xz --strip-components=1 -C "$SCRIPT_DIR/.musl-cross"
  info "musl cross-compiler ready"
}

build_binary() {
  info "Building release binary (x86_64-unknown-linux-musl)..."
  ensure_musl

  export CC="$MUSL_CC"
  export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="$MUSL_CC"

  RUSTFLAGS="-C strip=symbols -C codegen-units=1" \
    cargo build --release --target x86_64-unknown-linux-musl --manifest-path "$SCRIPT_DIR/Cargo.toml"

  local bin="$SCRIPT_DIR/target/x86_64-unknown-linux-musl/release/starcode-cli"
  if [ ! -f "$bin" ]; then
    error "Build failed: binary not found at $bin"
    exit 1
  fi

  cp "$bin" "$NPM_DIR/starcode-cli-linux-x64/starcode-cli"
  chmod +x "$NPM_DIR/starcode-cli-linux-x64/starcode-cli"
  info "Binary size: $(du -h "$bin" | cut -f1)"
}

# ── Update versions ───────────────────────────────
update_versions() {
  info "Updating package versions to $VERSION..."

  # Platform package
  node -e "
    const fs = require('fs');
    const p = '$NPM_DIR/starcode-cli-linux-x64/package.json';
    const d = JSON.parse(fs.readFileSync(p, 'utf8'));
    d.version = '$VERSION';
    fs.writeFileSync(p, JSON.stringify(d, null, 2) + '\n');
  "

  # Main package
  node -e "
    const fs = require('fs');
    const p = '$NPM_DIR/starcode-cli/package.json';
    const d = JSON.parse(fs.readFileSync(p, 'utf8'));
    d.version = '$VERSION';
    d.optionalDependencies['@starcode-cli/cli-linux-x64'] = '$VERSION';
    fs.writeFileSync(p, JSON.stringify(d, null, 2) + '\n');
  "
}

# ── Publish ───────────────────────────────────────
publish_packages() {
  info "Publishing @starcode-cli/cli-linux-x64@$VERSION..."
  cd "$NPM_DIR/starcode-cli-linux-x64"
  npm publish --access public 2>&1
  cd "$SCRIPT_DIR"

  info "Publishing starcode-cli@$VERSION..."
  cd "$NPM_DIR/starcode-cli"
  npm publish --access public 2>&1
  cd "$SCRIPT_DIR"
}

# ── Verify ────────────────────────────────────────
verify_published() {
  info "Verifying published packages..."

  if npm view "starcode-cli@$VERSION" &>/dev/null; then
    info "  ✓ starcode-cli@$VERSION"
  else
    error "  ✗ starcode-cli@$VERSION not found"
  fi

  if npm view "@starcode-cli/cli-linux-x64@$VERSION" &>/dev/null; then
    info "  ✓ @starcode-cli/cli-linux-x64@$VERSION"
  else
    error "  ✗ @starcode-cli/cli-linux-x64@$VERSION not found"
  fi
}

# ── Cleanup ───────────────────────────────────────
cleanup() {
  rm -rf "$SCRIPT_DIR/.musl-cross"
}

# ── Main ──────────────────────────────────────────
main() {
  echo ""
  echo -e "${BOLD}StarCode CLI - npm Publish${RESET}"
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
  echo ""

  build_binary
  update_versions
  publish_packages
  verify_published
  cleanup

  echo ""
  info "Done! Users can install with:"
  echo ""
  echo "  npm install -g starcode-cli@$VERSION"
  echo "  # or"
  echo "  npx starcode-cli --help"
  echo ""
}

main
