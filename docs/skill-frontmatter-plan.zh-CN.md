# 用户 SKILL.md frontmatter 语义实现 —— 方案

- 日期：2026-11-05
- 前提：内置 skill 已下线（`b8fa838`/`6ecbffd`），用户 SKILL.md 目录已接通（`7c9eb08`）
- 目标：让用户 SKILL.md 的 frontmatter **真正生效**，语义对齐 Claude Code

---

## 0. 先纠正我上一轮的一个说法

我上一轮说 `load_skills_from_dir` 是「零调用者的死代码」——**不准确**。实际调用者有 4 处：

| 调用点 | 用途 |
|---|---|
| `src/commands/skills.rs:23` | `/reload-skills` 扫描 |
| `src/commands/skills.rs:188,193` | `/skills list` |
| `src/commands/skills.rs:258,263` | `/skills show` |
| `src/commands/parity.rs:3069` | `/context` 统计 |

准确的说法是：**用户侧命令（`/skills list`）能读到 skill，但模型侧（`SkillTool`）读不到** —— 也就是说用户看得见、模型调不着。缺的是「模型可调用」这一段，以及 frontmatter 语义。

---

## 1. Claude Code 的 frontmatter 语义（exa 核实）

官方文档 <https://code.claude.com/docs/en/skills> 的完整参考。**关键设计是「两个正交的调用开关」**：

| frontmatter | 用户可 `/name` | 模型可自动调 | 何时进上下文 |
|---|---|---|---|
| （默认） | ✓ | ✓ | description 常驻，正文调用时才载 |
| `disable-model-invocation: true` | ✓ | ✗ | **description 不进上下文**，正文调用时才载 |
| `user-invocable: false` | ✗ | ✓ | description 常驻，正文调用时才载 |

其余字段：

| 字段 | 语义 | StarCode 现状 |
|---|---|---|
| `description` | 匹配依据 | ✅ 已解析 |
| `when_to_use` | 追加触发语 | ✅ 已解析，**未用** |
| `argument-hint` | 补全提示 | ✅ 已解析，未用 |
| `arguments` | 具名位置参数 `$name` | ✅ 已解析（`SkillArgument`），**未用** |
| `allowed-tools` | 本轮免确认放行，**下条消息即清** | ✅ 已解析，**未用** |
| `disallowed-tools` | 本轮从可用池移除 | ❌ 未解析 |
| `disable-model-invocation` | 模型不得自动调 | ❌ 未解析 |
| `user-invocable` | 不进 `/` 菜单 | ❌ 未解析 |
| `model` | 本轮换模型（`inherit` 保持） | ✅ 已解析，**未用** |
| `effort` | 本轮换 effort | ✅ 已解析，未用 |
| `context: fork` | 在隔离子代理里跑 | ✅ 已解析，未用 |
| `agent` | fork 时的子代理类型 | ❌ 未解析 |
| `background` | fork 是否后台（默认 true） | ❌ 未解析 |
| `paths` | glob 限制自动激活 | ✅ 已解析，**自定义 agent 用了，SKILL.md 没用** |
| `shell` | `!cmd` 用哪个 shell | ❌ 未解析 |
| `hooks` | 本 skill 生命周期钩子 | ❌ 未解析 |
| `metadata` | 自由键值 | ❌ 未解析（CC 也不解释） |

**最关键的两条**（也是用户最想要的）：
1. `disable-model-invocation` —— 「这个 skill 有副作用，别让模型自己决定触发」
2. `allowed-tools` —— 「这个 skill 跑的时候这些工具别老弹窗」

---

## 2. StarCode 缺什么

现状：`SkillTool` 把 skill 正文塞进 `<skill_sandbox>` 标签返回给模型。frontmatter 全部被忽略。

**这不是小修** —— 要让 `allowed-tools` 真正生效，得接进 `ToolExecutor::pass_gate`；要让 `model` 生效，得接进 `model_switch` 的发布机制。

---

## 3. 方案：分三层，每层独立可用

### 第 1 层：解析补全（纯 loader 改动，无运行时耦合）

**改 `src/agent/skills/loader.rs`** —— `SkillMetadata` 补字段：

```rust
pub struct SkillMetadata {
    // ... 现有字段不动 ...
    pub disable_model_invocation: bool,   // 新
    pub user_invocable: bool,             // 新
    pub disallowed_tools: Vec<String>,     // 新
    pub agent: Option<String>,             // 新
    pub background: Option<bool>,          // 新
    pub shell: Option<String>,             // 新
}
```

`parse_frontmatter` 补 key 分支。**布尔值解析要按 CC 的口径**：`yes/no/on/off/1/0`（任意大小写）+ `true/false`。现有的 `is_truthy` 只认 `true`，得扩展 —— 这点要单测锁住。

顺带修一个既有 bug：`parse_frontmatter` 用 `content.strip_prefix("---")` —— CC 明确要求 **`---` 必须是文件第一行**，否则整文件当正文。现在 `strip_prefix` 要求前缀恰好是 `---`，所以 `---\n` 能过但 `--- \n`（带尾空格）也能过，这个其实 OK；但 `parse_frontmatter` **不检查 `---` 是否在第一行**（`strip_prefix` 已经保证了）。真正的问题是 CC 说「YAML 解析失败也要加载，只是字段为空」—— 现在解析失败会 panic 吗？要看 `parse_argument_items`。

### 第 2 层：调用闸门（`disable-model-invocation` / `user-invocable`）

**这是「模型能不能自己调」的控制点。**

CC 的两条规则映射到 StarCode：

| CC | StarCode 对应 |
|---|---|
| `disable-model-invocation: true` | 该 skill **不进模型可调列表** —— 即 `SkillTool` 描述里不列它、模型按名字调也拒绝 |
| `user-invocable: false` | 该 skill **不进 `/` 菜单** —— 但模型能调 |

StarCode 的 `/` 菜单在 `src/commands/system.rs` 的 `ALL_COMMANDS`。用户 skill 目前**不在** `/` 菜单里（只有 `/skills list|show|new`），所以 `user-invocable` 暂时无处可作用 —— 但 `SkillTool` 的「用户直接调」路径存在（`skill` 参数），所以：

- `disable-model-invocation: true` → 模型侧不可见、不可调；用户通过 `/skills show` + 显式 `Skill(skill=...)` 仍可触发
- `user-invocable: false` → 模型侧可见可调（本次不动，因为没有 `/` 菜单入口）

**取舍**：`user-invocable` 在 StarCode 里没有对应的用户入口（用户 skill 不在 `/` 菜单），所以这条字段**先解析、先记录，但暂不产生行为差异**。我会把这一点写进文档，而不是假装实现了。

**第 2 层要做的事**：
1. `list_user_skill_names` 拆成两个列表：`model_callable`（排除 `disable-model-invocation`）与 `all`
2. 工具描述只列 `model_callable`
3. `SkillTool` 执行时，若目标是 `disable-model-invocation` 的 skill 且调用方是**模型**（而非用户），拒绝并说明原因

**怎么区分「模型调用」vs「用户调用」？** —— 这是本层的技术难点。看 `SkillInvocation` 的来源：工具调用都是模型发起的；用户也可以通过 `/skills` 命令触发。StarCode 里用户触发 skill 的路径是 `/skills show`（只显示，不执行）。所以：**目前所有 `SkillTool` 调用都是模型发起的** → `disable-model-invocation: true` 的 skill 应当**完全拒绝执行**（模型路径），用户想用就 `/skills show` 看内容。

这比 CC 更严格，但语义一致（CC 里 `disable-model-invocation` 的 skill 用户能 `/name` 调，模型不能；StarCode 没有 `/name` 入口，所以模型和用户都不能通过 SkillTool 调 —— 用户用 `/skills show`）。

### 第 3 层：运行时效果（`allowed-tools` / `model`）

这层要接运行时，是本次最重的部分。

**`allowed-tools`（本轮免确认）**：

CC 语义：「Tools Claude can use without asking permission during the turn that invokes this skill. The grant clears when you send the next message.」

StarCode 对应机制：`ToolExecutor::pass_gate` → `PolicyEngine::check`。要加一个**会话级、turn 生命周期的临时放行集合**：

```
Skill 执行 → 把 allowed-tools 写进一个 per-turn grant 表
下一条用户消息 → 清空
```

落点：`src/core/policy/policy_engine.rs` 加 `turn_grants: Mutex<HashSet<String>>`（或复用现有的 `CheckResult` 路径），`agent_loop` 每轮开始时清空。

难点：`SkillTool` 返回的是**一段文本**（skill 正文），不是「执行了 skill」。CC 里 skill 是**注入上下文**而非「执行完就结束」——真正跑工具的是模型。所以 `allowed-tools` 的时机是：**skill 正文被注入上下文后的这一轮**，直到用户下一条消息。

这意味着 `allowed-tools` 不能在 `SkillTool::execute` 里一次性写死，而要**持续到本轮结束**。实现：`SkillTool` 执行时把 grant 写进一个 turn 级表；`agent_loop` 每轮清空。写入点必须在 skill 注入之后 —— `SkillTool::execute` 正好是那个点。

**`model`（本轮换模型）**：

CC 语义：「Model to use when this skill is active... for the rest of the current turn. Accepts the same values as `/model`, or `inherit`.」

StarCode 已有 `model_switch::publish(cell, PendingModelSwitch)` —— 但那是**用户** `/model` 的路径，语义是「下一轮生效，不打断当前轮」。skill 的 `model` 语义不同：**本轮剩余部分**换模型。

要复用同一个 cell 吗？`agent::model_switch` 的 cell 是 `Arc<Mutex<Option<PendingModelSwitch>>>`，`execute_turn` 在每次 provider 请求前 drain。skill 注入后，**同一轮内**的下一次 provider 请求就该用新模型 —— 正好符合 `execute_turn` 的 drain 时机！

所以 `model` 可以直接复用：skill 执行时 publish 一个 model switch，`execute_turn` 下一轮 drain 时应用。差别是：用户 `/model` 是「下一条用户消息」，skill 是「本轮内」—— 但 `execute_turn` 的 drain 点本来就是「每次 provider 请求前」，两者都能工作。

**取舍**：`model` 与 `effort` 一起做（effort 的 override 机制在 `core/self_tuning` 和 `agent/compact` 里，`STAR_SELFTUNING_*` 那套是「模型自己调」，语义不同，需要另找落点）。我倾向**只做 `model`**，`effort` 留到下一轮 —— 因为 effort 的 override 点在 StarCode 里没有「per-turn」的概念。

**`context: fork` / `agent` / `background`**：

跑在隔离子代理里。StarCode 有 `AgentTool` + `subagent_type`，技术上能跑。但：
- CC 明确警告「`context: fork` 只对有明确任务的 skill 有意义」
- 这会让 `SkillTool` 从「注入文本」变成「启动子代理」，是**行为形态**的改变
- 复杂度和风险都显著高于前两层

**建议：本次不做**，在文档里说明「已解析但未实现」，并给出理由。

---

## 4. 实施顺序与可回滚性

每一层独立提交、独立可回滚：

| 提交 | 内容 | 风险 |
|---|---|---|
| A | loader 补字段 + 布尔解析口径 + 单测 | 零（纯解析） |
| B | `disable-model-invocation` 闸门 + 工具描述分流 + 单测 | 低 |
| C | `allowed-tools` per-turn grant + `model` 本轮切换 | 中（接运行时） |

A、B 明显该做。C 里 `allowed-tools` 和 `model` 又可以分开。

---

## 5. 我需要你定的三件事

1. **`allowed-tools` 的语义宽度**：CC 是「本轮免确认」。StarCode 的 `PolicyEngine` 有 `default`/`acceptEdits`/`plan`/`yolo` 四档模式。要按 CC 的「仅本轮、且仅该 skill 调用的那轮」实现（精确），还是先做粗粒度（`acceptEdits` 式的一轮放行）？我倾向**精确版**——粗粒度会和 `acceptEdits` 模式语义打架。

2. **`user-invocable` 怎么处理**：StarCode 的用户 skill **不在 `/` 菜单**里，所以这条字段无处可作用。选项：(a) 只解析不生效；(b) 顺便把用户 skill 接进 `/` 菜单，让 `user-invocable` 有意义。我倾向 **(a)**，因为 (b) 是新功能不是本次语义对齐。

3. **`context: fork` 要不要做**：我倾向**不做**（理由见 §3 第 3 层）。你如果要用「skill 跑在隔离子代理里」，那要做，但建议单独一轮。

---

## 6. 现状小结

- 已完成：内置 skill 删除、用户 SKILL.md 目录接通（模型可调）、团队成员动态化
- 门禁：1940 passed / 0 failed，fmt clean，clippy 零警告
- 本文档只是**方案**，尚未动代码 —— 等你定了上面三件事再开工
