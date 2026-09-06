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

### 1. 依赖关系 (blocks / blockedBy) — P0

**CCB 实现**:
```typescript
interface Task {
  blocks: string[];    // 本任务完成后解锁的任务 ID
  blockedBy: string[]; // 阻塞本任务的任务 ID
}
```

- `TaskUpdate` 工具可设置 `addBlocks` / `addBlockedBy`
- UI 显示: blocked 任务显示 `⊘` 图标 + 灰色
- `TaskGet` 检查 blockedBy 列表为空才可开始
- 自动状态流转: 所有 blocker completed → blocked 任务自动 unblock

**Starcode 需实现**:
- `TaskNode` 增加 `blocks: Vec<String>`, `blocked_by: Vec<String>`
- `TaskUpdate` 增加 `add_blocks` / `add_blocked_by` 参数
- UI: blocked 任务灰色显示 + 阻塞原因
- 逻辑: blocker 全部 completed 时自动清除 blocked_by

**涉及文件**: `src/ui/components/task_panel.rs`, `src/tools/` (TaskCreate/Update 工具)

---

### 2. 文件持久化 — P1

**CCB 实现**:
```
~/.claude/tasks/<listId>/<taskId>.json
```
- 每个 task 一个 JSON 文件
- `proper-lockfile` 文件锁防止并发写入
- 跨会话保持: 重启后任务列表恢复

**Starcode 需实现**:
- 存储路径: `.star/tasks/<list_id>/<task_id>.json` (通过 `Storage` 工具)
- 序列化: `serde_json` 写入 / 读取
- 加载: 启动时或首次打开 Task Panel 时加载
- 保存: 任务创建 / 更新 / 完成时写入

**涉及文件**: 新建 `src/core/tasks/storage.rs`, 修改 `task_panel.rs`

---

### 3. 30 秒完成 TTL — P1

**CCB 实现**:
```typescript
const RECENT_COMPLETED_TTL_MS = 30_000;
```
- 任务标记 completed 后保留 30 秒在列表中可见
- 超时后自动从 UI 移除 (数据仍持久化)

**Starcode 需实现**:
- `TaskNode` 增加 `completed_at: Option<Instant>`
- 渲染时过滤: `completed_at` 超过 30s 的不显示
- 或使用后台定时器清理

**涉及文件**: `src/ui/components/task_panel.rs`

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

### 5. Spinner inline 集成 — P1

**CCB 实现**:
- 执行中的任务行内显示 spinner 动画
- `Next: {subject}` 提示下一个 pending task
- Spinner 随机选择动词 + activeForm

**Starcode 需实现**:
- 在 in_progress 任务行首显示 spinner 帧
- 列表底部或末尾显示 "Next: {subject}"
- Spinner 帧来源: `status_line.rs` 已有 spinner 逻辑可复用

**涉及文件**: `task_panel.rs`, `src/ui/components/status_line.rs` (复用 spinner)

---

### 6. Ctrl+T 三态循环 — P1

**CCB 实现**:
```
Ctrl+T: none → tasks → teammates → none
```
- tasks: 显示 TaskListV2
- teammates: 显示 teammate 列表 (swarm 模式)
- none: 隐藏

**Starcode 需实现**:
- 在 `input.rs` 中绑定 Ctrl+T
- 三态循环: 隐藏 → Task Panel → Teammate Panel → 隐藏
- Teammate Panel 需要独立组件 (或暂用 placeholder)

**涉及文件**: `src/ui/events/input.rs`, `task_panel.rs`

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

## 实施建议

| 序号 | 任务 | 优先级 | 工作量 | 依赖 |
|------|------|--------|--------|------|
| 1 | 依赖关系 blocks/blockedBy | P0 | 中 | 需要 Task 工具支持 |
| 2 | 30s 完成 TTL | P1 | 小 | 无 |
| 3 | Spinner inline | P1 | 小 | 复用 status_line |
| 4 | 文件持久化 | P1 | 中 | Storage 工具 |
| 5 | Ctrl+T 三态 | P1 | 小 | 需 Teammate Panel |
| 6 | Owner 跟踪 | P2 | 小 | 多 agent 场景 |
| 7 | 底部状态药丸 | P2 | 小 | 多 agent 场景 |
| 8 | Background Tasks 对话框 | P2 | 大 | 后台任务系统 |
