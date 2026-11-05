# 用户 SKILL.md frontmatter 语义 —— 实现说明

- 日期：2026-11-05
- 前提：内置 skill 已下线，用户 SKILL.md 目录已接通
- 语义基准：Claude Code 官方文档 <https://code.claude.com/docs/en/skills>

---

## 1. 做了什么

把用户 SKILL.md 的 frontmatter 从「解析出来但不用」变成**真的生效**。

对齐的字段（官方 frontmatter 参考的全部非展示字段）：

| 字段 | 效果 | 落点 |
|---|---|---|
| `disable-model-invocation` | 模型不得自动调这个 skill | 工具描述过滤 + 调用时拒绝 |
| `user-invocable` | 用户不可显式调 | 解析 + 记录（见 §5 的说明） |
| `allowed-tools` | 本轮免确认放行 | `PolicyEngine` 的 per-turn 表 |
| `disallowed-tools` | 本轮从可用池移除 | 同上 |
| `model` | 本轮剩余部分换模型 | `model_switch` 共享槽 |
| `effort` | 本轮思考力度 | `llm::thinking` 的 per-turn 档位 |
| `context: fork` | 在隔离子代理里跑 | `AsyncSubagentRunner` |
| `agent` | fork 时的子代理类型 | `SubagentType` 归一 |
| `background` | fork 是否后台（默认 true） | fork 分支 |
| `arguments` | 具名/位置参数替换 | `substitute_skill_arguments` |
| `shell` | `!cmd` 用哪个 shell；`0` = 关闭 | `execute_shell_in_prompt_with` |
| `description` / `when_to_use` / `argument-hint` / `paths` | 已有或纯展示 | — |

---

## 2. 两个正交的调用闸门

Claude Code 的核心设计（官方表格）：

| frontmatter | 用户可调 | 模型可自动调 | description 是否进上下文 |
|---|---|---|---|
| （默认） | ✓ | ✓ | 常驻 |
| `disable-model-invocation: true` | ✓ | ✗ | **不进** |
| `user-invocable: false` | ✗ | ✓ | 常驻 |

StarCode 的对应实现：

- **`disable-model-invocation`** —— 该 skill 不进 `SkillTool` 的可用清单（模型看不到），且被调用时直接拒绝并提示用 `/skills show`。
- **`user-invocable`** —— 解析并记录在 `SkillMetadata::user_may_invoke()`，但**当前不产生行为差异**：StarCode 的用户 skill 不在 `/` 菜单里（没有 `/name` 入口），所以「用户不可显式调」没有可作用的对象。

  这一点是**如实标注**的，不是「实现了但没测」—— 把用户 skill 接进 `/` 菜单是新功能，不属于本次的语义对齐。

---

## 3. 运行时效果的落点

### 3.1 `allowed-tools` / `disallowed-tools` → per-turn 授权表

`PolicyEngine` 新增两张表（`src/core/policy/policy_engine.rs`）：

```rust
turn_grants: HashSet<String>,   // 本轮免确认
turn_denials: HashSet<String>,  // 本轮禁用
```

**与 `session_allowances` 的区别是生命周期** —— 后者是整个会话（「总是允许」），前者只到下一条用户消息。

判定顺序（在 `PolicyEngine::check` 里）：

```
1. deny 规则                    ← 用户写的，压一切
2. disallowed-tools（本轮禁用）   ← 比「本会话总是允许」更具体、更当下
3. 会话「总是允许」
4. allowed-tools（本轮免确认）    ← 压审批模式，但**不压**显式 ask 规则
5. 审批模式（yolo / plan / default / acceptEdits）
6. allow 规则
7. ask 规则                     ← 「每次都问我」压过 skill 的免确认
8. 旧 PolicyRule 列表 → default_decision
```

**清空时机**：`agent_run.rs` 的 `run_stream` 开头（每条用户消息的起点）调
`bus.clear_turn_tool_policy()`。

**为什么走 `PolicyEngine` 而不是另开一条路**：这样 skill 的授权与用户写在
`settings.json` 里的 `allow`/`deny`/`ask` 规则**在同一个入口汇合**，不会出现
「skill 说能用、用户说 deny，结果照样用了」。

### 3.2 `model` → 模型切换共享槽

`PendingModelSwitch` 的槽原来由 `Agent::new` 各自新建 —— skill 和用户
`/model` 会互相覆盖。改成放在 `RuntimeServices` 上（`model_switch_cell`），
`Agent` 与 `SkillTool` 共用同一个。

生效时机天然正确：`execute_turn` 在**每次 provider 请求前** drain 这个槽，
所以 skill 注入后，同一轮的下一次请求就用新模型 —— 正是「本轮剩余部分」。

`model: inherit` = 保持当前模型（CC 的约定）。

### 3.3 `effort` → per-turn 档位

`llm::thinking` 新增 `TURN_EFFORT`（进程级原子量），优先级：

```
本轮（skill 的 effort） > 会话（用户 /effort） > STAR_THINKING_EFFORT > Off
```

与 `allowed-tools` 同一个清空时机。

### 3.4 `context: fork` → 隔离子代理

`SkillTool` 新增 `async_runner`（与 `AgentTool` **共用同一个**
`StarAgentRunner` —— 两处各建一个会让子代理注册表分家，`SendMessage` 就找不到人）。

- `background: true`（默认）→ `spawn_background`，立刻返回，结果随通知回流
- `background: false` → `run_foreground`（**新方法**），本轮等结果

skill 正文成为子代理的 prompt；子代理**看不到**当前对话历史（CC 语义）。

`agent` 字段走展示名 → serde slug 的归一（`Explore` → `explorer`、
`general-purpose` → `general_purpose`）。

---

## 4. 顺带修掉的两个真 bug

### 4.1 `allowed-tools: Bash Read Write` 没被切开

`parse_inline_csv` 原来**只按逗号切**。CC 的口径是「space- or comma-separated
string, or a YAML list」。

所以 `allowed-tools: Bash Read Write` 被当成一个叫 `Bash Read Write` 的工具 ——
授权**静默失效**（工具名对不上，免确认不生效，用户还在弹窗里点）。

修法：按 `,`、空格、Tab 三种分隔符切。

### 4.2 裸 `$name` 没被替换

`substitute_parameters` 只处理 `${argName}`（带花括号），而 CC 的写法是裸
`$name`。修的时候加了边界：

- 只替换**声明过的**参数名 —— 否则会顺手把正文里的 shell 变量（`$PATH`）也换掉
- 参数名后面不能紧跟标识符字符 —— `$issue_id` 不被 `$issue` 截胡
- 位置参数同理：`$10` 不被 `$1` 截胡

---

## 5. 两个「如实标注」而非假装实现的点

1. **`user-invocable` 不产生行为差异** —— StarCode 的用户 skill 不在 `/` 菜单，
   没有「用户显式调用」这个入口可作用。字段解析了、API 提供了
   （`SkillMetadata::user_may_invoke()`），但没有消费方。

2. **`when_to_use` / `argument-hint` 仍是纯展示** —— 前者本该参与「模型何时
   自动调这个 skill」的判断，但 StarCode 的自动触发是**规则驱动**的
   （`tool_routing/triggers.rs`），不是模型读描述后自己决定。把 `when_to_use`
   接进触发器是另一个功能。

---

## 6. 验证

```
cargo check --workspace --all-targets --locked   → exit 0
cargo fmt --all -- --check                       → clean
cargo clippy --workspace --all-targets           → 改动文件零警告
cargo test --workspace --lib -- --test-threads=1 → 1966 passed / 0 failed / 4 ignored
```

新增 **25 条**测试：

### `src/agent/skills/loader.rs`（12 条）

| 测试 | 锁住什么 |
|---|---|
| `booleans_accept_the_documented_spellings` | `true/false/yes/no/on/off/1/0` 任意大小写 |
| `user_invocable_defaults_to_true` | 默认 true —— 写错默认值会静默藏起用户的 skill |
| `an_unrecognized_boolean_is_not_false` | `maybe` 不是 false |
| `the_two_invocation_gates_are_orthogonal` | 两个闸门互不影响 |
| `tool_lists_accept_every_documented_form` | 空格/逗号/YAML 列表三种写法 |
| `fork_defaults_follow_claude_code` | `context: fork` + `background` 默认 true |
| `scalar_fields_are_parsed` | `model` / `agent` / `shell` |
| `frontmatter_must_start_on_the_first_line` | `---` 必须是第一行 |
| `arguments_are_substituted_by_position` | `$name` / `$1` / `$ARGUMENTS` |
| `a_missing_argument_leaves_the_placeholder` | 缺参数时占位符留着 |
| `no_declarations_means_no_substitution` | 无声明则不替换 |
| `shell_zero_disables_command_execution` | `shell: 0` 不执行 `!cmd` |

### `src/core/policy/policy_engine.rs`（7 条，`turn_grant_tests`）

| 测试 | 锁住什么 |
|---|---|
| `allowed_tools_skip_the_confirmation` | 免确认生效 |
| `tools_outside_the_grant_still_ask` | 不在名单里的照常确认（授权≠限制） |
| `disallowed_tools_beat_the_session_allowance` | 本轮禁用压过「总是允许」 |
| `an_explicit_ask_rule_beats_the_skill_grant` | 显式 `ask` 压过 skill 免确认 |
| `an_explicit_deny_rule_beats_the_skill_grant` | 显式 `deny` 压过 skill 免确认 |
| `the_grant_clears_with_the_next_user_message` | 下一条消息即清 |
| `granted_tool_names_are_canonicalized` | `bash` ≡ `Bash` |

### `src/agent/tools/skill.rs`（13 条）

其中 8 条是 frontmatter 端到端效果（走**生产接线**：真实 `MessageBus` →
`PolicyEngine::check`）：

| 测试 | 锁住什么 |
|---|---|
| `allowed_tools_take_effect_through_the_message_bus` | 真的免确认 |
| `disallowed_tools_take_effect_through_the_message_bus` | 真的禁用 |
| `the_grant_is_cleared_for_the_next_user_message` | 真的会被清 |
| `a_gated_skill_is_not_offered_to_the_model` | gated skill 不进模型清单 |
| `model_inherit_does_not_switch` | `inherit` 不换模型 |
| `a_model_field_publishes_a_switch` | `model` 发布切换 |
| `an_effort_field_sets_the_turn_effort` | effort 压会话档位、下一条消息后复原 |
| `the_agent_field_maps_to_a_subagent_type` | 展示名 → serde slug |

---

## 7. 关键代码位置

```
src/agent/skills/loader.rs:34        SkillMetadata（补 6 个字段）
src/agent/skills/loader.rs:97        SkillMetadata 的手写 Default（user_invocable: true）
src/agent/skills/loader.rs:376       substitute_parameters（裸 $name + 边界）
src/agent/skills/loader.rs:483       render_skill_prompt（shell 开关）
src/agent/skills/loader.rs:707       parse_inline_csv（空格也当分隔符）
src/agent/skills/loader.rs:729       is_truthy（CC 的布尔口径）
src/agent/skills/loader.rs:741       is_falsy
src/core/policy/policy_engine.rs:49  turn_grants / turn_denials
src/core/policy/policy_engine.rs:397 check() 的八级判定顺序
src/core/confirmation_bus/message_bus.rs:118  apply/clear_turn_tool_policy
src/agent/agent_run.rs:121           每条用户消息清空本轮策略
src/llm/thinking.rs:44               TURN_EFFORT（本轮档位）
src/core/config/runtime_services.rs:24  model_switch_cell（Agent 与 SkillTool 共用）
src/agent/tools/skill.rs:61          工具描述只列模型可调的 skill
src/agent/tools/skill.rs:367         disable-model-invocation 闸门
src/agent/tools/skill.rs:392         context: fork 分支
src/agent/tools/skill.rs:754         apply_skill_runtime_effects
src/agent/tools/skill.rs:826         run_skill_forked
```

---

## 8. 遗留

**内置 skill 删除后，`src/agent/skills/resources/*.md`（6 个）仍在** ——
它们是 `/extension` marketplace 的**安装模板**，装到
`.star/extensions/skills/<name>/SKILL.md` 后成为用户自定义 skill。现在
`load_user_skill_from_dirs` 会找到它们（之前连它都加载不了）。

**`effort` 与 `self_tuning` 的边界**：`STAR_SELFTUNING_*` 那套是「模型自己
调」的语义，与 skill 的「用户写死的 per-turn 覆盖」不同。这里实现的是后者，
两者不冲突但也没有合并 —— 模型自己调的那套仍然是 session 级。
