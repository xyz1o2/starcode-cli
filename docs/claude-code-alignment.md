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

### 2.2 语义分歧（已修，记录用）

**① 记忆加载语义相反 —— 已修（2026-10-13）。**

原本 CC 是**全量拼接**，StarCode 是**每层目录 first-hit 只取一个**，且 `STAR.md` / `STARCODE.md` 排在 `CLAUDE.md` 之前 —— **一个仓库同时存在 `STARCODE.md` 和 `CLAUDE.md` 时，两边看到的指令不一样。**

已改为 CC 拼接语义（`utils/project_context.rs`）：

- 新增 `collect_context_files_in_dir`：每层目录收**全部**指令文件，不再 first-hit
- 新支持 `CLAUDE.local.md`、`.claude/CLAUDE.md`、`~/.claude/CLAUDE.md`
- 目录遍历从 CWD 向上改为**从文件系统根向下到 CWD**（CC：越靠近启动目录的越晚读到）
- `STAR.md` / `STARCODE.md` 不再遮蔽 `CLAUDE.md`，改为并列拼接且排在其前
- `AGENTS.md` 改为**全局门控**：CWD 或任一上层存在 CLAUDE.md / CLAUDE.local.md 就不读，与 CC 默认一致
- `find_context_file_in_dir()` 未动（`find_project_root` 与 `/init`、`/memory` 仍依赖）

同时修掉两个与代码相反的注释：`collect_context_files` 原注释写「返回的 Vec 从高到低排列」而循环实为 CWD 优先；`load_and_merge_context_files` 原注释写「后面的级别（更靠近 CWD）优先级更高」而实际后面的更远。这两条注释本身就是该 bug 的成因。

**仍未对齐**：8000 字符截断（`DEFAULT_PROJECT_CONTEXT_MAX_CHARS`）与 CC 的「4 MiB 全量」不同 —— 有意保留，避免无脑拉高所有用户的 token 消耗。`@import` 展开、`.claude/rules/` 的 `paths:` 条件加载、`claudeMdExcludes` 均未实现。

**② 全局层有两个 settings 文件名 —— 不是 bug，刻意保留分工。** ✅ 已查 + 已收口（2026-10-13）

| 读取方 | 路径 | 管的 key |
|---|---|---|
| `SettingsManager` | `~/.star/user-settings.json` | `defaultModel`、force model、system settings |
| `provider_store` / `doctor` / `/config` | `~/.star/user-settings.json` | provider 列表、API key |
| `PermissionRuleSet` / `hooks::settings_hooks` | `~/.star/settings.json` | `permissions` 段、`hooks` 段 |

三者**键集不重叠**，所以「全局层写 allow/deny 不生效」这个猜测**不成立**。

**合并这两个文件被有意否决**：`UserSettings` 与 `ProviderConfig` 两个结构体反序列化同一文件、彼此忽略未知字段，合并要同时处理 provider 凭据持久化、`/config` 与 `doctor` 的读点，以及 provider_store 自带的 `providers.json` 迁移链 —— 风险远大于「名字看着乱」的收益。

真正会腐烂的是 **`Storage::global_settings_path()` 曾是死函数**，而 `settings_rules` 内联拼了一遍同样的路径字符串。已修：现在 `settings_rules` 与 `settings_hooks` 都走这一个常量，分工理由写进了 `storage.rs` 的函数文档。

### 2.3 确认缺失

| 项 | 现状 | 位置 |
|---|---|---|
| 配置根写死 | `const STAR_DIR: &str = ".star"`，**无环境变量可覆盖** | ❌ `core/config/storage.rs:8` |
| 全局层无 `.claude/` 回落 | 指令层已支持 `~/.claude/CLAUDE.md`；但 skills/agents/commands/等仍只读 `~/.star/*` | ❌ 部分 |
| 无 `CLAUDE.local.md` | **已修** | ✅ `utils/project_context.rs` |
| 无 `.claude/CLAUDE.md` | **已修**（`agent/system_prompt_merge.rs` 的旧实现无调用者，未基于它扩展） | ✅ 同上 |
| 无 `@import` 展开 | 未实现。CC 里 `@path` 会展开进 context，这里仍按字面量 | ❌ |
| 无 `.claude/rules/` | 无 `paths:` 条件加载 | ❌ |
| 无 `claudeMdExcludes` | — | ❌ |
| hooks 存在独立文件 | **已修**：`.star/hooks.json`（StarCode 自有，写入路径不变）与 settings.json 的 `hooks` 段（CC 风格，只读）**同时读取**，在 `list_hooks` 合并 | ✅ `core/hooks/settings_hooks.rs` |
| 无 hook `matcher` 过滤 | **已修**：`ManagedHook` 此前只按 event 过滤，带 matcher 的 hook 会对所有工具误触发。已按 CC 的字符集分派规则实现（精确/`\|`列表 → 精确匹配，其余 → 非锚定正则，编译失败 fail-closed） | ✅ `core/hooks/store.rs`、`runner.rs:33` |
| hook 事件缺口 | 无 `PermissionRequest` `PermissionDenied` `PostToolBatch` `StopFailure` `ConfigChange` `InstructionsLoaded` `CwdChanged` `FileChanged` `Elicitation` `WorktreeCreate` `PreModelSwitch` `PostModelSwitch` | ❌ |
| hook 事件自造名 | `BeforeAgent` / `AfterAgent` / `BeforeToolSelection` / `BeforeModel` / `AfterModel` 在 CC 中无对应 | ❌ `commands/hooks.rs:13,34` |
| 权限模式 | **已修（2026-10-13）**：`acceptEdits` 曾被折叠成 `Default`（CLI 与文档都写这个模式，实际给的是"写操作照样弹框"）。现在是独立一档：只读工具 + 文件编辑 + 只读 shell 命令免确认 | ✅ `core/policy/types.rs`、`policy_engine.rs::mode_auto_allows` |
| `default` 档只读也弹框 | **已修**：`default_decision = AskUser` 曾把 `Read`/`Grep`/`Glob` 也弹框；headless 下 `non_interactive` 再把它翻成 Deny，于是 `-p` 模式**每个工具都被拒**。现在按 CC 语义表放行只读工具 | ✅ `policy_engine.rs::check` 第 3 步 |
| 无 `dontAsk` 模式 | CI / headless 场景缺一个「其余直接拒」的档位 | ❌ |
| 无 auto 模式分类器 | — | ❌ |
| 无 sandbox | — | ❌ |
| 无 managed 层 | 只有 `STAR_PERMISSIONS_FILE` 一个显式入口 | ❌ |
| 无 `disableAllHooks` / `allowManagedHooksOnly` | `hooks_maybe_enabled_for_event`（`agent/hooks.rs:36`）只是有无 hook 的短路优化，fail-open，不是策略门控 | ❌ |
| 无 settings 文件监听 | 纯 300s TTL 缓存；权限规则靠 `reload_permission_rules` 命令式触发 | ❌ `config/settings_manager.rs:337` |
| 兜底路径丢审批模式 | **已修**：3 处 `unwrap_or_else` 现在都带上真实 `approval_mode`（新增 `StarAgent::policy_approval_mode()` 做枚举转换） | ✅ `ui/services/worker.rs`、`runtime/control_requests.rs`、`agent/skills/editor.rs` |
| `.star/` 缺 CC 对应目录 | 无 `rules/` `output-styles/` `workflows/` `themes/` `keybindings.json` `agent-memory/` | ❌ 见 §5 ④ |
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

### ✅ 已完成（2026-10-13）

| 项 | 落点 | 结果 |
|---|---|---|
| 记忆改全量拼接，移除 `STAR.md` 抢在 `CLAUDE.md` 前 | `utils/project_context.rs` | 新增 8 个测试，lib 套件 843 全绿 |
| 支持 `.claude/CLAUDE.md` / `CLAUDE.local.md` / `~/.claude/CLAUDE.md` | 同上 | 含 CC 的 `AGENTS.md` 全局门控 |
| hooks 改从 settings.json 的 `hooks` 段读 | `core/hooks/settings_hooks.rs`（新）+ `store.rs` | `.star/hooks.json` 保留为写入路径；导入项只读，`/hooks add\|remove` 动不了 |
| hook `matcher` 过滤（CC 的字符集分派规则） | `core/hooks/store.rs`、`runner.rs` | 修掉一个既有缺口：此前 hook 无工具级过滤 |
| 兜底路径丢审批模式（3 处） | `worker.rs`、`control_requests.rs`、`skills/editor.rs` | 新增 `StarAgent::policy_approval_mode()` |
| 全局 settings 路径单一来源 | `config/storage.rs`、`policy/settings_rules.rs` | 死函数 `global_settings_path()` 复活，分工写进函数文档 |

### ✅ 核心工作流 / UI 第二轮（2026-10-13，同日）

这一轮只处理「用户能不能把一个活干完」，按 §2 的现场核验结果逐条修，附加能力
（workflows / teams / plugins / sandbox / tool search）一律不动。

| 项 | 落点 | 结果 |
|---|---|---|
| `default` 档把只读工具也弹确认 | `core/policy/policy_engine.rs`（新增 `mode_auto_allows`）、`types/mod.rs` | 5 个新测试锁住语义；连带修好 headless `-p` 下"每个工具都被拒" |
| `acceptEdits` 被折叠成 `default` | `policy/types.rs`、`types/mod.rs`（两处枚举）、`settings_rules.rs`、`parity.rs` 的 `/mode` | Shift+Tab 循环扩为四档；显式 `ask` 规则仍压得住模式内置放行 |
| 确认等待不响应 abort（600s 静默） | `confirmation_bus/message_bus.rs`（新增 `request_cancellable`）、`agent/tool_executor.rs` | 3 个新测试；放弃等待时补发取消应答，把 UI 上的确认卡一并收掉 |
| 确认等待 600s 后自己超时，把「没回答」写成「被拒绝」 | `confirmation_bus/message_bus.rs`（`await_tool_decision`，去掉 `request_cancellable`/`request`）、`agent/tool_executor.rs`（新增 `pass_gate`/`Gate`，删掉 `STAR_CONFIRM_TIMEOUT`）、`agent/tool_helpers.rs`（删掉按确认超时推导的兜底） | 等待只剩三种结局：用户拍板、用户中断、总线关闭；工具兜底时限改挂在 `invocation.execute()` 上，不再把用户思考的时间算进工具预算 |
| Edit/Write 审批卡不显示 diff | `ui/components/confirmation_dialog.rs` | 卡片内联 diff 预览（上限 24 行）+ 现算增删行数 |
| diff 丢掉 `@@` hunk 头 | `ui/utils/render.rs::build_diff_block` | 2 个新测试；有 `@@` 就不再补 `═` 间隔线 |
| 滚动条从未渲染 | `ui/app/mod.rs`（活路径） | 死函数 `render_chat_history` 删除，滚动条挪进真正绘制的那条路径 |
| Tab 展开折叠的工具块选错条目 | `ui/events/input.rs::find_focused_tool_entry` | 按 `virtual_list` 的真实行高判断"覆盖视口中心"，不再拿 entry 下标与 `scroll/2` 比 |
| WebFetch / WebSearch / Task / MCP 无专属渲染 | `ui/components/tool_render.rs` | 调用行摘要 + 结果块摘要，MCP 标出 server 名 |
| 无对话内搜索 | 新增 `ui/components/chat_search.rs`、`ChatSearchState`、`Ctrl+F` | 与 `Ctrl+Shift+F`（搜仓库文件）分工；命中坐标与 `render_chat_lines` 同行号，有测试守着一致性 |
| 无 `--continue`，`/continue` 打开的是菜单 | `main.rs`、`commands/chat.rs::continue_cmd` | `-c/--continue` = `--resume latest`；`/continue` 无参直接接最近会话 |
| `/status` 不报配置来源 | `commands/core.rs::setting_sources_section` | 列出 4 个 settings 文件（是否存在、各自负责什么）+ 生效规则数 |
| 中断后没有提示行 | `ui/components/interrupted.rs` 复活 + `ui/app/runtime.rs` 宽限期 | 在唯一知道"用户按了 Esc"的地方推 `Interrupted · What should I do instead?` |
| 帮助文案把 Tab 说成补全 | `ui/components/help_popup.rs`、`events/keymap.rs` | 按真实行为改写；键表文档补上"两个搜索键的分工" |
| `/eval` 是悬空声明 | `commands/system.rs` | 声明删除，并加测试记录历史悬空命令名单 |

lib 套件 914 全绿，`cargo fmt --check` 绿，本次改动范围内 clippy 无新增告警。

### 待做

| P | 项 | 落点 | 预估 |
|---|---|---|---|
| **P1** | `@import` 展开 | `utils/project_context.rs` | M |
| **P1** | `.mcp.json` 作为回落 | `core/utils/paths.rs:337` | S |
| **P1** | `STAR_CONFIG_DIR` 环境变量，对应 `CLAUDE_CONFIG_DIR` | `core/config/storage.rs:8` | S |
| **P1** | `ApprovalMode` 补 `dontAsk`（headless/CI 用）。`acceptEdits` 已于 2026-10-13 独立成档，剩下三档（`auto`/`dontAsk`/`bypassPermissions` 中的 `auto` 与 `dontAsk`）仍缺 | `core/policy/types.rs`、`main.rs` | M |
| **P1** | MCP tool search，schema 延迟加载 | `core/tools/tool_registry.rs` | L |
| **P2** | `.claude/rules/` + `paths:` frontmatter 条件加载 | 新模块 | M |
| **P2** | 补 hook 事件 `PermissionRequest` / `PostToolBatch` / `ConfigChange` / `InstructionsLoaded` | `core/hooks/store.rs` | M |
| **P2** | settings 文件监听 + `ConfigChange` 事件（替代 300s TTL） | `config/settings_manager.rs` | M |
| **P2** | subagent frontmatter 补 `omitClaudeMd` / `maxTurns` / `permissionMode` / description token 预算 | `commands/agents/` | M |
| **P2** | Dynamic workflows（JS runtime + `agent`/`pipeline`/`parallel`） | 重做 `core/workflow/` | XL |
| **P3** | auto mode 分类器 / sandbox | 需第二个模型调用通道 | XL |

自造事件名的别名映射（`BeforeAgent`→`UserPromptSubmit` 等）已从列表移除：CC 的事件名本来就能被 `ManagedHookEvent::parse()` 识别（两边共享同一套 `#[serde(rename)]`），导入路径不需要映射表。真正缺的是 CC 独有的那 12 个事件，见 P2。

P2 的 workflows 价值最高（把子 agent 并行从消耗主 context 变成零 context 成本），但 `core/workflow/` 现在是死代码，等于从零写，不建议和小改动混在一轮里。

---

## 4. 几个可以直接抄的具体设计

不改架构也能抄的细节：

1. **matcher 按字符集分派** —— 只有 `[A-Za-z0-9_ \-,|]` 走精确匹配，其余走非锚定正则。规则简单、无歧义。
2. **`if` 字段复用权限规则语法** 再收一层，但明确声明「best-effort，硬阻止用 permissions」。`settings_rules.rs` 里 allow 匹配更严（`&&`/`;`/管道每段都要命中、命令替换不给 allow）正是同一思路，可以把这个边界写进文档。
3. **hooks exec form vs shell form** —— 设了 `args` 就不过 shell。
4. **auto memory 硬预算 + 超限报错让模型重写索引**，而非静默截断。`project_context.rs` 现在的 8000 字符硬截断会让模型以为文件就这么多内容，是有损的。
5. **可观测性作为产品功能。** StarCode 现状：`/context` 已用 `get_context_file_paths()` + `load_merged_project_context()` 列出加载了哪些记忆文件（对标 CC 的 `/context` ✓）；`/mcp` 列 server 健康态；`/tasks` 列 subagent；`/doctor` 输出工具可用性 + 日志可写性 + transcript 摘要。

   **「加载了哪些配置源」已于 2026-10-13 补上** —— `/status` 现在有 `Setting sources` 段（`commands/core.rs::setting_sources_section`）：列出 4 个 settings 文件（是否存在、各自负责什么键）+ 生效的 allow/ask/deny 条数。理由与本节原判断一致 —— §2.2 ② 那个 `settings.json` / `user-settings.json` 分工问题只能靠它暴露。

   > `commands/doctor.rs` 曾有一节 `## Harness Health` 读 `.star/eval-results.json`，已随 eval 评测体系一起删除（commit `76a0262`）。本条不再引用它。
6. **settings 列表 vs 标量的两类合并语义**，写进文档比隐含在代码里好。

---

## 5. 未验证 / 待查清单

本文初版列的 6 项待查已于 2026-10-13 全部查完，结论如下。

**① `change_listeners` 是死字段。** `settings_manager.rs:306` 定义、`:338` 初始化为空，全仓无注册、无 notify。`SettingsManager` 是纯 300s TTL 缓存（`:337`），**没有任何文件监听**。权限规则的"热更新"靠 `MessageBus::reload_permission_rules`（`confirmation_bus/message_bus.rs:46`）由命令显式触发 —— `control_requests.rs:685` 的注释说的就是这件事。

**② 全局层 settings 无冲突。** 见 §2.2 ② —— 三方读两个文件名，但 key 不重叠。

**③ `hooks_maybe_enabled_for_event` 门控**（`agent/hooks.rs:36`）只是"该事件有没有启用 hook"的短路优化，`Err(_) => true` 为 fail-open。**它不是 managed 策略门控** —— CC 的 `disableAllHooks` / `allowManagedHooksOnly` 在 StarCode 完全没有对应物。这一项从「待查」转为 §2.3 的缺失项。

**④ `.star/` 实际文件清单**（`.claude/` 回落改造时必须知道的全部落点）：

| 层 | 文件 / 目录 |
|---|---|
| `~/.star/` | `user-settings.json` `settings.json` `providers.json`(legacy) `installation_id` `model-cache.json` `mcp-oauth-tokens.json` `memory.md` `ignore` `commands/` `skills/` `agents/` `policies/` `i18n/` `extensions/` `marketplace/` `history/` `teams/` `transcripts/` `tmp/` `bin/` |
| `.star/`（项目） | `settings.json` `settings.local.json` `settings.jsonc` `permissions.json`(legacy) `permission_grants.json` `mcp.json` `hooks.json` `commands/` `skills/` `agents/` `i18n/` `lens/` `sessions/` `logs/` `history/` `tmp/checkpoints/` `file-history/` `images/` `extensions/` `agent-teams.json` `agent-teams/runs/` `pr_subscriptions/` `session_tags.json` `auto_compact.jsonl` `STAR.md`(legacy context) |

对照 CC 的 `.claude/` 布局，**完全没有对应物**的是：`rules/` `output-styles/` `workflows/` `themes/` `keybindings.json` `agent-memory/`。这些就是 §3 里缺失项要补的位置。

**⑤ `global_settings_path()` 无调用方。** 见 §2.2 ②。

**⑥ `study_or_copy_projects/claude-code-main` 是 Claude Code v2.8.2**，包内文件 mtime 全为 2026-07-05 03:12（整包拷贝，时间戳无意义，以 `package.json` 版本为准）。落后当前文档约 3 个月。**结论：可用于 UI 层渲染参考，不可用于任何架构判断。**

### 查完后新发现的两个问题

**A. 兜底路径丢审批模式 —— 已修（2026-10-13）。** `PolicyEngine::with_project_rules` 原先有 4 个构造点，只有主路径 `runtime_bootstrap.rs:40` 传了 `approval_mode`，另外 3 处 `unwrap_or_else` 兜底用 `PolicyEngineConfig::default()`：用户选 yolo/plan，runtime 一缺席确认判断就悄悄退回逐次询问。现已全部带上真实模式（新增 `StarAgent::approval_mode()` / `policy_approval_mode()` 做两个 `ApprovalMode` 枚举之间的转换）。

**B. `permissions.json` 与 `settings.json` 双格式并存。** `policy_engine.rs:66` 注释写明「装载 `.star/permissions.json`（旧格式）+ settings.json 的 `permissions` 段（新格式）」，对应 `storage.rs:167`。迁移到 `.claude/settings.json` 时要保住旧格式读取，否则老用户配置静默失效。

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