#!/bin/sh
# Build Linux release binary and package it into cloudflare-web/dist/
# Run this script from the repo root (starcode-cli-main/)
set -e

ROOT="$(cd "$(dirname "$0")" && pwd)"
DIST="${ROOT}/cloudflare-web/dist"

echo "Building starcode-cli for Linux (release)..."
cd "${ROOT}/starcode-cli"
cargo build --release --locked
cd "${ROOT}"

# Extract version from Cargo.toml (single source of truth)
VERSION=$(grep -m1 '^version' "${ROOT}/starcode-cli/Cargo.toml" | sed 's/.*"\(.*\)"/\1/')
echo "v${VERSION}" > "${DIST}/version.txt"
echo "Version: v${VERSION}"

BIN_SRC="${ROOT}/starcode-cli/target/release/starcode-cli"
cp "${BIN_SRC}" "${DIST}/starcode-cli"
tar czf "${DIST}/starcode-cli-linux-x86_64.tar.gz" -C "${DIST}" starcode-cli
rm "${DIST}/starcode-cli"

echo ""
echo "Done: ${DIST}/starcode-cli-linux-x86_64.tar.gz  (v${VERSION})"
