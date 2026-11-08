# StarCode 项目问题分析报告

- 日期：2026-11-04
- 范围：TUI 交互、工具路由、代码搜索、调用图、配置菜单、测试质量
- 证据来源：源码检查、真实 ARTEX 仓库冒烟、单元测试、全量测试、用户截图

## 1. 结论摘要

项目当前的主要问题不是单一 UI 缺陷，而是三类状态没有统一：

1. **工具状态与 UI 状态不同步**：审批选择、模型列表刷新、思考块、工具结果高度变化时，状态已经变了，但缓存文档或滚动锚点仍是旧状态，因此出现“鼠标点一下才动”“页面跳动”“刷新闪空”。
2. **自动路由过度依赖硬编码条件**：CodebaseSearch、RepoMap、CodeGraph/Explore 和 SubAgent 已注册，但自动触发链条曾不完整，环境变量、工具短名单、语言闸门、预算缓存键相互叠加，导致工具“存在但不触发”。
3. **搜索链路存在多套实现和多个根目录语义**：旧 `src/tools/search.rs` 与新 `src/core/tools/grep.rs` 并存；旧 SearchTool 曾使用进程 cwd 而不是 Config 的项目根；CodebaseSearch 的预算档位曾产生互不兼容的缓存命名空间。

最近一次全量 lib 测试结果：

```text
1845 passed / 37 failed / 3 ignored
```

37 个失败在未改动基线中同样存在，主要是测试环境的用户目录/profile 不一致导致的 commands/agents/hooks/permissions 测试失败；本轮修改没有新增失败。

## 2. 已确认问题清单

### P0：工具审批上下键状态与画面不同步

**表现**：权限卡片中按 ↑/↓，选中项不移动；点击鼠标后才更新。

**原因**：审批处理修改了 `pending_confirmation_choice`，但 `DocumentKey` 没有包含审批选择、反馈模式、审批结果等字段，聊天文档缓存继续复用旧行；另有残留 `InputModal` 抢键问题。

**已修复**：

- `ToolConfirmation` 优先于 `InputModal`；
- `DocumentKey` 纳入 `pending_confirmation_choice`、反馈模式/内容、权限解释/调试状态和 outcome 指纹；
- 增加 stale input modal 与 tool confirmation 回归测试。

### P0：审批卡片变成结果卡片时页面跳动

审批卡片变成结果卡片后条目高度改变，旧实现没有补偿 scroll anchor，导致整页移动。

已增加 `VirtualList::prefix_height`，在非 auto-follow 模式下按条目前高度差补偿 `state.scroll`；底部跟随模式仍按底部计算。

### P1：思考块状态层级与过渡不清晰

已修复：

- 进行中：`∴ Thinking…`；
- 完成：`∴ Thought for ...`；
- 历史无冻结时长：`∴ Thought`；
- 去掉 `len() > 5` 的半途出现阈值；
- 流式期间保持块可见；
- 进行中与完成态使用相同的标题结构，避免完成瞬间宽度暴涨导致折行跳动；
- 点击判定与渲染判定统一。

剩余边界：展开态正文流式增长仍会增加文档高度，目前由 `streaming_height_floor` 与滚动锚点机制减轻跳动，但尚未做完整动画过渡。

### P1：Grep 存在两套实现且项目根不统一

`src/tools/search.rs` 与 `src/core/tools/grep.rs` 并存，runtime 实际注册旧 `SearchTool`。旧实现曾默认使用进程 cwd，而不是 `Config::target_dir()`，在 WSL/IDE/子目录启动时会搜错目录。

已修复：

- 增加 `SearchTool::new_with_base_dir`；
- runtime 使用配置项目根；
- 默认排除 `.star/`，避免 session JSON、日志和历史输出污染 Grep。

后续建议统一两套 Search/Grep 实现。

### P1：CodebaseSearch auto 预算造成缓存命名空间分裂

`budget_profile="auto"` 以前会因为预算参数不同生成另一套 cache key，而 watcher 只填默认预算缓存，导致 auto 查询永久 Warming，界面显示 `0 ranked results`，模型随后绕开 CodebaseSearch 使用 Grep/RepoMap。

已修复：缓存身份不再区分预算档位；auto 只影响输出候选数量，不再开孤立引擎。查询缓存也不再缓存 Warming 输出。

### P1：CodebaseSearch / RepoMap / Explore 自动路由不完整

已补齐：

- 中文/自然语言请求的 query rewrite；
- 首轮 CodebaseSearch prefetch 默认开启并保留内容形状判断；
- `AutoTriggerKind::RepoMap`：项目概览、仓库熟悉、结构分析；
- `AutoTriggerKind::Explore`：调用者、影响面、调用链问题；
- Skill/SubAgent fallback 默认开启；
- 语言闸门分为“判定阶段”和“有效 query 阶段”。

剩余风险：query rewrite 依赖一次额外 LLM 调用，服务失败时会保守跳过纯中文词法查询；环境变量使用 `OnceLock` 缓存，运行中修改需重启。

### P1：CallGraph 子目录生成多个数据库

`RepoMap(path=repo/agent)` 以前会生成 `repo/agent/.star/context/callgraph.db`。现在 CallGraph/RepoMap 向上查找 `.git`，统一使用 Git 项目根，数据库固定落到 `repo/.star/context/callgraph.db`。

### P2：TodoWrite 参数形状过于严格

已改为兼容单条 todo 对象、`items/tasks/task_list/todo_list` 别名、缺省 status/activeForm 和条目级额外字段；顶层旧式 `operation` envelope 仍拒绝，错误信息带可复制 JSON 形状。

### P1：工具折叠摘要与展开正文取自不同字段，计数互相矛盾

**表现**：`glob(dir_path=..., pattern=**/*.svg)` 折叠态写 `Found 1 file`，点开却是"没找到"；`LS` 折叠态的文件数把表头和分割线也算进去了。

**原因**：折叠摘要走 `tr.output` 的**非空行计数**，而这个数字和工具的真实结果量没关系：

- `Glob` 空结果时 UI 侧 output 是单行 `"No files found"` → 被数成 1 个文件；
- `Glob` 有命中时 UI 侧 output 是分组树（`"1 files\n  a.svg"`）→ 2 行 → 被数成 2 个文件；
- `LS` 的 output 是表格（表头 + 分割线 + 条目 + `Total: N items`）→ 行数远多于条目数。

这与之前修掉的 `Grep`（`"No results found"` 被写成 `Found 1 matches`）是同一类缺陷：摘要不看工具自己写的权威数字，只去数正文行。

**已修复**：

- `Glob` 折叠摘要改用 `tool_result.data` 里的真实文件列表计数（`data` 就是 `found_file_paths`），无 `data` 时退回解析输出头部的 `{count} files` / `Found N file(s)`；
- `Glob` 空结果显示 `No files`，不再显示 `Found 1 file`；
- `LS` 折叠摘要读末行 `Total: N items`，空目录显示 `Empty`，解析不出总数时退回通用预览而不编造数字；
- 新增 `count_glob_paths_in_output` / `parse_ls_total_items` / `is_empty_directory_notice` 三个解析函数。

**验证**：`src/core/tools/glob.rs` 新增真实工具集成测试（命中时 `data` 必须是路径数组；空结果时 output 必须是 `No files found` 且 `data` 为 `None`），`src/ui/components/tool_render.rs` 新增 5 个渲染回归测试。

### P1：内部路由收据泄漏到用户可见输出

**表现**：助手消息里出现 `[ROUTE] referee | workflow=project-analysis | stages=... | primary=...`，把工具轨迹和最终回答割裂开。

**原因**：模型的上层提示要求每次回复输出路由收据，模型把它当普通正文返回；项目侧没有任何提示词要求输出它，也没有拦截它。源码检索确认 `src/core/prompts/**` 与 `src/**` 中都不存在该模板。

**已修复**：在 `src/core/prompts/system-prompts/system-prompt-core-identity.md` 的 COMMUNICATION RULES 里明确禁止输出内部路由字段。

**边界**：这是提示词层约束，不是硬过滤。若上层提示词再次要求输出该格式，仍会复现；彻底方案是把路由收据移出模型的可见输出通道（例如只写入 debug 日志）。

### P1：空 reasoning 也创建 ChatEntry，产生空的 `Thought` 块

**表现**：工具调用前后各出现一次 `∴ Thought (ctrl+o to expand)`，但展开后没有内容。

**原因**：`StreamMessage::ReasoningDelta` / `Thinking` 的处理分支先创建 `ChatEntry::assistant("").with_reasoning("")` 再追加正文；provider 发空 delta（心跳/过渡标记）时，就会留下一个 `reasoning_content = Some("")` 的条目。渲染层又用"有 reasoning_content **或**正在 streaming"作为显示条件，于是空块被画出来。

**已修复**：

- 空 reasoning delta 直接丢弃，不创建条目；
- 渲染层只在 reasoning 正文非空时渲染 `Thought`；
- 冻结耗时也改为要求正文非空，避免空块显示假耗时。

## 3. 配置与默认值问题

当前 Thinking、Context Window、Language、Approval Mode 的默认值总体合理，但 `Auto` 的来源不透明。设置菜单建议显示：

```text
Thinking        Medium      model default
Context Window  Auto        detected from model
Language        Auto        detected: zh-CN
Approval Mode   Default     project setting
```

Models 菜单建议保留旧模型列表，刷新时只更新 `Refreshing…` 状态；当前模型显示 `Current`，刷新动作不默认抢焦点。

## 4. 测试与工程质量问题

### 全量测试

```text
1845 passed / 37 failed / 3 ignored
```

37 个失败在基线中同样存在，集中于 commands/agents、permissions、hooks、external_configs。根因是 tempdir 测试路径与 `dirs::home_dir()` 位于不同 Windows profile，临时项目读取了真实 `.star` 状态。测试隔离仍需进一步加强。

### 其它质量问题

- Clippy 仍有大量 advisory warnings，包括 private interface、重复属性、参数过多和复杂类型；
- lib 测试依赖 `--test-threads=1`，说明部分测试依赖进程级环境变量和全局状态；
- 两套 Grep/Search 实现继续并存，存在行为漂移风险；
- 自动路由缺少统一的“为什么触发/为什么跳过”可观测摘要，虽然已有 debug log，但还没有稳定的 UI 诊断入口。

## 5. 优先级建议

### P0

1. 保持审批/模型菜单上下键与 DocumentKey 同步；
2. 保持审批结果、模型刷新、思考块的滚动锚点；
3. CodebaseSearch 共享缓存引擎，禁止预算档位制造孤立缓存。

### P1

1. 合并两套 Grep/Search 实现；
2. 增加自动路由诊断：`route / score / reason / active_tools / env_enabled`；
3. 设置菜单显示 Auto 的真实来源；
4. 记录 query rewrite 的成功、超时和空结果统计；
5. 将 RepoMap/Explore 的自动触发结果显示在任务状态中。

### P2

1. 建立 TUI 截图和终端交互回归测试；
2. 清理 37 个环境相关失败测试的 profile 隔离；
3. 逐步处理 Clippy advisory warnings；
4. 将工具请求、审批结果、工具输出按任务级分组，减少 Thought/Tool/Thought 的碎片化。

## 6. 未解决项

- 展开态思考正文流式增长仍会改变文档高度；
- query rewrite 失败时，纯中文请求会被安全地跳过，而不是盲搜中文 token；
- 子 Agent 自动触发仍受 Skill 工具是否进入当前短名单影响，后续可以引入类似 Claude Code Tool Search 的按需工具发现；
- Exa MCP 当前未挂载，外部调研使用内置搜索完成。
