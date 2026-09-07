# 边角扫描 — 第三轮深度检查

> 生成时间: 2026-09-06
> 覆盖: 行为差异、工具对比、MCP、i18n、unwrap 风险、配置、测试覆盖、CCB 未分析特性

---

## 一、核心逻辑 TODO (非 bridge/ssh/voice/sdk)

之前文档覆盖了大型桩模块，这些是**核心路径中的零散 TODO**:

| 文件 | 行号 | TODO 内容 |
|------|------|-----------|
| `src/core/voice/enhanced.rs` | 149, 160, 165, 173 | Anthropic STT / Doubao ASR / Local Whisper / Anthropic TTS 全部 `Err("not yet implemented")` |
| `src/core/voice/stt.rs` | 115, 164 | 流式转录返回错误 |
| `src/commands/compat.rs` | 284 | `"Vim mode is not implemented yet in this TUI."` |
| `src/commands/mod.rs` | 395 | 多个 slash 命令返回 "declared but not implemented yet" |
| `src/core/context/selection.rs` | 134 | parse Cargo.toml for dependencies |
| `src/core/lsp/instance.rs` | 60 | 实际启动 LSP 服务器进程 |
| `src/core/permissions/manager.rs` | 63, 135 | Pass project ID |
| `src/core/secure_storage/keychain.rs` | 23 | 系统钥匙串存储 |
| `src/tools/lsp/mod.rs` | 241, 536 | 语言检测 / 递归打印 |
| `src/core/chrome/mod.rs` | 180 | "Action not yet implemented" |
| `src/core/computer_use/mod.rs` | 411 | "Action not yet implemented" |
| `src/core/remote/enhanced.rs` | 174 | "Command not yet implemented" |
| `src/core/workflow/mod.rs` | 340 | "Step completed (type not implemented)" |

---

## 二、Vim Mode

**位置**: `src/commands/compat.rs:284`

**状态**: 命令 `/vim` 存在但返回 "not implemented yet"

**CCB**: Claude Code 无内置 vim 模式 (依赖终端 vim binding)

**Feature flag**: `vim_mode` 在 `feature_flags.rs` 中标记为 `enabled, 100% rollout`，但实际实现为空 — flag 和实现不一致。

---

## 三、工具对比: StarCode vs Claude Code

### StarCode 注册工具 (55+)

**core_runtime.rs**: Edit, WriteFile, ReadFile, ReadManyFiles, GlobMatch, ListDir, Search(Grep), ExitPlanMode, AskUserQuestion, Wait, WebSearch, Lsp, NextEdit

**agent_runtime.rs**: SmartEdit, MultiEdit, NotebookEdit, NotebookRead, WebFetch, GitInsight, GhPrComments, Memory, Agent, TodoWrite, GetDiagnostics, EnterPlanMode, ExitPlanMode, EnterWorktree, ExitWorktree, ProjectMap, RunTests, SemanticSearch, Shell(Bash), BackgroundTask, CronCreate, CronList, CronDelete, RemoteTrigger, Snip, SuggestBackgroundPR, ScheduleWakeup, McpAuth, SendMessage, TaskGet, TaskList, TaskUpdate, TaskOutput, Monitor, Brief, Workflow, GitPrSubscribe, GitRewind, GitCommitAttribution, GitAutofixPr, McpListResources, McpReadResource, Skill, ToolSearch

### Claude Code 无对应工具 (StarCode 独有) — ✅ 全部已验证

| 工具 | 说明 | 状态 |
|------|------|------|
| BackgroundTaskTool | 后台任务管理 | ✅ 209 行, .star/completed_tasks |
| CronCreate/List/DeleteTool | 定时任务 | ✅ 387 行, execute ~147 行 |
| RemoteTriggerTool | 远程触发 | ✅ 239 行 |
| SnipTool | 代码片段 | ✅ src/core/tools/snip.rs |
| SuggestBackgroundPRTool | 后台 PR 建议 | ✅ 229 行 |
| ScheduleWakeupTool | 唤醒调度 | ✅ 181 行, execute ~79 行 |
| MonitorTool | 进程监控 | ✅ 206 行 |
| BriefTool | 简报 | ✅ 146 行, execute ~60 行 |
| WorkflowTool | 工作流 | ⚠️ 桩实现 (见 parity-orphaned) |
| GitPrSubscribeTool | PR 订阅 | ✅ 185 行 |
| GitRewindTool | Git 回退 | ✅ 202 行, execute ~111 行 |
| GitCommitAttributionTool | 提交归因 | ✅ 154 行, execute ~56 行 |
| GitAutofixPrTool | PR 自动修复 | ✅ 221 行, execute ~132 行 |
| McpListResourcesTool | MCP 资源列表 | ✅ execute ~68 行 |
| McpReadResourceTool | MCP 资源读取 | ✅ (同上) |
| SkillTool | 技能执行 | ✅ 342 行 |
| ToolSearchTool | 工具搜索 | ✅ 774 行 |
| SemanticSearchTool | 语义搜索 | ✅ 984 行 |
| ProjectMapTool | 项目地图 | ✅ 887 行 |
| GetDiagnosticsTool | 诊断信息 | ✅ 349 行 |
| NextEditTool | 下一步编辑 | ✅ 209 行 |
| SmartEditTool | 智能编辑 | ✅ 334 行, LLM fix 策略 |

### CCB 有但 StarCode 可能缺失的行为差异

| 特性 | CCB | StarCode | 差距 |
|------|-----|----------|------|
| 工具并行执行 | `partitionToolCalls()` 分离只读/写工具, 只读批量并行 (默认 10 并发) | `execute_batch` 已实现: 读写分组 + 只读并行 (chunked join_all) | ✅ 已实现 |
| Retry 逻辑 | 819 行 `withRetry.ts`: 按状态码策略、jitter、session 级持久、rate-limit header 解析 | 230 行 `retry.rs`: 状态码策略 + Retry-After 解析 + jitter + 单元测试 | ✅ 已增强 |
| Rate limit 消息 | 专用 `rateLimitMessages.ts` + `mockRateLimits.ts` | `error_kind.rs` 完整实现: 分类 + Retry-After 解析 + hint + RecoveryManager | ✅ 已实现 |
| Cost tracking | `cost-tracker.ts` + `costHook.ts` 专用 | `cost.rs` 完整实现: 8 厂商定价 + cache 分离 + per-response 计算 | ✅ 已实现 |
| Token estimation | 专用 `tokenEstimation.ts` 服务 | `token_counter.rs` 基础估算 (ASCII/多字节自适应, 89 行) | P2 — 功能够用, 精度可提升 |
| CLI transport | SSE / WebSocket / Hybrid / SerialBatch 四种 | MCP + Bridge 两种 | 覆盖差距 |
| Diagnostic tracking | 专用 `diagnosticTracking.ts` | 无对应 | 缺失 |
| VCR (请求录制/回放) | `vcr.ts` | 无对应 | 缺失 |
| Sleep prevention | `preventSleep.ts` | 无对应 | 缺失 |
| Personas / Modes | `src/modes/personas/` | `modes.rs` 无 personas | 部分缺失 |
| Buddy/companion | `src/buddy/` 宠物系统 | 无 | 纯装饰, 不影响功能 |

---

## 四、MCP 差距

**缺失**: Resource subscriptions (`resources/subscribe` / `notifications/resources/updated`)

StarCode 有 `McpListResourcesTool` 和 `McpReadResourceTool` 但无订阅机制。资源变更时无法被动通知。

**风险**: `src/core/mcp/oauth_utils.rs:49,53` — URL 解析 `.unwrap()` 可能在畸形 URL 时 panic。

---

## 五、系统提示文件覆盖

**总计**: 85 个 `.md` 文件

**直接引用**: 16 个 (system-prompt.md, planner-system.md 等)

**工具描述**: 69 个 `tool-description-*.md` — 通过 `tool_descriptions.rs` 的 key map 动态加载

**疑似孤立**: `system-prompt-planning.md` — 无直接 `load_prompt()` 调用, 可能通过 prompt bundle 间接加载

---

## 六、i18n 硬编码字符串

### 已修复 ✅

以下文件中的硬编码字符串已全部改为 `i18n::t()` 调用:
- `src/ui/app/logic.rs` — "状态：已读取" 等
- `src/ui/app/runtime.rs` — "正在加载配置和工具…"
- `src/ui/events/input.rs` — "已切换模型", "已选择", "已复制代码块", "已配置并切换到"
- `src/ui/events/clipboard_paste.rs` — "已粘贴图片", "已粘贴", "已粘贴块"
- `src/ui/components/confirmation_dialog.rs` — "未知工具"

### 仍为硬编码英文 (应走 `t()`)

| 文件 | 行号 | 字符串 | 状态 |
|------|------|--------|------|
| `src/ui/components/agent_progress.rs` | 77 | `"Done"` | ✅ 已修复 |
| `src/ui/components/agent_group_render.rs` | 276-277 | `"Done"`, `"Failed"` | ✅ 已使用 i18n |
| `src/ui/components/agent_task_render.rs` | 187, 190-191 | `"Failed"`, `"Done"` | ✅ 已使用 i18n |
| `src/ui/components/error_overlay.rs` | 107 | `"Error"` | ⚠️ 返回 `&'static str`, 无法用 `t()` |
| `src/ui/components/global_search.rs` | 173 | `"Error"` | ✅ 已使用 i18n |
| `src/ui/components/tool_render.rs` | 808 | `"Done"` | ✅ 已使用 i18n |

---

## 七、生产代码 unwrap/panic 风险

### 高风险: `lock().unwrap()` — 可毒化整个进程

| 文件 | 行号 | 锁对象 |
|------|------|--------|
| `src/agent/approval.rs` | 35, 41, 61 | approval mode lock |
| `src/agent/context.rs` | 327, 334, 657, 719 | context hash caches |
| `src/agent/workflows/star_agent.rs` | 700, 720, 746, 752 | approval mode lock |
| `src/core/auto_mode/mod.rs` | 138, 143, 148, 162, 168, 175 | auto mode state |
| `src/core/context/storage.rs` | 17, 22, 27, 32 | context cache |
| `src/core/monitor/mod.rs` | 130, 156, 174, 179, 184, 189, 190 | process monitor |
| `src/core/proactive/enhanced.rs` | 125, 131, 141, 200, 207, 213, 219 | proactive state |
| `src/core/remote/enhanced.rs` | 143, 152, 167, 181, 194 | remote connections |

**总计**: 40+ 个 `lock().unwrap()` 调用。任何一次 panic 会毒化 Mutex，后续所有访问都 panic。

### 中风险: 文件 I/O unwrap

| 文件 | 行号 | 说明 |
|------|------|------|
| `src/core/analytics/sink.rs` | 106, 112, 115 | `writeln!(...).unwrap()` — 磁盘满/权限问题会 panic |

### 中风险: CSS Selector unwrap

| 文件 | 行号 | 说明 |
|------|------|------|
| `src/core/tools/web_search.rs` | 192-567 | 14+ 个 `Selector::parse(...).unwrap()` — 畸形选择器字符串会 panic |

### 其他生产 unwrap

| 文件 | 行号 | 风险 | 状态 |
|------|------|------|------|
| `src/core/wiki.rs` | 107 | `pages.last().unwrap()` — 刚 push 过, 安全 | ✅ 无需修复 |
| `src/core/trigger_scheduler.rs` | 191 | `secs_until_hhmm().unwrap()` — 测试代码 | ✅ 测试代码 |
| `src/core/analytics/metrics.rs` | 181 | `partial_cmp().unwrap()` — NaN 比较 | ✅ 已修复 |
| `src/core/context/chunking.rs` | 83 | `separators.last().unwrap()` | ✅ 已修复 (let Some guard) |
| `src/core/context/types.rs` | 190 | `Duration::from_std().unwrap()` | ✅ 已修复 (unwrap_or 1h) |
| `src/core/config/migration.rs` | 40 | `config.as_object_mut().unwrap()` — 前置检查保证 | ✅ 无需修复 |
| `src/core/config/settings_manager.rs` | 574 | `api_key_opt.clone().unwrap()` | ✅ 已修复 (if let Some) |
| `src/agent/streaming_executor.rs` | 283 | `semaphore.acquire().await.unwrap()` | ✅ 已修复 (match + return) |
| `src/agent/loop_engineering.rs` | 515 | `recent_actions.back().unwrap()` — len>=5 保证 | ✅ 无需修复 |
| `src/core/context/structure_index.rs` | 197-406 | 多个 `Regex::new().unwrap()` | ✅ 已改为 expect() |
| `src/core/tools/edit.rs` | 136 | `Regex::new().unwrap()` | ✅ 已改为 expect() |
| `src/agent/token_budget.rs` | 253, 277 | `panic!("Expected ...")` — 测试代码 | ✅ 测试代码 |

---

## 八、未测试的关键模块

**610 个源文件** 无 `#[cfg(test)]` 模块。关键缺失:

### 核心 Agent 逻辑 (无测试)
- `agent_core.rs`, `agent_run.rs`, `approval.rs`, `session.rs`
- `planner.rs`, `streaming_executor.rs`, `worktree.rs`, `reflection.rs`, `router.rs`

### 核心工具 (无测试)
- `shell.rs`, `read_file.rs`, `write_file.rs`, `glob.rs`, `grep.rs`, `ls.rs`
- `agent_tool.rs`, `multi_edit.rs`, `notebook_edit.rs`, `notebook_read.rs`
- `project_map.rs`, `semantic_search.rs`, `workflow.rs`

### 配置/基础设施 (无测试)
- `settings_manager.rs`, `runtime_bootstrap.rs`, `provider_resolution.rs`
- MCP: `client.rs`, `manager.rs`, `transport.rs`
- Permissions: `manager.rs`, `evaluator.rs`

### LLM (无测试)
- `client.rs`, `message_pipeline.rs`, `openai_compatible.rs`
- Providers: `bedrock.rs`, `gemini.rs`, `grok.rs`, `vertex.rs`

---

## 九、配置字段消费验证 — ✅ 全部已接线

| 字段 | 位置 | 说明 | 状态 |
|------|------|------|------|
| `output_style` | `settings_manager.rs:188` | 注入 system prompt (concise/verbose 指令) | ✅ 已接线 |
| `context_window` | `settings_manager.rs:192` | 从 settings 传入 ConfigParameters → token budget | ✅ 已接线 |
| `thinking_effort` | `settings_manager.rs:183` | 通过 set_session_effort() 传播到 LLM 请求 | ✅ 已接线 |

---

## 十、CCB 有但未分析的特性

| 特性 | CCB 位置 | StarCode 状态 | 优先级 |
|------|----------|--------------|--------|
| **工具并行执行** | `toolOrchestration.ts` partitionToolCalls | `execute_batch` 已实现: 读写分组 + 只读并行 | ✅ P1 已完成 |
| **Retry 策略** | `withRetry.ts` 819 行 | 300+ 行: 状态码策略 + Retry-After + 5 tests | ✅ P1 已完成 |
| **Cost tracking** | `cost-tracker.ts` + hook | 8厂商定价 + cache 分离 | ✅ P2 已实现 |
| **Token estimation** | 专用服务 | 89 行基础估算 (ASCII/多字节自适应) | P2 — 够用 |
| **Rate limit 消息** | 专用服务 + mock | LlmErrorKind + diagnose + RecoveryManager | ✅ P2 已实现 |
| **Diagnostic tracking** | `diagnosticTracking.ts` | 无 | P3 |
| **VCR (录制/回放)** | `vcr.ts` | 无 | P3 |
| **Sleep prevention** | `preventSleep.ts` | 无 | P3 |
| **CLI transport** | SSE/WS/Hybrid/Serial | MCP+Bridge 两种 | P3 |
| **Personas** | `src/modes/personas/` | 仅 modes.rs | P3 |

---

## 更新建议 (新增到优先级列表)

### P0 — 安全 + 稳定
- **`lock().unwrap()` 治理** ✅ — 40+ 处已全部改为 `unwrap_or_else(|e| e.into_inner())`
- **web_search `Selector::unwrap()`** ✅ — 14 处已改为 `expect("valid CSS selector: ...")`
- **Vim mode flag 与实现不一致** ✅ — `/vim` 命令已实现 toggle 功能

### P1 — 健壮性 + 性能
- **工具并行执行** ✅ — `tool_executor::execute_batch` 已实现: 读写分组 + 只读并行 (chunked join_all, 可配并发数)
- **Retry 逻辑增强** ✅ — 已从 152 行增强到 230 行: 状态码策略 (429/5xx 分类) + Retry-After header 解析 + jitter + 5 个单元测试
- **Config 字段消费验证** ✅ — `context_window` 已从 settings 读取; `output_style` 已注入 system prompt; `thinking_effort` 已正确接线

### P2 — 国际化 + 覆盖
- **i18n 硬编码修复** ✅ — 大部分已修复 (剩余 `&'static str` 返回类型受架构限制)
- **测试覆盖** — 核心工具 (shell/read/write/glob/grep) 无测试

### P3 — 可选增强
- Diagnostic tracking / VCR / Sleep prevention / Personas
