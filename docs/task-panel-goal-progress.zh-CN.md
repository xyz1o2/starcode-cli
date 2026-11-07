# task 面板接入 goal 进度条 — 问题排查与实现总结

日期：2026-11-07 · 分支：`sc-dev-m`

## 问题

用户执行 `/goal progress 832da9b6 50` 后，task 面板不动，追问「为什么它不动呢」。

## 根因

不是 bug，是数据通路不通：

- **task 面板**（`src/ui/components/task_panel.rs`）只读 `TaskManager` → `.star/tasks.json`（TodoWrite / `/task add` 的任务清单）。
- **`/goal` 系列**（`src/commands/parity.rs` 的 `goal_set_progress`）写的是另一套存储：`GoalManager` → `.star/goals.json`，只往聊天流推一条 `📈 832da9b6 → 50%` 文本。
- `src/ui/` 全部 124 个文件此前没有任何 goal 引用——面板对 goals.json 既不监听也不显示。

面板自身的刷新机制（`reload_if_stale` 每帧比对 tasks.json 的 mtime，1Hz tick 驱动）一直是好的，所以任务清单能自动更新，goal 不行。

## 改动

### `src/ui/components/task_panel.rs`

| 位置 | 内容 |
|------|------|
| `TaskPanel` 字段 | 新增 `active_goals: Vec<Goal>` 与 `last_goals_mtime: Option<SystemTime>` |
| `goals_file_for_workspace` | 项目级 goal 文件路径 `cwd/.star/goals.json`，与 `commands::parity::goal_manager` 同源 |
| `load_active_goals_from` | 解析 `Vec<Goal>`，只留 `InProgress`（done / ≥100% / 取消自动退场），新建的排前面（与 `/goal list` 排序一致） |
| `reload_goals` + `reload_if_stale` | goals.json 的 mtime 自愈，走既有的 1Hz tick（`ChatState::tick_task_panel`）；**不受任务编辑锁影响**，文件被删时清空面板段 |
| `check_auto_hide` | active goal 常驻面板：`has_active` 计入 goals，5 秒收起计时不会把进度条收走；手动隐藏不受影响 |
| `render_task_panel_mut` | 首段渲染 goal 行：`🎯 <title> [█████░░░░░] 50% · 832da9b6`（10 格条 + 百分比 + 8 位短 id） |
| `next` / `previous` / `get_selected_task_id` | 高亮索引跳过 goal 行（可选区间 `[goal行, goal行+flat.len())`） |
| `progress_bar` | 自由函数，便于单测 0/50/100/越界形态 |

### `src/ui/app/mod.rs`

`task_panel_h` 高度把 goal 行数一起计入，cap 随 goal 行数上浮（`8 + goal_count`）——否则任务多时 8 行 cap 先到，goal 段会被挤出可视面积。`goal_count == 0` 时行为与旧版完全一致。

## 刷新链路

```
/goal progress 832da9b6 50   （parity.rs → GoalManager 落盘 goals.json）
        ↓ mtime 变化
tick_task_panel（事件循环 1Hz）→ TaskPanel::reload_if_stale → reload_goals
        ↓
下一帧 render_task_panel_mut → 进度条 [█████░░░░░] 50%
```

约 1 秒内生效。`/goal done` 或 progress≥100 后该行自动从面板退场。

## 验证

- `cargo check --all-targets`：通过，无错误。
- `cargo fmt --all`：无漂移。
- `cargo test --lib task_panel`：20/20 通过，新增 4 条：
  - `load_active_goals_keeps_inprogress_only_newest_first`（只留 InProgress、新排前）
  - `load_active_goals_survives_a_missing_file`（文件缺失返回空）
  - `active_goal_holds_the_panel_open`（active goal 不启动收起计时）
  - `selection_offsets_skip_goal_rows`（高亮/选中索引跳过 goal 行）
  - `progress_bar_boundaries`（0/50/100/越界条形）
- clippy：仅剩两条存量债（`TaskPanel` 缺 `Default` impl、`if let Ok(_)` 冗余匹配），均与本次改动无关且 CI 不做 `-D warnings` 门控。

## 排查中的坑

- `then_some` 是**急切求值**：`expr.then_some(i - offset)` 里 `i - offset` 无条件计算，i < offset 时直接减法下溢 panic；判等分支要用惰性的 `then(|| i - offset)`。
- 面板高度在 `app/mod.rs` 的 layout 里按条目数硬算，改渲染内容必须同步改高度公式，否则新行直接不可见。
