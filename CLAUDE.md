# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

StarCode CLI (`starcode-cli`) — a Rust terminal-UI coding agent (ratatui 0.30 + crossterm + tokio) that talks to OpenAI-compatible and Anthropic-style providers. One crate: lib (`src/lib.rs`) plus one bin (`src/main.rs`), ~190k lines across `src/`.

The codebase is deliberately modeled on Claude Code — comments say "对标 Claude Code" throughout. Read-only reference copies live in `study_or_copy_projects/` (gitignored): `claude-code-main`, `claude-code-rust`, `claude-code-system-prompts-main`, `tuie-main`. Consult them when matching behavior or terminal rendering; `docs/ui-tool-render.md` is a worked example of that comparison.

## Commands

```bash
cargo check --all-targets        # fastest correctness gate — use this while iterating
cargo build --release           # → target/release/starcode-cli
cargo run                       # start the TUI

# tests
cargo test --lib                            # the real suite: 176 unit tests inside src/
cargo test --lib commands::parity           # one module
cargo test --lib detects_cargo_verifiers    # one test by name
cargo test --lib -- --nocapture             # see println! from tests

# CI gates (.github/workflows/ci.yml, runs with -Dwarnings)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib
cargo test --workspace --doc
cargo test --workspace --test eval_harness_live -- --test-threads=1
```

### Gate state — verify, don't assume

Both lint gates fail on pre-existing code, so a green run isn't the bar; *not adding new problems* is.

- `cargo fmt --check`: 43 files differ.
- `cargo clippy`: 615 lib warnings. Scope to your work: `cargo clippy --lib 2>&1 | rg <your_file>`.
- `cargo test --lib`: 170 pass, 6–8 fail. Pre-existing failures are `agent::mcp_permission`, `agent::tool_enhanced::tests::test_otlp_logger`, `core::auto_mode::dangerous_patterns` (×2), `core::tools::verify_edit::tests::test_extract_line_number`, `ui::components::chat_input::tests::test_border_color_default`, and `utils::checkpoint_manager` (cwd-dependent, flaky). Check a failing test at HEAD before concluding your change broke it.

## Architecture

### Two entry paths (`src/main.rs`)

- **Headless** (`-p/--prompt`): builds `Config` + `StarAgent` synchronously, calls `StarAgent::process_user_message`, prints JSONL or text. No UI.
- **Interactive**: enters the TUI *immediately* with a loading screen while `Config::initialize()` + `StarAgent::new()` run in a spawned task under a 30s timeout, arriving over a `oneshot` channel (`main.rs:548-629`). Startup failures therefore look like a stuck loading screen, not a crash — the `[INIT]` breadcrumbs in `.star/logs/agent.log` are how you diagnose them.

Subcommands bypass both paths: `mcp`, `git`, `init`, `eval`.

### UI ↔ Agent: two channels, one worker

The entire runtime is two capacity-100 `mpsc` channels (`src/ui/app/runtime.rs:1132`) and one tokio task:

```
UI  --AgentRequest-->  agent_worker  --StreamMessage-->  UI
```

- `src/runtime/messages.rs` — the protocol, and the first file to read. `AgentRequest` (UI→Agent: `SendMessage`, `Abort`, `SetModel`, `GenerateNote`, `PluginOp`, …) and `StreamMessage` (Agent→UI: `TextDelta`, `ToolCalls`, `ToolResult`, `AgentTaskUpdate`, `Done`, …) define every legal interaction between the two halves.
- `src/ui/services/worker.rs` — `agent_worker` drains `steering_queue` before `rx` so interrupts beat queued input. `SendMessage` goes to `runtime::agent_runtime::process_message`; everything else to `runtime::control_requests::handle_request`.
- `src/runtime/` — the seam: `agent_runtime` (per message: preflight hooks → streaming session → after-agent hooks → deferred actions), `streaming_session`, `control_requests`, `confirmation_bridge`, `hooks`, `checkpoints`.
- `src/ui/services/stream.rs` — `handle_stream_update`, the single `match` translating every `StreamMessage` into `ChatState` mutations, processed by the UI loop in 8ms batches.

Threading model is documented at `src/ui/app/runtime.rs:1-28`: tokio main thread renders, a dedicated `std::thread` reads keys from `/dev/tty`, the agent worker is a task, plus a watchdog for ANR detection. All UI state is one struct — `ChatState` in `src/ui/state/store.rs`, 228 public fields.

### Input routing (`src/ui/app/logic.rs`, `enqueue_user_message`)

Every submitted line funnels through here and is dispatched by prefix before anything reaches the model: `/` → `commands::handle_command`; `#` → `/memory add`; `!` → direct shell via `ShellExecutionService` (refused in Plan mode); `@` → `at_processor` inlines file contents and fires `MarkFilesAsRead`. Only then does `AgentRequest::SendMessage` go out.

### Slash commands — four touch points

Adding or changing a command means editing all of these:

1. `src/commands/system.rs` — declare it in `ALL_COMMANDS: &[SlashCommand{name, alt_names, description, category, sub_commands}]`. The `category` string is load-bearing twice: `format_help()` (~line 2154) only prints a hardcoded list of categories, so an unlisted category hides the command from `/help`; and `is_declared_pending()` (~line 1893) treats `category: "Pending"` as "declared but unimplemented".
2. Implement it in a topic module — `parity.rs` (Claude Code-parity commands), `extended.rs`, `compat.rs`, `permissions.rs`, `plugin.rs`, `provider.rs`, `agents/`, …
3. `src/commands/mod.rs` — add a `match name` arm in `handle_command`. **Order matters**: the `n if system::is_declared_pending(n)` arm near the end swallows anything still marked Pending, and a duplicate arm earlier in the match silently wins over a later one.
4. Flip the category off `"Pending"` — otherwise the command is declared and dispatched but still answers with the placeholder.

Signature is `async fn name(ctx: CommandContext<'_>, args: …) -> CommandResult`, where `CommandContext { state: &mut ChatState, agent_tx: &Sender<AgentRequest> }` (`src/commands/execution.rs`). Older commands take `Vec<String>`, newer parity ones take `&[String]`; the dispatch arm passes `args` or `&args` to match. Read-only commands push markdown into `ctx.state.chat_history`; commands that need the model send `AgentRequest::SendMessage`, or `GenerateNote` for side-channel output that must not enter the main context.

### Tools

`ToolRegistry` (`src/core/tools/tool_registry.rs`) is a `HashMap<String, Arc<dyn BaseDeclarativeTool>>` behind a generation counter, with a cached `Vec<FunctionDeclaration>` — that cache is what the LLM sees. Two traits in `src/core/tools/tools.rs`: `BaseDeclarativeTool` (name / description / schema / `create_invocation`) and `ToolInvocation` (`should_confirm_execute` → `execute`).

Registration is centralized in `src/core/config/runtime_bootstrap/`: `core_runtime.rs` (file, search, navigation tools, each gated by `is_core_tool_enabled`) and `agent_runtime.rs` (the ~54 needing the LLM client or heavier wiring). Plugin tools are swapped in via `sync_plugin_tools`, which refuses names colliding with built-ins. Aliases resolve through `canonical_tool_name` (`src/core/tools/constants.rs`) — e.g. `view_file` → `Read`.

Implementations are split between `src/core/tools/` (~60 files) and `src/tools/` (bash, search, todo, git_insight, lsp, …). Presence in the tree does not mean a tool is live — see "Not everything is wired up".

### Prompts and tool descriptions are `.md`, not Rust

`src/core/prompts/system-prompts/` holds 85 markdown files embedded at compile time via `rust_embed` (`SystemPrompts`) and overridable at runtime: `STAR_PROMPT_DIR` > `~/.starcode/prompts/` > `./.star/prompts/` > embedded, with mtime-based cache invalidation (`src/core/prompts/loader.rs`). 68 are `tool-description-*.md`; per `tool_descriptions.rs`, the frontmatter `description:` becomes the schema description sent to the LLM while the file body joins the system prompt bundle. **To change what the model is told about a tool, edit the `.md`** — the registry-name → file-key mapping is a hand-maintained `HashMap` in that file.

### Permission / confirmation flow

A tool needing approval publishes `ToolConfirmationRequest` to `MessageBus` (`src/core/confirmation_bus/`), which consults `PolicyEngine::check` (`src/core/policy/policy_engine.rs`). Allow and Deny resolve immediately; ask-user broadcasts, the worker's `bus_rx` picks it up, `runtime/confirmation_bridge.rs` renders it into a `StreamMessage::ToolConfirmationRequest` (diff view, generic dialog, or `ask_user_question` options), and the answer returns as `AgentRequest::ConfirmTool`. `ApprovalMode` is only `Default | Plan | Yolo` (`src/core/policy/types.rs`); `--permission-mode acceptEdits` and `bypassPermissions` are aliases for `default` and `yolo` (`main.rs:637`), not distinct modes.

### Agent internals

`StarAgent` (`src/agent/workflows/star_agent.rs`, re-exported as `agent::StarAgent`) is a thin wrapper — abort flag, approval-mode lock, MCP manager, steering queue — around `Agent` (`src/agent/agent_core.rs`), which owns the machinery: `ToolExecutor`, `StreamingToolExecutor`, `CompactManager`, `ContextEngine` (lazily initialized; the constructor does no I/O), `session_messages`, token budget tracker, stall detector. The turn loop is `Agent::run_agentic_loop` → `execute_turn` (`src/agent/agent_loop.rs`), bounded by `max_session_turns` (default 200), with per-turn tool shortlisting (`src/agent/tool_routing/`), repeat-loop detection, nudges, and compression checks. Compaction is a family of strategies under `src/agent/compact/` (auto, reactive, micro, tool-output, snip); retrieval and indexing live in `src/core/context/` (tree-sitter chunking, symbol and call-graph indexes).

### Config and provider resolution

`ConfigParameters` (`src/core/config/config_types.rs`) is a ~90-field struct built in one shot in `main.rs`, almost all `None`. Effective model / base URL / API key come from `provider_resolution.rs`, which returns each value paired with a `SourceRef` naming its origin — precedence is session → CLI flag → `STAR_*` env → `ANTHROPIC_*` env → provider store → user settings. When a "wrong model" bug shows up, that `SourceRef` is the answer.

Settings live in `~/.star/settings.json` (global) and `./.star/settings.json` (project). Build every path through `Storage` (`src/core/config/storage.rs`) rather than joining `.star` by hand. Project instructions injected into the system prompt come from the first existing of `STAR.md`, `STARCODE.md`, `CLAUDE.md` (`CONTEXT_FILE_CANDIDATES`, `src/utils/project_context.rs:11`), truncated to 8000 chars.

### Behavior is env-var-gated to an unusual degree

337 distinct `STAR_*` variables are read across the tree, almost always inline at the point of use (`std::env::var("STAR_…").ok().and_then(…).unwrap_or(default)`), with no central registry. When runtime behavior diverges from what the code appears to do, grep `STAR_` in that file. Frequently relevant: `STAR_API_KEY`, `STAR_BASE_URL`, `STAR_MODEL`, `STAR_CONTEXT_WINDOW`, `STAR_AUTO_COMPACT`, `STAR_LLM_TIMEOUT`, `STAR_TOOL_TIMEOUT_SECS`, `STAR_LOG_ENABLED`, `STAR_LOG_DIR`, `STAR_PROMPT_DIR`.

### Debugging a TUI

You cannot print — ratatui owns the terminal. File logging is **on by default** and writes to `.star/logs/starcode_debug.log` and `.star/logs/agent.log` via `utils::logging::{append_debug_log_line, append_agent_log_line}`; `STAR_LOG_DIR` relocates it and `STAR_LOG_ENABLED=0` disables it. Existing call sites tag lines `[Worker]`, `[AgentRuntime]`, `[INIT]`, `[UI]` — follow that.

## Conventions

- **Comments and doc comments in Chinese; user-visible strings in English.** `src/commands/` follows this strictly. Older code (e.g. `src/ui/app/logic.rs`) still has Chinese in user-facing strings.
- Localized strings go through `i18n::t(key, zh, en)` (`src/core/i18n.rs`): dictionary hit first, else the language-appropriate literal.
- Large modules open with a `///` header block covering architecture, threading, and error handling (`ui/app/runtime.rs`, `runtime/messages.rs`, `ui/services/worker.rs`). Match that when adding a module of comparable weight.
- `src/lib.rs` and `src/main.rs` both open with a blanket `#![allow(dead_code, unused_imports, unused_variables, unused_mut, unused_assignments)]`. The compiler will not tell you something is unused — decide by grepping for call sites.
- rustfmt `max_width = 100`; clippy.toml sets `too-many-lines-threshold = 100`, `too-many-arguments-threshold = 10`.

## Repo facts that will otherwise cost you time

**Not everything is wired up.** Several subsystems compile, read plausible, and are never constructed. Verified: `ModelFallbackManager` (`src/agent/model_fallback.rs`) is only ever built inside its own `#[cfg(test)]` module, so `STAR_MODEL_FALLBACK_*` is ignored; `PolicyEngine::load_permission_rules` (`policy_engine.rs:52`) has zero callers, so `.star/permissions.json` rules are advisory; `WebBrowserTool` implements `BaseDeclarativeTool` but `WebBrowserTool::new` is never called, so the model cannot reach it; the analytics HTTP sink logs `"[Analytics HTTP] Would send …"` instead of sending. Before building on any subsystem, grep for its constructor and confirm something in `runtime_bootstrap` or the agent actually reaches it.

**`tests/` is mostly dead.** Cargo builds only `tests/lib.rs` and `tests/eval_harness_live.rs`. `tests/lib.rs` declares just `mod core;`, and `tests/core/mod.rs` declares just `test_deferred` + `test_permissions` — so `tests/agent/`, `tests/config/`, `tests/llm/`, `tests/scenarios/`, `tests/tools/`, `tests/common/`, and `tests/core/{context,tasks,tools}_test.rs` are never compiled. Worse, `.gitignore:28` is `**/test_*.rs`, so the two files `tests/core/mod.rs` *does* declare are untracked and `cargo test` cannot build `tests/lib.rs` from a fresh clone. Put new tests in a `#[cfg(test)] mod tests` inside the `src/` file under test — that is where all 176 working tests live — and avoid naming any new file `test_*.rs`.

**Orphaned directories.** `src/constants/` and `src/sdk/` are declared by neither `lib.rs` nor `main.rs`; editing them changes nothing. Root `config.toml` is read by no code (its `[ui.system_prompts] dir` even points at a path that does not exist) — real configuration is `~/.star/settings.json`, `./.star/settings.json`, and env vars.

**`Cargo.lock` is gitignored and untracked** despite this being a binary crate, so dependency versions re-resolve on every fresh checkout and CI run.

**README drift.** The README describes a nested `starcode-cli/` layout (`src/` is at the repo root) and calls the binary `starcode` (it is `starcode-cli`; `install.sh` does not rename it).
