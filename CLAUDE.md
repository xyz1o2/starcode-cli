# CLAUDE.md

本文件是 StarCode CLI（`starcode-cli`）的仓库级开发指南。只记录当前仍有效、能影响实现或验证方式的约束；易过期的计数、历史事故和已删除文件不在这里维护。

## 项目概览

StarCode 是 Rust TUI coding agent，使用 ratatui 0.30、crossterm、tokio，兼容 OpenAI-style 与 Anthropic-style provider。一个 crate：`src/lib.rs` + `src/main.rs`。代码刻意对标 Claude Code；需要匹配终端行为时，可参考 gitignored 的 `study_or_copy_projects/` 副本，但架构基线以 `docs/claude-code-alignment.md` 为准，不要用已过时的 `docs/parity-*.md` 做架构决策。

`docs/` 只放活文档；结案报告、事故记录、一次性计划放 `docs/archive/`，移动时使用 `git mv` 保留历史。

## 常用命令与验证

```bash
cargo check --all-targets
cargo build --release
cargo run

cargo test --lib
cargo test --lib <module-or-test-name>
cargo test --lib -- --nocapture

cargo check --workspace --all-targets --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets
cargo test --workspace --lib -- --test-threads=1
cargo test --workspace --doc
```

提交前至少运行与改动相关的检查；声称全绿前重新运行对应 gate，不要依赖本文件中的历史结果。lib 测试必须使用 `--test-threads=1`：`utils::checkpoint_manager` 会修改进程级 `STAR_FILE_HISTORY_DIR_OVERRIDE`，并行执行会互相删除状态。CI 的阻塞检查是 locked check、fmt、测试和 build；clippy 当前未启用 `-D warnings`。

新测试优先放在被测 `src/` 文件的 `#[cfg(test)] mod tests` 中。Cargo 只把 `tests/` 顶层文件作为 integration target；不要把测试放进未声明的子目录。

## 运行入口与架构

### 入口

- `-p/--prompt` 是 headless 路径：构造 `Config` 与 `StarAgent`，调用 `process_user_message`，输出文本或 JSONL。
- 交互路径先进入 TUI loading screen，再在 spawned task 中初始化 `Config` 与 `StarAgent`（30 秒超时），通过 `oneshot` 返回。启动异常通常表现为 loading 卡住；查看 `.star/logs/agent.log` 中的 `[INIT]`。
- `mcp`、`git`、`init`、`doctor` 等 subcommand 绕过上述两条路径。

### UI 与 agent 通道

运行时是两个容量为 100 的 mpsc channel 和一个 agent worker：

```text
UI --AgentRequest--> agent_worker --StreamMessage--> UI
```

- `src/runtime/messages.rs` 是 UI/agent 协议的首要入口，定义所有 `AgentRequest` 与 `StreamMessage`。
- `src/ui/services/worker.rs` 先排空 `steering_queue`，因此中断/转向优先于普通队列；消息处理进入 `runtime::agent_runtime`。
- `src/ui/services/stream.rs::handle_stream_update` 是把每种 `StreamMessage` 转成 `ChatState` 变化的集中匹配点。
- `src/ui/app/runtime.rs` 负责 tokio 渲染线程、TTY 读键线程、agent task 和 ANR watchdog。不要在 TUI 渲染路径打印；使用 `utils::logging` 的文件日志（默认 `.star/logs/starcode_debug.log` 和 `.star/logs/agent.log`，可由 `STAR_LOG_DIR` 调整，`STAR_LOG_ENABLED=0` 关闭）。

外部 harness 的现有接口：

- `--output-format stream-json` 输出工具调用、文本增量、usage、错误，并以 `result` 行结束；事件名称是外部契约，手工序列化在 `src/agent/messaging/events.rs`。
- `.star/remote/inbox.jsonl` 与 `outbox.jsonl` 支持 `send_message`、`approve_tool`、`cancel`、`ping`，使用 `request_id` 配对。
- `starcode-cli mcp serve` 只暴露 `search_code` 和 `trace_call_chain`，不是完整 agent tool set。
- `src/core/bridge/` 当前没有启动 WebSocket listener；不要把它当作已接线的控制面。

### UI：TEA / Model–Update–View

- `src/ui/events/`：输入与状态更新，不绘制。
- `src/ui/components/`：只读 `ChatState` 并绘制，不放 key decoding 或业务 `handle_*`。
- `src/ui/services/`：由 Update 驱动的状态机和逻辑。
- `src/ui/state/`：状态数据及其方法。

`render_*` 只能写两类状态：布局记忆（如 `last_chat_area`、`last_max_scroll`）和 memoization（如 `rendered_cache`、`virtual_list`）。计时器、模式切换、业务状态迁移必须放在事件循环/tick 或 service 中，不能因为 render 被调用而发生。组件若完整拥有自己的私有状态，可保留组件旁的输入处理；目前例子包括 task panel、theme picker、global search。

真实聊天绘制路径是 `render_chat_lines` + `write_lines_to_buffer`；修改 UI 前先确认新文件确实被调用。聊天区域始终比终端少一列，因为最后一列给 scrollbar；不要用“是否可滚动”动态决定宽度。

## 快照测试

UI 外观测试使用 `src/ui/test_support/`：

```rust
use crate::ui::test_support::{assert_lines_render, assert_render, ChatState::test_fixture};

let state = ChatState::test_fixture();
assert_render!("name", 44, 5, |f| render_input(f, &state, f.area()));
let lines = render_todo_checklist(&tc, 52, &theme());
assert_lines_render!("name_lines", 52, 5, &lines);
```

- 必须用 `ChatState::test_fixture()`，不要用会读取本机 history/env 的 `ChatState::new()`。
- 本项目 snapshot 同时编码字符、颜色和 modifier；颜色变化也应产生 diff。
- 新 baseline 第一次故意失败并打印结果；确认内容后再运行一次。故意更新使用 `UPDATE_SNAPSHOTS=1 cargo test --lib`。
- 一个测试只创建一个 snapshot；依赖墙钟的 uptime、spinner 等不要做 snapshot，直接断言状态。

## 输入、steering 与 slash command

`src/ui/app/logic.rs::enqueue_user_message` 是输入总入口：

- `/` → `commands::handle_command`
- `!` → `ShellExecutionService`（Plan mode 拒绝）
- `@` → `at_processor`，内联文件并发送 `MarkFilesAsRead`
- `#` 是普通 Markdown 文本；memory 必须通过 `/memory add`

运行中的普通文本走 steering，不是排队：`steer_user_message` → `AgentRequest::SendMessage` → steering queue，在下一 provider turn 注入。命令和 shell 仍进入 `pending_user_messages`。steering 消息必须有自己的 `message_id`；`agent_run.rs` 用 `push_or_merge_user_message` 合并相邻 user turn，避免中断后留下连续 user messages。

steering 会把 assistant 输出切成两半，用户消息插在中间，因此切点必须是 markdown 安全边界。使用 `markdown_parser::split_markdown_at_safe_boundary`：段落边界、fence 外；若位于 fence 内，切到 opening line 之前。不要把列表、表格或 fenced code block 从中间切开。

新增或修改 slash command 必须触及四处：

1. `src/commands/system.rs` 的 `ALL_COMMANDS` 声明；category 必须属于 help 显示的类别。
2. 对应 topic module 的实现。
3. `src/commands/mod.rs` 的 `handle_command` match arm；注意重复 arm 和 `is_declared_pending` arm 的顺序。
4. 实现后移除 `Pending` category，否则仍会走占位响应。

命令签名是 `async fn name(ctx: CommandContext<'_>, args: …) -> CommandResult`。只读命令写 `ctx.state.chat_history`；需要模型时发送 `AgentRequest::SendMessage`；不应进入主上下文的侧输出使用 `GenerateNote`。

## Tools

`ToolRegistry` 在 `src/core/tools/tool_registry.rs` 中维护工具及缓存的 `Vec<FunctionDeclaration>`；LLM 看到的是缓存。工具 trait 在 `src/core/tools/tools.rs`：`BaseDeclarativeTool` 和 `ToolInvocation`。

注册主要位于 `src/core/config/runtime_bootstrap/`：`core_runtime.rs` 负责文件、搜索、导航类工具，`agent_runtime.rs` 负责需要 LLM 或重依赖的工具；插件通过 `sync_plugin_tools` 注入且不能与 built-in 重名。工具存在于 tree 不代表已接线，使用前 grep constructor 和 bootstrap call site。

内置工具名是 Claude Code 风格的 PascalCase：`Read`、`Write`、`Edit`、`MultiEdit`、`SmartEdit`、`LS`、`Grep`、`Glob`、`Bash`、`Agent`、`Skill`、`TodoWrite`、`AskUserQuestion`、`EnterPlanMode`、`ExitPlanMode`、`TaskGet`、`TaskList`、`TaskUpdate`、`TaskOutput`、`CronCreate`、`CronList`、`CronDelete`、`ToolSearch`、`SendMessage`、`WorkflowTool`、`Mcp*` 等。不要新增 snake_case 别名。

`ToolName` (`src/core/tools/constants.rs`) 是唯一真相源：`as_str()` 是注册名，`from_str()` 只接受准确拼写。只有以下两处允许大小写折叠：

- `find_builtin_tool_name`：供人输入工具名时只折叠大小写。
- `is_core_tool_enabled` 与 hook matcher：配置值比较时不区分大小写。

`tool_name_consistency` 测试用于防止 schema 名和注册名漂移。

## CodebaseSearch

`src/core/context/` 的检索是 lexical BM25-style，不是 embedding：tree-sitter AST chunks，co-occurrence lexical、exact lexical、literal ripgrep 三路通过 RRF fusion。CJK 使用 bigram；不要假设中文问题能召回英文 identifier。

必须保持以下约束：

- watcher 与 `CodebaseSearchTool` 共享 `search_cache::shared_search_cache()`，否则后台索引不会被工具复用。
- stopwords 不进入 index，否则高频词会压低真实匹配。
- `should_trigger_codebase_search_with_flag` 只判断 enabled/attempted/tool presence；query 形状由 `score_rank_locate_route` 决定。"why/explain" 是评分惩罚，不是 veto。
- 冷索引返回 `Warming` 时，agent 应等待索引后重试，而不是依赖模型自行重试。

## Prompts、配置与环境变量

系统 prompt 和 tool description 在 `src/core/prompts/system-prompts/` 的 `.md` 中，通过 `rust_embed` 内嵌。运行时覆盖顺序为：`STAR_PROMPT_DIR` → `~/.starcode/prompts/` → `./.star/prompts/` → embedded。tool description 的 frontmatter `description:` 进入 schema，文件 body 进入 system prompt；改模型看到的工具说明时编辑 `.md`，并检查 `tool_descriptions.rs` 的 registry-name mapping。

配置路径统一经过 `Storage`，不要手写 `.star` 路径。全局设置是 `~/.star/settings.json`，项目设置是 `./.star/settings.json`；项目上下文优先读取 `STAR.md`、`STARCODE.md`、`CLAUDE.md`，最多注入 8000 字符。

环境变量多在使用点直接读取，没有集中 registry；文档级清单与消费点由 `src/core/star_env_registry.rs` 的测试校验。常用变量包括 `STAR_API_KEY`、`STAR_BASE_URL`、`STAR_MODEL`、`STAR_CONTEXT_WINDOW`、`STAR_AUTO_COMPACT`、`STAR_TOOL_TIMEOUT_SECS`、`STAR_LOG_ENABLED`、`STAR_LOG_DIR`、`STAR_PROMPT_DIR`。

`~/.star/user-settings.json` 和 `.star/settings.json` 支持顶层 `env` block。`SettingsManager::apply_env_block` 必须在 main 早期、tokio worker 启动前调用；优先级为 project > user > shell，不能 unset，且不能覆盖 `PATH`、`HOME`、`PWD`。运行中不要 `std::env::set_var`；动态覆盖走 self-tuning 的 RwLock。

self-tuning 的优先级是 `override > env > default`，仅允许真正按使用点读取或每 turn 重建的 knobs。`STAR_SELFTUNING_LOCKED` 可锁定 key，`STAR_SELFTUNING_MAX_SETS`（默认 12）限制成功 set 次数；reset 不消耗额度。model 和 context window 不是 self-tunable，分别通过 `/model` 和 `/context` 修改；持久化 override 在 `~/.star/self-tuning.json`，不写入 settings。

## i18n

调用点只能使用单参 `i18n::t1("key")`；文案只维护在 `i18n/en-US.json` 与 `i18n/zh-CN.json`。不要恢复旧的三参形式。词典通过 `src/core/i18n_dict.rs` 内嵌，外部覆盖顺序为 embedded → `cwd/i18n/` → `~/.starcode/i18n/` → `.star/i18n/`。缺 key 时返回 key 本身。

修改文案后检查 `src/core/i18n.rs` 的四类测试：源码 key 必须存在于两份词典、词典不得有孤儿 key、两种语言 key 集合相同、换行必须是真换行而不是字面量 `\\n`。动态 key 必须登记到 `DYNAMIC_KEY_SOURCES`；插值使用词典模板的 `{name}` / `{count}` 等占位符。

## Permission / confirmation

工具确认链路是：`PolicyEngine::check` → `MessageBus` → worker/`runtime/confirmation_bridge.rs` → `StreamMessage::ToolConfirmationRequest` → `AgentRequest::ConfirmTool`。

`ApprovalMode` 为 `Default`、`AcceptEdits`、`Plan`、`BypassPermissions`、`Auto`。显式 `ask` rule 优先于 session/persisted grant；显式 `deny` 优先级最高，包括 bypass。`AskUser` 的默认 fallback 不等于匹配了显式 ask rule，只有 `CheckResult::explicit_rule` 能证明来源。

`ToolExecutor::pass_gate` 只有 `Gate::Open` / `Gate::Closed(reason)`，确认等待没有 timeout：`await_tool_decision` 只在用户决定、abort、bus close 或 `NoOneToAsk` 时结束。拒绝是 `Ok(confirmed == false)`，不是 error。watchdog 只能包住 `invocation.execute(...)`，不能包住用户确认等待或整个 `execute_batch`。

权限缓存必须先读再写：`requires_invocation_confirmation()` 不是阻止读取 grant 的 veto。项目 grant 必须落在最近的 `<project>/.star`，不能错误地落到 `~/.star`；使用 `paths::find_nearest_project_star_dir` / `Storage`。

编写会回答确认提示的测试时：在 spawn 前订阅 broadcast receiver；接收循环用 `while let Ok(msg)`，再用 `if let Message::ToolConfirmationRequest(...)` 过滤，不能用会被其他消息终止的 `while let Ok(Message::ToolConfirmationRequest(...))`。

## Model switch 与 agent internals

运行中的 `/model` 不阻塞当前 turn：新选择在下一次 provider request 生效，当前 provider request 不重启。共享 cell 在 `agent::model_switch`；`runtime/session.rs` 写入，`Agent::execute_turn` 在每次 provider request 前读取，并通过 `model_switch::resolve` 解析 provider。UI 在 acknowledgement 前显示 requested model 和 `⏳`；确认提示必须放在 always-visible status line，不能只放 processing spinner。

`StarAgent`（`src/agent/workflows/star_agent.rs`）是 wrapper；核心状态在 `Agent`（`src/agent/agent_core.rs`），包括 tool executor、compact manager、lazy context engine、session messages、token budget 和 stall detector。主循环是 `run_agentic_loop` → `execute_turn`；工具路由、重复检测、nudge 与 compaction 位于 `src/agent/`。

## 其他不可忽略的事实

- 不要因为模块“能编译、看起来合理”就认为已接线；先找 constructor 和 runtime bootstrap。analytics HTTP sink、Langfuse 等纸面能力尤其要先核实 live call site。
- `PolicyEngine::load_permission_rules`、`ModelFallbackManager` 是 live path，不要重新标记为 dead code；同样不要假设 `WebBrowserTool` 存在。
- `Cargo.lock` 已提交；修改 `Cargo.toml` 时同步更新 lockfile，CI/release 使用 `--locked`。
- README 的目录和 binary 名称有漂移；真实布局以仓库和 Cargo 配置为准，binary 是 `starcode-cli`。

## 编码约定

- 注释和 doc comment 使用中文；用户可见字符串使用英文。新文案统一走 `i18n::t1(key)`。
- 大型模块开头写 `///` header，说明架构、线程模型和错误处理。
- `src/lib.rs`、`src/main.rs` 有宽松的 dead-code/unused allow；判断代码是否有效要 grep call site，不能只看编译器。
- rustfmt `max_width = 100`；`clippy.toml` 的 too-many-lines threshold 为 100、too-many-arguments threshold 为 10。
