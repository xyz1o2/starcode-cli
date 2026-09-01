# STAR.md

This file provides guidance to StarCode CLI when working with code in this repository.

## Project Overview

StarCode CLI is a Rust-based conversational AI coding assistant that provides an interactive terminal UI for software engineering tasks. It supports 20+ LLM providers and includes features like file operations, semantic code search, LSP integration, MCP server support, and AI-assisted git workflows.

## Build and Development Commands

### Building
```bash
# Standard build
cargo build

# Release build
cargo build --release

# Install locally from source
cargo install --path .
# OR use the install script
./install.sh
```

### Running
```bash
# Interactive mode (default)
cargo run

# Headless mode (single prompt, outputs JSON)
echo "Your prompt" | cargo run -- --prompt

# With specific directory
cargo run -- -d /path/to/project

# Subcommands
cargo run -- git commit    # AI-assisted git commit
cargo run -- git pr        # Create PR with AI assistance
cargo run -- mcp list      # List MCP servers
cargo run -- init          # Generate CLAUDE.md for current project
```

### Testing
```bash
# Run all tests
cargo test

# Run specific test
cargo test test_semantic_search_integration

# Run with output visible
cargo test -- --nocapture

# Run specific test file pattern
cargo test --test ace_integration_test
```

### Environment Setup
Copy `.env.example` to `.env` and configure:
- `STAR_API_KEY` - API key for the LLM provider
- `STAR_BASE_URL` - Base URL for the provider API
- `STAR_MODEL` - Model name (e.g., `gpt-4o`, `deepseek-chat`)
- `STAR_OPENAI_COMPATIBLE` - Set to `true` for OpenAI-compatible endpoints

## High-Level Architecture

### Core Modules

**Agent System** (`src/agent/`)
- `mod.rs` - Central orchestration logic for AI interactions (212KB, core of the system)
- `tool_executor.rs` - Tool execution pipeline with validation
- `message_classifier.rs` - Classifies and routes incoming messages
- `prompt_builder.rs` - Constructs prompts with context
- `summarizer.rs` - Context compression and summarization
- `validator.rs` - Response validation
- `workflows/` - Workflow definitions including context compression
- `skills/` - SubAgent system for specialized tasks
- `messaging/` - Async message handling system

**Tool System** (`src/tools/`)
- `mod.rs` - 20+ declarative tools for file operations, bash execution, search (97KB)
- `editor.rs` - Multi-file editing capabilities
- `lsp.rs` - Language Server Protocol integration
- `git_insight.rs` - Git repository analysis
- `memory.rs` - Persistent memory storage
- `github_pr_comments.rs` - PR comment handling
- `mcp_tool.rs` - MCP (Model Context Protocol) tool integration

**LLM Providers** (`src/llm/`)
- `mod.rs` - Provider trait definitions and factory
- `client.rs` - `StarClient` unified interface
- Individual provider implementations: `openai.rs`, `deepseek.rs`, `anthropic.rs`, `doubao.rs`, `moonshot.rs`, `zhipu.rs`, `siliconflow.rs`, etc.
- `openai_compatible.rs` - Generic OpenAI-compatible provider for custom endpoints

**UI System** (`src/ui/`)
- `app/runtime.rs` - Main UI event loop (~30 FPS target with Ratatui)
- `components/` - UI widgets: chat, command palette (Ctrl+P), dialogs, status line
- `services/worker.rs` - Async agent communication
- `state/` - UI state management
- Uses `ratatui` with `crossterm` for terminal rendering

**Configuration** (`src/core/config/`)
- `config.rs` - Main configuration structure
- `provider_store.rs` - LLM provider credential management
- `settings_manager.rs` - User settings persistence
- `providers.rs` - Provider metadata registry

**Core Services** (`src/core/`)
- `tools/` - Core tool trait definitions
- `tasks/` - Task management system (stored in `.star/tasks.json`)
- `confirmation_bus/` - Message bus for user confirmations
- `routing/` - Request routing engine

### Key Data Flow

1. **User Input** → UI event loop (`ui/app/runtime.rs`) → Worker service
2. **Agent Processing** (`agent/mod.rs`) → Message classification → Prompt building
3. **LLM Call** (`llm/client.rs`) → Provider-specific implementation → Streaming response
4. **Tool Execution** (`agent/tool_executor.rs`) → Tool validation → Execution → Result formatting
5. **Response Streaming** → UI components → Terminal output

### Concurrency Model

- **Async Runtime**: Tokio with full features
- **UI Thread**: Main thread handles terminal UI (~33ms frame time target)
- **Worker Thread**: Separate async task for agent communication via channels
- **Watchdog**: Background task monitoring loop health (alerts after 5s inactivity)
- **Git Status**: Background refresh every 5 seconds

### Configuration Hierarchy

Configuration resolution order (highest to lowest priority):
1. CLI arguments (`--api-key`, `--base-url`, `--model`)
2. Environment variables (`STAR_API_KEY`, `STAR_BASE_URL`, `STAR_MODEL`)
3. Provider store (`~/.star/provider-store.json`)
4. User settings (`~/.star/user-settings.json`)

### Important File Locations

- User config: `~/.star/`
- Tasks: `.star/tasks.json` (per-project)
- Logs: `.star/logs/` (when `STAR_LOG_ENABLED=true`)
- System prompts: `src/agent/prompts/system-prompts/`

### Code Style Notes

- Chinese comments and strings are used throughout the codebase
- The project follows Rust 2021 edition
- Heavy use of `async-trait` for LLM provider abstraction
- Error types use custom `LlmError` enum with provider-specific variants
- Windows-specific handling for GBK/GB18030 encoding in bash tool (`src/tools/mod.rs`)

## 智能体任务效率系统分析（修订版——5项优化后）

> 本分析基于以下已实施的优化：
> 1. 双模语义路由（长度快速路径 + LLM可选升级）
> 2. 优先级驱动的自动触发调度
> 3. SubAgent 共享上下文缓存
> 4. 基于文件路径的任务依赖自动推断
> 5. 结构化团队轮次记忆（JSON持久化）

### 一、复杂度分级机制（双模设计）

`src/agent/router.rs` 采用两层分级架构，兼顾速度与精度：

**第一层 — 同步快速路径** (`Router::classify()`)：
| 级别 | 判定条件（纯长度/历史，零模型开销） |
|---|---|
| **Simple** | 输入 ≤200字符 且 历史 <4轮 |
| **Medium** | 201-800字符 或 历史 4-7轮 |
| **Complex** | >800字符 或 历史 ≥8轮 |

**第二层 — LLM语义升级** (`Router::classify_with_semantic_upgrade()`)：
- 仅对长度判为 Simple 的请求触发（Medium/Complex 已足够准确）
- 环境变量 `STAR_SEMANTIC_ROUTING=true` 启用，默认关闭 —— 用户主动选择精度优先
- 超短输入（<10字符）直接跳过，避免无意义调用
- 极简分类提示词："Classify this coding task. Reply with one word: Simple, Medium, or Complex."
- 2秒超时（`STAR_SEMANTIC_ROUTING_TIMEOUT_SECS`，clamp至 1-5秒），超时回退到长度判定
- 不包含任何硬编码关键词或语言依赖 —— 语义判断完全交给模型

这解决了原先的"50字'加OAuth2登录'被判为Simple"的问题 —— 当用户启用语义路由后，模型能将短但高复杂度的输入升级为 Medium/Complex。

**策略提示词注入** (`agent_run.rs:41-72`)，三级不同行为指引：

| 级别 | 策略链 | 预期轮次 |
|---|---|---|
| Simple | Search → Read → Edit → Verify（跳过影响分析） | 1-3轮 |
| Medium | Search → Read → Impact-Check → Edit → Verify（multi_edit优先） | 3-10轮 |
| Complex | Plan → Search → Understand → Impact-Analyze → Phase-Edit → Verify（可选auto_plan） | 10-200轮 |

### 二、小工程流程（Simple分级）

关键路径与效率优化点：

1. **首轮静态上下文** — history==0 且未启用 `STAR_DYNAMIC_CONTEXT_FIRST_TURN` 时，仅加载静态上下文，跳过昂贵的 ContextEngine 语义索引。实测节省 200-500ms 首轮延迟。
2. **工具短名单** — 首轮展示精简工具集，降低LLM选择开销。
3. **零规划开销** — `maybe_generate_auto_plan()` 对 Simple 直接返回 `AutoPlanDecision::Skipped`。
4. **首轮预取** (`handle_prefetch`) — 检测到 overview/概念类请求时预取 `project_map` 或 `semantic_search`，消除第二轮的往返延迟。
5. **快速路径策略** — 提示词不要求影响分析、不提及 plan mode，让模型直奔编辑。

效率特征：**低延迟、低轮次、低token消耗**。适合 typo修复、单行改动、简单查询。

### 三、中工程流程（Medium分级）

核心效率机制：

1. **全量动态上下文** — `ContextEngine.load_context_for_project()` 加载项目记忆、规则文件、语义索引，提供 LLM 充足的工程背景。
2. **影响检查策略** — 策略要求"改签名前搜索所有调用点、一起修改"，防止Medium任务常见的遗漏关联修改。
3. **自动触发：优先级调度** — `tool_routing/triggers.rs` 的 `select_best_auto_trigger()` 收集所有符合条件的触发器，按优先级（Verification=10→SemanticSearch=5→JsonFallback=4→Navigator=2→ProjectMap=2→Analyzer=1→Editor=1）排序后取最优。替代了原先的串行 if-else 链，从最坏7次无效轮次降至**每轮至多1次有效触发**。
4. **语义搜索相关性评分** — `score_semantic_search_relevance()` 对 "how/why/explain/架构/design" 类查询加权+3，对 "find/search/定位" 加权+1，使得语义搜索触发更精确。
5. **上下文压缩多策略** — CompactManager 在 92% token阈值触发，含 auto_compact/reactive_compact/micro_compact。
6. **工具循环检测 + 只读轮次限制** — 防止死循环和"只搜不改"的拖延。

效率特征：**平衡的轮次-质量权衡**，适合跨文件改动、接口修改、中等重构。

### 四、大工程流程（Complex分级）

#### 4.1 自动规划

- `STAR_AUTO_PLAN=true` 启用，Complex + 历史≤6轮触发
- 独立 Planner LLM 调用，4秒超时，6000字符截断
- 计划以 `[AUTO_PLAN]...[END_AUTO_PLAN]` 注入系统消息

#### 4.2 任务分解与自动依赖推断

- `TaskManager.get_execution_plan()` — Kahn算法拓扑排序，返回分层并行执行计划
- `add_task_with_auto_deps()` — 新任务添加时自动推断与已有任务的依赖关系
- **依赖推断规则** (`infer_dependencies()`)：
  - 纯基于文件路径重叠（正则提取 `src/foo/bar.rs` 等模式）
  - 同父任务约束（仅同一 parent_id 下的子任务间推断，避免跨模块误关联）
  - 零关键词依赖、零语言假设 —— 适用于任何编程语言的工程
- 环检测 (`detect_cycles()`) 防止依赖死锁
- 持久化在 `.star/tasks.json`，去重添加（`add_task_dedup`，标题归一化）

#### 4.3 SubAgent系统（含共享上下文）

5个专职代理：

| 代理 | 职责 | 匹配权重 |
|---|---|---|
| SearchAgent | 代码检索与上下文召回 | 关键词≥1 → 700，can_handle → 100 |
| AnalyzerAgent | 结构分析与问题研判 | 同上 |
| EditorAgent | 代码改动与重构 | 同上 |
| NavigatorAgent | 递归追踪依赖与调用链 | 同上 |
| AutoFixAgent | 测试失败分析与自动修复循环 | 同上 |

**共享上下文缓存（新增优化）**：
- `SubAgentManager.set_shared_context(context, project_root)` — 父级 Agent 在执行子任务前注入已加载的项目上下文
- `enrich_task_with_context()` — 在每个子任务 params 中注入 `_shared_context`（截断至3000字符）和 `_project_root`
- 每个子代理不再独立执行昂贵的 ContextEngine 索引，5路并行场景下**省去4次重复加载**（~2-8秒）
- 集成点：`task_executor.rs:320`（批量执行循环）和 `skills/mod.rs:293`（`execute_parallel`）

#### 4.4 团队执行（含结构化轮次记忆）

`/agents team run` — 5成员并行/管道模式，git-worktree隔离。

**结构化团队上下文（新增优化）**：
- `StructuredTeamContext` (`team_execution.rs:124-204`)：
  ```json
  {
    "files_changed": {"src/foo.rs": ["editor", "analyzer"], ...},
    "round_summaries": [{"round": 1, "total_outcomes": 5, "success_count": 4, ...}],
    "unresolved_errors": ["compilation error in src/bar.rs:42..."]
  }
  ```
- `update_from_round()` — 每轮结束后从摘要中提取文件路径、去重错误（保留最近10条）、保留最近8轮摘要
- `render()` — 输出 JSON 包裹在 `[STRUCTURED_TEAM_CONTEXT]...[END_STRUCTURED_TEAM_CONTEXT]` 中注入下轮目标
- `build_round_objective()` — 接受 `Option<&StructuredTeamContext>`，高轮次下替代纯文本内存
- 解决了原先"纯文本传递 → 高轮次关键信息丢失"的问题

#### 4.5 循环工程

```
LoopState { max_turns: 200, max_consecutive_failures: 8, budget: {50 calls/turn, $50/turn} }
策略链: Normal → RetryWithDifferentArgs → FallbackToSimplerTool → SkipAndContinue → BreakAndReport
```

- `AttemptHistory` 记录最近30次尝试；`StructuredError` 自动解析编译/测试/运行时错误
- 失败时注入 `[LOOP_CONTEXT]` 系统消息

### 五、跨级别通用效率机制

| 机制 | 文件 | 效率收益 |
|---|---|---|
| 上下文压缩 | `src/agent/compact/` | 92%阈值触发，多策略防token溢出 |
| 工具预检 | `src/agent/tool_preflight.rs` | 执行前校验参数，减少无效调用 |
| 钩子系统 | `src/core/hooks/` | 4阶段钩子注入 |
| 检查点恢复 | `src/agent/checkpoint.rs` | 会话持久化，中断可继续 |
| 审批模式 | `src/agent/approval.rs` | Plan/Auto模式动态切换 |
| 工具路由 | `src/agent/tool_routing/helpers.rs` | 每轮动态Top-K工具选择 |
| 首轮预取 | `src/agent/agent_loop.rs:handle_prefetch` | project_map/semantic_search预热 |

### 六、优化后架构评估

**相比优化前，已解决的局限：**

| 局限（优化前） | 解决方案 | 状态 |
|---|---|---|
| 复杂度纯长度判断，短输入语义缺失 | 双模路由：`STAR_SEMANTIC_ROUTING=true` 启用LLM语义升级，极简提示词，2秒超时回退 | ✅ 已解决 |
| SubAgent各自独立加载上下文，重复开销 | `SubAgentManager` 共享上下文缓存 + `enrich_task_with_context()` 注入 | ✅ 已解决 |
| 自动触发串行尝试，最坏7次无效轮次 | 优先级驱动 `select_best_auto_trigger()` 单次最优选择 + 相关性评分 | ✅ 已解决 |
| 任务依赖仅为声明式，无自动推断 | `infer_dependencies()` 纯路径重叠推断 + 同父约束 | ✅ 已解决 |
| 团队轮次间纯文本传递，信息丢失 | `StructuredTeamContext` JSON持久化 + files_changed/errors去重 | ✅ 已解决 |

**剩余改进空间：**

- **Router语义升级默认关闭** — 需用户主动设 `STAR_SEMANTIC_ROUTING=true`。可考虑在检测到特定模式时自动启用（如输入虽短但包含 "OAuth/JWT/refactor/migrate" 等概念密度高的词汇时提示开启）
- **SubAgent共享上下文是静态缓存** — 子代理修改文件后缓存不会更新。可考虑在 `enrich_task_with_context` 中注入前一子代理的产出摘要
- **依赖推断仅路径级别** — 不感知语义依赖（如"A模块的类型定义被B模块引用"的跨文件引用关系）。可考虑集成LSP/LSIF的符号引用图
- **团队执行的结构化上下文** — 当前使用JSON文本注入，可考虑在 SubAgent 内部直接解析并用于工具调用决策（如跳过已修改文件）

### 七、关键配置环境变量

| 变量 | 默认值 | 作用 |
|---|---|---|
| `STAR_SEMANTIC_ROUTING` | false | 启用LLM语义复杂度升级（2秒超时） |
| `STAR_SEMANTIC_ROUTING_TIMEOUT_SECS` | 2 (clamp 1-5) | 语义路由超时秒数 |
| `STAR_AUTO_PLAN` | false | 启用大工程自动规划 |
| `STAR_AUTO_PLAN_MAX_HISTORY` | 6 | 自动规划的历史轮次上限 |
| `STAR_AUTO_PLAN_MAX_CHARS` | 6000 | 自动规划最大字符数 |
| `STAR_AUTO_PLAN_TIMEOUT_SECS` | 4 | 自动规划超时秒数 |
| `STAR_TASK_MAX_PARALLELISM` | 8 | 子任务最大并行度 |
| `STAR_TASK_MAX_STEPS` | 20 | 单子任务最大步数 |
| `STAR_ENABLE_FIRST_TURN_PREFETCH` | false | 首轮预取 project_map/semantic_search |
| `STAR_ENABLE_AUTO_SEMANTIC_SEARCH` | false | 自动语义搜索触发 |
| `STAR_ENABLE_AUTO_SKILL_FALLBACKS` | false | 自动技能回退链 |
| `STARCODE_JSON_FALLBACK` | false | JSON工具调用回退 |
| `STAR_DYNAMIC_CONTEXT_FIRST_TURN` | false | 首轮即加载动态上下文 |
| `STAR_THINKING_EFFORT` | (none) | Thinking/reasoning努力级别 (none/low/medium/high)，支持多种提供商 |

### 八、数据流全景（按工程规模）

```
用户输入
  │
  ├─ Router::classify_with_semantic_upgrade() ──── 双模复杂度分级
  │    ├─ 同步快速路径（长度/历史判定）── 始终执行
  │    └─ LLM语义升级（STAR_SEMANTIC_ROUTING=true时）── 仅对Simple升级
  │
  ├─ [Simple] ──→ 静态上下文 ──→ 首轮预取 ──→ 直接执行 ──→ 完成
  │                 (1-3轮, 14工具短名单)
  │
  ├─ [Medium] ──→ 动态上下文 ──→ 策略注入 ──→ 多轮执行 ──→ 优先级触发器 ──→ 完成
  │                 (3-10轮)        影响检查     select_best   单次最优选择
  │
  └─ [Complex] ──→ [auto_plan] ──→ 任务分解 ──→ SubAgent并行 ──→ 合成 ──→ 完成
                      (opt-in)     add_task_    (共享上下文缓存)  synthesize
                                   with_auto_deps
                                   (路径推断依赖)
```
