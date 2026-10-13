# 构建与发布

面向贡献者：怎么在本地构建，以及怎么打一个多平台发布包。

## 一、本地构建

```bash
cargo check --all-targets       # 最快的正确性门禁，迭代时用这个
cargo build --release           # → target/release/starcode-cli
cargo test --lib                # 单元测试都在 src/ 里，不在 tests/
```

`./install.sh`（Windows：`.\install.ps1`）会构建 release 并在 `~/.cargo/bin` 下建
`sc` / `starcode` 两个别名，三者指向同一个二进制。纯 `cargo build --release`
只会得到 `starcode-cli` 这一个名字。

### 系统依赖

`reqwest` / `rig-core` 都用 `rustls`，而 rustls 默认走 **aws-lc-rs**，需要编译 C
和汇编代码。因此：

| 平台 | 需要 |
| --- | --- |
| Linux | `cmake`、`pkg-config`、`build-essential`（gcc） |
| macOS | Xcode command line tools（已装 perl） |
| Windows | MSVC build tools + CMake；`windows-latest` 镜像已自带，无需额外安装 |

Debian/Ubuntu 一行装齐：

```bash
sudo apt-get install -y cmake pkg-config build-essential
```

## 二、发布一个新版本

发布完全由 tag 驱动。

```bash
# 1. 先把 Cargo.toml 的 version 改好，这是唯一的版本号来源
#    version = "0.3.1"

# 2. 提交 + 打 tag（tag 必须带 v 前缀，且要和 Cargo.toml 完全一致）
git commit -am "release: v0.3.1"
git tag v0.3.1
git push origin main --tags
```

推完 tag，`.github/workflows/release.yml` 自动跑完剩下的全部步骤。版本对不上会
在第一个 job 就失败并明确报错，不会产出半成品 Release。

也可以手动触发（用于重跑失败的流水线）：

```bash
gh workflow run release.yml -f tag=v0.3.1
```

### 流水线做了什么

```
validate   校验 tag 与 Cargo.toml 版本一致
   ↓
test       单元测试 + 文档测试
   ↓
build      8 个平台并行编译、打包、上传
   ↓
release    汇总产物、生成 SHA256SUMS.txt、创建 GitHub Release
```

`fail-fast: false`，所以某一个平台挂掉不会掩盖其他平台的结果；最后的 `release`
job 只在所有平台都成功后才会创建 Release。

### 产物格式

每个平台一个压缩包，命名 `starcode-cli-<版本>-<target>`：

| target | 压缩包 | 说明 |
| --- | --- | --- |
| `x86_64-unknown-linux-gnu` | `.tar.gz` | glibc，原生构建 |
| `aarch64-unknown-linux-gnu` | `.tar.gz` | ARM64 glibc（Graviton / 树莓派等） |
| `x86_64-unknown-linux-musl` | `.tar.gz` | 静态链接，不依赖 glibc，Alpine 可直接跑 |
| `aarch64-unknown-linux-musl` | `.tar.gz` | ARM64 静态链接 |
| `x86_64-apple-darwin` | `.tar.gz` | Intel Mac |
| `aarch64-apple-darwin` | `.tar.gz` | Apple Silicon |
| `x86_64-pc-windows-msvc` | `.zip` | Windows x64 |
| `aarch64-pc-windows-msvc` | `.zip` | Windows ARM64 |

每个压缩包里是同名目录，包含：

```
starcode-cli-<version>-<target>/
├── starcode-cli      # 或 starcode-cli.exe
├── LICENSE
└── README.md
```

外加仓库根目录的 `SHA256SUMS.txt`，校验用：

```bash
sha256sum -c SHA256SUMS.txt          # Linux
shasum -a 256 -c SHA256SUMS.txt      # macOS
```

### 交叉编译是怎么做的

GitHub 托管 runner 只有 x86_64，所以非本机架构需要交叉编译：

- **Linux 交叉**（arm64 gnu、x64/arm64 musl）用 `cargo-zigbuild`，由 zig 提供
  `zig cc`，一次解决交叉链接器和 musl 头文件，不必手工维护 toolchain 文件。
- **macOS** 两个架构都用各自的原生 runner（`macos-13` / `macos-14`），不走交叉
  编译，避开 SDK 和链接的坑。
- **Windows ARM64** 用 `cargo-xwin`，它自带 Windows SDK 和 MSVC CRT。

原生平台在打包前会跑一次冒烟测试（`--version` 和 `--help`），交叉编译出来的
二进制在本机跑不了，那部分平台跳过。

## 三、lint 的现状

`cargo fmt --all -- --check` 现在是**硬门禁**，债务已清零。

`cargo clippy --workspace --all-targets` 还有约 470 条存量警告（lib target 471 条，
0 error），因此**没有**加 `-D warnings` —— 那样 CI 会长期红灯。这一步只作建议，
警告照常打进日志。清完之后把 `-- -D warnings` 加回 `ci.yml` 就能恢复成硬门禁。

不要用全局 `RUSTFLAGS=-Dwarnings`：它会连依赖 crate 的警告一起变成错误，而且会让
所有依赖被迫重新编译。

另外，lib 测试必须串行跑（`-- --test-threads=1`）：`utils::checkpoint_manager` 的测试
共用全局环境变量，并行执行时它们会互相把对方的 override 删掉。

## 四、常见问题

**`cargo build --locked` 报 lock file 需要更新**
`Cargo.lock` 现在是入库文件。升级依赖后要把它一起提交，不要单独改
`Cargo.toml` 的版本号。

**Linux 上编译报 cmake / C compiler 相关错误**
装 `cmake pkg-config build-essential`（见上面「系统依赖」）。本地装过 Rust 但
没装这些包时最容易踩。

**CI 报 `tests/lib.rs` 相关的模块找不到**
这个集成测试目标已经删除。它的模块声明指向的文件从来不存在于仓库里，导致全新
clone 无法编译 `cargo test --workspace`。新的测试请写在被测文件内部的
`#[cfg(test)] mod tests` 里。

**交叉编译卡在 aws-lc-sys**
说明该平台缺 C 工具链。Linux 交叉场景下用 `cargo zigbuild` 而不是裸
`cargo build --target`，它会自动带上正确的 C 编译器。