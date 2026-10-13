# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

StarCode CLI (`starcode-cli`) — a Rust terminal-UI coding agent (ratatui 0.30 + crossterm + tokio) that talks to OpenAI-compatible and Anthropic-style providers. One crate: lib (`src/lib.rs`) plus one bin (`src/main.rs`), ~188k lines across 696 files in `src/`.

The codebase is deliberately modeled on Claude Code — comments say "对标 Claude Code" throughout. Read-only reference copies live in `study_or_copy_projects/` (gitignored): `claude-code-main`, `claude-code-rust`, `claude-code-system-prompts-main`, `tuie-main`. Consult them when matching behavior or terminal rendering; `docs/ui-tool-render.md` is a worked example of that comparison.

**对标文档看 `docs/claude-code-alignment.md`**（2026-10-13 快照，source of truth 是官方文档 v2.1.2xx）。不要用 `docs/parity-*.md` 系列做架构决策：那批文档基于泄漏源码 v2.1.88，已过期，且含已被证伪的「未接线」结论（PolicyEngine、ModelFallbackManager、Workflow）—— 具体作废原因见各文件顶部声明与新文档 §6。`study_or_copy_projects/` 下的副本只适合做 UI 层参考。

## Commands

```bash
cargo check --all-targets        # fastest correctness gate — use this while iterating
cargo build --release           # → target/release/starcode-cli
cargo run                       # start the TUI

# tests
cargo test --lib                            # the real suite: ~850 unit tests inside src/
cargo test --lib commands::parity           # one module
cargo test --lib detects_cargo_verifiers    # one test by name
cargo test --lib -- --nocapture             # see println! from tests

# CI gates (.github/workflows/ci.yml) — see "Gate state"
cargo check --workspace --all-targets --locked   # the real blocking gate
cargo fmt --all -- --check                       # blocking gate, currently green
cargo clippy --workspace --all-targets          # advisory in CI, no -D warnings yet
cargo test --workspace --lib -- --test-threads=1
cargo test --workspace --doc
```

### Gate state — verify, don't assume

- `cargo fmt --check`: **red as of 2026-10-13** — the "debt cleared" note below was wrong.
  Run it before claiming green; the outstanding files are a handful of `core/permission*`
  and `ui/events/*` modules left over from the UI refactors.
- `cargo clippy --workspace --all-targets`: ~470 warnings, 0 errors. Not gated: CI runs it
  without `-D warnings`. Scope to your work: `cargo clippy --lib 2>&1 | rg <your_file>`.
- `cargo test --workspace --lib -- --test-threads=1`: green as of 2026-10-13, 852 tests.

`--test-threads=1` is mandatory for the lib tests: `utils::checkpoint_manager` mutates the
process-global `STAR_FILE_HISTORY_DIR_OVERRIDE`, so parallel tests delete each other's
state and fail. Six of its tests fail without it.

The blocking gates are `cargo check --workspace --all-targets --locked`, the test jobs and
the build jobs. Once the clippy debt is cleared, add `-- -D warnings` back to the clippy
step in `ci.yml`.

## Architecture

### Two entry paths (`src/main.rs`)

- **Headless** (`-p/--prompt`): builds `Config` + `StarAgent` synchronously, calls `StarAgent::process_user_message`, prints JSONL or text. No UI.
- **Interactive**: enters the TUI *immediately* with a loading screen while `Config::initialize()` + `StarAgent::new()` run in a spawned task under a 30s timeout, arriving over a `oneshot` channel (`main.rs:548-629`). Startup failures therefore look like a stuck loading screen, not a crash — the `[INIT]` breadcrumbs in `.star/logs/agent.log` are how you diagnose them.

Subcommands bypass both paths: `mcp`, `git`, `init`.

### UI ↔ Agent: two channels, one worker

The entire runtime is two capacity-100 `mpsc` channels (`src/ui/app/runtime.rs:1390`) and one tokio task:

```
UI  --AgentRequest-->  agent_worker  --StreamMessage-->  UI
```

- `src/runtime/messages.rs` — the protocol, and the first file to read. `AgentRequest` (UI→Agent: `SendMessage`, `Abort`, `SetModel`, `GenerateNote`, `PluginOp`, …) and `StreamMessage` (Agent→UI: `TextDelta`, `ToolCalls`, `ToolResult`, `AgentTaskUpdate`, `Done`, …) define every legal interaction between the two halves.
- `src/ui/services/worker.rs` — `agent_worker` drains `steering_queue` before `rx` so interrupts beat queued input. `SendMessage` goes to `runtime::agent_runtime::process_message`; everything else to `runtime::control_requests::handle_request`.
- `src/runtime/` — the seam: `agent_runtime` (per message: preflight hooks → streaming session → after-agent hooks → deferred actions), `streaming_session`, `control_requests`, `confirmation_bridge`, `hooks`, `checkpoints`.
- `src/ui/services/stream.rs` — `handle_stream_update`, the single `match` translating every `StreamMessage` into `ChatState` mutations, processed by the UI loop in 8ms batches.

Threading model is documented at `src/ui/app/runtime.rs:1-28`: tokio main thread renders, a dedicated `std::thread` reads keys from `/dev/tty`, the agent worker is a task, plus a watchdog for ANR detection. All UI state is one struct — `ChatState` in `src/ui/state/store.rs`, ~190 public fields (the exact count keeps drifting with every UI refactor; grep it rather than trusting a number).

### UI architecture: TEA (Model–Update–View)

`src/ui/` follows the Elm Architecture, ratified here because ratatui officially offers it alongside the Component pattern and this codebase's central key dispatch is already TEA-shaped (`ui/events/key_context.rs::KeyContext` is the update dispatcher). The rule:

- **`src/ui/events/`** — Update. Key/mouse in, `ChatState` mutated, nothing drawn.
- **`src/ui/components/`** — View. Read `ChatState`, draw a buffer. No `handle_*` functions, no key decoding.
- **`src/ui/services/`** — state machines and logic that Update drives (`stream.rs`, `suggestions.rs`, `worker.rs`, `at_processor.rs`).
- **`src/ui/state/`** — data plus its own methods.

**A `render_*` function may write exactly two kinds of state**, and nothing else:

1. **Layout memory** — `last_terminal_width`, `last_chat_area`, `last_chat_height`, `last_max_scroll`. ratatui's rendering docs call this out as *the* standard immediate-mode workaround: the view only learns its area at draw time, so it remembers last frame's area and corrects on the next one, at the cost of one frame of drift.
2. **Memoization** — `rendered_cache`, `virtual_list`, `total_rendered_lines`.

Anything that is a *business* state machine — a timer, a mode transition, anything deciding what state becomes next — does not belong in a render path, even though `render_*` takes `&mut ChatState`. That signature is deliberate and sanctioned: `List`, `Table`, and `Scrollbar` are `StatefulWidget`s, so ratatui explicitly permits `fn view(model: &mut Model, f: &mut Frame)`.

The 1.5s post-interrupt grace period is the worked example. It used to be implemented **twice** — once correctly in `runtime.rs`, once inside `render_chat_lines`, where merely drawing the chat area consumed the timer and cleared `is_streaming`. Both copies are now one block in the event loop. Anything shaped like it belongs on the 1Hz tick alongside `tick_task_panel` / `expire_toasts`, not in a component.

**Component-pattern exception:** a few widgets own genuinely private state, so their input handling stays beside their render code rather than moving to `events/`. That is the officially blessed alternative, not a leak: `task_panel::handle_edit_input`, `theme_picker::handle_theme_picker_input`, `global_search::handle_global_search_input`. Add to this list only for a struct that owns its own state end to end — not as a way to park key handling in `components/`.

Refs: <https://ratatui.rs/concepts/rendering/> (immediate vs retained), <https://ratatui.rs/concepts/application-patterns/the-elm-architecture/> (the `&mut` view sanction), <https://ratatui.rs/concepts/application-patterns/component-architecture/>.

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

342 distinct `STAR_*` variables are read across the tree, almost always inline at the point of use (`std::env::var("STAR_…").ok().and_then(…).unwrap_or(default)`), with no central registry. When runtime behavior diverges from what the code appears to do, grep `STAR_` in that file. Frequently relevant: `STAR_API_KEY`, `STAR_BASE_URL`, `STAR_MODEL`, `STAR_CONTEXT_WINDOW`, `STAR_AUTO_COMPACT`, `STAR_LLM_TIMEOUT`, `STAR_TOOL_TIMEOUT_SECS`, `STAR_LOG_ENABLED`, `STAR_LOG_DIR`, `STAR_PROMPT_DIR`.

### Ten of those knobs the model can now turn itself

`core/self_tuning` adds an **override layer above env** for the settings that are read at the point of use or rebuilt per turn: the 8 compaction knobs (`agent/compact/mod.rs::from_env`), `tools.max_result_chars` (`result_budget.rs`), and `tools.confirmed_call_timeout_secs` (`control_requests.rs`). Resolution is `override > env > default`, so existing `STAR_*` configuration is untouched and still wins over defaults. The model's entry point is the `self_config` tool (`inspect` / `set` / `reset`).

Four constraints, all deliberate:

- **Only genuinely live knobs are listed.** `knobs.rs` admits a setting only if its read site parses on use. `STAR_LLM_TIMEOUT` is baked into `reqwest::Client`'s `.timeout()` and `Config`'s `context_window` is frozen in `Config::new` — giving the model a write surface for those would make it believe it had tuned something.
- **No `std::env::set_var`.** tokio is multi-threaded, so writing process env from a tool races other threads' `getenv`. The `RwLock` table also keeps "the user set this" distinguishable from "the model set this", which `inspect` must report.
- **`STAR_SELFTUNING_LOCKED`** (comma-separated keys) is an operator lock; the model cannot write a locked knob. Without it the override layer's "override wins" would silently defeat a value someone deliberately pinned.
- **`STAR_SELFTUNING_MAX_SETS`** (default 12) caps successful writes per session. `reset` never spends budget, so a model can always undo a bad change.

Model and context window are still **not** self-tunable: `agent_run.rs` freezes them into the per-message fresh `Agent` at turn start, so they apply to the next user message via the existing `RuntimeSettingsMutation` path. `inspect` lists them as read-only and points at `/model` and `/context`. Persisted overrides land in `~/.star/self-tuning.json` (`Storage::self_tuning_path`), deliberately **not** `settings.json` — a within-session adjustment should not become the user's permanent configuration.

### Debugging a TUI

You cannot print — ratatui owns the terminal. File logging is **on by default** and writes to `.star/logs/starcode_debug.log` and `.star/logs/agent.log` via `utils::logging::{append_debug_log_line, append_agent_log_line}`; `STAR_LOG_DIR` relocates it and `STAR_LOG_ENABLED=0` disables it. Existing call sites tag lines `[Worker]`, `[AgentRuntime]`, `[INIT]`, `[UI]` — follow that.

## Conventions

- **Comments and doc comments in Chinese; user-visible strings in English.** `src/commands/` follows this strictly. Older code (e.g. `src/ui/app/logic.rs`) still has Chinese in user-facing strings.
- Localized strings go through `i18n::t(key, zh, en)` (`src/core/i18n.rs`): dictionary hit first, else the language-appropriate literal.
- Large modules open with a `///` header block covering architecture, threading, and error handling (`ui/app/runtime.rs`, `runtime/messages.rs`, `ui/services/worker.rs`). Match that when adding a module of comparable weight.
- `src/lib.rs` and `src/main.rs` both open with a blanket `#![allow(dead_code, unused_imports, unused_variables, unused_mut, unused_assignments)]`. The compiler will not tell you something is unused — decide by grepping for call sites.
- rustfmt `max_width = 100`; clippy.toml sets `too-many-lines-threshold = 100`, `too-many-arguments-threshold = 10`.

## Repo facts that will otherwise cost you time

**Not everything is wired up.** Several subsystems compile, read plausible, and are never constructed. Verified: the analytics HTTP sink logs `"[Analytics HTTP] Would send …"` instead of sending (`core/analytics/sink.rs:133`), and the Langfuse client does the same (`core/langfuse/client.rs:315`). Before building on any subsystem, grep for its constructor and confirm something in `runtime_bootstrap` or the agent actually reaches it.

**Two claims in this section were wrong until 2026-10-13 — do not reintroduce them.**

- `PolicyEngine::load_permission_rules` is **live**. The real entry point is `PolicyEngine::with_project_rules` (`policy_engine.rs:60`), which calls `load_project_permissions` → `load_permission_rules` (`policy_engine.rs:91`). The only runtime construction site is `runtime_bootstrap.rs:40`, so `.star/settings.json` `permissions` **are** enforced. The old note here said it had zero callers and cited `policy_engine.rs:52` — both wrong.
- `WebBrowserTool` **no longer exists** in the tree. Only `TOOL_WEB_BROWSER` (`constants/tools.rs:42`) survives. An earlier version of this section described it as "implements `BaseDeclarativeTool` but `new` is never called".

**Counter-example worth remembering:** `ModelFallbackManager` (`src/agent/model_fallback.rs`) was long listed here as dead ("only built in `#[cfg(test)]`"). That was wrong — `agent_core.rs` constructs it and `agent_llm.rs` calls `is_fallback_eligible_error`/`try_fallback` on the live path, so `STAR_MODEL_FALLBACK_*` does take effect. Because `#![allow(dead_code)]` is on, plausibility is not evidence; only a constructor grep is.

**Writing a test that answers a confirmation prompt — use `if let`, not `while let`.** The message bus carries more than one message type. A responder written as `while let Ok(Message::ToolConfirmationRequest(r)) = rx.recv().await` **exits the loop** the moment any other variant arrives, and then every later `bus.request(...)` has nobody to answer it and blocks until the confirmation timeout (default 10 minutes). The symptom is misleading: the test looks like it fails on its *first* assertion and takes exactly 600s. Use `while let Ok(msg) = rx.recv().await { if let Message::ToolConfirmationRequest(r) = msg { … } }`. `agent::tool_executor::tests::allow_for_session_prevents_the_second_prompt` is the worked example.

**`PolicyDecision::AskUser` has two sources; do not conflate them.** An explicit `ask` rule in `settings.json` means "ask me every time", and it must outrank a session grant. But `PolicyEngine::new`'s fallback `default_decision` is also `AskUser` (`policy_engine.rs`), which only means "no rule matched, be conservative". When the two were conflated, `tool_executor` set `policy_requires_confirmation` for both, which made its `!policy_requires_confirmation` gate permanently false — so **"Allow for session" and "Always allow" never took effect for any tool**, and every confirmation dialog claimed to match an `ask` rule that didn't exist. `CheckResult::explicit_rule` now carries the provenance; only a real matched rule sets that flag. If you add another `AskUser` return path in `PolicyEngine::check`, you must decide which side of that flag it belongs on.

**Harness architecture baseline:** `docs/claude-code-alignment.md` is the single source of truth for Claude Code parity (snapshot 2026-10-13, built from official docs v2.1.2xx). The `docs/parity-*.md` files that used to carry architecture claims have been deleted; the five that remain cover UI rendering and UI framework structure only.

**`tests/` is mostly dead.** Cargo compiles only the top-level `tests/*.rs` files as integration targets, and only the scratch probes `zz_accuracy_probe.rs` / `zz_quality_probe.rs` remain there (the `eval_harness_live.rs` target was deleted with the eval harness on 2026-10-13). Everything in a subdirectory — `agent/`, `config/`, `llm/`, `scenarios/`, `tools/`, `common/`, `core/` — is only compiled if a `tests/*.rs` declares it, and none does. A `tests/lib.rs` used to declare `mod core` → `mod test_deferred` + `test_permissions`, but those two files were never committed, so `cargo test --workspace` could not compile from a fresh clone; that target has been deleted. Put new tests in a `#[cfg(test)] mod tests` inside the `src/` file under test — that is where all ~850 working tests live. `.gitignore` anchors the scratch-file patterns to the repo root (`/test_*.rs`) precisely because the old `**/test_*.rs` used to swallow files under `tests/`.

**The eval harness is gone (2026-10-13).** `starcode eval`, `eval/`, and `src/agent/eval_*.rs` were deleted. Two independent reasons: the mechanism layer's behavior rules validated `simulate_tool_calls_from_prompt` — which echoes the expected `tool_sequence` back as "simulated calls" — so it could not fail; and the live layer's fixture had been vendored as a gitlink without `.gitmodules`, so every fresh clone got an empty `eval/fixtures/rust-lab` and all ten live tasks silently skipped. The CI test asserted "everything is skipped", which is why the breakage went unnoticed. Consequence worth knowing: `Router::classify` and the skill heuristic now have **no** test coverage.

**`/eval` is a dangling declaration (2026-10-13).** `src/commands/system.rs:987` still declares `name: "eval"` in `ALL_COMMANDS` with `category: "Automation"`, but `src/commands/mod.rs` has **no `"eval" =>` dispatch arm** — the handler went with the eval harness. Result: `/eval` shows in `/help` and in `/` autocomplete, and typing it falls through to `plugin_command_fallback` and errors with `Unknown command: /eval`. It is a code leftover, not a doc one; removing the declaration from `ALL_COMMANDS` is the fix (note `is_declared_pending()` only catches `category: "Pending"`, so nothing warns about this today).

**Orphaned directories.** `src/constants/` and `src/sdk/` are declared by neither `lib.rs` nor `main.rs`; editing them changes nothing. Root `config.toml` is read by no code (its `[ui.system_prompts] dir` even points at a path that does not exist) — real configuration is `~/.star/settings.json`, `./.star/settings.json`, and env vars.

**`Cargo.lock` is committed** — it used to be gitignored despite this being a binary crate, so dependency versions re-resolved on every fresh checkout and every `--locked` build in CI/release failed. Build and release jobs now all pass `--locked`. Keep the lockfile in sync with `Cargo.toml` in the same commit when bumping dependencies.

**README drift.** The README describes a nested `starcode-cli/` layout (`src/` is at the repo root) and calls the binary `starcode` (it is `starcode-cli`; `install.sh` does not rename it).
