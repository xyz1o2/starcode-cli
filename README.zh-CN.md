# StarCode CLI

**[English](./README.md) · [简体中文](#简体中文)**

<p align="center">
  <img src="https://img.shields.io/badge/version-0.3.0-blue" alt="version">
  <img src="https://img.shields.io/badge/rust-2021-orange" alt="rust edition">
  <img src="https://img.shields.io/badge/license-MIT-green" alt="license">
  <img src="https://img.shields.io/badge/platform-cross--platform-lightgrey" alt="platform">
</p>

## 简体中文

一个跑在终端里的对话式 AI 编程助手 —— 100% Rust 编写，对标 Claude Code。基于 ratatui 0.30 + crossterm + tokio，支持 OpenAI 兼容与 Anthropic 风格的各类模型提供商。

### 特性

- **多模型提供商** —— 支持 OpenAI、Anthropic 及任意 OpenAI 兼容端点；provider store 管理多套凭证，运行时可切换。
- **交互式 TUI** —— 流式输出、diff 预览、语法高亮、多主题、鼠标操作。
- **无头模式** —— 脚本驱动单次提问，返回 JSONL 或纯文本。
- **丰富工具链** —— 文件读/写/编辑/Notebook、Shell、ripgrep 搜索、AST 感知的代码库搜索、git 洞察、子 Agent、待办、定时任务、worktree、MCP 服务器。
- **权限系统** —— 每个工具调用都可交互确认；Plan 模式把 Agent 限制为只读操作。
- **项目上下文** —— 自动读取项目里的 `STAR.md` / `STARCODE.md` / `CLAUDE.md` / `AGENTS.md` 作为指令。
- **会话管理** —— 保存、恢复、管理对话历史。
- **国际化** —— 支持中文和英文界面。
- **跨平台** —— Linux、macOS、Windows。

### 安装

#### 从源码安装（推荐）

```bash
git clone https://github.com/xyz1o2/starcode-cli.git
cd starcode-cli

./install.sh          # Windows: .\install.ps1
```

`install.sh` 只构建**一个**二进制（`starcode-cli`），然后在 `~/.cargo/bin` 里建立两个符号链接别名，所以下面三个名字启动的是同一个程序：

```bash
sc              # 简写
starcode
starcode-cli
```

> 注意：直接 `cargo build --release` 只会得到 `starcode-cli` 这个名字，`sc` / `starcode` 别名由安装脚本创建。

#### 通过 Cargo 安装

```bash
cargo install --git https://github.com/xyz1o2/starcode-cli.git starcode-cli
```

#### 预编译二进制

从 [Releases](https://github.com/xyz1o2/starcode-cli/releases) 页面下载对应平台的版本。

### 快速开始

#### 1. 配置 API Key

```bash
# 方式一：环境变量
export STAR_API_KEY="your-api-key"

# 方式二：配置文件（见下方「配置」）

# 方式三：一次性 CLI 参数
sc -k "your-api-key"
```

#### 2. 启动交互式会话

```bash
sc                                    # 交互式 TUI
sc "解释一下这个项目的结构"                    # 带初始消息启动
```

#### 3. 无头模式（脚本化）

```bash
sc -p "当前目录下有哪些文件？"
sc -p "列出所有 Rust 文件" --output-format text
```

### 配置

凭证与模型设置按以下优先级解析（前者优先）：会话内覆盖 → CLI 参数 → `STAR_*` 环境变量 → `ANTHROPIC_*` 环境变量 → provider store → 用户配置文件。

#### 用户配置 —— `~/.star/user-settings.json`

```jsonc
{
  "apiKey": "your-api-key",
  "baseUrl": "https://api.openai.com/v1",
  "defaultModel": "gpt-4",
  "isOpenAICompatible": true,
  "uiLanguage": "zh",            // "en" | "zh"
  "thinkingEffort": "medium",
  "outputStyle": "default",
  "contextWindow": 200000
}
```

#### 项目配置 —— `.star/settings.json`

从当前目录**向上**逐级查找到项目根目录（支持带注释的 `.jsonc`）。共享的、需要纳入版本管理的配置放这里。全局配置位于 `~/.star/settings.json`。

#### 环境变量

| 变量 | 用途 |
| --- | --- |
| `STAR_API_KEY` | API 密钥 |
| `STAR_BASE_URL` | 自定义 API 基础 URL |
| `STAR_MODEL` | 默认模型 |
| `STAR_CONTEXT_WINDOW` | 上下文窗口大小 |
| `STAR_LLM_TIMEOUT` | LLM 请求超时 |
| `STAR_TOOL_TIMEOUT_SECS` | 工具执行超时 |
| `STAR_LOG_DIR` | 重定位日志目录 |
| `STAR_LOG_ENABLED` | 设为 `0` 关闭文件日志 |

#### 项目指令

StarCode 会从项目根目录下依次查找 `STAR.md`、`STARCODE.md`、`CLAUDE.md`、`AGENTS.md`，取第一个存在的文件（截断到 8000 字符）。执行 `starcode init` 可生成 `STAR.md` 模板。

### 命令

#### 交互模式

```bash
starcode                              # 启动交互式会话
starcode -d /path/to/project          # 指定工作目录
starcode --resume                     # 恢复最近一次会话
starcode --resume <session-id>        # 恢复指定会话
starcode --dangerously-skip-permissions   # 跳过所有权限提示（危险！）
```

#### 无头模式

```bash
starcode -p "你的提问"
starcode -p "你的提问" --output-format jsonl
starcode -p "你的提问" --output-format text
starcode -p "你的提问" --max-turns 50 --max-tool-rounds 200
```

#### 子命令

```bash
starcode init                          # 生成 STAR.md 模板
starcode doctor                        # 诊断配置、凭证、工具链、日志
starcode mcp add <名称> <命令>          # 注册 MCP 服务器
starcode mcp list                      # 列出已配置的服务器
starcode mcp remove <名称>             # 移除服务器
starcode git commit                    # AI 辅助 git 操作
starcode git diff
starcode git status
```

#### 权限模式

```bash
starcode --permission-mode default     # 执行前询问（默认）
starcode --permission-mode plan        # 只读，先规划再动手
starcode --permission-mode yolo        # 跳过所有权限（危险！）
```

`acceptEdits` 与 `bypassPermissions` 分别是 `default` 和 `yolo` 的别名。

#### TUI 内的斜杠命令

内置约 300 条斜杠命令，涵盖 Automation、Tools、Session、Config、Security、Git、Debug、MCP、Memory 等分类。输入 `/` 浏览，`/help` 查看完整列表。

### 内置工具

| 工具 | 功能 |
| --- | --- |
| `Read` / `Write` / `Edit` / `MultiEdit` / `SmartEdit` | 文件读取与精确编辑（SmartEdit 会对格式错误的编辑做一次 LLM 修复） |
| `NotebookRead` / `NotebookEdit` | Jupyter Notebook 支持 |
| `Bash` | Shell 执行，带沙箱与超时 |
| `Grep` / `Glob` | 基于 ripgrep 的搜索与文件模式匹配 |
| `CodebaseSearch` | 基于 AST 的代码库语义搜索 |
| `Agent` / `SendMessage` | 派生并驱动子 Agent |
| `TodoWrite` / `TaskGet` | 任务与待办跟踪 |
| `WebFetch` | 抓取并分析网页 |
| `GitInsight` / `GhPrComments` | Git 历史与 GitHub PR 上下文 |
| `EnterPlanMode` / `ExitPlanMode` | Plan 模式控制 |
| `EnterWorktree` / `ExitWorktree` | 隔离的 git worktree |
| `CronCreate` / `CronList` / `CronDelete` | 定时任务 |
| `BackgroundTask` / `ScheduleWakeup` / `RemoteTrigger` | 后台与延迟执行 |
| `Memory` | 持久化 Agent 记忆 |
| `GetDiagnostics` / `RunTests` / `ProjectMap` | LSP 诊断、测试运行、项目概览 |
| `Skill` / `ToolSearch` | 技能调用与工具发现 |

再加上已配置的 MCP 服务器贡献的工具。

### MCP 支持

StarCode 通过 Model Context Protocol 扩展能力：

```bash
starcode mcp add filesystem "npx -y @modelcontextprotocol/server-filesystem /path/to/dir"
starcode mcp list
starcode mcp remove filesystem
```

### 开发

```bash
cargo check --all-targets      # 最快的正确性检查，迭代时用这个
cargo build --release          # → target/release/starcode-cli
cargo test --lib               # 单元测试都在 src/ 内
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

#### 项目结构

```
starcode-cli/
├── src/
│   ├── main.rs         # 二进制入口：CLI 解析、无头路径、TUI 引导
│   ├── lib.rs          # 库根
│   ├── agent/          # Agent 内核、回环、压缩、工具路由、模型降级
│   ├── commands/       # 斜杠命令及其分发
│   ├── core/           # 配置、工具、策略、上下文引擎、确认总线、i18n
│   ├── llm/            # LLM 客户端与流式
│   ├── runtime/        # UI ↔ Agent 协议与运行时接缝
│   ├── tools/          # 较重的工具实现（bash、搜索、todo、git、lsp）
│   ├── types/          # 共享类型
│   ├── ui/             # 终端 UI（ratatui）：状态、组件、服务
│   └── utils/          # 日志、路径、项目上下文、杂项
├── eval/               # 评测任务定义
├── i18n/               # 翻译
├── install.sh / .ps1 / .bat
└── Cargo.toml
```

#### 调试 TUI

TUI 里没法 `println!`。文件日志**默认开启**，写入 `.star/logs/starcode_debug.log` 和 `.star/logs/agent.log`。用 `STAR_LOG_DIR` 重定位，或 `STAR_LOG_ENABLED=0` 关闭。如果程序起来了但加载界面一直不消失，`starcode doctor` 可以在不启动 TUI 的情况下报告问题所在。

### 评测

StarCode 内置评测框架：

```bash
starcode eval --tasks eval/tasks.json
starcode eval --tasks eval/tasks.json --trials 3
starcode eval --tasks eval/tasks.json --report-md eval-report.md
starcode eval --baseline .star/eval-baseline.json
```

### 常见问题

**找不到 API Key** —— `echo $STAR_API_KEY`、检查 `~/.star/user-settings.json`，或执行 `starcode doctor`。

**编译失败** —— 确认已安装 Rust（`rustc --version`），然后 `rustup update` 与 `cargo clean && cargo build --release`。

**卡在加载界面** —— 启动失败不会崩溃，而是让加载界面卡住。查看 `.star/logs/agent.log` 里的 `[INIT]` 记录，或执行 `starcode doctor`。

### 参与贡献

1. Fork 本仓库
2. 创建特性分支（`git checkout -b feature/amazing-feature`）
3. 提交更改（`git commit -m 'Add amazing feature'`）
4. 推送分支（`git push origin feature/amazing-feature`）
5. 发起 Pull Request

### 致谢

- 使用 [Rust](https://www.rust-lang.org/) 构建
- 终端 UI 由 [Ratatui](https://github.com/ratatui/ratatui) 驱动
- LLM 集成通过 [Rig](https://github.com/0xPlaygrounds/rig)
- MCP 支持遵循 [Model Context Protocol](https://modelcontextprotocol.io/)

### 支持

- [问题反馈](https://github.com/xyz1o2/starcode-cli/issues)
- [讨论区](https://github.com/xyz1o2/starcode-cli/discussions)

---

<p align="center">
  Made by <a href="https://github.com/xyz1o2">xyz1o2</a>
</p>

<p align="center">
  <a href="https://github.com/xyz1o2/starcode-cli/stargazers">
    <img src="https://img.shields.io/github/stars/xyz1o2/starcode-cli?style=social" alt="Stars">
  </a>
  <a href="https://github.com/xyz1o2/starcode-cli/network/members">
    <img src="https://img.shields.io/badge/forks-grey?style=social" alt="Forks">
  </a>
</p>
