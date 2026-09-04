#!/bin/bash
set -e

# 只构建一个二进制（release 约 55 MB），`sc` 和 `starcode` 做成指向它的符号链接。
# 声明三个 [[bin]] 也能达到同样效果，但那要链三遍、占三份磁盘。
BIN_NAME="starcode-cli"
ALIASES=("sc" "starcode")

echo "Installing ${BIN_NAME}..."

# Detect available cores and pick a CPU-friendly job count
CORES=$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)
JOBS=$(( (CORES + 1) / 2 ))   # ceil(cores/2), at least 1
if [ "$JOBS" -lt 1 ]; then JOBS=1; fi
export CARGO_BUILD_JOBS="$JOBS"

echo "  cores=$CORES jobs=$JOBS"
echo ""

# Build release with CPU-friendly profile overrides.
echo "[1/2] Building release..."
cargo build --release --bin "$BIN_NAME" \
    --config 'profile.release.opt-level=2' \
    --config 'profile.release.lto="off"' \
    --config 'profile.release.codegen-units=32'

# Install: just copy the already-built binary (skip cargo install which rebuilds)
echo ""
echo "[2/2] Installing binary and command aliases..."
INSTALL_DIR="${HOME}/.cargo/bin"
mkdir -p "$INSTALL_DIR"
# Remove old binary first to avoid "Text file busy" error when updating running binary
rm -f "$INSTALL_DIR/$BIN_NAME" 2>/dev/null || true
cp "target/release/$BIN_NAME" "$INSTALL_DIR/$BIN_NAME"
chmod +x "$INSTALL_DIR/$BIN_NAME"
echo "  $BIN_NAME"

# 相对链接目标，安装目录整体搬走也不会断。
# 链接（而不是拷贝）还有个好处：下次覆盖二进制时别名自动跟着更新。
for alias in "${ALIASES[@]}"; do
    rm -f "$INSTALL_DIR/$alias" 2>/dev/null || true
    if ln -s "$BIN_NAME" "$INSTALL_DIR/$alias" 2>/dev/null; then
        echo "  $alias -> $BIN_NAME"
    else
        # 不支持符号链接的文件系统（某些挂载的 exFAT/NTFS）退化成拷贝
        cp "$INSTALL_DIR/$BIN_NAME" "$INSTALL_DIR/$alias"
        chmod +x "$INSTALL_DIR/$alias"
        echo "  $alias (copy — this filesystem has no symlinks)"
    fi
done

echo ""
echo "Installation successful!"
echo "Installed to: $INSTALL_DIR"
echo "Run 'sc' to start (or 'starcode' / '$BIN_NAME')."
if ! command -v sc >/dev/null 2>&1; then
    echo ""
    echo "Note: '$INSTALL_DIR' is not on your PATH yet. Add it with:"
    echo "  export PATH=\"\$HOME/.cargo/bin:\$PATH\""
fi
