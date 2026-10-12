#!/bin/bash
set -e

# 只构建一个二进制（release 约 55 MB），`sc` 和 `starcode` 做成指向它的符号链接。
# 声明三个 [[bin]] 也能达到同样效果，但那要链三遍、占三份磁盘。
BIN_NAME="starcode-cli"
ALIASES=("sc" "starcode")

# ---------------------------------------------------------------------------
# 依赖预检
# ---------------------------------------------------------------------------
# 为什么要在编译前查：Rust 的构建脚本会拉起 C 工具链（proc-macro2/libc/memchr
# 走 `cc` crate，aws-lc-sys 还要 cmake）。缺了会在半路炸出
#   error: linker `cc` not found
# 那时候依赖已经下了几十兆、编了一堆，报错信息还完全不说是缺哪个包。
AUTO_DEPS="${SC_INSTALL_DEPS:-0}"
for arg in "$@"; do
    case "$arg" in
        --install-deps) AUTO_DEPS=1 ;;
        -h|--help)
            echo "Usage: bash install.sh [--install-deps]"
            echo "  --install-deps  缺依赖时自动装（需要 root 或 sudo），默认只提示"
            exit 0 ;;
    esac
done

have() { command -v "$1" >/dev/null 2>&1; }

# 把包名映射到当前系统的包管理器。参数：debian 包名 / rpm 系（含 arch）包名 / apk+brew 包名。
# 没有包管理器时返回 1。
pkg_install_cmd() {
    local deb="$1" rpm="$2" other="$3" cmd sudo=""
    if [ "$(id -u)" -ne 0 ]; then
        if have sudo; then sudo="sudo "
        elif have doas; then sudo="doas "
        else return 1; fi
    fi
    if   have apt-get; then cmd="${sudo}apt-get install -y $deb"
    elif have dnf;     then cmd="${sudo}dnf install -y $rpm"
    elif have yum;     then cmd="${sudo}yum install -y $rpm"
    elif have zypper;  then cmd="${sudo}zypper install -y $rpm"
    elif have pacman;  then cmd="${sudo}pacman -S --needed $rpm"
    elif have apk;     then cmd="${sudo}apk add $other"
    elif have brew;    then cmd="brew install $other"          # brew 不能带 sudo
    else return 1; fi
    echo "$cmd"
}

# 缺依赖时按需自动装；不可自动装则原样返回非 0
auto_install() {
    local deb="$1" rpm="$2" other="$3" cmd
    cmd="$(pkg_install_cmd "$deb" "$rpm" "$other")" || return 1
    echo "  运行: $cmd"
    # shellcheck disable=SC2086
    eval "$cmd" || return 1
}

preflight() {
    local os; os="$(uname -s)"
    local missing=""     # 缺的系统包（累积，最后给一条安装命令）
    local fatal=""       # 没法治的（rust 缺失/装错版本）

    # --- Rust 工具链 -------------------------------------------------------
    if ! have cargo; then
        fatal="${fatal}  [x] cargo 不在 PATH\n"
    elif [ "$os" = "Linux" ]; then
        # WSL 常见坑：装的是 Windows 版 Rust（x86_64-pc-windows-msvc），在 Linux 下编不动。
        local host; host="$(rustc -vV 2>/dev/null | sed -n 's/^host: //p')"
        case "$host" in
            *linux*) ;;
            *) fatal="${fatal}  [x] rustc 的 host 是 ${host:-未知}，不是 Linux 目标\n"
               fatal="${fatal}      WSL 里要装 Linux 版 Rust：curl https://sh.rustup.rs -sSf | sh\n" ;;
        esac
    fi

    # --- C 工具链（`cc` crate 就是找 cc/gcc/clang 之一）--------------------
    local cc_found="" c
    for c in cc gcc clang; do
        if have "$c"; then cc_found="$c"; break; fi
    done
    if [ -z "$cc_found" ]; then
        missing="${missing}build-essential "
    else
        # 编译器在但链不动（最小化镜像 / 缺 libc 开发头文件）：真编一个 hello 试试
        local t; t="$(mktemp -d 2>/dev/null || echo "/tmp/sc-check-$$")"
        mkdir -p "$t" 2>/dev/null || t="/tmp/sc-check-$$"; mkdir -p "$t" 2>/dev/null || true
        if ! printf 'int main(void){return 0;}\n' > "$t/t.c" 2>/dev/null \
           || ! "$cc_found" "$t/t.c" -o "$t/t.out" >/dev/null 2>&1; then
            missing="${missing}build-essential "
            echo "  [!] $cc_found 存在但链接失败，通常是缺 libc 开发头文件"
        fi
        rm -rf "$t" 2>/dev/null || true
    fi

    # --- aws-lc-sys（rustls 默认 provider）要 cmake，make 是它的默认生成器 --
    if [ "$os" = "Linux" ] || [ "$os" = "Darwin" ]; then
        have cmake || missing="${missing}cmake "
        have make  || missing="${missing}make "
    fi

    # 注意：这里到处用 `if` / `||` 而不是裸的 `&&`，因为脚本开了 set -e，
    # 形如 `[ -n "$x" ] && y` 的语句在条件为假时会直接把脚本干掉。
    if [ -n "$fatal" ]; then printf '%b' "$fatal"; fi

    if [ -z "$missing" ] && [ -z "$fatal" ]; then
        echo "  [ok] 依赖齐全（cargo ✓  C 编译器: $cc_found  cmake ✓  make ✓）"
        return 0
    fi
    if [ -z "$missing" ]; then
        echo "      Rust 工具链也需修复：https://rustup.rs"
        return 1
    fi

    # 只在提示里列真正缺的东西，装的时候按发行版补齐同一套
    echo "  [x] 缺少构建依赖: $missing"
    if [ "$AUTO_DEPS" = "1" ]; then
        echo "  --install-deps: 尝试自动安装..."
        if ! auto_install "build-essential cmake make" "gcc cmake make" "build-base cmake"; then
            echo "  自动安装失败，请看上面输出，或手动执行下面的命令。"
            return 1
        fi
        hash -r 2>/dev/null || true
        # 复检
        cc_found=""
        for c in cc gcc clang; do
            if have "$c"; then cc_found="$c"; break; fi
        done
        local still=""
        have cmake || still="${still}cmake "
        have make  || still="${still}make "
        if [ -n "$still" ] || [ -z "$cc_found" ]; then
            local lack="$still"
            if [ -z "$cc_found" ]; then lack="${lack}C编译器"; fi
            echo "  [x] 装完仍缺: ${lack}"
            echo "      或者你用的不是系统包管理器，手动补：https://brew.sh"
            return 1
        fi
        echo "  [ok] C 工具链已就绪 (编译器: $cc_found, cmake ✓, make ✓)"
        if [ -n "$fatal" ]; then return 1; fi
        return 0
    fi

    local cmd; cmd="$(pkg_install_cmd "build-essential cmake make" "gcc cmake make" "build-base cmake" || true)"
    if [ -n "$cmd" ]; then
        echo "      安装: $cmd"
        echo "      或直接: bash install.sh --install-deps"
    else
        echo "      手动安装: C 编译器(cc/gcc/clang) + cmake + make"
        echo "      (无包管理器时参考 https://brew.sh 或 https://msys2.org)"
    fi
    if [ -n "$fatal" ]; then
        echo "      Rust 工具链也需修复：https://rustup.rs"
    fi
    return 1
}

echo "Installing ${BIN_NAME}..."
echo "[0/3] Checking build dependencies..."
if ! preflight; then
    echo ""
    echo "Aborted: fix the dependencies above and re-run 'bash install.sh'."
    exit 1
fi

# Detect available cores and pick a CPU-friendly job count
CORES=$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)
JOBS=$(( (CORES + 1) / 2 ))   # ceil(cores/2), at least 1
if [ "$JOBS" -lt 1 ]; then JOBS=1; fi
export CARGO_BUILD_JOBS="$JOBS"

echo "  cores=$CORES jobs=$JOBS"
echo ""

# Build release with CPU-friendly profile overrides.
echo "[2/3] Building release..."
cargo build --release --bin "$BIN_NAME" \
    --config 'profile.release.opt-level=2' \
    --config 'profile.release.lto="off"' \
    --config 'profile.release.codegen-units=32'

# Install: just copy the already-built binary (skip cargo install which rebuilds)
echo ""
echo "[3/3] Installing binary and command aliases..."
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
