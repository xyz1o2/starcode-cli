# CCB 细节对标跟踪

日期：2026-11-17。参考：`.code/claude-code-main`，只读使用；架构决策依据 `docs/claude-code-alignment.md` 与当前生产调用链。参考副本为反编译后修改版本，其文档和功能声明不能替代源码证据。

## 验收方法

按可验证的细节逐项推进：输入/schema → 配置或请求 → 真实执行消费者 → 输出/生命周期 → 回归测试。仅声明字段、增加提示或编译通过不算功能完成。保留有意差异；没有运行证据的能力不得标为已实现。当前范围是多 Agent、Explore、fork、消息与任务生命周期、自定义定义和相关提示；不把“完备”解释为整个参考仓库的所有产品功能已移植。

## 当前差距

| 项目 | 状态 | 证据及下一步 |
|---|---|---|
| Explore 专用提示与工具集合门 | 已在上一轮完成 | `src/agent/prompt_builder.rs`、`src/agent/tool_executor.rs` 同时限制 schema 和执行；不能采纳旧隔离工作树中“无工具门”的结论 |
| fork 结构化消息 | 已在上一轮完成 | 保留完成的消息与工具配对，排除当前未完成批次；不承诺相同系统提示或精确工具前缀 |
| poor 状态与文案 | 已在上一轮完成 | 仅管理本地建议状态和偏好持久化，自动建议生成尚无生产调用，不宣称降低模型费用 |
| 调用级 model | 本轮已实现，完整门禁运行中 | AgentToolParams → FullInput → SubAgentRequest → runner；空值/inherit 继承，模型 ID 在当前 provider 上使用，不修改父会话。执行器在每次调用注入真实父客户端快照，同步/后台 runner 不再只用注册时的旧模型；构造级回归通过，尚无最终 HTTP 请求级验证 |
| 后台邮箱身份 | 本轮已实现，验证中 | spawn 前以任务 ID 绑定 request.identity 并完成注册；名称只作别名，避免重名并发任务抢读同一邮箱；匿名、命名、skill fork、subtask 使用统一入口；前台 async-runner 路径同样绑定身份，显式替换 hub 同步更新注册侧和执行侧 |
| 终态与广播 | 本轮已实现，完整门禁运行中 | 广播和可寻址名称只包括 Running；同名新任务重置运行状态，旧 ID 注销不能影响新任务。先注销再发布终态通知；测试以收到终态为条件立即断言 ID/名称不可寻址 |
| 子代理消息发件人 | 本轮已实现，完整门禁运行中 | 共享 SendMessage 工具原先读取父 Config，可能误报 lead；执行器现向 invocation 注入当前 agent_identity，不修改共享 Config。新增实际投递并由 lead mailbox drain 的回归 |
| 自定义 tools/disallowedTools | 已确认缺口，编辑受审批阻挡 | `src/agent/skills/custom.rs` 执行路径未将声明工具集合写入子配置；应复用执行器工具门并与父限制求交。可写代理委派被自动审批以“只读代理拒绝后转交属于权限绕过”拒绝，未换路径执行该编辑 |
| 自定义类型与 background | 已确认缺口 | 定义注册在 Skill 路径，AgentTool 固定内置类型，route 的 definition-background 参数固定 false；需统一定义解析入口 |
| 子代理 effort / worktree | 已确认缺口 | request 未贯穿对应运行配置；worktree 必须约束真实工具 cwd 和 git 路径，不能只创建目录 |
| 后台停止、输出查询、恢复 | 已确认缺口 | 未统一任务 registry/取消句柄；TaskOutput 与 subagent 输出分离，SendMessage 不恢复终态 transcript |
| 子代理 hooks 与 memory | 已确认缺口 | 需 identity 隔离、真实生命周期收尾与上下文注入；不能拿未接线 executor 代替实现 |
| Explore Bash | 有意保留现有权限门，强只读未完成 | 工具白名单不等于命令参数沙箱；现有免审批只读分类只看程序名的部分路径仍需单独审计 |
| shell Workflow | 已接线但有差异 | 列表与执行目录不一致；JSON 步骤非零退出没有收敛为整体失败 |
| Dynamic Workflow | 独立大项，尚未实现 | 当前 shell WorkflowTool 不能当成 JS agent/pipeline/parallel 编排、journal/replay、取消和恢复；不要继续扩充无生产构造的旧 WorkflowManager |

## 本轮验证

全目标编译与子代理、邮箱、AgentTool 定向测试运行中。完成后填入实际输出；不使用上一轮门禁替代。本轮没有运行参考项目脚本、真实模型请求或费用测试，没有提交或推送。
