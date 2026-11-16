# `.code` 多 Agent、Explore 与 `/poor` 源码考察

日期：2026-11-16。状态：调查已完成，随后按用户要求实施修复。下文的专项对照记录修复前快照，当前变更见文末“后续修复”。调查阶段全部结论为静态源码分析。

## 范围与证据口径

- 参考目录：`.code/claude-code-main/`，`package.json:2-3` 声明包名 `claude-code-best`，版本 `2.8.4`。
- 该目录的 `CLAUDE.md:7` 自述为反编译并继续修改的实现，不能直接视为官方 Claude Code 的现行规范。根项目的架构对齐基线仍为 `docs/claude-code-alignment.md`。
- `git -C .code/claude-code-main rev-parse` 向上找到了 StarCode 外层仓库，返回的是外层 HEAD，不能据此给参考副本标注提交版本。
- 本次只读考察源码与提示词，不运行参考项目的构建脚本，不调用外部模型服务，不修改生产代码。已有 `a.html` 不在范围内。
- 文件行号为本次本地快照；“接线”只表示静态调用链已确认，不表示运行验证通过。

## 已核对的提示组装事实

### 主会话的委派建议有条件

参考 `src/constants/prompts.ts:300-303`：fork 模式与普通子 Agent 模式使用不同的委派说明。普通模式强调任务与专用 Agent 的描述匹配、独立任务可并行、保护主上下文、不要过度委派、不要重复做已经委派出去的搜索。

参考 `src/constants/prompts.ts:357-365`：只有 Agent 工具可用、Explore/Plan 已启用且未开启 fork 时，才注入 Explore 搜索建议。明确的文件、类、函数搜索建议直接用搜索工具；广泛调查才考虑 Explore，判断依据包括直接搜索不足或明显需要多次查询。具体阈值待 Explore 调用链核对。

这里的提示原则值得参考：委派应有任务边界和收益判断；提示中提到的默认行为必须与当前 feature gate 和工具 schema 一致。

### `/poor` 会影响独立验证提示

参考 `src/constants/prompts.ts:374-380`：独立验证提示同时依赖 Agent 可用、`VERIFICATION_AGENT`、实验配置 `tengu_hive_evidence`，以及 `!isPoorModeActive()`。不能将其描述为对所有构建、所有用户默认强制的验证契约。

该提示要求非简单实现完成后由独立 Agent 验证，并要求主 Agent抽查验证报告中的命令证据；这是一段条件提示。`/poor` 是否还裁剪运行时的 Agent 定义、后台调用和 schema，由专项调用链核对。

### 会话提示存在应用层缓存

参考 `src/constants/prompts.ts:469-473`：`session_guidance` 用 `systemPromptSection` 包装。
参考 `src/constants/systemPromptSections.ts:16-18,43-55,65-67`：普通段按名称缓存，清理入口 `clearSystemPromptSections` 同时清除提示段和 beta header latch；文件注释说明常规清理时机为 `/clear` 与 `/compact`。

因此需要额外核对 `/poor` 切换时是否清理提示段缓存。仅有状态布尔值改变，不足以证明已经发送给模型的验证提示随之改变。

### StarCode 已有适合承载迁移的提示装载路径

- `src/core/prompts/tool_descriptions.rs:206-209`：工具 schema 描述读取对应 Markdown 的 frontmatter，缺失时由调用方回退。
- `src/agent/tool_executor.rs:339-346`：构建真实工具定义时调用上述解析器，故工具描述 Markdown 确实进入模型工具定义。
- `src/core/prompts/tool_descriptions.rs:228-240`：长说明正文有独立解析入口，供发现工具时附带使用说明。
- `src/agent/prompt_builder.rs:250-305,660-701`：核心指南按核心工具映射选择、排序、去 frontmatter、按大小预算组装，过长会截断或丢弃。
- `src/agent/prompt_builder.rs:340-359`：系统提示整体覆盖文件命中时，直接跳过默认组装；仅修改内嵌指南不保证在启用整体覆盖的会话里生效。
- `src/agent/prompt_builder.rs:318-331`：静态与准静态段分开处理缓存。引入新的运行模式时，应明确变化落在哪一段以及如何失效。

迁移提示词应落在现有 Markdown 与装载链上，并检查核心指南大小预算、工具 schema、执行行为的一致性。

## 专项对照

| 主题 | 参考实现与证据 | StarCode 与证据 | 结论与迁移边界 |
|---|---|---|---|
| 普通子 Agent / fork | `packages/builtin-tools/src/tools/AgentTool/AgentTool.tsx:611,756` 与 `runAgent.ts:377`：普通代理使用独立提示与任务消息；fork 带父历史、父系统提示与工具池 | `src/agent/subagent/router.rs:109-118` 创建空的父上下文字段；`src/core/tools/agent_tool.rs:376-419` 消费该请求，没有补入父历史；有历史时也只拼接每条最多 500 字符的文本 | Agent-tool fork 尚不能承诺完整父会话继承。参考提示提到 `fork:true`，但当前参考 schema 通过省略类型配合 feature gate 分流，提示本身也有漂移 |
| `/subtask` | 参考 fork 保留结构化消息，不能仅用文本摘要替代 | `src/agent/workflows/star_agent.rs:84-112` 调 `session_context_snapshot()` 并传 `with_parent_context`；`src/agent/subagent/runner.rs:39` 安装结构化父上下文 | `/subtask` 与 Agent-tool fork 是两条路径，不能概括成“Rust 没有 fork”或“两者都完整继承” |
| 后台与通知 | `agentToolUtils.ts:609` 处理完成、终止、失败及通知 | `runtime_bootstrap/agent_runtime.rs:85` 注入 async runner；`subagent/runner.rs:688,767,834` 启动、执行、入通知队列；`agent_loop.rs:288,455` 消费通知并等待后台结果 | 普通后台任务与主会话汇总已经静态接线，提示应要求等待真实终态结果，不能把 launch/progress 当发现 |
| SendMessage 恢复 | `SendMessageTool/SendMessageTool.ts:909` 对运行目标投递，对终止目标走恢复；`AgentTool/resumeAgent.ts:64` 从 transcript 恢复 | `src/agent/subagent/runner.rs:851-859` 终态注销；`:914` 投递走 hub，未知目标尝试 peer 分流或报错。`tool-description-sendmessage.md:7` 的 continuing 表述未区分运行中与已完成 | 当前链路支持运行中沟通，未发现参考式完成后自动恢复。提示不应扩大承诺 |
| 并行委派 | `AgentTool/prompt.ts:156,184` 鼓励独立任务同一消息并发启动 | `tool-description-runagent.md:24` 有并行建议；`tool_executor.rs:2143` 批处理并发；Agent 的只读分类允许进入该执行分组 | 分类标签不保证子任务只读。简报应写明范围、输出、写权限，并避免多个编辑任务争抢同一文件 |
| Explore 提示 | `AgentTool/built-in/exploreAgent.ts:13-56` 有专门的搜索职责、禁止副作用、工具选择和直接文本报告指引；`:82` 绑定提示生成函数 | `src/agent/subagent/runner.rs:157-185` 设置 Explorer 白名单后仍走共享循环；`agent_run.rs:377` 使用 Agent 模式；`system-prompt-agent-mode.md:5-16` 含通用编辑流程 | 最值得迁移的是 Explore 专用职责提示。当前不是没有执行能力，而是缺少同等的专用提示选择 |
| Explore 广度 | `exploreAgent.ts:62` 的 quick / medium / very thorough 是自然语言简报协议，无独立 schema 字段；`:59` 的 3 是主代理是否值得委派的查询阈值 | 当前 Agent schema/request 未发现对应广度字段或三档提示 | 可以先采用自然语言协议；不能声称参考要求每个 Explore 至少搜索三次 |
| Explore 工具 | `exploreAgent.ts:67-73` 用黑名单；`agentToolUtils.ts:81,149,162` 过滤后仍可能包含 MCP 等工具 | `src/agent/subagent/runner.rs:81-94` 白名单有 12 项，包含 Bash、TodoWrite、AskUserQuestion；`tool_executor.rs:318,387,1129` 同时约束 schema 和执行 | 专项回传曾称 13 项，主会话逐项核对为 12 项。白名单已经强制执行，但不能因此宣称所有 Bash 参数均无副作用 |
| Explore 模型 | `exploreAgent.ts:78` 默认按用户类型选 haiku/inherit；`src/utils/model/agent.ts:43,69,78` 再处理环境覆盖、调用参数和定义 | `src/agent/subagent/runner.rs:194-203` 继承父 client 的模型、endpoint 与 provider；公开 Agent 参数未暴露 model | 不能把参考默认轻量模型直接等同于最终选择，也不能以改提示词实现 Rust 的独立模型选择 |
| `/poor` 状态 | `src/commands/poor/poorMode.ts:15-26` 首次读设置，setter 更新内存并写 userSettings；`src/commands.ts:161` 有 POOR feature gate | `src/commands/parity/core.rs:750-773` 仅设置 UI 的 poor_mode 和 proactive_suggestions.enabled；全 src/tests 的 poor_mode 消费者仅见初始化、命令与诊断显示 | 当前 `/poor` 没有相应运行时状态传递或持久化，回显“减少后台模型调用”缺乏调用链支持 |
| `/poor` 辅助任务 | `src/query/stopHooks.ts:132-165` 跳过自动建议、记忆提取和 autoDream；`SessionMemory/sessionMemory.ts:273`、`attachments.ts:2419`、`AgentSummary/agentSummary.ts:70` 另有跳过点 | `src/core/budget_mode.rs:82,257,269,281` 有独立管理器和 skip 方法，但全 src/tests 未找到模块外生产消费者；本地规则建议与后台 LLM 调用不是同一能力 | 应先盘点真实辅助模型调用，再接执行门控。一般 token 预算和本地建议开关都不能代替这条链路 |
| `/poor` 与验证 | 只省略 `prompts.ts:374-380` 的条件指导；`builtInAgents.ts:62` 注册 verification 不看 poor | Rust 编辑后的诊断/验证链存在，但不读 poor 状态，也不等同独立对抗验证 Agent | 不能描述为 poor 从运行时禁用了验证 Agent；诊断/测试验证与独立审查也须区分 |

表内省略 `.code/claude-code-main/` 前缀的 TypeScript 路径均相对参考目录；Rust 路径相对 StarCode 根目录。多 Agent 构建开关还需注意：参考 `scripts/defines.ts:61` 中 FORK_SUBAGENT 不在默认启用项，源码存在不等于当前构建启用。

## `/poor` 的额外语义与缓存问题

参考还有保留请求、仅改变模型选择的分支：`src/utils/permissions/permissionExplainer.ts:178`、`src/cli/handlers/autoMode.ts:93`、`src/utils/permissions/yoloClassifier.ts:1345`。这些分支仍受显式模型或配置优先级影响，不能笼统宣称全部关闭或总会降低费用。手动 `/summary` 的 `sessionMemory.ts:394,427` 路径没有 poor gate；切换后也未发现统一取消已经发出调用的逻辑。

专项检索覆盖 poor 命令、setter、设置页及设置状态更新链，未发现为 poor 切换主动清除 `session_guidance` 缓存。因此静态证据支持一个时序差异：后台任务消费点可以读取新的 poor 值，但模型仍可能收到切换前缓存的验证指导，直到对应缓存被清除。此结论尚未通过运行复现。

现有 `poorMode.test.ts:86` 等用例覆盖 setter 写设置和内存切换；题为首次读取的用例在断言前先调用 setter，不能据此认定冷启动加载或进程重启已得到测试。`AgentSummary/__tests__/agentSummary.test.ts:183` 覆盖 poor 下跳过一次摘要并重新调度，不能外推所有消费者都有集成覆盖。

## 值得采用的提示原则与实施顺序

1. **先保证提示与能力一致。** 澄清普通 Agent、Agent-tool fork、`/subtask` 的上下文差异，以及 SendMessage 能继续哪些状态的目标。参考库自己的 fork 参数文案也要修正后才能借鉴。
2. **给 Explorer 独立职责提示。** 说明只做定位、阅读与解释，按可用工具选择搜索方式，接受调用者指定的搜索广度，直接输出简短文本并附路径/行号，未知内容明确标注。避免继承通用编辑流程造成的职责冲突。
3. **简报给足背景，搜索范围要窄。** 普通代理需要目标、已知线索、已经排除的路径、输出要求和写权限；fork 需要明确的任务范围。独立任务可并行，委派者不重复搜索，不猜测尚未返回的结果。
4. **省费用由执行门控完成。** `/poor` 应围绕真实存在的可选辅助模型请求设计；提示省略只是配套行为。切换时必须同时定义运行时生效点、持久化和提示缓存失效语义。
5. **权限边界单独验证。** Explorer 保留 Bash，默认权限模式中的只读分类是自动允许依据，未命中的命令不一定被强制拒绝。复制只读提示不能代替命令参数级限制或沙箱策略。本次没有执行副作用命令，也未断言完整权限链已被审计。

## 验证与未决范围

完成三条专项源码检索，并在主会话抽查 fork 请求构造与消费、`/subtask` 父历史传递、终态注销、Explorer 工具清单与通用提示、poor setter、停止钩子和 Rust 状态消费者。只新增并更新本调查归档文档；未修改代码或提示词，未运行构建、测试或参考项目脚本，未调用外部 API。`a.html` 保持原状。

结论覆盖静态接线与文案一致性，不代表真实运行效果、完整权限审计或费用测量。调查完成后已结束循环。用户随后要求解决上述问题，进入以下修复阶段。

## 后续修复

- Explorer 类型随子 Config 进入提示构建，加载独立 `system-prompt-explorer.md`，不再附带通用编辑流程、委派指南和全工具目录；仍使用原流式循环、进度 tracker 与执行层工具白名单。
- Agent fork 的父会话由调用方执行器通过类型化接口注入 invocation，保留完整消息和工具结果，不再按每条 500 字符拼接文本。当前未完成的工具批次不进入子会话。显式指定子代理类型时不再被 fork 描述覆盖。
- SendMessage 使用说明明确只沟通运行中的本地 Agent，已停止目标不能通过该工具自动恢复。
- `/poor` 与 `/proactive` 分别持久化自己的用户层字段；启动按项目层 > 用户层 > 默认值解析。交互命令立即作用于当前会话，项目配置在下次启动重新参与解析；出现冲突时回显说明。关闭 poor 恢复建议偏好，poor 期间修改偏好不会打开建议；关闭时清除已有建议。`/suggestions on|off` 共用同一路径，配置导入和重置包含这两项偏好。
- 持久化不会把项目层合并状态写回另一个用户层字段，防止项目设置泄漏到其他项目；新增跨项目污染和保存失败回归测试。没有连接未接线的预算模块，没有增加自动模型任务。
- 本地建议的 `analyze_context` 未找到生产调用点。本次保证状态、持久化和回显一致，不声称恢复了自动建议生成，更不声称减少模型费用。
- Explorer 的工具集合限制与只读职责提示已接线，但没有新增 Bash 参数级只读强制；SendMessage 未实现终态 Agent 恢复。fork 继承完成的结构化消息，不承诺完全相同的 system prompt 或工具池。
- 最终验证：`cargo check --workspace --all-targets --locked` 通过；poor 定向测试 18 passed，配置定向测试 9 passed。保存失败测试补充 `/proactive` 分支后，`cargo test --workspace --lib -- --test-threads=1` 为 2280 passed / 4 ignored / 0 failed；`cargo test --workspace --doc` 为 0 passed / 3 ignored / 0 failed（当前文档测试均忽略）。`cargo fmt --all -- --check` 与 `git diff --check` 均通过。没有进行真实模型 API、费用测量或 TUI 交互实测。
