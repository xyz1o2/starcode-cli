# MCP 插件生态 / Skill 路由这条线的问题分析

- 日期：2026-11-05
- 范围：提交 `2149c59`（MCP 工具命名）、`ecf28da`（插件 MCP 注册）、`0d10a15`（扩展 skill 加载）、`e09077d`（skill 触发器修复）及其调用链
- 证据来源：源码检查（本文引用的 file:line 均为 `2149c59` 时点）

## 1. 结论摘要

这条线解决了三个真实故障（插件 MCP 工具进不了 registry、工具名带冒号被 provider 拒、自动触发器调用已下线的内置 skill），修复方向都对，但留下了两类问题：

1. **「拼名 / 拆名」往返约定只在 1/N 个调用点落地** —— 拆 `mcp__` 工具名的代码有十几处，只有一处用了新的还原函数，其余对插件 server 全部拆错名。
2. **往返不变量本身对含下划线的插件名不成立** —— marketplace 对插件名无字符校验，`split_once('_')` 的还原在插件名含 `_` 时必然拆错。

另有一个功能缺口（fallback 路径漏装插件 server）、一个静默断链（插件注册一失败全断）、一个未修完的盲触发（skill 触发器不读输入）。

## 2. 问题清单

### P0：拆名逻辑只在 1/N 个调用点接线（`2149c59`）

新代码建立了「拼名去冒号（`mcp_tool_name`）→ 拆名还原冒号（`server_name_from_tool_name`）」的约定（`src/core/mcp/mod.rs:44/58`），但全库独立拆 `mcp__<server>__<tool>` 的调用点里，**只有 `src/agent/tool_executor.rs:2655`（`execute_mcp_dynamic_tool`）用了还原函数**。其余调用点拆出的 server 名是 `plugin_exa_exa`，而 manager 注册键是 `plugin:exa:exa`：

| 调用点 | 影响 |
|---|---|
| `src/agent/mcp_permission.rs` `get_server_type` / `get_server_base_url` / `extract_mcp_info`（`splitn(3, "__")` 取 `parts[1]`） | 查 connections 必 miss，插件 server 的权限类型 / URL 审计失效 |
| `src/commands/activity.rs:144` `split_mcp_tool`（被 `fewer_permissions`、`team_onboarding` 复用） | 权限建议、团队统计里 server 名错误 |
| `src/agent/subagent/progress.rs:180` 展示转换 | 子代理进度显示的 server 名错乱 |
| `src/agent/tool_routing/helpers.rs:415` 动态工具判定 | 判定本身只看前缀，不受影响，但同文件若将来取 server 名会踩同雷 |

**修法建议**：把「拆 `mcp__` + 还原」收敛为 `core::mcp` 的唯一入口（如 `split_mcp_tool_name() -> (server, tool)`），所有调用点改走它。

### P0：往返不变量对含下划线的插件名不成立（`2149c59`）

`server_name_from_tool_name` 用 `split_once('_')` 切两段：`plugin_<plugin>_<server>` → `plugin:<plugin>:<server>`。**插件名含 `_` 时直接崩**：

```text
plugin:my_plugin:search → mcp__plugin_my_plugin_search__t
                        → split_once('_') → ("my", "plugin_search")
                        → plugin:my:plugin_search   ✗「server not found」
```

- `is_valid_plugin_mcp_server_name`（`src/core/plugins/mcp.rs:60`）只校验 **server** 名；marketplace 的插件 `name` 字段没有任何校验拦下划线
- `src/core/mcp/mod.rs` 的往返测试全用 `my-plugin`（连字符），恰好绕开这颗雷
- `src/core/mcp/mod.rs:52` 的注释声称「插件名不含下划线之外的分隔符」，但没有排除下划线本身——注释与事实不符

**修法建议**（二选一）：marketplace 安装入口校验插件名 `[a-zA-Z0-9-]`；或还原时 `rsplit_once` 从右切，并要求 server 段先过 `is_valid_plugin_mcp_server_name`。顺带：普通 server 真叫 `plugin_x_y` 时会被前缀判定误还原成 `plugin:x:y`——按前缀猜不如查注册表。

### P1：fallback 路径漏装插件 server（`ecf28da` 没修完）

`src/agent/workflows/star_agent.rs::initialize_mcp` 只在 `runtime_mcp_manager` 分支调了 `load_plugin_mcp_servers`；**`fallback_mcp_manager` 分支照旧**只跑 `initialize_mcp_servers()` + `discover_all()`。走 fallback 的会话里插件 MCP 工具不存在——与本次要修的「装了搜不到」是同型 bug 换了个入口。

### P1：插件注册失败是静默且会断链（`ecf28da`）

- 失败只写 `[MCP] failed to register plugin MCP servers` debug 日志（`star_agent.rs::load_plugin_mcp_servers`），用户侧表现为「/plugin 显示已装但调不着」，排查全靠翻日志
- `MCPManager::initialize_plugin_mcp_servers`（`src/core/mcp/manager.rs:221`）里一个 server 的 `register_plugin_server` 返回 Err 就提前返回，**后续插件 server 全部不注册**——一个坏 manifest 连累所有插件

**修法建议**：逐 server 收集错误而不是 `?` 短路；失败时至少升级为用户可见的告警（对齐 Claude Code 的 `mcp list` 语义）。

### P2：skill 触发器修了「调不存在的」，没修「盲触发」（`e09077d`）

`src/agent/tool_routing/triggers.rs` 里 `should_trigger_navigator/analyzer/editor_skill_with_flag` 的 `user_input` 仍是 `_` 前缀参数——只要链上前置满足（如 `codebase_search_attempted`）且用户恰好有同名 skill，**任何无关输入都会触发**。这正是同一批提交里批判 CodebaseSearch 的「看起来在跑、实际盲打」，标准不一致。`e09077d` 只对「内置 skill 下线」这一种死法加了 `SkillAvailability` 闸门，没对「skill 存在但不相关」做判分。

路由词表的毛边（同文件）：

- `CONCEPTUAL_MARKERS` 与 `FUNCTIONAL_VERBS` 都含「原理」——同一词进两张语义相反的表
- `names_another_tool` 里 `"read "`（尾空格）会误杀 "read the error"，`" codebase_search"`（首空格）与 `"codegraph"`（无空格）风格不一
- `has_exact_identifier_shape` 注释说 snake_case 会否决，实现却要求 snake_case 里还得有大写字母——纯小写下划线词（`parse_toml_config`）漏网（现有测试靠 TOOL_WORDS 的 `"grep"` 兜住，不是靠形状判定）
- 中文触发词「哪些」非常宽：「这些配置哪些需要改」会触发全库检索

### P3：工作区卫生

`src/agent/coordinator/prompt.rs`、`src/agent/subagent/notification.rs` 长期显示"已修改"（±174 行行尾翻转）。根因**不是工作区脏**：`git cat-file -p` 证实索引里的 blob 本身就是 CRLF（最后提交 `207a042` 早于 `.gitattributes` 的 `*.rs eol=lf` 规则），于是每次 checkout 原样写出 CRLF 字节，status 拿清洗后的 LF 与 CRLF 的索引 blob 比较，永远判不等——`checkout --` 与 `checkout-index -f` 都无法消除。

唯一解法是 `git add --renormalize <两个文件>` 后提交，让索引 blob 变成 LF。在 renormalize 提交之前，任何"丢弃更改"操作对这两个文件都无效。

## 3. 修复优先级

1. **P0 拆名收敛**：`split_mcp_tool_name()` 单一入口 + 全调用点改造（正确性，会出错数据）
2. **P0 插件名下划线**：marketplace 校验或 `rsplit_once`（正确性）
3. **P1 fallback 分支**补 `load_plugin_mcp_servers`（功能缺口）
4. **P1 注册失败**逐 server 容错 + 可见告警（可运维性）
5. **P2 skill 触发器**补输入判分（设计债，非 bug）

## 4. 修复记录（2026-11-05）

- **P0-1 已修**：`core::mcp::split_mcp_tool_name()` 成为拆名唯一入口（带冒号还原 + 残缺名判 None）；改造四个调用点——`tool_executor::execute_mcp_dynamic_tool`、`commands/activity::split_mcp_tool`（签名改为 owned，顺带 `mcp_server_counts` 显示正确）、`mcp_permission` 的 `get_server_type` / `get_server_base_url` / `extract_mcp_info`（只有一处被测试引用，其余本就无生产调用者，防御性修正）、`subagent/progress::user_facing_tool_name`。
- **P0-2 已修**：采取 **marketplace 校验**方案——新增 `plugins::is_valid_plugin_name`（`[a-zA-Z0-9-]`，不含下划线），`discover_plugin_mcp_servers` 里含非法字符的插件整个跳过并记 debug 日志。不选 `rsplit_once` 的原因：插件名与 server 名都含 `_` 时从左切和从右切必有一个拆错，结构上无解，只能在校验端堵死。**前缀碰撞（普通 server 恰好叫 `plugin_x_y`）也已修**，见下条。
- **P0-2 尾巴已修（前缀碰撞查注册表）**：`core::mcp` 新增 `split_mcp_tool_name_disambiguated(name, is_registered)` —— 还原名 `plugin:x:y` 在册用还原名；否则原始段（`raw_mcp_server_name` 暴露）在册退回原始段；都不在册保持还原名（报错行为与旧版一致）；两边同在册优先还原名（显式注册的插件 server 证据更强）。有注册表的两个路径已改走它：执行路径 `MCPManager::split_mcp_tool_name(name)`（`execute_mcp_dynamic_tool` 取到 manager 后再拆名）；权限路径 `McpServerManager::resolve_connection`（`get_server_type` / `get_server_base_url` 复用，按 connections 消歧）。无注册表可查的纯展示调用点（activity 统计 / subagent 进度）仍走纯拆名，碰撞场景显示错名代价可接受；`mcp_tool_name` 拼名约定不变。
- **P1 fallback 已修**：`initialize_mcp` 的 `fallback_mcp_manager` 分支在 `discover_all()` 之前同样调用 `load_plugin_mcp_servers`。
- **P1 注册断链已修**：`MCPManager::initialize_plugin_mcp_servers` 改为逐 server 容错（失败名收集进聚合错误，不再 `?` 短路连累其它 server）；`load_plugin_mcp_servers` 失败时升级为 `log::warn!`（原来只进 debug 日志）。
- **P2 未修**：触发器输入判分是路由启发式的设计调整，需要重新设计三个 skill 触发器的相关性判分并重建回归样本，不宜顺手改——留独立任务。
- **P3 已根治**：两个 CRLF blob 经 `git add --renormalize` 后提交（`c94b7ff`），status 消失。丢弃（`checkout --` / `checkout-index -f`）对该根因无效，已在 §2 P3 说明。

**验证**：`cargo check --all-targets` 0 errors；定向测试 `core::mcp` / `plugins` / `activity` / `fewer_permissions` / `mcp_permission` / `mcp_tool_name_is_shortened` / `star_agent` 全绿（`--test-threads=1`）；`cargo fmt --check` 通过；相关文件无新增 clippy 提示。前缀碰撞修复另有 3 个定向测试（消歧真值表、manager 注册表消歧、connections 消歧）与新增的 37→40 个 `mcp` 过滤测试全绿。

## 5. 插件部分自查（2026-11-06 第二轮）

P0-P3 修复后又把插件子系统（`core/plugins` 全模块 + `core/extensions/marketplace` + 安装/更新链路）过了一遍，发现并修复三个此前清单没有覆盖的问题：

- **安装入口没有名字闸门（已修）**：`install_plugin_local` / `install_plugin_git`（marketplace 批量安装也漏斗到这两个）此前不校验 `is_valid_plugin_name`，带 `_` 的名字能装进 manifest，但发现端只静默跳过（debug 日志）——用户面对「已装但什么都调不着」。现在新增 `validate_new_plugin_name` 在**装之前**拒绝（先于目录落盘），错误信息写明原因。
- **`normalize_plugin_name` 保留 `_`（已修）**：`commands/plugin.rs` 的用户输入名 / git 仓库名推断都走它，`my_plugin` 会原样装成一个永不加载的插件。现在 `_` 归一成 `-`，归一结果过安装闸门（两条规则互锁，有测试钉住）。
- **`update_plugin` 不感知 scope（已修）**：git 插件 pull 成功后用 `load_manifest` / `save_manifest`（仅 project scope）更新 `installed_at`，user scope 装的插件循环空转、时间戳永不更新。改为按 `entry.scope` 读写 `load_manifest_scoped` / `save_manifest_scoped`。

**记录但不修**：

- `prepare_install_destination` 先删旧插件目录再复制（local 更新失败时旧的也没了）——需要先 copy-and-swap，改安装流程，单独立项。
- `core/extensions/marketplace.rs`（extension marketplace，与 plugin marketplace 是两套）：`install_mcp_npm` 按**相对路径**写 `.star/mcp.json`（依赖 cwd）；`DEFAULT_INDEX_URL` 指向 Claude Code 官方 marketplace，格式是 `.claude-plugin/marketplace.json`，注释自认「may need adaptation」。属于扩展生态重建范畴，不在本条线。

**验证**：`cargo check --all-targets` 0 errors；`cargo fmt --check` 通过；`install_gate_tests`（安装闸门拒绝 + manifest 无残留、normalize 归一互锁）2 个新测试全绿；`plugin` 过滤测试 19 个全过。
