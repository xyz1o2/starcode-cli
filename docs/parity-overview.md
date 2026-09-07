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
| [Tool 渲染](#2-tool-渲染) | `ui-tool-render.md` | [`parity-tool-render-gaps.md`](parity-tool-render-gaps.md) | ✅ 100% | 全部格式优化已完成 |
| [Task UI](#3-task-ui) | `ui-benchmark-ccb.md` §II | [`parity-task.md`](parity-task.md) | 🟢 P0/P1 | 5/8 已完成 |
| [Permission UI](#4-permission-ui) | `ui-benchmark-ccb.md` §III | [`parity-permission.md`](parity-permission.md) | 🟢 P0/P1 | 反馈输入+快速批准+模式指示器 |
| [Agent Progress UI](#5-agent-progress-ui) | `ui-benchmark-ccb.md` §IV | [`parity-progress.md`](parity-progress.md) | 🟢 P0/P1 | activeForm+进度条+Next Task |
| [Slash Commands](#6-slash-commands) | [`parity-commands.md`](parity-commands.md) | — | ✅ 95% | 全部 Pending 已实现 |
| [Agent 内部机制](#7-agent-内部机制) | [`parity-agent.md`](parity-agent.md) | — | 🟡 70% | 核心已对标, 部分 TODO |
| [UI 框架架构](#8-ui-框架架构) | `ui-benchmark-ccb.md` §V | [`parity-ui-framework.md`](parity-ui-framework.md) | 🔴 20% | 7 项架构重构 |
| [孤立模块 / 未接线](#9-孤立模块--未接线) | — | [`parity-orphaned.md`](parity-orphaned.md) | 🔴 0% | 22 个模块未接线或桩实现 |
| [边角扫描](#10-边角扫描) | — | [`parity-edge-cases.md`](parity-edge-cases.md) | — | unwrap 风险 / i18n / 测试 / CCB 未分析特性 |

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

**状态**: ✅ 全部完成

### 已完成
- ToolResult 整体缩进 (`  ⎿ ` 前缀)
- ANSI 颜色保留 (256色 / truecolor / dim/italic/reverse/crossed-out)
- Span 级宽度计算 + CJK 感知
- `build_tool_body_block` ANSI 感知渲染
- 折叠预览颜色保留
- Edit diff 折叠摘要 ("Added N lines, removed M lines")
- 7 个单元测试
- ToolCall Header 格式 (`● ToolName(args)` + 终端宽度截断)
- View/搜索工具单行摘要 ("Read N lines", "Found N matches", "Found N files")
- Bash 结果摘要 ("Done" 空输出, "Running in the background" 后台任务)
- Write 结果摘要 ("Wrote N lines to {path}")

---

## 3. Task UI

**文档**: [`ui-benchmark-ccb.md`](ui-benchmark-ccb.md) §II

**状态**: 🟢 P0/P1 已完成，剩余 P2 多 agent 特性

### 已实现
- `task_panel.rs` (858行): `TaskPanel` 结构体
- TodoWrite 工具渲染: CC 风格 checklist 图标
- 任务列表显示
- 文件持久化 (`save_to_file` / `load_from_file`)
- 依赖关系 (blocks / blockedBy + 自动解锁)
- 30s completed TTL 自动清除
- Spinner inline 集成 (dots variant)
- Ctrl+T 切换任务面板
- Next Task 提示 (`find_next_task_hint`)
- activeForm 显示 ("Running tests" vs "Run tests")

### 未实现 (vs Claude Code Task V2)

| 特性 | CCB | StarCode | 优先级 |
|------|-----|----------|--------|
| 文件锁 + 多 agent 协作 | ✅ (proper-lockfile) | ❌ | P2 |
| Owner / Teammate 跟踪 | ✅ | ❌ | P2 |
| Background tasks 对话框 | ✅ | ❌ | P2 |
| 底部状态 pills | ✅ | ❌ | P2 |

---

## 4. Permission UI

**文档**: [`ui-benchmark-ccb.md`](ui-benchmark-ccb.md) §III

**状态**: 🟢 P0/P1 已完成，特化渲染已实现

### 已实现
- `confirmation_dialog.rs` (1,295行)
- `RiskLevel` 枚举 + 颜色常量
- 统一确认对话框
- 特化渲染: EditFile (diff), CreateFile, DeleteFile, ShellCommand, NetworkRequest, AskUserQuestion
- Feedback input (Tab 切换模式, Enter 提交)
- Shift+Tab 快速批准 (ProceedOnce)
- Ctrl+E 切换风险解释区
- Ctrl+D 切换 debug 详情
- 数字键/字母键选择选项 (1/y, 2/s, 3/a, 4/d/n)
- 底部状态栏模式指示器 (⏵⏵ default / ⏸ plan / ⏵⏵ yolo)

### 未实现

| 特性 | CCB | StarCode | 优先级 |
|------|-----|----------|--------|
| AI 风险解释 (lazy-load, 异步获取) | ✅ | ❌ | P2 |
| 分类器自动审批 | ✅ | ❌ | P2 |
| WorkerBadge (@agentName) | ✅ | ❌ | P2 |
| TrustDialog (首次信任扫描) | ✅ | ❌ | P2 |

---

## 5. Agent Progress UI

**文档**: [`ui-benchmark-ccb.md`](ui-benchmark-ccb.md) §IV

**状态**: 🟢 P0/P1 已完成

### 已实现
- activeForm 显示 (执行中工具的 spinner + 描述)
- TaskList inline (已完成数/总数 进度条)
- Next Task hint (完成一个后提示下一个)
- `progress_hint` 字段 + `emit_progress_update()` 推送

### 未实现

| 特性 | CCB | StarCode | 优先级 |
|------|-----|----------|--------|
| 完整的 inline task 进度 (嵌套子任务) | ✅ | ❓ 待验证 | — |

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

> 详细清单见 [`parity-orphaned.md`](parity-orphaned.md) — 22 个模块

### 快速概览

| 分类 | 数量 | 模块 |
|------|------|------|
| 已确认孤立 (从未构造) | 4 | ModelFallbackManager, PolicyEngine, WebBrowserTool, Analytics |
| 完全桩实现 (方法为空/占位符) | 12 | Bridge, SSH, Voice, SDK, LSP, Workflow, ContextStore, SkillTool, Edit Trust, Chrome, Keychain, OTLP/Langfuse |
| 解析/索引占位符 | 2 | Structure Index (正则→tree-sitter), Context Selection |
| Eval 桩 | 1 | Eval Harness E2E |
| 死环境变量 | 5 组 | Voice(6), Bridge(9), SDK(3), Daemon(6), Chrome(1) |

### 优先接线

| 优先级 | 模块 | 理由 |
|--------|------|------|
| P0 | PolicyEngine + Edit trust | 安全检查缺失 |
| P1 | ModelFallbackManager, Structure Index, Context Store | 可靠性 + 代码智能 |
| P2 | SkillTool, Workflow, LSP | 功能完整性 |
| P3 | Bridge, Voice, SSH, Chrome, SDK 等 | 高级/可选功能 |

---

## 10. 边角扫描

> 详细清单见 [`parity-edge-cases.md`](parity-edge-cases.md)

### 关键发现

| 分类 | 数量 | 严重度 |
|------|------|--------|
| `lock().unwrap()` — 可毒化进程 | 40+ 处 | 🔴 高 |
| `Selector::parse().unwrap()` — CSS panic | 14 处 | 🔴 高 |
| i18n 硬编码字符串 (中+英) | 22 处 | 🟡 中 |
| 核心工具无测试 (shell/read/write/glob/grep) | 13 文件 | 🟡 中 |
| CCB 未分析特性 (并行执行/Retry/Cost 等) | 10 项 | 🟡 中 |
| Config 字段可能未消费 | 3 字段 | 🟡 中 |

### CCB 未覆盖特性 (StarCode 缺失)

| 特性 | CCB | StarCode | 影响 |
|------|-----|----------|------|
| 工具并行执行 | partitionToolCalls 10 并发 | 顺序执行 | 性能 |
| Retry 策略 | 819 行按状态码+jitter | 152 行基础版 | 健壮性 |
| Cost tracking | 专用 hook | 有限 | 精度 |
| Diagnostic tracking | 专用 | 无 | 可观测 |
| VCR 录制/回放 | 专用 | 无 | 调试 |
| Sleep prevention | 专用 | 无 | UX |

---

## 优先级建议

### P0 — 安全 + 稳定
1. **`lock().unwrap()` 治理** — 40+ 处, 任何一次 panic 毒化进程
2. **`Selector::unwrap()` 修复** — 14 处 web_search CSS panic 风险
3. **PolicyEngine::load_permission_rules** — 权限规则不生效
4. **Edit trust logic** — 不受信任文件夹安全检查缺失
5. **Vim mode flag 与实现不一致** — flag 标 enabled 但代码 "not implemented"

### P1 — 体验 + 可靠性 + 性能
6. **统一 Modal Stack** — modal 共存状态管理
7. **Component trait** — 统一组件范式
8. **ModelFallbackManager** — 模型回退

### ✅ 已完成
- Task 依赖关系 (blocks/blockedBy) — 多步骤任务编排
- Permission Feedback input — 确认时提供额外上下文
- 工具并行执行 — 只读工具批量并行 (execute_batch)
- Retry 逻辑增强 — 按状态码策略 + jitter + Retry-After
- Tool 渲染格式 — 全部 11 项已完成
- Next Task hint — 完成后提示下一个任务
- activeForm 显示 — 执行中工具的 spinner + 描述
- 底部状态栏模式指示器 — ⏵⏵ default / ⏸ plan / ⏵⏵ yolo

### P2 — 功能完整性 + 国际化
14. **ChatState 拆分** — 330+ 字段
15. **Structure Index → tree-sitter** — 代码索引精度
16. **Context Store** — 上下文持久化
17. **i18n 硬编码修复** — 剩余英文字符串
18. **SkillTool** — 模型得到假响应
19. **Workflow Engine** (非 Shell 步骤)
20. **Langfuse / OTLP flush** — 可观测性

### ✅ 已完成
- Task 文件持久化 — 跨会话任务状态 (task_persistence.rs)
- 底部模式指示器 — 权限模式可见性 (status_line)
- Shift+Tab 快速批准 — 减少确认摩擦 (input.rs)

### P3 — 可选增强
24. **AI 风险解释** / **TrustDialog** / **Render 脏标记**
25. **LSP Instance** / **Bridge** / **Voice** / **SSH** / **Chrome** / **SDK**
26. **Diagnostic tracking** / **VCR** / **Sleep prevention** / **Personas**

---

## 附: 环境变量参考

仓库中读取 337 个不同的 `STAR_*` 环境变量，几乎都是内联读取无集中注册。
排查行为偏差时: `grep -r 'STAR_' <file>` 查看该文件的环境变量用法。

常用变量: `STAR_API_KEY` `STAR_BASE_URL` `STAR_MODEL` `STAR_CONTEXT_WINDOW`
`STAR_AUTO_COMPACT` `STAR_LLM_TIMEOUT` `STAR_TOOL_TIMEOUT_SECS`
`STAR_LOG_ENABLED` `STAR_LOG_DIR` `STAR_PROMPT_DIR`
