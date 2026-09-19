# 修复计划 — Task 面板与 Explore 子代理

> 日期：2026-09-20
> 涉及：`src/ui/components/task_panel.rs`、`src/ui/app/mod.rs`、`src/agent/skills/explore.rs`、`src/agent/tools/skill.rs`
> 参考：CCB `TaskListV2.tsx`（源码见 [zackautocracy/claude-code](https://github.com/zackautocracy/claude-code/blob/4b9d30f7/src/components/TaskListV2.tsx)）、CCB issue #58297 / #60095 / #53603

本文是动工前的设计稿。每项给出**根因 → 改法 → 涉及文件 → 测试 → 风险**，未拍板的决策点单列在末尾。

---

## 已完成（本轮前几个改动）

| 项 | 内容 |
|---|---|
| Provider 表单 | 非活动字段补回边框、hint 不再重复两行（`provider_form.rs`） |
| Spinner 门控 | 只在 `is_processing` 时转，agent 停了显示静止 `●`（`task_panel.rs` + `app/mod.rs`） |
| 面板标题 | 改为 ` Todo · {activeForm} `，去掉计数（`task_panel.rs`） |

---

## A. Todo 优先级排序 — 建议现在做

### 根因

`flatten_tasks`（`task_panel.rs:600`）按 `root_ids` 插入序遍历，**没有任何状态优先级**。面板高度上限 8 行（`app/mod.rs:38`），所以谁排前面、谁被截掉，完全取决于模型写 `todos` 数组的顺序——已完成的历史任务可能把进行中的任务挤出可视区。

CCB 的优先级（`TaskListV2.tsx` 源码）：

```
recent_completed(≤30s) → in_progress → pending(未阻塞) → pending(已阻塞) → older_completed
```

组内 `byIdAsc`。文档来源：[How Claude Code works — Task system](https://notes.tsukino.dev/99-%E5%B7%A5%E5%85%B7%E4%B8%8E%E5%8F%82%E8%80%83/repos/how-claude-code-works/en/docs/15-task-system)。

### 改法

在 `TaskPanel` 上加一个排序方法，只动**根节点**顺序，子树跟着父节点走（深度序不变）：

```rust
/// 对标 CCB TaskListV2 的优先级排序：最近完成 → 进行中 → 未阻塞 →
/// 已阻塞 → 更早完成。只在 root_ids 层面重排，子节点跟着父节点走，
/// 树的缩进结构不被打散。
///
/// 组内保持 root_ids 的原始顺序（≈ CCB 的 byIdAsc，id 本就是创建序），
/// 不引入额外的 id 解析。
fn sorted_root_ids(&self) -> Vec<String>;

fn priority_rank(node: &TaskNode, now: DateTime<Utc>) -> u8 {
    match node.status {
        Completed if 最近完成(≤30s, 复用 is_completed_expired 的反向) => 0,
        InProgress => 1,
        Pending if 未阻塞 => 2,
        Pending | Blocked => 3,   // 阻塞的排最后
        Completed => 4,
        Skipped => 4,
    }
}
```

`flatten_tasks` 改为遍历 `sorted_root_ids()`，`collect_nodes` 递归部分**不动**（保持子树原序）。

30s 判定复用已有的 `COMPLETED_TTL_SECS` / `completed_at`，不新增常量。

### 涉及文件

- `src/ui/components/task_panel.rs` — `sorted_root_ids` + `flatten_tasks` 改一行

### 测试

在 `task_panel.rs` 的 `#[cfg(test)] mod tests` 里加：

- `sorted_root_ids_groups_by_status` — 造 5 个不同状态的根，断言输出是 `recent_completed / in_progress / pending / blocked / older_completed`
- `sort_is_stable_within_bucket` — 三个 pending 根，断言组内顺序不变
- `sort_keeps_children_attached` — 父节点排序后子节点仍紧跟父节点，缩进前缀不变
- `sorting_respects_30s_recent_completed_window` — 31s 前完成的落进 older 桶

### 风险

**低**。纯显示层，不写盘、不改 `TaskGraph`。已有测试 `next_todo_graph_maps_fields_and_order` 守着存储顺序，不受影响。

唯一要注意的是 `Active` 视图模式过滤（`collect_nodes` 里的 `is_self_visible`）在排序**之后**生效，两者正交，不要在排序里重复做过滤。

---

## B. 残留 InProgress 让面板永不收起 — 建议现在做

### 根因（新发现，和 spinner 同源）

`check_auto_hide`（`task_panel.rs:274`）：

```rust
let has_active = nodes.values().any(|n| matches!(
    n.status, TaskStatus::Pending | TaskStatus::InProgress | TaskStatus::Blocked
));
```

模型收尾时经常不把最后一项标 completed（正是上一轮 spinner 问题的同一个原因）。于是这项永远 `InProgress` → `has_active` 恒为 true → 5 秒计时器永不启动 → **面板一直挂在输入框上方，直到会话结束**。

spinner 那边已经按"agent 是否在跑"判断真假活跃，这里还是按数据状态判断，两个地方标准不一致。

### 改法

把 `check_auto_hide` 的"活跃"定义和 spinner 对齐——**进行中只有在 agent 真在跑时才算活跃**：

```rust
pub fn check_auto_hide(&mut self, agent_active: bool) {
    let has_active = self.task_manager.graph.nodes.values().any(|n| match n.status {
        TaskStatus::Pending | TaskStatus::Blocked => true,
        TaskStatus::InProgress => agent_active,   // 停了就不算活跃
        _ => false,
    });
    // …其余不变
}
```

调用点 `app/mod.rs:29` 传入 `state.is_processing`。

### 涉及文件

- `src/ui/components/task_panel.rs` — `check_auto_hide` 签名 + 判定
- `src/ui/app/mod.rs:29` — 传参

### 测试

- `auto_hide_starts_when_agent_idle_with_stuck_in_progress` — 一个 InProgress + `agent_active=false` → `auto_hide_at` 被设上
- `auto_hide_held_while_agent_running` — 同样一个 InProgress + `agent_active=true` → `auto_hide_at == None`
- `pending_blocks_auto_hide_regardless_of_agent` — Pending 项在两种状态下都不收起

`check_auto_hide` 目前没有测试（`TaskPanel` 的测试都在别处），这些是新增。

### 风险

**低-中**。行为变化要讲清楚：agent 停了之后，残留未完成的清单**会在 5 秒后收起**。这是有意的——面板存在是为了显示"正在做什么"，agent 停了还挂着就是误导（同 CCB issue #58297 的判定：状态由 turn 生命周期驱动，不由数据状态驱动）。用户想看完整清单随时 Ctrl+T 重新打开。

反向风险：agent 频繁启停（多轮短对话）时面板可能闪。5 秒延迟本来就为缓冲这个，且 `auto_hide_at` 在新一轮 `is_processing=true` 时会被 `has_active` 分支重置为 `None`。

---

## C. 标题是否恢复计数 — 待决策

### 现状

当前是 ` Todo · {activeForm} `。但搜到的 CCB `TaskListV2.tsx` 源码显示，CCB 的 standalone 头**是带计数的**：

```
{tasks.length} tasks ({completedCount} done, {inProgressCount > 0 ? `${inProgressCount} in progress, ` : ''}{pendingCount} open)
```

即 ` 5 tasks (2 done, 1 in progress, 2 open)`。所以"CCB 叫 Todo"这个前提不成立。

### 我的建议：不恢复计数

理由：在 B 修复之前，"1 in progress" 在 agent 停了之后仍是数据事实但语义错误；B 修复之后面板会收起，计数更没有展示窗口。activity 后缀（`· Running tests`）信息量比计数高，且只在 agent 跑时出现，不会误导。

### 若要恢复

改 `render_task_panel_mut` 的 title 构造即可，`total/completed/pending/in_progress` 计数循环上次没删（只删了变量），加回来 5 行。注意 `in_progress` 计数要和 spinner 一样按 `agent_active` 折算，否则重蹈覆辙。

---

## D. Explore 子代理接线 — 待决策，动静最大

### 现状回顾（详见 `docs/parity-agent.md` + 上一轮评估）

两条互不相通的路径：

| | Path A: `AgentTool`（`Task`） | Path B: `SkillTool`（`skill`） |
|---|---|---|
| 注册 | `agent_runtime.rs:96` 无条件 | `agent_runtime.rs:255` 默认开 |
| 指定 explore | `subagent_type: "explorer"` | `skill: "explore"` |
| UI 进度 | ✅ 发 `AgentTaskUpdate` chunk | ❌ 无 sink，静默内联 |
| 自动触发 | 无 | 无（`AutoTriggerKind` 无 ExploreSkill 变体） |

`ExploreAgent` 注册在 Path B 的 manager 里（`skill.rs:37`），跑起来既不上 UI 也不会被自动派发。而 `team_execution.rs:90` 已经用 Path A 的方式注册过一份 `ExploreAgent`——等于半成品。

CCB 的做法（文档 + issue 佐证）：**只有一把 Agent/Task 工具**，`subagent_type: "explorer"`，且 v2.1.198 起子代理默认后台跑，`/agents` 面板有 Running tab。没有"skill 工具里塞子代理"这种双轨结构。

### 选项

**D1（最小）**：给 `SkillInvocation::execute` 和 `ExploreAgent` 透传 `SubAgentProgressSink`，复用 runner 的 `AgentTaskUpdate`。
- 优点：ExploreAgent 立刻能在 `bg_agent_selector` 露面
- 缺点：双轨结构保留，`AutoTriggerKind` 还是不会选 explore；两套注册要一直同步
- **硬约束**：必须发终止事件（Completed/Failed），否则复刻 CCB issue #60095——行卡 Running 永不消失，和 B 的症状一样

**D2（中等）**：D1 + 在 `AutoTriggerKind` 加 `ExploreSkill` 变体，语义搜索命中但结果不够时升级到 explore。
- 优点：真"智能触发"
- 缺点：所有 auto-trigger 开关默认关（`STAR_ENABLE_AUTO_SKILL_FALLBACKS` 等），链上 prerequisite 也都关着，要先开闸才能看到效果

**D3（最彻底，推荐）**：把 `ExploreAgent` 从 `SkillTool` 迁到 `AsyncSubagentRunner` 的注册表，两条路并成一条。
- 优点：和 CCB 同构，触发/UI/生命周期都自然有了，`team_execution.rs` 那份重复注册可以删
- 缺点：`ExploreAgent` 现在的执行模型（`run_codebase_search_for_skill` + 可选 deep filter）和 runner 的 chunk 流模型不完全对齐，要改 `execute` 适配；`SkillTool` 的 `skill: "explore"` 入口要保留兼容还是砍掉，得定

### 我的建议

先做 A + B（都是低风险、独立可测），D 单独立项。D 的前提是先回答两个问题：

1. 要"自动触发"还是"用了能看见"？——决定做 D2 还是 D1
2. D3 里 `skill: "explore"` 这个旧入口留不留？——留着是双轨，砍了是破坏兼容（记忆里 [[no-compat-shims-dedupe-case]] 的规矩是不留兜底）

---

## 实施顺序

| 步骤 | 项 | 风险 | 依赖决策 | 状态 |
|------|----|----|----------|------|
| 1 | A. 优先级排序 | 低 | 无 | ✅ 已完成 |
| 2 | B. auto-hide 对齐 agent 活跃判定 | 低-中 | 无 | ✅ 已完成 |
| 3 | C. 标题计数 | 低 | 已定：完备对标 CCB | ✅ 已完成 |
| 4 | D. Explore 接线 | 中-高 | 已定：D3 单轨 | ✅ 已完成 |

### 已落地的改动

**A — `src/ui/components/task_panel.rs`**
- `flatten_tasks` 改为遍历新的 `sorted_root_ids()`
- `sorted_root_ids()` / `priority_rank()` / `has_unresolved_dependency()`：最近完成(≤30s) → 进行中 → 未阻塞 → 已阻塞 → 更早完成，稳定排序，只在根层面重排、子树跟着走
- 测试：分组、组内稳定、子树不被打散、30s 窗口、阻塞优先级

**B — `src/ui/components/task_panel.rs` + `src/ui/app/mod.rs`**
- `check_auto_hide(&mut self, agent_active: bool)`：`InProgress` 只在 agent 在跑时算活跃
- 调用点传 `state.is_processing`
- 测试：agent 停了残留 in_progress 启动计时 / agent 在跑按住不收 / 超时后收起 / pending 两种状态都不收起

**C — `src/ui/components/task_panel.rs`**

恢复 CCB `TaskListV2` 的计数头格式，并保留 activity 后缀：

```
 5 tasks (2 done, 1 in progress, 2 open) · Running tests 
```

- `in_progress` 计数只在 >0 时出现（同参考实现）
- agent 在跑时追加当前进行中项的 activeForm；停了就只有计数
- 计数与 activity 互补：计数描述状态分布，activity 描述当前动作；状态行的 spinner 动词不重复

**D — Explore 并入 AgentTool 单轨（D3）**

调查发现接线问题比文档预期更严重：`AgentTool` 的 `SyncNamedAgent` 路径造的是**通用 `StarAgent`**，`subagent_type` 只当 UI 标签用，根本不调 `ExploreAgent`——`SubagentType` 在系统提示词选择里完全没参与。所以之前"explorer"跑了通用 agent，真正检索专家只能靠 `skill` 工具触发，而那条路径不上 UI。

改动：

- `src/core/agents/mod.rs` — `SubAgentRequest` 加 `subagent_type` 字段 + `with_subagent_type()`
- `src/agent/subagent/router.rs` — `build_request` 把类型透进 request（三处异步路径也补齐）
- `src/agent/subagent/runner.rs` — `run_with_progress` 开头按类型派发：`Explorer` → 新增 `run_explore()`，跑 `ExploreAgent` 的确定性检索并包成 `SubAgentResult`；开始时推一条 `Searching codebase…` 进度，UI 行不再只是 "Initializing"
- `src/agent/tools/skill.rs` — 移除 `ExploreAgent` 注册和导入，注释说明它走单轨
- `team_execution.rs` 的注册**保留**：那是 agent teams 子系统，仍走 SubAgent trait 派发，与 AgentTool 单轨无关

测试（subagent 模块 14 个全过）：
- `explorer_dispatch_runs_search_not_llm` — 真 client + temp dir，验 `run_explore` 跑的是检索（0 token、output 非空），不是 LLM 循环。deep_filter 默认关，不碰网络
- `only_explorer_is_routed_to_search` — 类型判定输入侧
- `request_carries_subagent_type_through_route` / `request_defaults_to_general_purpose_type` — 路由侧透传

**验证**：`cargo test --lib` 664 passed / 10 failed，失败全部是 CLAUDE.md 记录的既有失败（`checkpoint_manager` 这轮 4 个全挂，是它的 cwd 依赖老毛病；其余 6 个名单完全一致），我改的模块一个没挂。`cargo fmt --all --check` 0 diff。clippy 在新代码上无新增命中。

### 已知边界

- 通用路径（`GeneralPurpose` 等）的单测受 `ToolRegistry not initialized` 限制起不来，只覆盖了输入侧判定；explorer 派发本身是真跑通的
- `run_explore` 上报 0 token / 0 tool_use——检索本身不产生这些统计，UI 行会显示检索开始/完成的 last_tool_info
- `skill: "explore"` 不再注册到 SkillTool；想触发检索走 `Agent` 工具的 `subagent_type: "explorer"`

---

## 参考来源

- [TaskListV2.tsx 源码](https://github.com/zackautocracy/claude-code/blob/4b9d30f7/src/components/TaskListV2.tsx)
- [How Claude Code works — Task system（优先级排序、30s TTL、5s 收起）](https://notes.tsukino.dev/99-%E5%B7%A5%E5%85%B7%E4%B8%8E%E5%8F%82%E8%80%83/repos/how-claude-code-works/en/docs/15-task-system)
- [CCB issue #58297 — agents view 状态应由 turn 生命周期驱动（v2.1.141 修复）](https://github.com/anthropics/claude-code/issues/58297)
- [CCB issue #60095 — 子代理退出后残留 Running，父会话无终止事件](https://github.com/anthropics/claude-code/issues/60095)
- [CCB issue #53603 — spinner 应只在 in_progress 时渲染任务文本](https://github.com/anthropics/claude-code/issues/53603)
- [Claude Code docs — Agents（v2.1.198 子代理默认后台）](https://code.claude.com/docs/en/agents)
