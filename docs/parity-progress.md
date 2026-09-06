# Agent Progress UI 对标分析 — 缺失项详细设计

> 参考: `study_or_copy_projects/claude-code-main/` — AgentProgressLine / Spinner
> 当前: `src/ui/components/agent_progress.rs`

## 现状

已有基础 agent 进度行组件，具体实现细节待验证。

## 缺失项

---

### 1. Next Task 提示 — P1

**CCB 实现**:
```
AgentProgressLine 底部:
  Next: {next_pending_task.subject}
```
- 从 TaskListV2 中查找第一个 `status === 'pending'` 且未被 blocked 的任务
- 显示为灰色小字, 在当前任务 spinner 行下方

**Starcode 需实现**:
- 读取 `TaskPanel.tasks`, 过滤 pending + unblocked
- 取第一个, 显示 "Next: {subject}"
- 颜色: 灰色 (dim)
- 无 pending 任务时不显示

**涉及文件**: `src/ui/components/agent_progress.rs`, `task_panel.rs` (数据源)

---

### 2. activeForm 显示 — 待验证

**CCB 实现**:
```tsx
<Spinner activeForm={task.activeForm} />
// "Searching for files..." / "Reading config..."
```
- `activeForm` 是工具调用时的动态描述
- 来源: `ToolUseBlock` 的 `activeForm` 字段, 或工具元数据

**Starcode 需确认**:
- `StreamMessage` 中是否有 `activeForm` 字段?
- 进度行是否已显示此信息?
- 如缺失: 需要从 tool call 元数据中提取

**涉及文件**: `src/runtime/messages.rs`, `agent_progress.rs`

---

### 3. TaskList inline 集成 — 待验证

**CCB 实现**:
```tsx
// AgentProgressLine 内嵌无 header 的 TaskListV2
<TaskListV2 tasks={activeTasks} showHeader={false} maxVisible={3} />
```
- 执行中的任务列表, 无标题栏, 最多显示 3 行
- 仅显示 in_progress + 最近 completed 的任务

**Starcode 需确认**:
- 当前 Task Panel 是否可嵌入到进度区域?
- 是否需要独立的 inline 版本?

---

### 4. Spinner 随机动词 — P2

**CCB 实现**:
```typescript
const VERBS = ["Thinking", "Processing", "Working", "Computing", "Analyzing"];
// 每次随机选一个 + activeForm
```

**Starcode 需实现**:
- 静态动词列表 + 随机选择
- 或使用已有的 spinner 帧动画

**涉及文件**: `agent_progress.rs`

---

## 实施建议

| 序号 | 任务 | 优先级 | 工作量 | 前置条件 |
|------|------|--------|--------|----------|
| 1 | 验证 activeForm 和 inline TaskList | — | 小 | 需要代码审查 |
| 2 | Next Task 提示 | P1 | 小 | Task Panel 数据可访问 |
| 3 | Spinner 随机动词 | P2 | 小 | 无 |
