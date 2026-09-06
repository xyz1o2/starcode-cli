# StarCode vs Claude Code — 对标分析总览

> 生成时间: 2026-09-06
> 基于仓库 HEAD (`80c5eb3`) + `study_or_copy_projects/claude-code-main` 参考

本文档汇总所有子系统的对标状态，按模块分目录索引。
每个子系统的详细分析见对应 `docs/` 文件。

---

## 目录

| 子系统 | 详细文档 | 缺失项文档 | 完成度 | 说明 |
|--------|---------|-----------|--------|------|
| [Explore UI](#1-explore-ui) | `ui-explore-ccb.md` | — | ✅ 100% | GlobalSearch / QuickOpen / HistorySearch |
| [Tool 渲染](#2-tool-渲染) | `ui-tool-render.md` | [`parity-tool-render-gaps.md`](parity-tool-render-gaps.md) | 🟡 60% | 4 项格式优化 |
| [Task UI](#3-task-ui) | `ui-benchmark-ccb.md` §II | [`parity-task.md`](parity-task.md) | 🟡 40% | 8 项缺失 |
| [Permission UI](#4-permission-ui) | `ui-benchmark-ccb.md` §III | [`parity-permission.md`](parity-permission.md) | 🟡 30% | 8 项缺失 |
| [Agent Progress UI](#5-agent-progress-ui) | `ui-benchmark-ccb.md` §IV | [`parity-progress.md`](parity-progress.md) | 🟡 50% | 4 项缺失 |
| [Slash Commands](#6-slash-commands) | [`parity-commands.md`](parity-commands.md) | — | ✅ 95% | 全部 Pending 已实现 |
| [Agent 内部机制](#7-agent-内部机制) | [`parity-agent.md`](parity-agent.md) | — | 🟡 70% | 核心已对标, 部分 TODO |
| [UI 框架架构](#8-ui-框架架构) | `ui-benchmark-ccb.md` §V | [`parity-ui-framework.md`](parity-ui-framework.md) | 🔴 20% | 7 项架构重构 |
| [孤立模块 / 未接线](#9-孤立模块--未接线) | — | [`parity-orphaned.md`](parity-orphaned.md) | 🔴 0% | 11 个模块未接线 |

---

## 1. Explore UI

**文档**: [`ui-explore-ccb.md`](ui-explore-ccb.md)

**状态**: ✅ 全部 24 项已完成 (Phase 1–8)

已实现：
- FuzzyPicker 共享抽象 (`fuzzy_picker.rs`)
- 响应式布局 (140/120/100 列阈值自动切右/下预览)
- readline 键绑定、Tab/Shift+Tab 操作
- Ripgrep 搜索 + 代际计数器
- 预览 / 高亮 / 滚动指示器 / 路径截断
- Overlay 协调 (Modal Stack 集成)

**剩余微优化** (不影响功能):
- `direction='up'` atuin 风格 (搜索框在底部) — 当前为标准垂直布局
- Stream ripgrep (增量结果) vs 当前 one-shot ripgrep — 可优化

---

## 2. Tool 渲染

**文档**: [`ui-tool-render.md`](ui-tool-render.md)

**状态**: 🟡 已完成基础，剩余 4 项格式优化

### 已完成 (commit `c2966d1`)
- ToolResult 整体缩进 (`  ⎿ ` 前缀)
- ANSI 颜色保留 (256色 / truecolor / dim/italic/reverse/crossed-out)
- Span 级宽度计算 + CJK 感知
- `build_tool_body_block` ANSI 感知渲染
- 折叠预览颜色保留
- Edit diff 折叠摘要 ("Added N lines, removed M lines")
- 7 个单元测试

### 未完成 (ui-tool-render.md §III)

| # | 内容 | 当前状态 | 目标 |
|---|------|---------|------|
| 3.1 | ToolCall header 格式 | `● bash <gray command>` 60字符截断 | `● ToolName(args)` 终端宽度截断 |
| 3.2 | View/搜索工具摘要 | 8行预览 | 单行摘要 ("Read N lines", "Found N matches") + "Tab to expand" |
| 3.3 | Bash 空结果 / 后台任务 | 原样输出 | 空→"Done"; 后台→"Running in the background" |
| 3.4 | Write 结果摘要 | 原样输出 | "Wrote N lines to {path}" |

---

## 3. Task UI

**文档**: [`ui-benchmark-ccb.md`](ui-benchmark-ccb.md) §II

**状态**: 🟡 基础面板已实现，V2 特性缺失

### 已实现
- `task_panel.rs` (858行): `TaskPanel` 结构体
- TodoWrite 工具渲染: CC 风格 checklist 图标
- 任务列表显示

### 未实现 (vs Claude Code Task V2)

| 特性 | CCB | StarCode | 优先级 |
|------|-----|----------|--------|
| 文件持久化 (JSON per task) | ✅ | ❌ 内存 only | P1 |
| 文件锁 + 多 agent 协作 | ✅ (proper-lockfile) | ❌ | P2 |
| 依赖关系 (blocks/blockedBy) | ✅ | ❌ | P0 |
| Owner / Teammate 跟踪 | ✅ | ❌ | P2 |
| 30s completed TTL 自动清除 | ✅ | ❌ | P1 |
| Spinner inline 集成 | ✅ | ❓ 待验证 | — |
| Ctrl+T 三态循环 (tasks/teammates/none) | ✅ | ❓ 待验证 | — |
| Background tasks 对话框 | ✅ | ❓ 待验证 | — |
| 底部状态 pills | ✅ | ❌ | P2 |

---

## 4. Permission UI

**文档**: [`ui-benchmark-ccb.md`](ui-benchmark-ccb.md) §III

**状态**: 🟡 统一对话框已实现，CC 的特化组件缺失

### 已实现
- `confirmation_dialog.rs` (1,295行)
- `RiskLevel` 枚举 + 颜色常量
- 统一确认对话框

### 未实现

| 特性 | CCB | StarCode | 优先级 |
|------|-----|----------|--------|
| 12+ 特化组件 (per-tool dispatched) | ✅ | ❌ 统一对话框 | P1 |
| Feedback input (Tab 切换模式) | ✅ | ❌ | P0 |
| AI 风险解释 (Ctrl+E, lazy-load) | ✅ | ❌ | P2 |
| Shift+Tab 快速批准 | ✅ | ❌ | P1 |
| 分类器自动审批 | ✅ | ❌ | P2 |
| WorkerBadge (@agentName) | ✅ | ❌ | P2 |
| TrustDialog (首次信任扫描) | ✅ | ❌ | P2 |
| 底部状态栏模式指示器 | ✅ | ❌ | P1 |

---

## 5. Agent Progress UI

**文档**: [`ui-benchmark-ccb.md`](ui-benchmark-ccb.md) §IV

**状态**: 🟡 基础进度已实现

### 已实现
- activeForm 显示 (待验证)
- TaskList inline (待验证)

### 未实现

| 特性 | CCB | StarCode | 优先级 |
|------|-----|----------|--------|
| Next task hint | ✅ | ❌ | P1 |
| 完整的 inline task 进度 | ✅ | ❓ 待验证 | — |

---

## 6. Slash Commands

**文档**: [`parity-commands.md`](parity-commands.md)

**状态**: ✅ 所有 Pending 命令已实现

`src/commands/parity.rs` (2,400+ 行) 实现了全部原先标记为 `category: "Pending"` 的命令。`system.rs` 中有测试断言没有命令仍为 Pending。

### 已实现的命令 (30+)
`/env` `/release-notes` `/history` `/mode` `/output-style` `/tag`
`/keybindings` `/reload-plugins` `/statusline` `/poor` `/proactive`
`/advisor` `/autonomy` `/tui` `/network` `/attach` `/insights`
`/heapdump` `/debug-tool-call` `/issue` `/subscribe-pr`
`/install-github-app` `/install-slack-app` `/peers` `/send`
`/claim-main` `/bridge-kick` `/remote-control` `/mobile` `/desktop`
`/goal` `/job` `/monitor` `/daemon` `/coordinator`

---

## 7. Agent 内部机制

**文档**: [`parity-agent.md`](parity-agent.md)

**状态**: 🟡 大部分核心机制已对标实现

### 已实现
- **Compact 策略** (7 模块): grouping, cached micro-compact, compact warning hook, reactive compact, post-compact cleanup, time-based config, session memory compact
- **Streaming executor**: 工具编排, 流式工具执行
- **Enhanced executor** (10+ 模块): withheld mechanism, post-sampling hooks, bash classifier, simulated sed edit stripping, tool span management, tool attributes, input sanitization
- **Command queue**: queue command system, attachment messages, tool refresh, periodic task summaries
- **MCP permissions**: server type, server management, decision reason mapping, tool error classification
- **Message processing**: message constants, message factory, system reminder sibling compression
- **Query loop**: 8 continue conditions, 11 stop conditions
- **Coordinator**: tool filter / coordinator mode / prompt 状态机
- **Subagent**: fork subagent isolation

### 待完善
- 部分模块有 TODO 标记但未实现 (见 §9)
- 某些对标的行为未经过集成测试验证

---

## 8. UI 框架架构

**文档**: [`ui-benchmark-ccb.md`](ui-benchmark-ccb.md) §V

**状态**: 🔴 架构债务较大，依赖版本无需升级

### 无需升级
- ratatui 0.30, crossterm 0.28, tokio 1.0 — 均为当前版本

### 需要重构 (按优先级)

| 优先级 | 项目 | 当前状态 | 目标 |
|--------|------|---------|------|
| P0 | 统一 Modal Stack | 10+ `show_*` 布尔值, 仅 4 个在 modal_stack | 全部迁移 |
| P0 | Component trait | 无统一 trait | `Component { render, handle_event, update }` |
| P1 | ChatState 拆分 | 330+ 字段 mega-struct | 按职责拆分 |
| P1 | Action/Event enum | 3,213行 `input.rs` raw KeyEvent 匹配 | 枚举分发 |
| P1 | Render/State 分离 | `render_page()` 修改 state (scroll, total_rendered_lines) | 只读渲染 |
| P2 | 脏标记渲染 | 30fps 全量重绘 | Per-component dirty flags |
| P2 | 业务逻辑抽取 | `tool_render.rs` 混合 JSON 解析 / 路径格式化 / Span 构建 | 分层 |

---

## 9. 孤立模块 / 未接线

**这些子系统编译通过、代码可读，但从未被构造或调用。**

### 已确认孤立 (CLAUDE.md)

| 模块 | 位置 | 问题 |
|------|------|------|
| `ModelFallbackManager` | `src/agent/model_fallback.rs` | 仅在 `#[cfg(test)]` 中构造; `STAR_MODEL_FALLBACK_*` 被忽略 |
| `PolicyEngine::load_permission_rules` | `policy_engine.rs:52` | 零调用者; `.star/permissions.json` 规则仅为建议 |
| `WebBrowserTool` | — | 实现了 `BaseDeclarativeTool` 但 `new` 从未被调用 |
| Analytics HTTP sink | — | 日志 `"[Analytics HTTP] Would send ..."` 而非实际发送 |

### 存在 TODO 的骨架模块

| 模块 | 位置 | 未实现内容 |
|------|------|-----------|
| **Bridge** | `src/core/bridge/` | command/query 处理, 重连逻辑, WebSocket 发送/广播, Pong 响应, Web UI server |
| **SSH** | `src/core/ssh/` | 文件/目录部署 (`deploy.rs`), 认证方法 (`auth.rs`), 连接/执行逻辑 (`session.rs`) |
| **Voice** | `src/core/voice/` | Anthropic STT API, 流式转录, Doubao STT API (`stt.rs`), 实际音频捕获 (`capture.rs`) |
| **Secure Storage** | `src/core/secure_storage/keychain.rs` | 系统钥匙链存储实现 |
| **LSP** | `src/core/lsp/instance.rs` | 实际启动 LSP server 进程 |
| **Context** | `src/core/context/selection.rs` | 解析 Cargo.toml 依赖 |
| **SDK** | `src/sdk/mod.rs` | chat/tool/session 请求处理 |

### 孤立目录 (未被 lib.rs / main.rs 引用)

- `src/constants/`
- `src/sdk/`

### 其他已知问题

- 根目录 `config.toml` 无代码读取 (实际配置: `~/.star/settings.json`, `./.star/settings.json`, 环境变量)
- `tests/` 目录大部分不可编译 (仅 `tests/lib.rs` + `tests/eval_harness_live.rs` 有效)
- `.gitignore` 中 `**/test_*.rs` 阻止测试文件被跟踪
- `Cargo.lock` 被 gitignore (二进制 crate 不应如此)

---

## 优先级建议

### P0 — 核心体验差距
1. **Task 依赖关系** (blocks/blockedBy) — 影响多步骤任务编排
2. **Permission Feedback input** — 用户无法在确认时提供额外上下文
3. **统一 Modal Stack** — 多个 modal 共存时状态混乱
4. **Component trait** — 新组件开发缺乏统一范式

### P1 — 体验一致性
5. **Tool 渲染格式** (4 项) — 与 CC 输出风格一致
6. **Task 文件持久化** — 跨会话保持任务状态
7. **ChatState 拆分** — 330+ 字段难以维护
8. **底部模式指示器** — 用户不知道当前权限模式
9. **Shift+Tab 快速批准** — 减少确认摩擦

### P2 — 高级功能
10. **AI 风险解释** — 工具执行前展示风险说明
11. **文件锁 + 多 agent** — 多 agent 场景需要
12. **TrustDialog** — 首次信任扫描
13. **Render 脏标记** — 降低 CPU 占用
14. **孤立模块接线** — bridge / voice / SSH / LSP 等

---

## 附: 环境变量参考

仓库中读取 337 个不同的 `STAR_*` 环境变量，几乎都是内联读取无集中注册。
排查行为偏差时: `grep -r 'STAR_' <file>` 查看该文件的环境变量用法。

常用变量: `STAR_API_KEY` `STAR_BASE_URL` `STAR_MODEL` `STAR_CONTEXT_WINDOW`
`STAR_AUTO_COMPACT` `STAR_LLM_TIMEOUT` `STAR_TOOL_TIMEOUT_SECS`
`STAR_LOG_ENABLED` `STAR_LOG_DIR` `STAR_PROMPT_DIR`
