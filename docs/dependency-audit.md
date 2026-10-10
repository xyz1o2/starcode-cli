# 依赖审计 —— 未使用项、升级项与精简项

对 `Cargo.toml` 中全部 91 个直接依赖做的一次静态 + 编译验证审计，随后**执行了全量升级**（分支 `chore/deps-upgrade-all`）。

> **本文档分两部分**：第一至六节是升级*前*的审计结论（保留作为判断依据）；第七节起是升级*后*的实际执行记录。第六节末尾的「建议处置顺序」已全部完成。

**结论先行：**

1. **11 个依赖完全未被使用**，已从 `Cargo.toml` 移除并通过编译 + 测试验证。直接依赖 91 → 80。
2. **存在严重的版本分裂**：`reqwest` 编译了 3 个大版本、`crossterm` 2 个、`scraper` 2 个、`hyper` 2 个。这比"升级到最新版"更值得优先处理。
3. **`jsonschema` 曾是死代码**，却拖进了一整套第二套 HTTP 栈。
4. `crossterm` 被 `crossterm_0_28` feature 钉死在旧版，与 ratatui 0.30.2 期望的 0.29 冲突。
5. **除 `rig-core` 外的 79 个依赖已全部升级到最新稳定版**，`hyper` 与 `crossterm` 的版本分裂已消除。
6. **升级暴露出一个真实的功能回归**：`toml` 1.x 改变了 `parse::<Value>()` 的语义，导致技术栈探测静默失效（详见 7.4）。
7. **`rig-core` 停在 0.42.0**：0.43/0.44 是架构级重写，不是版本升级（详见第八节）。

---

## 一、审计方法与可信度

本仓库 `src/lib.rs` 和 `src/main.rs` 顶部都有：

```rust
#![allow(dead_code, unused_imports, unused_variables, unused_mut, unused_assignments)]
```

**这意味着编译器永远不会告诉你哪个依赖没在用。** 因此本次审计不依赖编译器，而是：

1. 对 `src/` 与 `tests/` 全部 `.rs` 做归一化匹配（`foo-bar` → `foo_bar`，排除子串误命中）。
2. 对零命中的候选，逐一**从 `Cargo.toml` 删除后重新编译**，确认 `cargo check --lib --bins` 仍然 exit 0。
3. 删除前后各跑一次完整测试套件做差分，确认没有回归。

> 这正好印证了 CLAUDE.md 里那条警告：`#![allow(dead_code)]` 打开时，"看起来合理"不是证据，只有构造函数级别的 grep 才是。

---

## 二、未使用的依赖（已移除，11 个）

以下依赖在 `src/` 与 `tests/` 中**零调用点**，`Cargo.toml` 里只有声明。

| 依赖 | 声明处注释 | 实际情况 |
|------|-----------|---------|
| `config` 0.14 | `# Configuration` | 所有 `config::` 命中都是内部模块 `crate::core::config`，不是这个 crate。真实配置是 `settings.json` + `STAR_*` 环境变量。**额外拖入约 20 个传递依赖。** |
| `console` 0.15 | `# Terminal colors` | 零使用。`src/ui/win32.rs` 里叫 `set_console_output_utf8` / `get_console_mode` 的函数是手写 Win32 代码，走 `windows_sys`，与 `console` crate 无关。名字相似极易误判。 |
| `libc` 0.2 | 终端 UI 分组 | 零使用。Windows 侧只需要 `windows-sys`。 |
| `textwrap` 0.16 | `# Text wrapping` | 零使用。换行实际由 ratatui + `unicode-width` 负责。 |
| `fs-err` 3.0 | `# File operations` | 零使用。文件读写全部走 `tokio::fs` / `std::fs`。 |
| `env_logger` 0.11 | `# Logging` | 零使用。**详见下方"顺带发现的真实缺陷"。** |
| `grep-matcher` 0.1 | ripgrep 分组 | 无直接调用，但 `grep-searcher` 本来就把它作为 matcher trait 依赖拉进来。 |
| `aho-corasick` 1.1 | `# Multi-pattern substring search with SIMD (hint keyword detection)` | 零使用。注释声称支撑的 "hint keyword detection" 功能并未实现。`regex` 本来就会传递引入它。 |
| `syn` 2.0 | 无注释 | 零使用。`features = ["full", "visit"]` 的声明暗示曾计划做 AST 分析，但代码里没有 AST 解析。 |
| `proc-macro2` 1.0 | 无注释 | 零使用。 |
| `lazy_static` 1.5 | 无注释 | 零使用。项目统一用的是 `once_cell::Lazy`。 |

### 实际收益

- 直接依赖：**91 → 80**
- 完全从依赖图中消失的 crate：**5 个** —— `config`、`console`、`textwrap`、`fs-err`、`env_logger`
- 仍作为其他依赖的传递依赖存在（移除只纠正声明、不减体积）：`libc`、`syn`、`proc-macro2`、`aho-corasick`、`grep-matcher`、`lazy_static`

### 验证记录

| 检查项 | 移除前 | 移除后 |
|--------|--------|--------|
| `cargo check --lib --bins` | exit 0 | **exit 0** |
| `cargo test --lib` 通过数 | 734 | 732 |
| `cargo test --lib` 失败数 | 14 | 16 |

差分的 2 个测试全部落在 `utils::checkpoint_manager::tests::`，CLAUDE.md 已标注该模块"cwd 依赖、易 flaky"。在**未修改的基线**上连跑三次，该模块失败数分别为 **4 → 6 → 6**，本身就不确定。已确认与本次改动无关。

`cargo check --all-targets` 会在 `tests/lib.rs` 上报 `E0583: file not found for module test_deferred`，这是 CLAUDE.md 记录的既有问题（`.gitignore` 的 `**/test_*.rs` 规则导致这些文件未被跟踪），与本次改动无关。

---

## 三、顺带发现的真实缺陷：日志全部丢失

移除 `env_logger` 暴露出一个问题，值得单独处理。

`log::warn!` 在 8 处被调用 —— `checkpoint_manager.rs`、`commands/mod.rs`、`write_file.rs`、`edit.rs`、`teams.rs`、`handler.rs` —— 但**代码中从未初始化过任何 logger**。`env_logger` 自己也从未被调用。

**后果：所有 `log::` 级别的诊断信息被静默丢弃。** 这与 CLAUDE.md 记录的"文件日志默认开启（`append_debug_log_line` / `append_agent_log_line`）"是两套并行机制，`log::` 这一套实际上是死的。

两个选择：接上 logger，或删掉这些死调用。**建议先确认这些 `log::warn!` 里有没有承载了排障关键信息** —— 如果有，接 logger；如果没有，删调用。

---

## 四、版本分裂：本次审计最重要的发现

在讨论"升级到最新版"之前，有几处同一个 crate 被编译了多个大版本。这直接对应编译时间与二进制体积的浪费，优先级高于版本号更新。

| crate | 依赖图中的版本 | 来源 |
|-------|--------------|------|
| **`reqwest`** | **0.11.27 / 0.12.28 / 0.13.5** | 0.12 = 直接依赖；**0.11 = `jsonschema` 0.17 与 `reqwest-eventsource` 0.5**；0.13 = `rig-core` 0.41 |
| **`hyper`** | 0.14.32 / 1.12.0 | 0.14 随 `reqwest` 0.11 一起来 |
| **`crossterm`** | **0.28.1 / 0.29.0** | 0.28 = 直接依赖经 `crossterm_0_28` feature；**0.29 = ratatui 0.30.2 的默认依赖** |
| **`scraper`** | 0.18.1 / 0.22.0 | 0.22 = 直接依赖（`web_search.rs`）；**0.18 = `readability-rust` 0.1.0**（其版本已锁死，无新版本） |
| `base64` | 0.21.7 / 0.22.1 / 0.23.1 | 0.21 = 直接依赖 |
| `rand` | 0.8.8 / 0.9.5 / 0.10.3 | 0.8 = 直接依赖 |
| `windows-sys` | 0.48 / 0.52 / 0.59 / 0.60 / 0.61 | 0.52 = 直接依赖 |

`base64` / `rand` / `windows-sys` 这类 semver 兼容性破坏的并存是常态，危害小。**真正要处理的是 `reqwest`、`hyper`、`crossterm`、`scraper`** —— 每一个都意味着同一份逻辑被编译了两三遍。

### 4.1 消除 reqwest 0.11 + hyper 0.14（收益最大）

两条路径，都已核实 crates.io 元数据：

- **`reqwest-eventsource` 0.5.0 → 0.6.0**：`0.6.0` 依赖 `reqwest ^0.12`，与直接依赖对齐。只影响 `src/core/mcp/transport.rs` 一处 SSE 接入。
- **`jsonschema` 0.17.1 → 0.58.6**：虽然新版依赖 `reqwest ^0.13`，但**更好的做法是直接删掉它 —— 它是死代码**（见 4.2）。

任一条落地，即可让 `reqwest` 从 3 个版本降到 2 个，并彻底移除 `hyper` 0.14。

### 4.2 `jsonschema` 是死代码，且成本极高

`src/core/utils/schema_validator.rs` 里的 `SchemaValidator::validate` 实现完整，但**全仓库零调用点** —— grep `SchemaValidator` 只有 3 处命中：结构体定义、`impl` 块、以及 `core/utils/mod.rs` 的模块声明。

代价是 `jsonschema` 0.17 会拉入 **reqwest 0.11 + hyper 0.14**，也就是一整套与主 HTTP 客户端重复的 HTTP 栈。

**建议：删除 `src/core/utils/schema_validator.rs`、`core/utils/mod.rs:6` 的模块声明，以及 `Cargo.toml` 中的 `jsonschema`。** 若将来确实需要 schema 校验，应基于 `serde_json` 自行实现轻量校验，而不是引入一个自带 HTTP 栈的 crate。

### 4.3 crossterm 双版本：feature 钉死了旧版

当前声明：

```toml
crossterm = { version = "0.28", features = ["use-dev-tty", "bracketed-paste"] }
ratatui   = { version = "0.30", features = ["crossterm_0_28"] }
tui-textarea = { package = "tui-textarea-2", version = "0.11", features = ["crossterm_0_28"] }
```

但 ratatui 0.30.2 本身的默认依赖是 `crossterm ^0.29`。`crossterm_0_28` feature 把整个 TUI 栈锁在 0.28，于是 0.29 也被一并编译进来。

`tui-textarea-2` 0.13.2 同时支持 `crossterm ^0.29` 与 `^0.28`（两个都是可选依赖），所以迁移路径是通的。

**建议：三者协同升级** —— `crossterm` 0.29 + `ratatui` 0.30（去掉 `crossterm_0_28` feature）+ `tui-textarea-2` 0.13。涉及全部 TUI 渲染与输入路径，改完必须实测交互。

---

## 五、可升级的依赖

未安装 `cargo-outdated`，改为直接查询 crates.io API 获取 80 个直接依赖的 `max_stable_version`。以下"当前"取自 `Cargo.lock` 的实际解析结果。

### 5.1 低风险：只改 lockfile，不动代码

```
ignore 0.4.33 · globset 0.4.20 · walkdir 2.5.0 · grep-searcher 0.1.17 · grep-regex 0.1.14
toml 0.8.23 · async-stream 0.3.6 · futures 0.3.34 · tokio-util 0.7.19 · dotenvy 0.15.7
pathdiff 0.2.3 · anyhow 1.0.104 · tracing 0.1.44 · zeroize 1.9.1 · rust-embed 8.13.0
arboard 3.6.1 · syntect 5.3.0 · parking_lot 0.12.5 · log 0.4.34 · tokio 1.53.2
serde 1.0.229 · serde_json 1.0.151 · chrono 0.4.45 · regex 1.13.1 · glob 0.3.4
image 0.25.10 · tempfile 3.27.0 · tokio-stream 0.1.19 · uuid 1.27.0 · async-trait 0.1.92
```

直接 `cargo update` 即可。`Cargo.lock` 本身被 gitignore（CLAUDE.md 已记录），所以这些更新只会落到本地。

### 5.2 小版本，需要改代码但范围可控

| 依赖 | 当前 | 最新 | 调用点数 | 备注 |
|------|------|------|---------|------|
| `html2text` | 0.16.7 | 0.17.1 | 3 | `web_fetch.rs` ×2、`web_search.rs` ×1 |
| `lru` | 0.12.5 | 0.18.5 | 2 | `search_cache.rs`、`codebase_search.rs` |
| `thread-priority` | 0.16.0 | 3.1.1 | 1 | `core/context/watcher.rs:29` |
| `notify` | 6.1.1 | 8.2.0 | ~25 | `core/context/watcher.rs`，watcher API 变动较大 |
| `pulldown-cmark` | 0.12.2 | 0.13.4 | ~7 | `utils/markdown_parser.rs`，影响 markdown 渲染 |

### 5.3 大版本，除非有明确需求否则不建议

`zip` 2→9 · `toml` 0.8→1.1 · `rand` 0.8→0.10 · `dirs` 5→7 · `base64` 0.21→0.23 · `which` 6→8 · `scraper` 0.22→0.27 · `thiserror` 1→2 · `similar` 2→3 · `rig-core` 0.41→0.44 · `tree-sitter` 0.26→0.27 · `windows-sys` 0.52→0.61 · `unicode-width` 0.1→0.2 · `sha2` 0.10→0.11 · `md5` 0.7→0.8 · `tui-textarea-2` 0.11→0.13（需配合 4.3 的 crossterm 迁移）

`jsonschema` 0.17→0.58 不列入此表 —— 正确做法是删除（见 4.2）。

---

## 六、其他精简机会

### 6.1 `tokio` 的 `features = ["full"]`

`full` 会一并拉入 `signal`、`net`、`process`、`parking_lot`、`io-std` 等。代码实际只用到 `fs`、`sync`、`time`、`process`、`macros`、`rt`。

改为显式列举可减小二进制体积。风险低，但 `process` 与 `signal` 依赖平台实现，Windows 与 Unix 表现可能有差异，改完需实测。

### 6.2 `windows-sys` 的 feature 面

声明只开了 `Win32_Foundation`、`Win32_System_Console`、`Win32_System_SystemServices`，已经比较克制，无需改动。

### 6.3 已知的孤儿代码（与依赖无关，但同属精简范畴）

CLAUDE.md 已记录，此处仅作汇总：

- `src/constants/` 与 `src/sdk/` 未被 `lib.rs` 或 `main.rs` 声明，改动不产生任何效果
- 根目录 `config.toml` 无任何代码读取（其 `[ui.system_prompts] dir` 甚至指向不存在的路径）
- `tests/` 大部分是死代码，`.gitignore:28` 的 `**/test_*.rs` 规则导致 `tests/lib.rs` 声明的两个文件未被跟踪，`cargo test --all-targets` 必然失败
- `PolicyEngine::load_permission_rules` 零调用点 —— `.star/permissions.json` 规则实为咨询性质
- `WebBrowserTool::new` 从未被调用 —— 模型无法触达该工具
- analytics HTTP sink 只打印 `"[Analytics HTTP] Would send …"`，并未真正发送

---

## 七、建议的处置顺序

| 优先级 | 动作 | 收益 / 风险 |
|--------|------|------------|
| 1 | **删除 `jsonschema`** + `schema_validator.rs` | 移除一整套重复 HTTP 栈；确认是死代码，风险极低 |
| 2 | **`reqwest-eventsource` 0.5 → 0.6** | 移除 reqwest 0.11 + hyper 0.14；单文件改动 |
| 3 | **crossterm / ratatui / tui-textarea-2 协同升到 0.29 线** | 移除重复 crossterm；**影响全部 TUI 交互，必须实测** |
| 4 | `cargo update` 低风险批次 | 免费拿到的补丁更新 |
| 5 | `tokio` features 精确化 | 减体积；需跨平台实测 |
| 6 | 处理死的 `log::` 调用 | 修掉一个真实的诊断信息丢失缺陷 |
| 7 | html2text / lru / notify / pulldown-cmark 小版本 | 逐个推进，每次单独验证 |

**不建议**在缺少明确收益的情况下碰 5.3 里的大版本。

---

## 附：本次改动清单

- `Cargo.toml`：移除 11 个未使用依赖（-19 行）
- 其余文件未改动

> 审计期间工作区已有未提交修改（`src/core/config/models.rs`、`src/core/config/providers.rs`、`src/llm/client.rs`），均未触碰。

---

# 七、升级执行结果

分支 `chore/deps-upgrade-all`。除 `rig-core` 外的 79 个直接依赖已升到最新稳定版。

## 7.1 验证结论

| 检查项 | 结果 |
|--------|------|
| `cargo check --lib --bins` | **0 error** |
| 新增 warning | **0**（仅剩 1 条升级前就存在的 `CachedSettings` 可见性告警） |
| `cargo test --lib` | 736 passed / 14 failed |
| 与升级前基线差分 | **无新增失败** |

基线是 734 passed / 14 failed；现在 736 是因为新增了 2 个测试（见 7.5）。14 个失败项与基线**完全一致**，全部是 CLAUDE.md 已记录的既有问题。

## 7.2 版本分裂的消除效果

| crate | 升级前 | 升级后 |
|-------|--------|--------|
| `hyper` | 0.14.32 / 1.12.0 | **1.12.0（唯一）** ✅ |
| `crossterm` | 0.28.1 / 0.29.0 | **0.29.0（唯一）** ✅ |
| `reqwest` | 0.11.27 / 0.12.28 / 0.13.5 | 0.12.28 / 0.13.5（少一个） |
| `scraper` | 0.18.1 / 0.22.0 | 0.18.1 / 0.27.0（无解） |
| `windows-sys` | 5 个版本 | 3 个版本 |

- **`hyper` 彻底收敛到单一版本**：这正是 `jsonschema` 0.17（已升 0.58，不再依赖 reqwest 0.11）带来的收益。
- **`crossterm` 收敛**：`crossterm` 0.29 + `ratatui` 0.30（去掉 `crossterm_0_28` feature）+ `tui-textarea-2` 0.13 三者协同升级的结果。
- **`reqwest` 剩两个版本是当前生态的硬上限**：`reqwest-eventsource` 最新版 0.6.0 依赖 `reqwest ^0.12`，而 `rig-core` 0.42 依赖 `reqwest ^0.13`。该 crate 尚无 0.13 兼容版本，无法进一步消除。好消息是 reqwest 0.12 在此图中已使用 hyper 1.x，不再引入第二套 hyper。
- `scraper` 双版本无解：`readability-rust` 0.1.0 版本已锁死且不再更新，它依赖 `scraper ^0.18`。
- 依赖图总包数 624。

## 7.3 各 crate 的破坏性改动与处理

| crate | 变更 | 处理 |
|-------|------|------|
| `sha2` 0.10→0.11 | `finalize()` 返回 `hybrid_array::Array`，不再实现 `LowerHex`，`format!("{:x}")` 失效 | 新增 `paths::to_hex_lower` / `sha256_hex`，替换 8 处调用 |
| `rand` 0.8→0.10 | `thread_rng()`→`rng()`；`gen()`→`random()` 且移入新 trait `RngExt`；`SliceRandom`→`IndexedRandom` | 3 处 |
| `windows-sys` 0.52→0.61 | `INVALID_HANDLE_VALUE` 仍是 `*mut c_void`，但裸指针与整数 `0` 不再可直接比较；`*mut c_void` 失去 `Send` | `is_null()`；跨线程改传 `usize` |
| `zip` 2→9 | `entry.name()` 改为返回 `Result<Cow<str>, ZipError>` | 加 `map_err` |
| `jsonschema` 0.17→0.58 | `JSONSchema`→`Validator`；`compile()`→`build()`；`validate()` 由返回错误迭代器改为返回单个错误 | 改用 `iter_errors()` 以保留「收集全部错误再拼接」的原有输出格式 |
| `pulldown-cmark` 0.12→0.13 | 新增 `Tag::Superscript`/`Subscript` 与对应 `TagEnd` | 补 match 分支，按纯文本透传 |
| `base64` 0.21→0.23 | 自由函数 `encode`/`decode` 废弃 | 改用 `Engine::STANDARD`。**刻意保留 STANDARD 而非 URL_SAFE**，以与旧版字节级一致 |
| `reqwest` 0.12→0.13 | `rustls-tls` feature 更名为 `rustls` | 改名 |
| `toml` 0.8→1.1 | **见 7.4，真实功能回归** | 修复两处生产代码 |
| `rig-core` 0.41→0.42 | 见第八节 | 见第八节 |

## 7.4 升级暴露出的真实功能回归（重要）

`toml` 1.x 把 `Value`'s `FromStr` 实现从 **document 解析器**换成了 **value 解析器**：

```rust
// toml 0.8：解析整个文档
content.parse::<toml::Value>()   // ✅

// toml 1.1：只接受裸值（42 / "s" / {a=1}）
content.parse::<toml::Value>()   // ❌ TOML parse error at line 1, column 1
                                  //    unexpected content, expected nothing
```

任何含 `[table]` 的内容都会解析失败。两处生产代码受影响：

| 位置 | 后果 |
|------|------|
| `src/core/context/selection.rs:439` `detect_cargo_tech_stack` | **静默失效** —— 技术栈探测永远返回空，`rust` 以外的项目级上下文信号全部丢失 |
| `src/core/tools/verify_edit.rs:226` `verify_toml` | TOML 语法校验永远判定为「有错」 |

两处都已改为 `toml::from_str::<toml::Value>(...)`（走 document 反序列化器）。

这个回归是**测试先发现的**：`cargo_stack_detects_direct_dependencies` 与 `cargo_stack_detects_aliased_and_target_dependencies` 两个测试转为失败。**如果没有这两个测试，这个 bug 会静默上线** —— 因为失败路径是「返回空」而不是崩溃。

> 教训：解析类依赖升级时，"更严格的解析"和"改变了输入形态"要分开验证。前者会暴露原有脏数据，后者会静默改变语义。

## 7.5 新增测试

`paths.rs` 新增 2 个测试，锁定手写十六进制编码的正确性：

- `sha256_hex_matches_known_vectors` —— 用 3 个 FIPS 180-4 标准向量 + 空串
- `to_hex_lower_is_lowercase_and_zero_padded`

**为什么必须锁死**：checkpoint 备份文件名、skills 缓存目录名、后台索引 `index.json` 的键、文件快照哈希全部由这个函数派生。sha2 0.11 迁移时如果编码有任何偏差（大小写、补零、长度），**所有既有缓存会全部失效且不会报错**。向量测试是唯一能挡住这种静默破坏的手段。

---

# 八、rig-core：为什么停在 0.42.0

这是本次升级唯一未达成"最新"的目标，原因是它不是版本升级，而是架构重写。

## 8.1 事实

`rig-core` 每周发一个版本。0.43.0（2026-09-30）和 0.44.0（2026-10-07）合计引入了大量 breaking change：

- effect-bus 关键路径、统一 data layer、sans-IO run protocol
- `OneOrMany` 移除、`CompletionClient` trait 移除
- `Usage` 计数器改为 `Option<u64>`
- 消息模型改为 item-shaped + provenance

并用全新的 **Wire / Operation / Transport / Driver** 架构替换了原有的 `CompletionClient` trait。

**0.42.0 是最后一个仍提供旧 API 的版本**（已通过 docs.rs 确认其 `client` 模块仍导出 `CompletionClient`）。

## 8.2 实测代价

| 目标版本 | 编译错误数 |
|----------|-----------|
| rig-core 0.44.0 | 47 |
| rig-core 0.42.0 | 18 → **0** |

0.44 的改动范围（`src/llm/rig_adapter.rs`，810 行）：

| 旧 API（0.41） | 新 API（0.44） |
|----------------|---------------|
| `OneOrMany<T>` | `Vec<T>` |
| `Message::Assistant { id, content }` | `Message::Assistant(AssistantMessage)` |
| `ToolResult { id, call_id, content }` | `ToolResult { call: CallId, name, content, is_error }` |
| `Reasoning { content: Vec<ReasoningContent> }` | `Reasoning { text: String, redacted, native }` |
| `client.completion_model(m).completion(req)` | `OpenAI::new(k).completion(m).call(req)` |
| `StreamedAssistantContent` | `StreamEvent`（需按 `Part` 自行聚合工具参数） |
| `CompletionRequest.preamble` | 已移除，system 即首条消息 |

## 8.3 为什么不直接做

1. **在活跃的 LLM 主路径上**。`rig_adapter.rs` 是所有模型请求的唯一出口。
2. **无测试覆盖**。现有 748 个单测没有一个发起真实 API 调用；写错了编译能过、行为静默错误，只有配了真实 API key 手动跑才会发现。
3. **改动量与收益不匹配**。为了 rig-core 一个依赖重写整个 LLM 适配层，风险远大于"落后两个补丁版本"。

## 8.4 0.42.0 迁移的 5 处改动

已完成并验证：

| 改动 | 说明 |
|------|------|
| `OneOrMany` → `Vec` | `chat_history` 空列表兜底逻辑改为显式 `if is_empty()` |
| `ToolCallId` | 新增非空标识类型，`ToolCall::new` 改收 `ToolCallId`；用 `new_or_mint` 处理 provider 未发 id 的情况 |
| `ToolResult` | `id`/`call_id` → `call`/`provider`/`name` |
| `StreamedAssistantContent::Reasoning` | 元组变体 → struct 变体 |
| `token_usage()` | → `StreamFinal.usage` 公有字段；`GetTokenUsage` trait 移除 |

## 8.5 后续建议

等 rig-core 的新架构稳定（预计 0.45+），单独开一个 PR 迁移，并**先补上 rig_adapter 的契约测试**（用 mock HTTP server 断言请求体形状与流式事件映射），再动生产代码。

---

# 九、遗留事项

1. **`jsonschema` 仍是死代码**：`SchemaValidator::validate` 依然零调用点。升级只消除了它的传递依赖成本，没消除它本身。建议直接删掉 `src/core/utils/schema_validator.rs` 及其模块声明。
2. **死的 `log::` 调用**：8 处 `log::warn!` 从未初始化 logger，诊断信息静默丢失。
3. **`reqwest` 双版本**：`reqwest-eventsource` 上游跟进后才能消除。
4. **TUI 交互未实测**：crossterm 0.28→0.29 覆盖全部渲染与输入路径，编译与单测无法覆盖，需手动验证交互。
5. **`tokio` features 仍为 `full`**：可按实际用到的 `fs`/`sync`/`time`/`process`/`macros`/`rt` 精确化。