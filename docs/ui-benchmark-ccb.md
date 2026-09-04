# Starcode UI 对标 Claude Code (CCB) — 完整报告

> 生成日期：2026-09-04
> 参考项目：`study_or_copy_projects/claude-code-main/`（CCB，TypeScript/React/Ink）
> 本项目：starcode-cli（Rust/ratatui 0.30/crossterm 0.28）

---

## 一、Explore UI（全局搜索）

### CCB 参考实现

| 对话框 | 快捷键 | 底层引擎 | 功能 |
|--------|--------|----------|------|
| **GlobalSearchDialog** | `Ctrl+Shift+F` | ripgrep 流式搜索 | 跨文件内容搜索，右侧预览 |
| **QuickOpenDialog** | `Ctrl+Shift+P` | Rust FileIndex 模糊匹配 | 文件快速跳转+预览 |
| **HistorySearchDialog** | `Ctrl+R` | 历史记录子序列匹配 | 搜索历史 prompt |

**核心设计**：

- 三者共用 `FuzzyPicker` 泛型组件（`@anthropic/ink` 内置）
- `direction="up"` 布局：搜索框在底部，列表向上展开（atuin 风格）
- 响应式：宽屏(≥140列)右侧预览，窄屏底部预览；高度 `Math.min(VISIBLE, rows-14)`
- `SearchBox` 圆角边框，`"suggestion"` 色聚焦态，光标 inverse block
- 全局搜索：ripgrep 参数 `-n --no-heading -i -m 10 -F`，100ms debounce，上限 500 结果
- 预览：异步 `readFileInRange`，`AbortController` 取消上一次，匹配文本高亮
- 选中动作：Enter 在编辑器打开，Tab 插入 `@file#L42`，Shift+Tab 插入路径
- Overlay 注册：`useRegisterOverlay()` 自动追踪 `AppState.activeOverlays`
- Feature gate：`QUICK_SEARCH` / `HISTORY_PICKER` 编译时裁剪

### Starcode 当前状态

**已有实现**：
- `show_global_search` / `global_search_state` — `highlight/search.rs`
- `show_quick_open` / `quick_open_state` — `highlight/quick_open.rs`
- `show_history_search` / `history_search_state` — `highlight/history.rs`

**差距分析**：

| 特性 | CCB | Starcode | 差距 |
|------|-----|----------|------|
| FuzzyPicker 泛型组件 | ✅ 复用同一组件 | ❌ 三个独立实现 | 缺少统一抽象 |
| 响应式预览布局 | ✅ 右/底自动切换 | ❓ 需确认 | 待验证 |
| ripgrep 流式+debounce | ✅ 100ms debounce | ❓ 需确认 | 待验证 |
| Overlay 自动注册 | ✅ `useRegisterOverlay` | ❌ 手动 `show_*` bool | 需迁移到 modal_stack |
| Feature gate 裁剪 | ✅ 编译时 | ❌ 运行时 bool | 无法 tree-shake |
| 光标/kill-ring 编辑 | ✅ 完整 readline | ❓ 需确认 | 待验证 |

---

## 二、Task UI（任务列表）

### CCB 参考实现 — 两套系统

**V1 TodoWriteTool**（SDK/非交互场景）：
- 内存态，`AppState.todos[agentId]`，无持久化
- 全部完成自动清空

**V2 Task 工具集**（交互场景，主用）：
- 6 个工具：`TaskCreate/Update/Get/List/Stop/Output`
- **文件持久化**：`~/.claude/tasks/<listId>/<taskId>.json`
- **文件锁**：`proper-lockfile`，支持 10+ 并发 swarm agent
- **依赖关系**：`blocks` / `blockedBy` 数组
- **Owner 系统**：自动分配，跨进程 mailbox 通知
- **Hook 系统**：`executeTaskCreatedHooks` / `executeTaskCompletedHooks`

**TaskListV2.tsx UI 细节**：

- 图标：`✓`(completed/green) `■`(in_progress/blue) `□`(pending)
- 最近完成保留 30 秒 TTL（`RECENT_COMPLETED_TTL_MS = 30_000`）
- 响应式截断：`rows ≤ 10 ? 0 : min(10, max(3, rows-14))`
- Owner 显示：仅 `columns ≥ 60` 且为活跃 teammate 时显示 `(@name)`
- Spinner 集成：执行时 inline 显示 + `Next: {subject}` 提示
- Ctrl+T 三态循环：`none → tasks → teammates → none`

**后台任务系统**（独立于 V2）：

- `AppState.tasks`：`pending | running | completed | failed | killed`
- 类型：`local_bash / local_agent / remote_agent / in_process_teammate / local_workflow`
- 底部状态栏：team 模式显示 `@main @researcher @coder` 药丸
- BackgroundTasksDialog：上/下/Enter/x(停止)/f(前台)/Esc

### Starcode 当前状态

**已有实现**：`src/ui/components/task_panel.rs`（858 行）

```
TaskPanel {
    tasks: Vec<TaskNode>,
    selected: usize,
    edit_mode: EditMode,
    view_mode: TaskViewMode,  // All | Active
    show_input: bool,
    input_area: TextArea,
}
```

**差距分析**：

| 特性 | CCB V2 | Starcode | 差距 |
|------|--------|----------|------|
| 文件持久化 | ✅ JSON per task | ❌ 内存/需确认 | 可能需要 |
| 文件锁+多 agent | ✅ proper-lockfile | ❌ 无 | 未来需要 |
| 依赖关系 blocks/blockedBy | ✅ | ❌ 无 | 重要缺失 |
| Owner/Teammate 跟踪 | ✅ | ❌ 无 | 多 agent 必需 |
| 30s 完成 TTL | ✅ | ❌ 无 | UX 细节 |
| Spinner inline 集成 | ✅ | ❓ 需确认 | 待验证 |
| Ctrl+T 三态循环 | ✅ tasks/teammates/none | ❌ 需确认 | 待验证 |
| 后台任务对话框 | ✅ BackgroundTasksDialog | ❓ 需确认 | 待验证 |
| 底部状态药丸 | ✅ | ❌ 无 | 可视化缺失 |

---

## 三、Permission UI（权限/确认对话框）

### CCB 参考实现

**架构分层**：

```
PermissionRequest (路由器)
  → permissionComponentForTool() 按工具名分发
    → BashPermissionRequest / FileEditPermissionRequest / ...
      → PermissionDialog (外壳：圆角边框 + 标题 + 颜色主题)
        → PermissionRequestTitle (粗体标题 + WorkerBadge)
        → [工具内容：diff / 命令文本 / plan markdown]
        → PermissionRuleExplanation (为什么需要审批)
        → Select (选项) + 反馈输入
```

**核心设计**：

- 边框：`borderStyle="round"`, top-only, 颜色按上下文切换（permission/warning/error/planMode）
- 选项系统：`OptionWithDescription` 带描述的选项
- 反馈系统：Tab 切换到输入模式，Yes 输入 "下一步做什么"，No 输入 "改什么"
- Ctrl+E：AI 风险解释（lazy-load，shimmer 加载态）
- Shift+Tab：快速批准（accept-edits 模式）
- 分类器自动审批：选项 disabled + 绿色 "Auto-approved" 副标题
- 规则持久化：`AddPermissionRules` 对话框选择保存位置
- Worker 等待态：`WorkerPendingPermission` spinner + `@agentName`

**TrustDialog**：首次打开项目时扫描危险配置（MCP/Hooks/env）

**模式指示器**（底部状态栏）：

```
⏵⏵ auto on (shift+tab to cycle)
```

颜色：auto=orange, bypass=red, plan=planMode

### Starcode 当前状态

**已有实现**：
- `confirmation_dialog.rs`（1,295 行）— 权限对话框
- `RiskLevel` 枚举：`Safe/Low/Medium/High/Critical`
- 颜色常量：`PERMISSION_COLOR`, `SUGGESTION_COLOR`, `SUCCESS_COLOR`, `ERROR_COLOR`

**差距分析**：

| 特性 | CCB | Starcode | 差距 |
|------|-----|----------|------|
| 按工具分发组件 | ✅ 12+ 专用组件 | ❌ 统一对话框 | 工具特化缺失 |
| 反馈输入系统 | ✅ Tab 切换 | ❌ 无 | 重要 UX |
| AI 风险解释 Ctrl+E | ✅ lazy-load | ❌ 无 | 高级功能 |
| Shift+Tab 快速批准 | ✅ | ❌ 无 | 快捷操作 |
| 分类器自动审批 | ✅ | ❌ 无 | 智能审批 |
| WorkerBadge | ✅ @agentName | ❌ 无 | 多 agent |
| TrustDialog | ✅ 首次信任 | ❌ 无 | 安全 |
| 模式底部指示器 | ✅ ⏵⏵ auto on | ❌ 无 | 可见性 |

---

## 四、Agent Progress UI（执行进度）

### CCB 参考实现

**AgentProgressLine.tsx**：

- Spinner 动画 + 当前 in-progress task 的 `activeForm`
- TaskListV2 inline 显示（无 header）
- Next: 提示下一个 pending task

**Spinner.tsx**：

- 随机动词 + 当前任务 activeForm
- 进度条（context usage）
- 底部可展开任务列表

### Starcode 当前状态

**已有实现**：`agent_progress.rs`（agent 进度行组件）

**差距分析**：

| 特性 | CCB | Starcode | 差距 |
|------|-----|----------|------|
| activeForm 显示 | ✅ | ❓ | 待验证 |
| TaskList inline | ✅ | ❓ | 待验证 |
| Next task 提示 | ✅ | ❌ 无 | UX |

---

## 五、UI 框架评估：需要升级吗？

### 依赖版本 — 不需要升级

| 依赖 | Starcode 版本 | 最新版本 | 状态 |
|------|--------------|----------|------|
| ratatui | **0.30** | 0.29+ | ✅ 已是最新 |
| crossterm | **0.28** | 0.28 | ✅ 当前 |
| tokio | **1.0** (full) | 1.0 | ✅ 当前 |

**框架本身不需要升级**。问题是架构层面的。

### 需要的架构改进（按优先级）

**P0 — 立即需要**：

1. **统一 Modal Stack**：当前 10+ 个 `show_*: bool` 只有 4 个在 `modal_stack` 中。全局搜索/快速打开/历史搜索/主题选择/统计/导出/压缩/上下文可视化/错误覆盖层都需要迁移。
2. **组件抽象**：没有 `Component` trait，每个组件是独立 render 函数，签名不统一。需要定义 `Component { render(), handle_event(), update() }` trait。

**P1 — 尽快需要**：

3. **ChatState 分解**：330+ 字段的巨结构需要拆分为 `InputState`, `ModalState`, `StreamingState`, `TaskPanelState`, `AgentState`。
4. **Action/Event 枚举**：当前 3,213 行的 `input.rs` 是原始 KeyEvent match。需要 `Action` 枚举 + dispatch。
5. **渲染与状态分离**：`render_page()` 中有状态变异（scroll、total_rendered_lines 等），应为纯函数。

**P2 — 中期改进**：

6. **渲染脏标记**：当前 30fps 全量重绘。需要 per-component dirty flag。
7. **业务逻辑外移**：`tool_render.rs` 混合了 JSON 解析、路径格式化和 Span 构建。

### 结论

> **不需要升级 ratatui/crossterm/tokio 版本**。需要的是**内部架构重构**：
> - 统一 modal stack
> - 引入 Component trait
> - 分解 ChatState
> - 用 Action 枚举替代 raw KeyEvent match
>
> 这些改动不改变依赖版本，只改变代码组织方式。当前的版本栈完全能支撑所有 CCB 级别的 UI 功能。

---

## 六、实施计划（按顺序）

| 序号 | 任务 | 优先级 | 涉及文件 |
|------|------|--------|----------|
| 1 | 统一 Modal Stack：10+ 个 `show_*` 迁移到 `modal_stack` | P0 | `state/store.rs`, `state/modal.rs`, `app/mod.rs`, `events/input.rs` |
| 2 | 定义 Component trait：统一 `render/handle_event/update` 接口 | P0 | 新建 `ui/components/mod.rs` |
| 3 | Explore UI：FuzzyPicker 泛型组件 + 响应式预览 | P1 | `highlight/search.rs`, `highlight/quick_open.rs`, `highlight/history.rs` |
| 4 | Task UI：blocks/blockedBy + Owner + 30s TTL + Spinner inline | P1 | `task_panel.rs`, `core/tasks/` |
| 5 | Permission UI：反馈输入 + Shift+Tab 快速批准 + 模式指示器 | P1 | `confirmation_dialog.rs` |
| 6 | ChatState 分解：330 字段拆分子状态 | P1 | `state/store.rs` |
