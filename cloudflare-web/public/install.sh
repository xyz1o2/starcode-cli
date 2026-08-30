#!/bin/sh
set -e

BIN="starcode-cli"
BASE_URL="https://starcode.help"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"

OS=$(uname -s)
ARCH=$(uname -m)

case "${OS}-${ARCH}" in
  Linux-x86_64)
    ASSET_URL="${BASE_URL}/dist/starcode-cli-linux-x86_64.tar.gz?v=$(date +%s)"
    ;;
  Darwin-*)
    echo ""
    echo "macOS binaries are coming soon!"
    echo ""
    echo "In the meantime, build from source:"
    echo "  git clone https://github.com/xyz1o2/starcode-cli"
    echo "  cd starcode-cli/starcode-cli"
    echo "  cargo install --path ."
    echo ""
    echo "Rust installer: https://rustup.rs"
    exit 0
    ;;
  *)
    echo "Unsupported platform: ${OS} ${ARCH}"
    echo "Please build from source: https://github.com/xyz1o2/starcode-cli"
    exit 1
    ;;
esac

VERSION=$(curl -fsSL "${BASE_URL}/dist/version.txt?v=$(date +%s)" 2>/dev/null | tr -d '[:space:]')
VERSION_LABEL=${VERSION:-"latest"}

echo "Downloading starcode-cli ${VERSION_LABEL}..."

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

curl -fsSL --progress-bar "${ASSET_URL}" | tar -xz -C "${TMP}"

if [ -w "${INSTALL_DIR}" ]; then
  install -m 755 "${TMP}/${BIN}" "${INSTALL_DIR}/${BIN}"
else
  sudo install -m 755 "${TMP}/${BIN}" "${INSTALL_DIR}/${BIN}"
fi

echo ""
echo "starcode-cli ${VERSION_LABEL} installed to ${INSTALL_DIR}/${BIN}"
echo "Run: starcode-cli --help"
