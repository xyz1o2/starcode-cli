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

---

# 附二：工具显示名（方案 C）

## 12. 问题

`tool_display_name` 把注册名映射成聊天区的短标签。四个问题：

1. **丢信息** —— `Edit` / `SmartEdit` / `MultiEdit` 全显示成 `edit`，
   用户看不出这次改的是整文件替换、智能定位还是批量多处。
2. **显示的名字在项目里不存在** —— `view` / `bash` / `search` 在 schema、
   文档、**权限规则**里都没有；用户想加规则要写 `Read(...)`。
3. **两套表互相不一致** —— 聊天行 `view`，状态行 `read file`；两张表都要手工维护。
4. **`complete_task` 是明确残留** —— commit `f50c171` 的提交信息写着
   *"no legacy-name compatibility: every "Todo"/"complete_task" match arm
   (registry alias table, tool routing shortlist, catalog, stream reload,
   **formatters, renderer**) now matches only "TodoWrite""*，
   但这条 arm 在 formatter 里活了下来。
   `find_by_name` 同类 —— 全仓从未有过对应的工具实现。

## 13. 最终决定：方案 C

保留 Claude Code 风格的短标签观感，但修掉上面两个实质问题：

| 项 | 处理 |
|---|---|
| `Edit` / `SmartEdit` / `MultiEdit` | 拆开：`edit` / `smart edit` / `multi edit` |
| `complete_task` | 删掉（项目早已硬切到 `TodoWrite`） |
| `find_by_name` | 删掉（全仓从未有过工具实现） |
| 其余（`view` / `bash` / `search` / `glob` / `write` / `plan` / `Update Todos`） | 保持现状 |
| 状态行 `format_tool_name_for_status` | 补上 `SmartEdit → "smart edit"`、`Edit → "edit file"`，与聊天行对齐 |

同时给 `tool_display_name` 补了文档，把"两条规矩"写明：

1. **不合并语义不同的工具**；
2. **不写已废弃的旧名**。

## 14. 验证

```text
cargo fmt --all -- --check                        通过
git diff --check                                  通过
cargo check --workspace --all-targets --locked 通过
ui::utils::format 8 passed（含 4 个新增）
tool_name_consistency 8 passed
全量 lib 1871 passed / 37 failed / 3 ignored
集成测试（含 UI 快照）全通过
```

37 个失败与改动前完全一致（Windows 用户 profile 隔离问题），无新增失败。

---

# 附三：TUI 闪屏（附三：速率受控揭示）

## 15. 现象

> tui 上的输出老是一块一块输出的，看着一闪一闪的。��以像调音软件音量那样，
> 快慢显示，人看起来会更舒服 —— 不管 LLM 流返回的快慢，也不应该闪屏。

参照物是 Claude Code v2.1.308 的界面（见 `.image/2026-11/`）：整段正文**一次
性铺开**，不做逐字动画；进度只由 spinner 行表达
（`✶ Wandering… (46s · ↓ 217 tokens)`）。

## 16. 根因：三个问题叠加

### ① 打字机进度没跟着索引位移（头号成因）

工具结果插到 `chat_history` 中间时（每次工具调用都插，`handle_tool_result_message`
走 `chat_history.insert(insert_pos, entry)`），后面那条正在流式的消息索引 +1。

而打字机进度 `TypewriterState::shown` 是**按条目索引**存的，`index_shift.rs` 只
位移了 `rendered_cache` / `last_rendered_stream_key` / `streaming_height_floor`，
**漏了打字机**。

于是 `preview()` 查不到这条的进度 → 走 `Cow::Borrowed(content)` 分支 →
**返回全文** → 整段话一帧蹦出来。

这就是"一块一块"的直接来源：每次工具调用后闪一次。

### ② 帧间隔无上限

```rust
let dt = self.last_tick.map(|t| now.duration_since(t).as_secs_f32()).unwrap_or(0.0);
```

`advance()` 只在动画帧里跑（`is_animating`）。思考阶段没有流式输出 → 打字机不推进
→ `last_tick` 停在几秒前。流式一开始，第一个 `dt` 是好几秒：

```text
credit += 90 cps × 6 (sprint 档) × 5 s = 2700 字符
```

→ 一帧揭示 2700 字。

### ③ 收尾时直接丢弃进度

`finalize_entry_streaming` 原本调 `typewriter.flush(idx)` —— 流一结束就把剩余文字
整块弹出来，屏幕上最后一段是"啪"一下出现的。

## 17. 修复：音量表模型

参照 VU 表指针：**位置平滑逼近目标，速度有上限，到点自然停**。

| 约束 | 实现 |
|---|---|
| 帧间隔有上限 | `MAX_FRAME_DT = 100ms` —— 停了很久也只多揭示 100ms 的量 |
| 单帧步长有上限 | `MAX_CHARS_PER_FRAME = 48` —— 任何一帧的视觉增量都有界 |
| 进度跟着索引位移 | `shift_indices_after_insert` / `shift_indices_after_removal` |
| 收尾不丢弃进度 | 流结束后进入"落针"阶段，按同样速率铺完才丢 |
| 落针阶段也要有动画帧 | `is_animating` 纳入 `typewriter.has_pending_reveal()` |
| 预览按进度截（不限流式中） | `message_render` 里去掉 `is_streaming_now` 条件 |

### 改动清单

| 文件 | 改动 |
|---|---|
| `src/ui/state/typewriter.rs` | 帧间隔/步长上限、落针阶段、索引位移方法、`has_pending_reveal` |
| `src/ui/services/stream/index_shift.rs` | 插入/删除时同步位移打字机进度 |
| `src/ui/services/stream/mod.rs` | 收尾不再 `flush` |
| `src/ui/components/message_render.rs` | 预览按进度截（不限流式中） |
| `src/ui/app/runtime/mod.rs` | `is_animating` 纳入打字机积压 |

### 回归测试

```text
a_long_gap_does_not_release_a_big_block_at_once              passed
the_tail_settles_instead_of_popping_in                       passed
a_settled_entry_returns_the_full_text                       passed
the_progress_follows_the_entry_index_when_a_tool_result_is_inserted  passed
large_backlog_is_caught_up_faster                            passed
preview_returns_full_text_after_stream_ends                  passed
```

---

# 附四：Bash 权限（对齐 Claude Code）

## 18. 现状差距

Claude Code v2.1.308 的规则（官方文档）：

> Claude Code recognizes a built-in set of Bash commands as read-only and
> runs them **without a permission prompt in every mode**.

StarCode 的差距：

| | Claude Code | StarCode（改前） |
|---|---|---|
| 只读命令集 | 所有模式都免确认 | **只挂在 `acceptEdits` 模式** |
| `git` 只读子命令 | 认（`status`/`log`/`diff`…） | 不认（整个 `git` 都不放行） |
| auto 模式的兜底 | 分类器**只拦不放**，不回退问人 | 分类器不确定 → **问人** |

## 19. 已改

### ① 只读命令集在所有模式生效

`default` 档位现在也放行只读命令 —— 用户抱怨的"bash 每次都要权限同意"就来自这里。

三条硬性拒绝条件原样保留（重定向 `>`/`<`、命令替换 `` ` ``/`$(`、分隔段逐个判定），
少放行的代价只是多一次确认，不会凭空多出风险面。

### ② 补上只读 `git` 子命令

`git` 整条放行不行 —— 它下面既有 `status` 也有 `push`/`reset --hard`/`clean -fd`。
新增白名单：

```text
status  log  diff  show  blame  describe  shortlog  ls-files
rev-parse  cat-file  grep  watchchanged  annotate
```

`commit` / `push` / `reset` / `clean` / `checkout` / `merge` / `rebase` / `stash`
/ `tag` / `branch` 一律不放行（`git branch -d` 会删分支）。

### ③ auto 模式的兜底语义（未改，列出供决策）

Claude Code 的 auto 模式里，分类器**只做"放行/拦截"，不回退问人** —— 判不出来就
拦截，模型自己换写法。StarCode 现在是"分类器判不出来 → 问人"。

这一条**没改**：改成"拦截"会让分类器故障时整轮任务直接卡死（provider 挂了 →
所有工具调用被拒 → 任务无法推进），比多问一次人更糟。

改动清单：

| 文件 | 改动 |
|---|---|
| `src/types/mod.rs` | `is_readonly_fs_command` 文档更新；新增 `is_readonly_git_command` 与只读 git 子命令白名单 |
| `src/core/policy/policy_engine.rs` | `default` 档位接入只读命令集 |

### 回归测试

```text
default_mode_allows_readonly_commands    passed
default_mode_still_asks_for_mutating_commands  passed
accept_edits_allows_edits_and_readonly_commands_only  passed
policy_engine 全部 19 passed
```

## 20. 验证

```text
cargo fmt --all -- --check                        通过
git diff --check                                  通过
cargo check --workspace --all-targets --locked 通过
全量 lib 1881 passed / 37 failed / 3 ignored
```

37 个失败与改动前完全一致（Windows 用户 profile 隔离问题），无新增失败。

---

# 附五：CodebaseSearch 的「找词不对」

## 21. 现象

```text
● CodebaseSearch(缺点)
  ⎿  The cross-file search index for '/mnt/h/test/ARTEX' is still building in the background
     (first build; typically finishes in seconds).
     No hits this turn — this is a cold index, not an empty result. Re-run `CodebaseSearch`
     with the same keywords once before giving up on it.
● CodebaseSearch(缺点 · retry)
  ⎿  CodebaseSearch · building index
```

两个问题：query 是中文词（必然零命中），而且工具让模型"用同样的词再搜一遍" → 死循环。

## 22. 地面真相（ARTEX 调试日志）

```text
[15:44:56] [ACE_AUTO] selected trigger CodebaseSearch (score=75, reason=ranked_cross_file_locate_shape)
[15:45:04] [ACE_REWRITE] query rewrite timed out after 8000ms
[15:45:05] [ACE_AUTO] CodebaseSearch cold-start warmup wait ready=true waited_ms=8000
[15:45:07] [ACE_AUTO] CodebaseSearch auto-run satisfied=false
```

三个缺陷叠加：

| # | 缺陷 | 机制 |
|---|---|---|
| **A** | 改写超时 → 回退 bigram → 中文 query 照发 | `rewrite_search_query` 8 秒超时 → `extract_search_keywords` 把中文切成 bigram（"缺点" → "缺点"） |
| **B** | 语言闸门过宽 | 判据是"索引里有没有**任何**中文 token"。ARTEX 有中文注释 → `has_cjk=true` → 放行，但"缺点"根本不在索引里 |
| **C** | 等就绪与工具判就绪不同源 | 等的是**进程级** `watcher::is_engine_ready()`；工具查的是**按 root + cache key** 且 mtime 对得上的引擎。`ready=true` 之后补跑仍报 "still building" |

## 23. 调研：参考实现怎么传参

| 工具 | 参数 | 检索方式 |
|---|---|---|
| Augment `grep-search` | `query`="Text or regex to search for" + `paths`/`include_globs`/`exclude_globs`/`max_results` | **词法**，调用方给代码文本 |
| Augment `codebase-retrieval` | `information_request`="A natural-language description…" | **语义**，独立引擎 |
| Windsurf FastContext | `query`=自然语言 + `project_root`/`exclude_paths`/`tree_depth` | 自然语言 → 后端模型决定搜什么 |
| Controller `codebase-search` | `query`="Natural language search query…" + `path` | 语义 |

关键结论：**这两个参考实现都不把自然语言喂给词法引擎**。Augment 干脆分成两个工具：词法的那个要"代码文本"，语义的那个才收自然语言。Windsurf 的自然语言进的是**后端模型**，不是倒排表。

而 StarCode 的 `CodebaseSearch` 是**词法**引擎（项目既定路线：不引入 embedding）。所以：

> ★ 入参契约必须保持"代码侧检索词"。让词法工具接受自然语言是**契约撒谎** ——
> 翻译是猜的，会超时（本例 8 秒），猜错了照样零命中。

所以我**撤回了**"入参接受自然语言"的改动：schema 与工具描述恢复成"code-side keywords"，并继续要求模型自己给出代码词汇。

## 24. 真正的修法

### ① 闸门改用**覆盖率**而不是"有没有中文 token"

判据从"索引里有没有中文 token"（过宽）换成"**这组词本身在不在索引里**"：

```rust
// src/agent/tool_routing/triggers.rs
if let Some(coverage) = term_coverage {
    if coverage <= 0.0 {
        return Err("language gate: query keywords are not in the index at all ...");
    }
}
```

覆盖率来自引擎已有的 `term_coverage()`（倒排表命中率），新增 `shared_root_term_coverage` 从缓存取。

**真实 ARTEX 验证**：

```text
ARTEX 索引: files=639
ARTEX: has_cjk=Some(true)  coverage("缺点")=Some(0.0)
闸门对 "缺点": allows=false
```

旧逻辑：`has_cjk=true` → 放行 → 发一次必然零命中的检索。
新逻辑：覆盖率 0 → 拦下。

### ② 改写失败就不发这一发

自动触发路径原来"改写失败 → 回退 bigram → 照发"。现在改成**改写失败就不触发**：

```rust
let rewritten = rewrite_search_query(&self.client, user_input).await;
let query = match rewritten {
    Some(terms) => terms,
    None => { /* 记日志，跳过触发 */ return TurnResult::Done; }
};
```

少一次检索好过给模型一个必然为空的答案 —— 空结果会被它当成「这个功能不存在」。

### ③ 等就绪与工具判就绪同源

`wait_for_index_ready` 改成按 root 查缓存（和 `resolve_engine` 完全相同的判据），不再等进程级标志：

```rust
pub fn root_engine_ready(root: &Path) -> bool { /* 缓存命中 + mtime 对得上 */ }
```

现场日志 `ready=true waited_ms=8000` 却仍 "still building" 的矛盾因此消失。

### ④ 工具零命中时区分"换个词"与"再搜一遍"

`is_cjk_only_query` 让冷启动路径（Warming）与 ripgrep 兜底路径都给出可执行的下一步：

```text
No hits — and the query itself is in Chinese, while the index holds English identifiers,
paths and English comments. Retrying the same Chinese words cannot match: pass code-side
terms instead ...
```

（原来的"重跑同样的词"对中文 query 是**有害建议**。）

### ⑤ 兜底：模型自己调时也不漏

`ToolExecutor` 保留 LLM client，在**工具入口**（`normalize_tool_args`）给 `CodebaseSearch` 的 query 做一次归一，模型绕过自动触发直接调也走同一条路。同轮内同一个 query 只翻译一次（缓存），保证补跑搜的是同一件事。

注意：这是**兜底**，不是契约 —— 工具描述仍然要求模型给代码侧词汇。

## 25. 改动清单

| 文件 | 改动 |
|---|---|
| `src/core/context/search_cache.rs` | 新增 `shared_root_term_coverage` / `root_term_coverage` |
| `src/agent/tool_routing/triggers.rs` | 闸门加覆盖率判据；新增 `codebase_search_language_gate_detail` |
| `src/agent/agent_tool_process.rs` | 改写失败即跳过触发；等待按 root |
| `src/core/tools/codebase_search.rs` | `wait_for_index_ready(root, …)` / `root_engine_ready` / `seed_shared_engine`；`is_cjk_only_query` 零命中诊断 |
| `src/agent/tool_executor.rs` | 保留 client；工具入口归一 CodebaseSearch query |

## 26. 回归测试

```text
the_language_gate_blocks_cjk_terms_that_are_not_in_the_index   passed
the_language_gate_allows_cjk_terms_that_are_in_the_index       passed
a_chinese_codebase_search_query_is_normalized_at_the_tool_entry passed
a_code_side_query_is_passed_through_untouched                 passed
codebase_search 41 passed
```

## 27. 仍然存在的边界

1. **翻译是猜的**。即使改写成功，"缺点" → `limitation / drawback / weakness` 也可能和仓库里的实际用词（`gap` / `known_issues` / `TODO`）不重合。这是词法引擎 + 不引入 embedding 的必然结果 —— 真要解决需要语义索引（Windsurf 的 Semble / Augment 的 Context Engine）。
2. **改写要 8 秒**且会超时。修法是**跳过**而不是硬扛。
3. **缺范围参数**。Augment `grep-search` 有 `paths`/`include_globs`/`exclude_globs`，Windsurf 有 `exclude_paths`；StarCode 只有 `path`。大仓库里"缩小范围"是最有效的提速手段，这条值得补。

---

# 附六：把「猜」变成「挑」（词表锚点）

## 28. 上一版留下的问题

附五修好了「不该发的检索」与「死循环」，但**翻译本身还是猜的**：

> "缺点" → `limitation` / `drawback` / `weakness`

这三个词很可能**一个都不在 ARTEX 的索引里** —— 于是改写"成功"了，检索照样零命中。

## 29. 参考实现怎么解决

Windsurf FastContext 的检索不是"给一句自然语言 → 搜"这么简单，它的管线是：

```text
1. Local Semantic Prefetch      → 预取本地候选 chunk
2. Lexical Anchoring            → 从本地仓库抽出"确切的候选文件名、路径 token、字面词"
3. Hotspot Repo Mapping         → BM25F 给目录打分，构造聚焦的子树
4. Remote Verification          → 把本地 chunk、anchors、hotspot map 一起交给后端模型
```

关键在第 2 步和第 4 步：**把仓库自己的词表摆进检索提示**，让模型在**这个仓库的命名里挑**，而不是凭空造词。

## 30. 实现

### 引擎侧：取词表

`SearchEngine::identifier_vocabulary(limit)` —— 从倒排表里挑出"长得像标识符"的词（含 `_` 或含大写字母），按文档数排序：

```rust
token.chars().count() >= 4
    && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    && (token.contains('_') || token.chars().any(|c| c.is_ascii_uppercase()))
```

纯英文单词（注释里的 the/handle/error）不算 —— 那是词，不是这个仓库的**命名**。

缓存侧：`shared_root_identifier_vocabulary(root, limit)`。

### 改写侧：把词表摆进提示

```rust
"Request: {user_input}

Repository identifiers (prefer these when they fit):
task_id, created_at, finding_id, report_finding, last_error, ...

Terms:"
```

system prompt 同步改口："If a list of repository identifiers is given, prefer terms from it — those are the words this codebase actually uses."

### 真实 ARTEX 验证

```text
ARTEX 索引: files=639
锚点=60
样例: task_id, created_at, root_domain, exploration_id, finding_id, tool_use,
      asset_ids, conversation_id, output_tokens, input_tokens, last_error,
      llm_profile_id, report_finding, cache_read_tokens, agent_key, ...
```

模型译"缺点"时能看到 `finding_id` / `report_finding` / `last_error` —— 而不是编一个 `weakness`。

### 修一个我自己引入的回归

附五里把 `rewrite_search_query` 的 `None` 当成"改写失败"，于是**所有英文请求的自动触发都被跳过了** —— 英文是主要使用场景，等于把功能改没了。

根因：`rewrite_search_query` 对"不需要改写"也返回 `None`（`needs_rewrite == false` 时提前 return None），与"改写失败"的 `None` **不是一回事**。

修法：抽 `build_auto_search_query_in`，先判 `needs_rewrite`：

```rust
if !needs_rewrite(user_input) {
    return Some(extract_search_keywords(user_input));   // 英文/代码侧 → 规则压缩
}
match rewrite_search_query_in(client, user_input, project_root).await {
    Some(terms) => Some(terms),
    None => None,   // 中文 + 改写失败 → 不发
}
```

回归测试 `an_english_request_still_produces_a_query` 钉住这条。

## 31. 改动清单

| 文件 | 改动 |
|---|---|
| `src/core/context/search_engine.rs` | 新增 `identifier_vocabulary` |
| `src/core/context/search_cache.rs` | 新增 `shared_root_identifier_vocabulary` / `root_identifier_vocabulary` |
| `src/agent/tool_routing/query_rewrite.rs` | `rewrite_search_query_in`（带锚点）、`build_auto_search_query_in`、锚点词表 |
| `src/agent/agent_tool_process.rs` | 自动触发改用带锚点的构造 |
| `src/agent/tool_executor.rs` | 工具入口兜底也带锚点 |

## 32. 验证

```text
cargo fmt --all -- --check                        通过
git diff --check                                  通过
cargo check --workspace --all-targets --locked 通过
query_rewrite 11 passed（含英文请求回归测试）
全量 lib 1884 passed / 37 failed / 3 ignored
```

37 个失败与改动前完全一致（Windows 用户 profile 隔离问题），无新增失败。

## 33. 剩下的边界

1. **锚点只覆盖高频标识符**。仓库里低频但精准的专有词可能不在前 60 个里 —— 模型仍可能译偏。
2. **词表来自倒排表**（分词后的 token），不是符号表。理论上可以用 CallGraph 的符号表更准，但那是另一条链路。
3. **缺范围参数**（`paths`/`include_globs`/`exclude_globs`）—— 仍然值得补。
