# Task UI 对标分析 — 缺失项详细设计

> 参考: `study_or_copy_projects/claude-code-main/` — TaskListV2 / TodoWriteTool
> 当前: `src/ui/components/task_panel.rs` (858 行)

## 现状

已实现基础面板:
- `TaskPanel` 结构体: tasks / selected / edit_mode / view_mode / show_input / input_area
- `TaskViewMode`: All | Active
- TodoWrite 工具渲染: CC 风格 checklist 图标

## 缺失项

---

### 1. 依赖关系 (blocks / blockedBy) — P0 — ✅ 已实现

**CCB 实现**:
```typescript
interface Task {
  blocks: string[];    // 本任务完成后解锁的任务 ID
  blockedBy: string[]; // 阻塞本任务的任务 ID
}
```

**StarCode 实现**:
- `TaskNode` 已有 `blocks: Vec<String>` 和 `dependencies: Vec<String>`
- `TaskUpdate` 已有 `add_blocks` / `add_blocked_by` 参数
- UI: blocked 任务显示 `⊘ blocked by #id1, #id2`
- 逻辑: 完成任务时自动解锁被阻塞的依赖

**涉及文件**: `src/core/tasks/models.rs`, `src/core/tools/task_management.rs`, `src/ui/components/task_panel.rs`

---

### 2. 文件持久化 — P1 — ✅ 已实现

**CCB 实现**:
```
~/.claude/tasks/<listId>/<taskId>.json
```

**StarCode 实现**:
- `TaskManager::save_to_file()` / `TaskManager::load_from_file()` 已实现
- 任务创建/更新/完成时自动保存
- 启动时自动加载

**涉及文件**: `src/core/tasks/manager.rs`, `src/ui/components/task_panel.rs`

---

### 3. 30 秒完成 TTL — P1 — ✅ 已实现

**CCB 实现**:
```typescript
const RECENT_COMPLETED_TTL_MS = 30_000;
```

**StarCode 实现**:
- `TaskNode` 已有 `completed_at: Option<DateTime<Utc>>`
- 渲染时过滤: completed_at 超过 30s 的不显示
- 自动清理机制已实现

**涉及文件**: `src/ui/components/task_panel.rs`, `src/core/tasks/models.rs`

---

### 4. Owner / Teammate 跟踪 — P2

**CCB 实现**:
- `TaskCreate` 自动设置 `owner = 当前 agent name`
- UI 仅在 `columns >= 60` 且为活跃 teammate 时显示 `(@name)`
- 跨进程 mailbox 通知 owner 变更

**Starcode 需实现**:
- `TaskNode` 增加 `owner: Option<String>`
- 当前 agent 名称从 config 或 session 获取
- 渲染: 条件显示 `(@name)`

**涉及文件**: `task_panel.rs`, `src/agent/` (获取 agent name)

---

### 5. Spinner inline 集成 — P1 — ✅ 已实现

**CCB 实现**:
- 执行中的任务行内显示 spinner 动画
- `Next: {subject}` 提示下一个 pending task
- Spinner 随机选择动词 + activeForm

**StarCode 实现**:
- Spinner 帧已实现 (dots variant)
- `Next: {subject}` 提示已实现 (`find_next_task_hint`)
- `active_form` 字段已实现 ("Running tests" vs "Run tests")
- 列表底部或末尾显示 "Next: {subject}"
- Spinner 帧来源: `status_line.rs` 已有 spinner 逻辑可复用

**涉及文件**: `task_panel.rs`, `src/ui/components/status_line.rs` (复用 spinner)

---

### 6. Ctrl+T 三态循环 — P1 — ✅ 部分实现

**CCB 实现**:
```
Ctrl+T: none → tasks → teammates → none
```

**StarCode 实现**:
- Ctrl+T 已绑定为任务面板切换
- 三态循环: 隐藏 → Task Panel → 隐藏 (无 Teammate Panel)
- Teammate Panel 未实现 (P2, 多 agent 场景)

**涉及文件**: `src/ui/events/input.rs`, `src/ui/components/task_panel.rs`

---

### 7. Background Tasks 对话框 — P2

**CCB 实现**:
```
BackgroundTasksDialog:
  - 列表: pending | running | completed | failed | killed
  - 类型: local_bash / local_agent / remote_agent / in_process_teammate / local_workflow
  - 操作: ↑/↓/Enter(详情)/x(停止)/f(前台)/Esc(关闭)
```

**Starcode 需实现**:
- 新组件 `BackgroundTasksDialog`
- 状态来源: `ShellExecutionService` 的后台任务 + agent 的后台 agent
- 操作: 停止 / 前台化 / 查看输出

**涉及文件**: 新建 `src/ui/components/background_tasks.rs`

---

### 8. 底部状态药丸 — P2

**CCB 实现**:
```
@main @researcher @coder  (底部状态栏, team 模式)
```
- 每个活跃 teammate 显示为一个药丸 (pill)
- 颜色: 活跃=绿色, 空闲=灰色

**Starcode 需实现**:
- 在底部状态栏区域渲染 teammate pills
- 数据来源: 当前活跃的 agent / teammate 列表
- 仅在多 agent 模式下显示

**涉及文件**: `src/ui/components/status_line.rs`

---

## 实施状态

| 序号 | 任务 | 优先级 | 状态 |
|------|------|--------|------|
| 1 | 依赖关系 blocks/blockedBy | P0 | ✅ 已实现 |
| 2 | 30s 完成 TTL | P1 | ✅ 已实现 |
| 3 | Spinner inline | P1 | ✅ 已实现 |
| 4 | 文件持久化 | P1 | ✅ 已实现 |
| 5 | Ctrl+T 三态 | P1 | ✅ 部分实现 (无 Teammate Panel) |
| 6 | Owner 跟踪 | P2 | ❌ 未实现 (多 agent 场景) |
| 7 | 底部状态药丸 | P2 | ❌ 未实现 (多 agent 场景) |
| 8 | Background Tasks 对话框 | P2 | ❌ 未实现 |
