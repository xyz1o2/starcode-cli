# StarCode CLI

**[English](#english) · [简体中文](./README.zh-CN.md)**

<p align="center">
  <img src="https://img.shields.io/badge/version-0.3.0-blue" alt="version">
  <img src="https://img.shields.io/badge/rust-2021-orange" alt="rust edition">
  <img src="https://img.shields.io/badge/license-MIT-green" alt="license">
  <img src="https://img.shields.io/badge/platform-cross--platform-lightgrey" alt="platform">
</p>

## English

A conversational AI coding agent that runs in your terminal — written in 100% Rust, modeled on Claude Code. Built on ratatui 0.30 + crossterm + tokio, and talks to OpenAI-compatible and Anthropic-style providers.

### Features

- **Multi-Provider** — OpenAI, Anthropic, and any OpenAI-compatible endpoint. Per-provider credentials with a provider store, switchable at runtime.
- **Interactive TUI** — streaming responses, diffs, syntax highlighting, themes, mouse support.
- **Headless Mode** — drive a prompt from a script and get JSONL or plain text back.
- **Rich Toolchain** — read/write/edit/notebook tools, shell, ripgrep search, AST-aware codebase search, git insight, sub-agents, todos, cron, worktrees, MCP servers.
- **Permissions** — every tool call can be confirmed interactively; plan mode restricts the agent to read-only actions.
- **Project Context** — instructions are picked up from `STAR.md` / `STARCODE.md` / `CLAUDE.md` / `AGENTS.md` in your project.
- **Sessions** — save, resume, and manage conversation history.
- **Internationalization** — English and Chinese UI.
- **Cross-Platform** — Linux, macOS, and Windows.

### Installation

#### From Source (Recommended)

```bash
git clone https://github.com/xyz1o2/starcode-cli.git
cd starcode-cli

./install.sh          # Windows: .\install.ps1
```

`install.sh` builds **one** binary (`starcode-cli`) and symlinks two aliases next to it in `~/.cargo/bin`, so all three start the same program:

```bash
sc              # short form
starcode
starcode-cli
```

> Note: `cargo build --release` on its own only gives you the `starcode-cli` name — the `sc` / `starcode` aliases are created by the install script.

#### Using Cargo

```bash
cargo install --git https://github.com/xyz1o2/starcode-cli.git starcode-cli
```

#### Pre-built Binaries

Download the latest release for your platform from the [Releases](https://github.com/xyz1o2/starcode-cli/releases) page.

### Quick Start

#### 1. Set up your API key

```bash
# Option 1: Environment variable
export STAR_API_KEY="your-api-key"

# Option 2: Config file (see "Configuration" below)

# Option 3: One-off CLI flag
sc -k "your-api-key"
```

#### 2. Start an interactive session

```bash
sc                                    # interactive TUI
sc "Explain the structure of this project"   # with an initial message
```

#### 3. Go headless for scripting

```bash
sc -p "What files are in the current directory?"
sc -p "List all Rust files" --output-format text
```

### Configuration

Credentials and model settings resolve in this order (first wins): in-session override → CLI flag → `STAR_*` env vars → `ANTHROPIC_*` env vars → provider store → user settings file.

#### User settings — `~/.star/user-settings.json`

```jsonc
{
  "apiKey": "your-api-key",
  "baseUrl": "https://api.openai.com/v1",
  "defaultModel": "gpt-5",
  "isOpenAICompatible": true,
  "uiLanguage": "en",            // "en" | "zh"
  "thinkingEffort": "medium",
  "outputStyle": "default",
  "contextWindow": 200000
}
```

#### Project settings — `.star/settings.json`

Searched from the current directory **upwards** to the project root (`.jsonc` with comments is supported). This is where shared, checked-in configuration lives. Global settings live at `~/.star/settings.json`.

#### Environment variables

| Variable                 | Purpose                            |
| ------------------------ | ---------------------------------- |
| `STAR_API_KEY`           | API key                            |
| `STAR_BASE_URL`          | Custom API base URL                |
| `STAR_MODEL`             | Default model                      |
| `STAR_CONTEXT_WINDOW`    | Context window size                |
| `STAR_LLM_TIMEOUT`       | LLM request timeout                |
| `STAR_TOOL_TIMEOUT_SECS` | Tool execution timeout             |
| `STAR_LOG_DIR`           | Relocate the log directory         |
| `STAR_LOG_ENABLED`       | Set to `0` to disable file logging |

#### Project instructions

StarCode reads instructions from the first existing of `STAR.md`, `STARCODE.md`, `CLAUDE.md`, `AGENTS.md` at the project root (truncated to 8000 chars). Use `starcode init` to scaffold a `STAR.md`.

### Commands

#### Interactive mode

```bash
starcode                              # start interactive session
starcode -d /path/to/project          # working directory
starcode --resume                     # resume the latest session
starcode --resume <session-id>        # resume a specific session
starcode --dangerously-skip-permissions   # skip all prompts (dangerous!)
```

#### Headless mode

```bash
starcode -p "Your prompt here"
starcode -p "Your prompt" --output-format jsonl
starcode -p "Your prompt" --output-format text
starcode -p "Your prompt" --max-turns 50 --max-tool-rounds 200
```

#### Subcommands

```bash
starcode init                          # scaffold STAR.md
starcode doctor                        # diagnose config, credentials, toolchain, logs
starcode mcp add <name> <command>      # register an MCP server
starcode mcp list                      # list configured servers
starcode mcp remove <name>             # remove a server
starcode git commit                    # AI-assisted git operations
starcode git diff
starcode git status
```

#### Permission modes

```bash
starcode --permission-mode default     # ask for permission (default)
starcode --permission-mode plan        # read-only; plans before touching anything
starcode --permission-mode yolo        # bypass all permissions (dangerous!)
```

`acceptEdits` and `bypassPermissions` are accepted as aliases for `default` and `yolo` respectively.

#### Slash commands in the TUI

Roughly 300 slash commands are available, grouped into Automation, Tools, Session, Config, Security, Git, Debug, MCP, Memory, and more. Type `/` to browse them, and `/help` for the full list.

### Built-in Tools

| Tool                                                  | What it does                                                                          |
| ----------------------------------------------------- | ------------------------------------------------------------------------------------- |
| `Read` / `Write` / `Edit` / `MultiEdit` / `SmartEdit` | File reading and precise editing (SmartEdit repairs malformed edits with an LLM pass) |
| `NotebookRead` / `NotebookEdit`                       | Jupyter notebook support                                                              |
| `Bash`                                                | Shell execution with sandboxing and timeouts                                          |
| `Grep` / `Glob`                                       | ripgrep-backed search and file patterns                                               |
| `CodebaseSearch`                                      | AST-aware semantic search over the codebase                                           |
| `Agent` / `SendMessage`                               | Spawn and steer sub-agents                                                            |
| `TodoWrite` / `TaskGet`                               | Task and todo tracking                                                                |
| `WebFetch`                                            | Fetch and analyze web pages                                                           |
| `GitInsight` / `GhPrComments`                         | Git history and GitHub PR context                                                     |
| `EnterPlanMode` / `ExitPlanMode`                      | Plan-mode control                                                                     |
| `EnterWorktree` / `ExitWorktree`                      | Isolated git worktrees                                                                |
| `CronCreate` / `CronList` / `CronDelete`              | Scheduled tasks                                                                       |
| `BackgroundTask` / `ScheduleWakeup` / `RemoteTrigger` | Background and deferred execution                                                     |
| `Memory`                                              | Persistent agent memory                                                               |
| `GetDiagnostics` / `RunTests` / `ProjectMap`          | LSP diagnostics, test runner, project overview                                        |
| `Skill` / `ToolSearch`                                | Skill invocation and tool discovery                                                   |

Plus any tools contributed by configured MCP servers.

### MCP Support

StarCode supports the Model Context Protocol for extensible toolchains:

```bash
starcode mcp add filesystem "npx -y @modelcontextprotocol/server-filesystem /path/to/dir"
starcode mcp list
starcode mcp remove filesystem
```

### Development

```bash
cargo check --all-targets      # fastest correctness gate — use while iterating
cargo build --release          # → target/release/starcode-cli
cargo test --lib               # unit tests live inside src/
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

#### Project structure

```
starcode-cli/
├── src/
│   ├── main.rs         # binary entry: CLI parse, headless path, TUI bootstrap
│   ├── lib.rs          # library root
│   ├── agent/          # agent core, turn loop, compaction, tool routing, fallback
│   ├── commands/       # slash commands and their dispatch
│   ├── core/           # config, tools, policy, context engine, confirmation bus, i18n
│   ├── llm/            # LLM clients and streaming
│   ├── runtime/        # UI ↔ Agent protocol and the runtime seam
│   ├── tools/          heavier tool implementations (bash, search, todo, git, lsp)
│   ├── types/          # shared types
│   ├── ui/             # terminal UI (ratatui): state, widgets, services
│   └── utils/          # logging, paths, project context, misc
├── eval/               # eval task definitions
├── i18n/               # translations
├── install.sh / .ps1 / .bat
└── Cargo.toml
```

#### Debugging the TUI

You can't `println!` from a TUI. File logging is **on by default** and writes to `.star/logs/starcode_debug.log` and `.star/logs/agent.log`. Set `STAR_LOG_DIR` to relocate them, or `STAR_LOG_ENABLED=0` to disable. If the app starts but the loading screen never clears, `starcode doctor` reports what's wrong without launching the TUI.

### Evaluation

StarCode ships with a built-in eval harness:

```bash
starcode eval --tasks eval/tasks.json
starcode eval --tasks eval/tasks.json --trials 3
starcode eval --tasks eval/tasks.json --report-md eval-report.md
starcode eval --baseline .star/eval-baseline.json
```

### Troubleshooting

**API key not found** — `echo $STAR_API_KEY`, check `~/.star/user-settings.json`, or run `starcode doctor`.

**Build fails** — make sure Rust is installed (`rustc --version`), then `rustup update` and `cargo clean && cargo build --release`.

**Stuck on the loading screen** — startup failures don't crash, they hang the loading view. Check `.star/logs/agent.log` for `[INIT]` breadcrumbs, or run `starcode doctor`.

### Contributing

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

### Acknowledgments

- Built with [Rust](https://www.rust-lang.org/)
- Terminal UI powered by [Ratatui](https://github.com/ratatui/ratatui)
- LLM integration via [Rig](https://github.com/0xPlaygrounds/rig)
- MCP support following the [Model Context Protocol](https://modelcontextprotocol.io/)

### Support

- [Report Issues](https://github.com/xyz1o2/starcode-cli/issues)
- [Discussions](https://github.com/xyz1o2/starcode-cli/discussions)

---

<p align="center">
  Made by <a href="https://github.com/xyz1o2">xyz1o2</a>
</p>

<p align="center">
  <a href="https://github.com/xyz1o2/starcode-cli/stargazers">
    <img src="https://img.shields.io/github/stars/xyz1o2/starcode-cli?style=social" alt="Stars">
  </a>
  <a href="https://github.com/xyz1o2/starcode-cli/network/members">
    <img src="https://img.shields.io/github/forks/xyz1o2/starcode-cli?style=social" alt="Forks">
  </a>
</p>
