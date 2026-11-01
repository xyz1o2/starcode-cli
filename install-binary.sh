#!/bin/sh
# 下载并安装 StarCode CLI 的预编译二进制（Linux / WSL / macOS）。
#
# 用法（README 里的推荐入口）：
#   curl -fsSL https://raw.githubusercontent.com/xyz1o2/starcode-cli/dev-m/install-binary.sh | sh
#
# 从源码编译请改用 ./install.sh。
#
# 设计要点：
# - 纯 POSIX sh（dash/busybox ash 也能跑），因为经 `curl | sh` 管道执行时
#   shebang 会被忽略，脚本实际跑在管道前端的 shell 里；
# - 产物名是 starcode-cli-<version>-<target>.tar.gz，版本号位可能为空
#   （历史命名问题），所以一律按「-<target>.tar.gz」后缀匹配，新旧命名通吃；
# - 校验 SHA256SUMS.txt（sha256sum 不可用时退 shasum，都没有就跳过并提示）。

set -eu

REPO="xyz1o2/starcode-cli"
BIN_NAME="starcode-cli"
ALIASES="sc starcode"

die() { echo "error: $*" >&2; exit 1; }

# ---- 平台检测 ------------------------------------------------------------
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Linux)
    # Alpine 等 musl 系发行版优先静态 musl 包；glibc 系优先原生 gnu 包，
    # 拿不到再退 musl（静态包在 glibc 上也能跑）。/etc/alpine-release 是
    # Alpine 的可靠指纹。
    if [ -f /etc/alpine-release ]; then
      FIRST=musl
      SECOND=gnu
    else
      FIRST=gnu
      SECOND=musl
    fi
    case "$ARCH" in
      x86_64)
        TARGETS="x86_64-unknown-linux-$FIRST x86_64-unknown-linux-$SECOND" ;;
      aarch64|arm64)
        TARGETS="aarch64-unknown-linux-$FIRST aarch64-unknown-linux-$SECOND" ;;
      *) die "unsupported Linux arch: $ARCH" ;;
    esac ;;
  Darwin)
    case "$ARCH" in
      x86_64)        TARGETS="x86_64-apple-darwin" ;;
      arm64|aarch64) TARGETS="aarch64-apple-darwin" ;;
      *) die "unsupported macOS arch: $ARCH" ;;
    esac ;;
  MINGW*|MSYS*|CYGWIN*)
    die "Git Bash / MSYS / Cygwin 检测到 Windows：请用 PowerShell 运行
  irm https://raw.githubusercontent.com/xyz1o2/starcode-cli/dev-m/install-binary.ps1 | iex" ;;
  *) die "unsupported OS: $OS (Windows 请用 install-binary.ps1)" ;;
esac

# ---- 取最新 release 的产物列表 -------------------------------------------
# 依赖 jq 不可取（macOS 不自带），直接 grep JSON。
API="https://api.github.com/repos/$REPO/releases/latest"
ASSETS="$(curl -fsSL "$API")" || die "failed to fetch release info from $API"

# 按优先级挑一个存在的产物。返回 URL，不存在则输出空串。
# 管道里 grep 无匹配返回 1，set -e 下要 || true 兜住。
pick_asset() {
  printf '%s' "$ASSETS" \
    | grep -oE "\"browser_download_url\": *\"[^\"]*-$1\.tar\.gz\"" \
    | head -n 1 | cut -d'"' -f4 || true
}

URL=""
TARGET=""
for t in $TARGETS; do
  cand="$(pick_asset "$t")"
  if [ -n "$cand" ]; then URL="$cand"; TARGET="$t"; break; fi
done
[ -n "$URL" ] || die "latest release has no archive for $OS $ARCH (tried: $TARGETS)"

FILE="$(basename "$URL")"
echo "Latest release: $FILE"

# ---- 下载 ----------------------------------------------------------------
TMP="$(mktemp -d 2>/dev/null || mktemp -d /tmp/sc.XXXXXX)" || die "mktemp failed"
trap 'rm -rf "$TMP"' EXIT INT TERM

curl -fsSL "$URL" -o "$TMP/$FILE" || die "download failed: $URL"

# ---- 校验 SHA256 ----------------------------------------------------------
sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d' ' -f1
  else
    echo ""
  fi
}

SUMS_URL="$(printf '%s' "$ASSETS" \
  | grep -oE '"browser_download_url": *"[^"]*SHA256SUMS\.txt"' \
  | head -n 1 | cut -d'"' -f4 || true)"

CHECKSUM_OK=0
if [ -n "$SUMS_URL" ]; then
  if curl -fsSL "$SUMS_URL" -o "$TMP/SHA256SUMS.txt" 2>/dev/null; then
    expected="$(awk -v f="$FILE" '$2 == f {print $1; exit}' "$TMP/SHA256SUMS.txt" || true)"
    if [ -n "$expected" ]; then
      actual="$(sha256_file "$TMP/$FILE")"
      if [ -z "$actual" ]; then
        echo "  [!] no sha256sum/shasum available, skipped checksum verification"
      elif [ "$actual" != "$expected" ]; then
        die "sha256 mismatch for $FILE (expected $expected, got $actual)"
      else
        CHECKSUM_OK=1
      fi
    fi
  fi
fi
if [ "$CHECKSUM_OK" = 1 ]; then
  echo "  checksum verified (sha256)"
fi

# ---- 解压 ----------------------------------------------------------------
tar -xzf "$TMP/$FILE" -C "$TMP" || die "extraction failed"
BIN="$(find "$TMP" -type f -name "$BIN_NAME" | head -n 1)"
[ -n "$BIN" ] || die "$BIN_NAME not found in the archive"

# ---- 安装（与 install.sh 同一套约定）--------------------------------------
INSTALL_DIR="${HOME}/.cargo/bin"
mkdir -p "$INSTALL_DIR"
# 先删旧文件，避免覆盖正在运行的二进制时 "Text file busy"
rm -f "$INSTALL_DIR/$BIN_NAME" 2>/dev/null || true
cp "$BIN" "$INSTALL_DIR/$BIN_NAME"
chmod +x "$INSTALL_DIR/$BIN_NAME"
echo "  $BIN_NAME -> $INSTALL_DIR"

# 别名做符号链接：覆盖二进制时别名自动跟着更新；文件系统不支持链接就退成拷贝。
for alias_name in $ALIASES; do
  rm -f "$INSTALL_DIR/$alias_name" 2>/dev/null || true
  if ln -s "$BIN_NAME" "$INSTALL_DIR/$alias_name" 2>/dev/null; then
    echo "  $alias_name -> $BIN_NAME"
  else
    cp "$INSTALL_DIR/$BIN_NAME" "$INSTALL_DIR/$alias_name"
    chmod +x "$INSTALL_DIR/$alias_name"
  fi
done

echo ""
echo "Installation successful!"
echo "Installed to: $INSTALL_DIR"
if ! command -v "$BIN_NAME" >/dev/null 2>&1; then
  cat <<EOF

Note: '$INSTALL_DIR' is not on your PATH yet. Add it with:
  echo 'export PATH="\$HOME/.cargo/bin:\$PATH"' >> ~/.bashrc
  # zsh 用户把 ~/.bashrc 换成 ~/.zshrc
EOF
fi
echo "Run 'sc' to start (or 'starcode' / '$BIN_NAME')."
