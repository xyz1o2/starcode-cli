#!/bin/bash
set -e

echo "Installing starcode-cli..."

# Detect available cores and pick a CPU-friendly job count
CORES=$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)
JOBS=$(( (CORES + 1) / 2 ))   # ceil(cores/2), at least 1
if [ "$JOBS" -lt 1 ]; then JOBS=1; fi
export CARGO_BUILD_JOBS="$JOBS"

echo "  cores=$CORES jobs=$JOBS"
echo ""

# Build release with CPU-friendly profile overrides.
echo "[1/2] Building release..."
cargo build --release --bin starcode-cli \
    --config 'profile.release.opt-level=2' \
    --config 'profile.release.lto="off"' \
    --config 'profile.release.codegen-units=32'

# Install: just copy the already-built binary (skip cargo install which rebuilds)
echo ""
echo "[2/2] Installing binary..."
INSTALL_DIR="${HOME}/.cargo/bin"
mkdir -p "$INSTALL_DIR"
# Remove old binary first to avoid "Text file busy" error when updating running binary
rm -f "$INSTALL_DIR/starcode-cli" 2>/dev/null || true
cp target/release/starcode-cli "$INSTALL_DIR/starcode-cli"
chmod +x "$INSTALL_DIR/starcode-cli"

echo ""
echo "Installation successful!"
echo "Installed to: $INSTALL_DIR/starcode-cli"
echo "Run 'starcode-cli' to start."
