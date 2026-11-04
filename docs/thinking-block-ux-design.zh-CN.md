# Thinking 块交互设计（zh-CN）

- 日期：2026-11-04
- 范围：`∴ Thinking… / ∴ Thought for Xs` 思考块的标签、展开策略、全流程状态流转

## 1. 问题

用户反馈：**"现在是只能看的到它在思考"**。

现象：推理正文（reasoning content）**永远不显示**，除非用户手动点开。用户能看到的只有一行 `∴ Thinking…`，以及输入框上方 spinner 行的 `⠹ Spelunking… (18s · esc to interrupt)`。

也就是说：模型在想什么，用户完全看不到。

## 2. 调研结论

业界一致做法（Claude Code CLI、Cursor、CopilotKit、shadcn/ui、Neon UI、Backstage）：

| 做法 | 说明 | 出处 |
|---|---|---|
| **流式期间自动展开** | 实时看到推理正文，不是只看到一个"正在思考" | Backstage `Reasoning` 组件；CopilotKit "Expands automatically so you can follow the model's thought process in real-time" |
| **结束后自动收起** | 收成一行收据 `Thought for 12s`，延迟约 800ms | Backstage `AUTO_COLLAPSE_DELAY_MS` |
| **手动意图压过自动化** | 三态 override：`null`=未干预 / `true`=强制展开 / `false`=强制收起 | Backstage `userOverride`；Neon UI "The reader's toggle always beats the automation" |
| **计时器实时走动** | `Thinking 5s…` → `Thought for 12s` | Backstage `triggerLabel`；Cursor 论坛反馈 |
| **历史消息默认收起** | 重开会话不闪开 | Backstage "Historical messages start collapsed instead of briefly flashing open" |
| **实时面只留一个** | 不要正文块和全局 spinner 同时说"思考中" | Neon UI / Koder 规范 |
| **展开提示必须常驻可见** | 不能只在 hover 时出现 | Cursor 论坛："the chevron should be visible at all times" |

关键反例（反面教材）：

- Claude Code VSCode 扩展有大量 "thinking block 无法展开" 的 issue（#52046、#51131、#59358、#55120）。根因不是数据缺失，而是**呈现层没有接线**。这说明"折叠后藏着"这条路上，每个实现都会踩坑。
- Koder 规范的 R4 明确写：*"If user already expanded manually: preserve expanded state"*。

## 3. 根因

StarCode 的展开判定散在渲染层就地计算：

```rust
// src/ui/components/message_render.rs（改前）
let is_expanded = state.is_transcript_mode
    || state.expanded_thinking_indices.contains(&entry_idx);
```

`expanded_thinking_indices` 只在**用户手动点开**时才会被写入 —— 自动化行为无处安放。因此折叠是唯一的常态，推理正文永远藏着。

另外两个附带问题：

1. **计时基准错了**：冻结耗时用的是回合级的 `state.processing_started_at`。一轮里有两个思考块（工具调用前后各一个）时，两个块会显示出同一个"从回合开始算"的时间。
2. **没有中间态**：只有"进行中（无耗时）"和"已完成（有耗时）"，没有"进行中 + 实时耗时"。

## 4. 新设计

### 4.1 状态流转

```text
        收到首个 reasoning delta
                 │
                 ▼
    ┌────────────────────────────┐
    │  流式中：自动展开           │  ∴ Thinking 7s…   + 正文实时可见
    │  reasoning_started_at 记录 │  ← 计时基准 = 这个块自己的起点
    └────────────┬───────────────┘
                 │ 流式结束 / 工具调用 / 正文开始
                 ▼
    ┌────────────────────────────┐
    │  冻结耗时，排定延迟收起     │  800ms 内仍保持展开
    └────────────┬───────────────┘
                 │ 延迟到期
                 ▼
    ┌────────────────────────────┐
    │  收起成一行收据             │  ∴ Thought for 14s (ctrl+o to expand)
    └────────────────────────────┘

用户手动点击（任何时刻）：
    展开 ↔ 收起，并写入 thinking_manual_override
    → 此后该块的自动行为永久失效
```

### 4.2 展开判定：单一入口

新增 `ChatState::is_thinking_expanded(entry_idx, is_streaming)`，优先级从高到低：

1. **transcript 模式** → 展开（"看全部原始输出"语义最高）
2. **用户手动干预过** → 按用户的意思（`Some(true)`/`Some(false)`）
3. **流式进行中** → 展开
4. **其余** → 收起

渲染层、鼠标事件、测试全部走这一个入口，不再各自就地判断。

### 4.3 三态手动覆盖

```rust
pub thinking_manual_override: HashMap<usize, bool>
```

为什么不是布尔或只用 `expanded_thinking_indices`：

- 布尔表达不了"用户没管过"；
- `expanded_thinking_indices` 混装了"自动展开的"和"用户展开的"，自动化无法区分 —— 下一轮就会把用户展开的块收掉。

### 4.4 标签文案

| 状态 | 标签 | 展开提示 |
|---|---|---|
| 流式中 | `∴ Thinking 7s…` | 无（本来就开着） |
| 已完成 | `∴ Thought for 14s` | `(ctrl+o to expand)` |
| 历史恢复（无基准） | `∴ Thought` | `(ctrl+o to expand)` |
| 取消中 | `∴ Canceling…` | — |

进行中带实时耗时，是"在推进"和"卡住了"的唯一可辨信号 —— 只有 `Thinking…` 的话两者长得一模一样。

### 4.5 计时基准

新增 `ChatEntry::reasoning_started_at`（`#[serde(skip)]`，不落盘）：

- 第一次收到该块的 reasoning 正文时记录；
- 冻结耗时优先用它，回落到回合级 `processing_started_at`；
- 一轮多个思考块时，各自显示各自的思考时长。

## 5. 改动清单

| 文件 | 改动 |
|---|---|
| `src/ui/state/store/types.rs` | 新增 `THINKING_AUTO_COLLAPSE_DELAY`（800ms） |
| `src/ui/state/store/mod.rs` | 新增 `thinking_manual_override` / `thinking_collapse_at`；新增 `is_thinking_expanded` / `toggle_thinking_expansion` / `auto_expand_thinking` / `schedule_thinking_collapse` / `tick_thinking_collapse`；transcript 关闭时保留手动展开的块 |
| `src/types/mod.rs` | 新增 `ChatEntry::reasoning_started_at` |
| `src/ui/components/message_render.rs` | 展开判定改走统一入口；进行中标签带实时耗时；新增视觉核对测试 |
| `src/ui/services/stream/mod.rs` | reasoning/ thinking delta 首次到达时记录计时起点并自动展开；`finalize_entry_streaming` 用块级基准冻结耗时并排定延迟收起 |
| `src/ui/app/runtime/mod.rs` | 每帧 tick 延迟收起（到期则重绘） |
| `src/ui/events/mouse.rs` | 点击走 `toggle_thinking_expansion`（记录手动意图）；展开判定带上流式状态 |
| `src/ui/services/stream/index_shift.rs` | 插入/删除条目时同步位移手动覆盖表与待收起索引 |
| `src/ui/components/chat_history.rs` | `DocumentKey` 纳入手动覆盖表（否则改了状态屏幕不刷新） |

## 6. 验证

### 渲染核对（`target/thinking_states.txt`）

```text
状态 1：流式中（自动展开）
∴ Thinking 7s…
  先看看项目结构。

  再确认工具调用。

这是最终回答。▌

状态 2：已完成（自动收起成一行收据）
∴ Thought for 14s (ctrl+o to expand)

已经想完了。

状态 3：历史恢复（无耗时基准）
∴ Thought (ctrl+o to expand)

恢复的历史。

状态 4：用户手动收起后（流式中）
∴ Thinking 7s… (ctrl+o to expand)

这是最终回答。▌

状态 5：完成态手动展开
∴ Thought for 14s
  推理正文在这里。

已经想完了。
```

### 回归测试

```text
ui::state::store::tests
  a_streaming_thinking_block_is_expanded_by_default        passed
  a_finished_thinking_block_collapses_back                 passed
  a_manual_collapse_survives_the_next_streaming_delta      passed
  a_manual_expand_survives_the_auto_collapse               passed
  the_auto_collapse_waits_before_folding                   passed
  turning_off_transcript_keeps_manually_expanded_blocks    passed

ui::services::stream::tests
  streaming_reasoning_auto_expands_the_thinking_block      passed
  a_finished_thinking_block_is_scheduled_to_collapse        passed

ui::components::message_render::tests
  thinking_states_look_right                               passed
```

全量 lib 测试：`1861 passed / 37 failed / 3 ignored`。37 个失败与改动前完全一致（Windows 用户 profile 隔离问题，见 `docs/project-issues-report.zh-CN.md`）。

## 7. 已知边界

1. **展开态正文流式增长仍会改变文档高度**。已有 `streaming_height_floor` 与滚动锚点补偿减轻跳动，但没有做完整的高度动画过渡。
2. **同一轮多个思考块各自计时**。这是修正后的行为（原来全部显示回合耗时），但用户可能预期看到"整轮共想了多久"——那个数字在 spinner 行上（`Spelunking… 18s`）。
3. **延迟收起用固定 800ms**，不做自适应。极长的推理段落里，800ms 只够扫一眼最后一句。
4. **transcript 模式仍是全局开关**（Ctrl+O），不是"展开当前这一块"。业界多数实现也把单块展开交给点击，全局展开是独立入口。

---

# 附：工具名统一（同期改动）

## 8. 问题

工具名在项目里有**三个真相源**、**四套匹配写法**：

1. `core::tools::constants::ToolName` 枚举 —— 注册名的**声明**（唯一应该权威的）
2. `src/constants/tools.rs` —— 一份**手抄**的工具名清单（零用户，而且抄错了：
   `TOOL_LS = "ListDir"`，而注册名早已改成 `LS`）
3. 各处散落的字符串字面量 —— `"WebFetch" | "web_fetch"` 这种别名列表

匹配写法也有四种：

| 写法 | 例子 | 问题 |
|---|---|---|
| 原始名直接比 | `tool_name == "Bash"` | 模型写 `bash` 就漏判 |
| `canonical_tool_name` 后比 | `canonical_tool_name(x) == "Bash"` | 只认精确注册名，`bash` 仍漏判 |
| `to_lowercase()` 后比小写 arm | `matches!(x.to_lowercase(), "bash")` | arm 写成小写，违反"注册名即唯一名" |
| 别名列表 | `"WebFetch" \| "web_fetch"` | 后半个是**死分支**（不是注册名，`get_tool()` 找不到） |

## 9. 统一后的约定

**唯一真相源**：`ToolName` 枚举。手抄清单删除。

**三个归一函数，分工明确**（各有测试钉住）：

| 函数 | 语义 | 用在哪 |
|---|---|---|
| `canonical_tool_name` | **精确**注册名，未知原样返回 | 执行器 —— 要区分"模型拼错了"（报 Unknown tool）和"插件真叫这个"（照常执行） |
| `find_builtin_tool_name` | 忽略大小写**查找**注册名，找不到给 `None` | 用户手输工具名（`/permissions add`、hook matcher） |
| `resolve_registered_tool_name` | 忽略大小写**解析**成注册名，找不到原样返回 | 渲染 / 分类 / 策略判定 —— 这些地方 arm 只写注册名 |

**arm 一律写注册名**（PascalCase），大小写漂移在入口一次性归一：

```rust
let name = resolve_registered_tool_name(&tc.function.name);
match name {
    "TodoWrite" => { /* … */ }
    "WebFetch" | "WebSearch" => { /* … */ }
    _ => { /* … */ }
}
```

## 10. 改动清单

| 类别 | 文件 |
|---|---|
| 删掉手抄清单 | `src/constants/tools.rs`（删）、`src/constants/mod.rs` |
| 归一函数分工 | `src/core/tools/constants.rs`（新增 `resolve_registered_tool_name`、修正 `canonical_tool_name` 的文档） |
| 匹配点归一 | `src/agent/{streaming_executor,enhanced_executor,tool_enhanced,cache_warning,loop_engineering,sensitive_paths,reflection,agent_tool_process,tool_routing/helpers,subagent/progress}.rs` |
| | `src/runtime/{session,stream_chunks}.rs` |
| | `src/ui/{components/tool_render,components/format,components/confirmation_dialog,services/stream/tool_result,state/store/mod}.rs` |
| | `src/core/{auto_mode/classifier,policy/policy_engine,policy/settings_rules,config/runtime_bootstrap/agent_runtime}.rs` |
| | `src/commands/permissions.rs`（`normalize_tool_name` 改为委托统一入口） |
| | `src/types/mod.rs`（`is_safe_query_tool` 内部归一） |

顺带修掉的语义 bug：

- `is_safe_query_tool` 的文档声称"调用方已归一"，但策略引擎拿到的是**模型原样发出的名字** → 只读工具因大小写不同而被当成"要确认的工具"。现在函数内部归一。
- `settings_rules.rs` 自带一个 `canonical()` 包装，用的是不折叠大小写的 `canonical_tool_name` → 注释说"大小写漂移在这里抹平"，实际没抹平。改用统一入口。
- `auto_mode/classifier.rs` 的注释说"入口已由 canonical_tool_name 归一"，但它的调用方并没有归一 → 现在函数内部归一。

## 11. 守卫

`src/core/tools/tool_name_consistency.rs` 从 4 条扩到 8 条：

```text
every_declared_tool_name_is_a_registered_name                       passed
no_declared_name_uses_the_legacy_snake_case_spelling               passed
runtime_probe::registered_tools_use_the_canonical_pascal_case_names passed
the_special_names_elsewhere_are_still_registered                   passed
ui_tool_name_match_arms_have_no_snake_case_builtin_aliases         passed
ui_tool_name_match_arms_use_registered_names                       passed
no_legacy_snake_case_tool_name_is_used_anywhere                     passed
there_is_no_second_handwritten_tool_name_list                       passed
```

关键两条：

- `no_legacy_snake_case_tool_name_is_used_anywhere` —— 扫**整个 `src/`**，
  找"旧 snake_case 拼法被当成工具名用"的地方。跳过 `mod tests`（那里的断言
  **故意**提到旧拼法，证明它不是别名）。
- `there_is_no_second_handwritten_tool_name_list` —— 手抄清单不许复活。

两条都实测过能抓到问题：临时把 `"web_fetch"` 加回 match arm，守卫立刻报出
文件与行号。

## 12. 验证

```text
cargo fmt --all -- --check                        通过
git diff --check                                  通过
cargo check --workspace --all-targets --locked 通过
constants::alias_tests 9 passed
tool_name_consistency 8 passed
全量 lib 1868 passed / 37 failed / 3 ignored
```

37 个失败与改动前完全一致（Windows 用户 profile 隔离问题），无新增失败。
