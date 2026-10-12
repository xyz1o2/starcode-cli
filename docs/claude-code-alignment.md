# StarCode × Claude Code 对齐基线

> 快照日期：2026-10-13
> Source of truth：Claude Code 官方文档（`code.claude.com/docs`），版本线 v2.1.2xx
> 适用范围：本文是**架构对标的唯一依据**，取代 2026-10-13 删除的 `docs/parity-*.md` 架构系列

---

## 0. 这份文档是什么

不是完成度自评，是**基线 + 差距**的对照表。左侧是官方当前设计（附文档出处），右侧是 StarCode 实测现状（附文件与行号）。

每条现状标注验证状态：

| 标记 | 含义 |
|---|---|
| ✅ | 已 grep 构造点 / 调用点确认 |
| ⚠️ | 读了代码但未确认接线，动手前需再查 |
| ❌ | 已确认缺失 |

**纪律**：仓库根 `CLAUDE.md` 明确写了「`#![allow(dead_code)]` 开着，可读性不是证据，只有构造点 grep 才是」。本文所有 ✅ 都按此标准核过一遍。旧的 `parity-*.md` 正是在这条上翻车了（§6）。

---

## 1. 官方设计基线

### 1.1 配置根与设置优先级

配置根是 `.claude/`，全局层 `~/.claude/`，另有一份 app state 在 `~/.claude.json`。
`CLAUDE_CONFIG_DIR` 环境变量可整体搬迁配置目录（Windows 下 `~/.claude` = `%USERPROFILE%\.claude`）。

**五层 precedence，高的覆盖低的**：

| # | 层 | 文件 |
|---|---|---|
| 1 | Managed | `managed-settings.json` / MDM / claude.ai console |
| 2 | Command line | `claude --settings` |
| 3 | Project local | `.claude/settings.local.json`（gitignored） |
| 4 | Shared project | `.claude/settings.json`（committed） |
| 5 | User | `~/.claude/settings.json` |

合并语义分两类，官方明确规定：

- **list 合并** —— `permissions.allow` 等在各层求并集
- **scalar 取最具体** —— `model` 用 local 的值

四个例外走自己的规则：`fallbackModel`（有序链，整份取最高层）、`modelPicker`、`availableModels`、`modelSettings`。

**作用域硬约束**：`permissions.defaultMode` 取 `auto` / `bypassPermissions` 时，**在 project / local 文件里不生效**，只能 user 或 managed 配。

> 出处：[Settings files and precedence](https://code.claude.com/docs/en/settings)

### 1.2 Hooks

Hooks 住在 settings.json 的 `hooks` 段里，**不是独立文件**。三层嵌套：

```
事件 (PreToolUse) → matcher group → handler[]
```

handler 五种类型：`command` / `http` / `mcp_tool` / `prompt` / `agent`。

**hook 位置共 7 处**：`~/.claude/settings.json`、`.claude/settings.json`、`.claude/settings.local.json`、managed、plugin 的 `hooks/hooks.json`、**skill frontmatter**、**subagent frontmatter**。

**事件按生命周期分四组**：

| 组 | 事件 |
|---|---|
| per-session | `SessionStart` `Setup` `SessionEnd` |
| per-turn | `UserPromptSubmit` `UserPromptExpansion` `Stop` `StopFailure` |
| per-tool-call | `PreToolUse` `PermissionRequest` `PermissionDenied` `PostToolUse` `PostToolUseFailure` `PostToolBatch` |
| 其他 | `Notification` `MessageDisplay` `SubagentStart` `SubagentStop` `TaskCreated` `TaskCompleted` `TeammateIdle` `InstructionsLoaded` `ConfigChange` `CwdChanged` `DirectoryAdded` `FileChanged` `WorktreeCreate` `WorktreeRemove` `PreCompact` `PostCompact` `PreModelSwitch` `PostModelSwitch` `Elicitation` `ElicitationResult` |

**matcher 求值按字符集分派**（规则简单到能背）：

| matcher 内容 | 求值方式 |
|---|---|
| `*` / 空 / 省略 | 全匹配 |
| 只有 `[A-Za-z0-9_ \-,|]` | 精确串，或 `\|`/`,` 分隔的精确串列表 |
| 含其他任何字符 | **非锚定 JS 正则**（`RegExp.prototype.test`） |

**handler 公共字段**：`type` `if` `timeout` `statusMessage` `once`。
`if` 复用权限规则语法（`Bash(git *)`、`Edit(*.ts)`）再收一层，但官方明确写：**`if` 是 best-effort，硬阻止必须用 permissions**。

**exec form vs shell form**：设了 `args` 就直接 spawn 可执行文件，不过 shell，每个元素原样传递 —— 专门为 `${CLAUDE_PROJECT_DIR}` 这类路径占位符设计，防引号/空格再解析。

**settings 文件热重载**，每次变更 fire `ConfigChange`。

> 出处：[Hooks reference](https://code.claude.com/docs/en/hooks)

### 1.3 记忆与指令加载

**加载位置，从宽到窄**：

| Scope | 位置 |
|---|---|
| Managed policy | `/etc/claude-code/CLAUDE.md`（Win: `C:\Program Files\ClaudeCode\CLAUDE.md`） |
| User | `~/.claude/CLAUDE.md` |
| Project | `./CLAUDE.md` 或 `./.claude/CLAUDE.md` |
| Local | `./CLAUDE.local.md`（gitignored） |

**关键语义：全部拼接，不覆盖。** 顺序从文件系统根到 CWD，同目录内 `CLAUDE.local.md` 排在 `CLAUDE.md` 之后。

**子目录懒加载**：CWD 以下子目录的 CLAUDE.md 不预载，等 Claude 碰该目录文件时注入。

**`.claude/rules/`**（CC 现在推荐的拆分方式）：
- `*.md` 递归发现，支持子目录（`.claude/rules/frontend/react.md`）
- frontmatter `paths:` glob 做**按文件路径的条件加载** —— Claude 读/写/编辑匹配文件时才注入
- 支持花括号展开，整个 `paths` 列表共享 1000 pattern / 4 MiB 预算
- 无 `paths` 的 rule 与 `.claude/CLAUDE.md` 同优先级预载

**`@path/to/import`**：CLAUDE.md 里导入其他文件，最多 4 跳，跳过 code span 和 fenced block；跳出工作目录的 import 属 external，首次需批准。

**`AGENTS.md`**：默认**只在没有 CLAUDE.md / CLAUDE.local.md 时**才读。`~/.claude/CLAUDE.md` 和 `.claude/rules/` 不计入该判定。可用 `Project instructions` 设为 `claude-md-and-agents-md` 等四档。

**`claudeMdExcludes`**：按 glob 排除特定 CLAUDE.md / rules 文件。

**体积约束**：目标 <200 行；单个文件 >4 MiB 直接跳过；超长在启动和 `/status` 时告警。

**Auto memory**（Claude 自己写的笔记）：
- 位置 `~/.claude/projects/<repo>/memory/`，按 git 仓库路径归并，所有 worktree 共享
- `MEMORY.md` 是索引，每次会话加载**前 200 行 / 25 KB（先到者为准）**
- 细节拆到 topic 文件，按需读
- frontmatter `type: user | feedback | project | reference`
- **写超预算直接返回错误让模型重写索引**，不是静默截断
- 受 retention 清理豁免

> 出处：[How Claude remembers your project](https://code.claude.com/docs/en/memory) · [Explore the .claude directory](https://code.claude.com/docs/en/claude-directory)

### 1.4 权限模式

**六个模式**：

| 值 | CLI 显示 | 免确认可执行 |
|---|---|---|
| `default` | Manual | 只读 |
| `acceptEdits` | Accept edits | 读 + 文件编辑 + 常用 fs 命令 |
| `plan` | Plan | 读 + 分类器批准的只读命令 |
| `auto` | Auto | 全部，**后台分类器兜底** |
| `dontAsk` | Don't ask | 工作目录内读 + 预批准工具；其余**直接拒** |
| `bypassPermissions` | Bypass | 全部 |

`manual` 是 `default` 的别名。**v2.1.283 起 `auto` 是交互式终端和 VS Code 的内置起始模式。**

**auto mode 的核心设计**：第二个模型（分类器）审每个动作。它看到**用户请求 + 工具调用**，刻意**不看模型自己写的散文** —— 防止模型把请求说服掉。服务端侧没有 verdict 就**拒绝而非放行**。

默认阻止清单（节选，覆盖面远超一般实现）：下载即执行（`curl | bash`）、生产部署与迁移、云存储批量删除、IAM / 仓库权限授予、force push、`git reset --hard` / `clean -fd` / `stash drop`、`terraform/pulumi/cdk destroy`、写 secret manager、改 DNS / TLS 证书、合并未审批 PR、approve 自己的 PR、关掉 CI checks、把凭据打进 transcript、访问 IMDS `169.254.169.254`、读宿主凭据、连 sibling 容器、改 base URL / webhook / registry mirror 指向第三方（含 `.env.example`）、注释掉守护安全行为的测试断言、创建 DaemonSet / admission webhook、隧道与反向外壳、push 机密到公开仓库。

**sandbox 是独立的一层**，由 OS 强制执行，不依赖模型决策；`auto-allow`（沙箱边界内自动批）与 `auto`（分类器审）两个机制独立且可叠加。

hook 层面为此新增 `PermissionRequest` / `PermissionDenied` 两个事件；被拒时可用 `hookSpecificOutput.retry: true` 告诉模型可以换路子重试。

> 出处：[Choose a permission mode](https://code.claude.com/docs/en/permission-modes) · [Configure the sandboxed Bash tool](https://code.claude.com/docs/en/sandboxing)

### 1.5 多 agent 编排

官方把并行拆成四种原语，分界线是**谁持有计划、中间结果存在哪**：

| | Subagents | Skills | Agent teams | Workflows |
|---|---|---|---|---|
| 谁决定下一步 | Claude 逐轮 | Claude 跟 prompt | lead agent 逐轮 | **脚本** |
| 中间结果在哪 | Claude context | Claude context | 共享 task list | **脚本变量** |
| 规模 | 每轮几个 | 同 subagent | 几个长期 peer | 几十~几百 |

**Dynamic workflow** = `.claude/workflows/*.js`：

```javascript
export const meta = { name: 'audit-routes', description: '...' }
const found = await agent('List every .ts file under src/routes/.', { schema: {...} })
const audits = await pipeline(found.files, f => agent(`Audit ${f}...`, { label: f }))
return audits.filter(Boolean)
```

核心思想：**把计划移进代码**，中间结果留在脚本变量，Claude context 只装最终答案 —— 编排本身零 token。

- 原语仅五个：`agent()` / `pipeline()` / `parallel()` / `phase()` / `log()`，加 `args` 全局
- **脚本刻意不给 filesystem / shell** —— agents 干活，脚本只调度
- **确定性靠抛异常保证**：`Date.now()` / `Math.random()` / `new Date()` 在脚本内直接 throw，保证重放时 `agent()` 调用序列一致；时间戳从 `args` 传入
- **重放语义按 agent 启动顺序推进**：已完成的返回存档结果；第一个 prompt 变了的 agent **及其之后所有** agent 重跑
- **prompt cache 错峰**：fan-out 时同前缀 agent 最多 hold 5 秒（`CLAUDE_CODE_WORKFLOW_PREFIX_STAGGER_MS`）再一起放，共享第一个 agent 建立的缓存前缀
- **硬上限**：默认 16 并发（按 CPU 数降，`CLAUDE_CODE_WORKFLOW_MAX_CONCURRENT_AGENTS` 1–256）、单次 `parallel()/pipeline()` 4096 项、单次运行 1000 agent、禁止 `import()`
- `ultracode` 关键字或 `/effort ultracode` 让每个实质任务自动写 workflow；size guideline 是**建议不是限制**（unrestricted / small<5 / medium<10 / large<50）
- 内置 `/deep-research`

> 出处：[Orchestrate subagents at scale with dynamic workflows](https://code.claude.com/docs/en/workflows)

### 1.6 Subagent

frontmatter 字段：`name` `description` `tools` `disallowedTools` `model` `permissionMode` `maxTurns` `skills` `mcpServers` `hooks` `memory` `background` `omitClaudeMd` `effort` `isolation` `color` `initialPrompt` `experimental.cacheTtl`。

关键机制：

- **所有 subagent 的 `description` 合计上限 15,000 tokens**，超了启动警告并报总数
- `omitClaudeMd: true` 关闭 CLAUDE.md 加载；内置 Explore / Plan **默认就跳过** CLAUDE.md 和 git status 快照
- `isolation: worktree` + 强制校验 git 不得逃出 worktree
- 模型解析顺序：per-invocation `model` 参数 > frontmatter `model`（`inherit` = 主会话）> `CLAUDE_CODE_SUBAGENT_MODEL` > 主会话模型
- 后台 subagent 工具裁剪到白名单（21 内建 + MCP），前后台**同一份定义解析出不同工具集**
- 目录热监控，但只监控会话启动时已存在的目录
- 跳过规则：无 `name` / `---` 不在首行 / `name` 含 `:` 或超 256 字符 / YAML 解析失败 → 当文档忽略

> 出处：[Create custom subagents](https://code.claude.com/docs/en/sub-agents)

### 1.7 MCP

- **Tool search 默认开启**：MCP 工具 schema 不进初始上下文，模型用 `ToolSearch` 按需检索；关闭时改用 `WaitForMcpServers` 阻塞
- 三层 scope：local / project（根目录 `.mcp.json`）/ user，后两者存 `~/.claude.json`
- `CLAUDE_PROJECT_DIR` + MCP `roots/list` 协议，server 可主动问"当前允许访问哪些目录"
- v1/v2 双 runtime，协议 revision 2026-07-28；v2 走常开流收 `list_changed`
- 输出 >10k tokens 警告、25k 截断；>50k 字符落盘成文件
- 主会话里 >2 分钟未完成的 MCP 调用**自动转后台任务**（`CLAUDE_CODE_MCP_AUTO_BACKGROUND_MS`）
- 重连：中途掉线指数退避 5 次；首连瞬时错误重试 3 次；认证错误不重试
- idle timeout：HTTP 类 5 分钟、stdio 30 分钟
- 工具命名 `mcp__<server>__<tool>`；plugin 自带的为 `mcp__plugin_<plugin>_<server>__<tool>`

> 出处：[Connect Claude Code to tools via MCP](https://code.claude.com/docs/en/mcp)

---

## 2. StarCode 现状

### 2.1 已对上的部分

| 项 | 现状 | 位置 |
|---|---|---|
| 项目记忆读取 | 读根目录 `CLAUDE.md` / `AGENTS.md` | ✅ `utils/project_context.rs:13` |
| 多级记忆合并 | 从 CWD 向上遍历到项目根，带 mtime 指纹缓存 | ✅ `utils/project_context.rs:143,419` |
| `/init` 语义 | 已有 CLAUDE.md 时改进它而非新建遮蔽文件 | ✅ `commands/init.rs:18` |
| Anthropic 协议 | `anthropic-version: 2023-06-01`、thinking budget/effort、prompt caching | ✅ `llm/mod.rs:69`、`llm/thinking.rs` |
| provider 环境变量 | `ANTHROPIC_API_KEY` / `BASE_URL` / `MODEL` 是解析的一层，带 `SourceRef` | ✅ `core/config/provider_resolution.rs:13-18` |
| 权限规则**语法** | `Tool(specifier)` + `allow`/`ask`/`deny`/`defaultMode` 与 CC 同构 | ✅ `core/policy/settings_rules.rs:1-23` |
| 权限三级判定 | deny > ask > allow，deny 连 yolo 都挡得住 | ✅ 同上 |
| 规则合并 + trust 分级 | 可提交的 `settings.json` 只能加规则、不能抬升审批模式 | ✅ `settings_rules.rs:468-489` |
| PolicyEngine 接线 | `with_project_rules` 是唯一构造点 | ✅ `core/config/runtime_bootstrap.rs:34-49` |
| headless 拒绝策略 | `non_interactive` 打开，`AskUser` 无人应答时直接拒 | ✅ 同上 |
| hook 事件名（部分） | `PreToolUse` 等沿用 CC 命名，含 alias 映射 | ✅ `core/hooks/store.rs:32-79`、`agent/hooks.rs:125-174` |
| Headless 入口 | `-p/--prompt` 直出 JSONL 或文本 | ✅ `main.rs:52` |

### 2.2 语义分歧（比路径差异更值得先修）

**① 记忆加载语义相反。**

CC 是**全量拼接**，StarCode 是**每层目录 first-hit 只取一个**：

```rust
// utils/project_context.rs:13
const CONTEXT_FILE_CANDIDATES: &[&str] = &["STAR.md", "STARCODE.md", "CLAUDE.md", "AGENTS.md"];
```

`STAR.md` / `STARCODE.md` 排在 `CLAUDE.md` 之前，而 CC 根本没有 STAR.md 这个概念。**一个仓库同时存在 `STARCODE.md` 和 `CLAUDE.md` 时，两边看到的指令不一样。**

另外 StarCode 的 8000 字符截断（`CLAUDE.md:111`）与 CC 的「4 MiB 全量」不是一回事。

**② 全局 settings 有两个不同文件名。**

| 读取方 | 路径 |
|---|---|
| `SettingsManager` | `~/.star/user-settings.json` ⚠️ |
| `PermissionRuleSet` / `Storage::global_settings_path` | `~/.star/settings.json` ✅ |

即用户在全局层写 `settings.json` 里的 allow/deny 可能不生效（走的是另一个文件名）。需动手前用 `settings_manager` 确认加载顺序。

### 2.3 确认缺失

| 项 | 现状 | 位置 |
|---|---|---|
| 配置根写死 | `const STAR_DIR: &str = ".star"`，**无环境变量可覆盖** | ❌ `core/config/storage.rs:8` |
| 全局层无 `.claude/` 回落 | 只读 `~/.star/*` | ❌ |
| 无 `CLAUDE.local.md` | — | ❌ |
| 无 `.claude/CLAUDE.md` | 有代码但**无调用者**（见下） | ❌ |
| 无 `.claude/rules/` | 无 `paths:` 条件加载 | ❌ |
| 无 `@import` 展开 | — | ❌ |
| 无 `claudeMdExcludes` | — | ❌ |
| hooks 存在独立文件 | `.star/hooks.json`，非 settings.json 的 `hooks` 段 | ❌ `core/hooks/store.rs:142-144` |
| hook 事件缺口 | 无 `PermissionRequest` `PermissionDenied` `PostToolBatch` `StopFailure` `ConfigChange` `InstructionsLoaded` `CwdChanged` `FileChanged` `Elicitation` `WorktreeCreate` `PreModelSwitch` `PostModelSwitch` | ❌ |
| hook 事件自造名 | `BeforeAgent` / `AfterAgent` / `BeforeToolSelection` / `BeforeModel` / `AfterModel` 在 CC 中无对应 | ❌ `commands/hooks.rs:13,34` |
| 权限模式 | 仅 `Default \| Plan \| Yolo`；`acceptEdits`/`bypassPermissions` 在 CLI 是别名，折叠成 default/yolo | ❌ `core/policy/types.rs:44-50`、`main.rs:121-123,414,733` |
| 无 `dontAsk` 模式 | CI / headless 场景缺一个「其余直接拒」的档位 | ❌ |
| 无 auto 模式分类器 | — | ❌ |
| 无 sandbox | — | ❌ |
| 无 managed 层 | 只有 `STAR_PERMISSIONS_FILE` 一个显式入口 | ❌ |
| MCP 只读 `.star/mcp.json` | 不读项目根 `.mcp.json` | ❌ `core/utils/paths.rs:337`、`storage.rs:130` |
| 无 tool search | 缓存的 `Vec<FunctionDeclaration>` **全量注入**模型 | ❌ `core/tools/tool_registry.rs:121-153` |
| subagent 字段薄 | 无 `omitClaudeMd` / `maxTurns` / `permissionMode` / `effort` / `isolation` / `background` / description token 预算 | ❌ `commands/agents/custom_agents.rs` |
| workflows | `WorkflowTool` 读 `.star/workflows/*.{json,sh}`（活）；`WorkflowManager`（`core/workflow/`）**无构造点，是死代码** | ⚠️→✅ 见下 |
| workflows 路径打架 | `core/tools/workflow.rs:54` 读 `.star/workflows`；`core/workflow/mod.rs:129` 和 `commands/compat.rs:580` 读 `.claude/workflows` | ✅ |
| settings 热重载 | `SettingsManager` 是 300 秒 TTL 缓存，非文件监听 | ✅ `core/config/settings_manager.rs:337` |
| 无 `~/.claude.json` 等价层 | — | ❌ |
| skills / commands / agents 目录 | 全走 `.star/` + `~/.star/`，无 `.claude/` 回落 | ✅ `storage.rs:49-70,133-141` |

**死代码确认**：`agent/system_prompt_merge.rs:120-166` 的 `load_claudemd_files` 会读 `.claude/CLAUDE.md`、`.claude/commands/*.md`、`~/.claude/CLAUDE.md`，但 `SystemPromptMerger` 全仓只有 `agent/mod.rs:46` 的 mod 声明，无任何构造或调用点。**看着能用，实际不走。** 修 `.claude/` 记忆兼容时不要基于它扩展，直接在 `project_context.rs` 上改。

**workflows 确认**：`WorkflowManager`（`core/workflow/mod.rs:113`）除自身定义外无引用；`WorkflowTool` 在 `runtime_bootstrap/agent_runtime.rs:205` 注册，是活的。已删除的 `parity-orphaned.md` 曾把 Workflow 整体列为桩实现，**结论过宽**。

---

## 3. 差距矩阵与优先级

优先级按「能直接吃掉现成配置」和「改动面」排，不按功能重要性排。

| P | 项 | 落点 | 预估 |
|---|---|---|---|
| **P0** | 记忆改全量拼接，移除 `STAR.md` 抢在 `CLAUDE.md` 前 | `utils/project_context.rs` | S |
| **P0** | 支持 `.claude/CLAUDE.md` / `CLAUDE.local.md` / `@import` | 同上 | S |
| **P0** | hooks 改从 settings.json 的 `hooks` 段读（`.star/hooks.json` 保留为扩展） | `core/hooks/store.rs` + `settings_rules.rs` | M |
| **P0** | 统一全局 settings 文件名（`user-settings.json` vs `settings.json`） | `config/settings_manager.rs:312`、`storage.rs:37` | S |
| **P1** | `.mcp.json` 作为回落 | `core/utils/paths.rs:337` | S |
| **P1** | `STAR_CONFIG_DIR` 环境变量，对应 `CLAUDE_CONFIG_DIR` | `core/config/storage.rs:8` | S |
| **P1** | `ApprovalMode` 扩到六个 + `dontAsk`（headless/CI 用） | `core/policy/types.rs`、`main.rs` | M |
| **P1** | MCP tool search，schema 延迟加载 | `core/tools/tool_registry.rs` | L |
| **P2** | `.claude/rules/` + `paths:` frontmatter 条件加载 | 新模块 | M |
| **P2** | 补 hook 事件 `PermissionRequest` / `PostToolBatch` / `ConfigChange` / `InstructionsLoaded` | `core/hooks/store.rs` | M |
| **P2** | 自造事件名做别名映射（`BeforeAgent`→`UserPromptSubmit`、`AfterModel`→`PostModelSwitch`），保留旧名 | 同上 | S |
| **P2** | subagent frontmatter 补 `omitClaudeMd` / `maxTurns` / `permissionMode` / description token 预算 | `commands/agents/` | M |
| **P2** | Dynamic workflows（JS runtime + `agent`/`pipeline`/`parallel`） | 重做 `core/workflow/` | XL |
| **P3** | auto mode 分类器 / sandbox | 需第二个模型调用通道 | XL |

**P0 四项合计约 300 行**，做完 StarCode 可直接消费绝大多数存量 `.claude/` 仓库。

P2 的 workflows 虽然价值最高（把子 agent 并行从消耗主 context 变成零 context 成本），但工程量 XL，且 `core/workflow/` 现在是死代码，等于从零写，不建议和 P0 混在一轮里。

---

## 4. 几个可以直接抄的具体设计

不改架构也能抄的细节：

1. **matcher 按字符集分派** —— 只有 `[A-Za-z0-9_ \-,|]` 走精确匹配，其余走非锚定正则。规则简单、无歧义。
2. **`if` 字段复用权限规则语法** 再收一层，但明确声明「best-effort，硬阻止用 permissions」。`settings_rules.rs` 里 allow 匹配更严（`&&`/`;`/管道每段都要命中、命令替换不给 allow）正是同一思路，可以把这个边界写进文档。
3. **hooks exec form vs shell form** —— 设了 `args` 就不过 shell。
4. **auto memory 硬预算 + 超限报错让模型重写索引**，而非静默截断。`project_context.rs` 现在的 8000 字符硬截断会让模型以为文件就这么多内容，是有损的。
5. **可观测性作为产品功能** —— `/context` 列加载了哪些记忆、`/status` 列加载了哪些 settings、`/mcp` 列 server 健康态、`/tasks` 列 subagent 跑的哪个模型。`commands/doctor.rs` 已有雏形（`## Harness Health`），可扩。
6. **settings 列表 vs 标量的两类合并语义**，写进文档比隐含在代码里好。

---

## 5. 未验证 / 待查清单

本文没做到的事，动手前需要先查：

- [ ] `SettingsManager` 的 `change_listeners` 字段（`settings_manager.rs:306`）是否有真实文件监听注册，还是死字段
- [ ] `SettingsManager` 与 `PermissionRuleSet` 的加载顺序 —— 用户在全局层写 `allow` 到底哪个先生效
- [ ] `hooks_maybe_enabled_for_event`（`agent/hooks.rs:125`）的门控语义是否与 CC 的 managed 约束对齐
- [ ] StarCode 全量读取的 `.star/` 文件清单（本文只覆盖了权限、hooks、mcp、skills、commands、agents、settings、context、sessions、logs、file-history、lens、memory）
- [ ] `storage.rs:37 global_settings_path()` 的实际调用方 —— 是否有第三条读 `~/.star/settings.json` 的路径
- [ ] `study_or_copy_projects/claude-code-main` 的快照日期与对应 CC 版本，判断还能作为 UI 层参考用多久

---

## 6. 旧 parity-* 文档的处置

`docs/parity-*.md` 系列（生成于 2026-09-06，基于 HEAD `80c5eb3` + 泄漏源码）**不再作为对标依据**，原因：

1. **结论已错**。已删除的 `parity-orphaned.md` 把 `PolicyEngine` 和 `ModelFallbackManager` 列为「从未构造」，但 `runtime_bootstrap.rs:40` 是 PolicyEngine 的唯一构造点，且经 `with_project_rules`（`policy_engine.rs:60`）→ `load_project_permissions` → `load_permission_rules`（`policy_engine.rs:91`）真实装载了磁盘规则。仓库 `CLAUDE.md` 当时也写着「`load_permission_rules` 零调用者」，同样已修正。
2. **对着上一代架构**。泄漏源码是 v2.1.88（2026-03），官方文档已到 v2.1.2xx，中间 dynamic workflows、auto mode 默认化、tool search 默认开启、35+ hook 事件、`.claude/rules/` 条件加载等全是新的。
3. **完成度自评不可信**。标着 ✅ 100% 的项（如 Explore UI、Tool 渲染）是 UI 渲染格式，与 harness 架构对标是两件事。

**仍可参考**：`docs/ui-tool-render.md`（终端渲染对比）、`docs/ui-benchmark-ccb.md`（UI 视觉基准）。这两份是 UI 层观察，不受配置架构变化影响。

**已执行的处置（2026-10-13）**：

- **已删除 5 份整体作废的**：`parity-overview.md`、`parity-agent.md`、`parity-commands.md`、`parity-orphaned.md`、`parity-edge-cases.md`。删除前逐条写明作废原因，其中 `parity-orphaned.md` 列出了被证伪的三项（PolicyEngine、ModelFallbackManager、Workflow）。git 历史中仍可取回。
- **保留 5 份 UI 层的**：`parity-task.md`、`parity-progress.md`、`parity-permission.md`、`parity-tool-render-gaps.md`、`parity-ui-framework.md`。它们只覆盖终端渲染与 UI 框架结构，不受配置架构变化影响。`parity-permission.md` 中「需要先接线 `PolicyEngine`」的错误说法已就地改正为指向本文。
- **`CLAUDE.md`** 的 Project 段落加了对标文档指针；「Not everything is wired up」一节订正了两条错误判断（`PolicyEngine::load_permission_rules` 实为活路径；`WebBrowserTool` 已从树中移除），并补记了 `core/langfuse/client.rs:315` 这条此前漏列的 stub。
- `README.md` 未引用 `parity-*.md`，无需改动。