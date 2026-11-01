# Agent Progress UI 对标分析 — 缺失项详细设计

> 参考: `study_or_copy_projects/claude-code-main/` — AgentProgressLine / Spinner
> 当前: `src/ui/components/agent_progress.rs`

## 现状

已有基础 agent 进度行组件，具体实现细节待验证。

## 缺失项

---

### 1. Next Task 提示 — P1 — ✅ 已实现

**CCB 实现**:
```
AgentProgressLine 底部:
  Next: {next_pending_task.subject}
```

**StarCode 实现**:
- `task_panel.rs` 已有 `find_next_task_hint()` 函数
- 优先级: 最近完成任务的未阻塞依赖 > 第一个 pending 任务
- 渲染: 任务面板底部显示 "Next: {subject}"

**涉及文件**: `src/ui/components/task_panel.rs`

---

### 2. activeForm 显示 — ✅ 已实现

**CCB 实现**:
```tsx
<Spinner activeForm={task.activeForm} />
// "Searching for files..." / "Reading config..."
```

**StarCode 实现**:
- `TaskNode` 已有 `active_form: Option<String>` 字段
- 渲染: 进行中的行显示 activeForm ("Running tests"), 其余显示 title ("Run tests")
- 无 activeForm 时回退到 title

**涉及文件**: `src/core/tasks/models.rs`, `src/ui/components/task_panel.rs`

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
