# CCB 问题清单核查

日期：2026-11-17。基线：`c540e72` 加当前未提交工作流修改。核查方式：静态源码及生产调用链；未运行攻击载荷、密钥枚举、危险命令或 TUI 延迟实测。此前已启动、随后完成的工作流验证结果为全目标编译通过、23 项 workflow 定向测试通过、4 项 i18n 测试通过；这些测试没有覆盖取消、进程树治理或本清单全部问题。本报告核查问题存在性，不代表已修复。A1–A4、B1–B4、C1–C9 均已完成静态核查；安全部分已整合独立只读核查，UI 部分由主线程直接核对源码及调用链。固定时延、实际误触率及跨终端表现仍需动态验证。

## 判级原则

当前没有足够证据将 A 组四项一律定为 P0。A1 参数变代码与 A2 argv 暴露优先按 P1 安全缺陷处理，明确执行授权、操作系统和本机可见性边界；A3 是异步运行时阻塞与子进程治理问题；A4 经后续回归纠正为审批上下文键盘归属错误：正常主入口提前切换权限模式，未达到直接批准分支，建议 P1 交互安全问题。固定延迟、误判比例和跨环境表现未经动态测量，不作为已证实指标。

## 已核实边界

### B1 锁污染

部分成立。`src/core/context/call_graph_db.rs:101/124/153/161` 的四处运行时 `expect` 均存在，token 表和同步锁为进程级共享对象；`src/core/context/indexer.rs:204/244/258` 三处也存在，但后两处 collector 锁在每次 `index_project` 内新建，并不跨调用永久保存；如果该 panic 继续展开穿过持有 INDEX_IO_LOCK 的外层调用，仍可能连带污染进程级索引锁。持锁代码发生展开式 panic 后，共享锁上的后续调用可持续 panic；没有证据证明任一处必然导致整个进程或所有功能崩溃。`ToolExecutor::execute` 在 `src/agent/tool_executor.rs:464` 捕获工具 panic，后台索引部分路径使用 `spawn_blocking` 并处理 JoinError。watcher 独立 worker 没有相同恢复边界，仍有相关索引功能失效风险。简单 `into_inner` 不能替代受损数据一致性检查。

### B2 Workflow 取消与超时

部分成立。`src/core/tools/workflow.rs:62` 忽略取消信号，没有内部超时；主循环 `src/agent/agent_tool_process.rs:542` 的批处理还传入空取消 token，WorkflowTool 不在 barrier 工具清单中。因而补 invocation 的 select 本身不够，还需连通取消信号。

但并非完全没有超时：`src/agent/tool_executor.rs:33/543` 提供默认 1800 秒 watchdog，覆盖注册工具执行。HEAD 的 workflow 没有 kill_on_drop，watchdog 到期也未保证终止子进程；当前工作区已在 JSON/sh 两条命令路径增加 kill_on_drop，尚不保证整个进程树终止。未运行死循环复现。

### B3 shell child 槽断言

竞态与 agent worker 崩溃未证实。`src/core/tools/shell.rs:1082/1167` 两处 expect 存在，但后台分支取出句柄后立即返回；前台分支在交给后台 watcher 前独占句柄，转后台后同样立即返回。stdout/stderr reader 并不持有 child_slot。未找到可达的并发取空序列。外层工具执行还存在 catch_unwind，不能声称该断言必然 panic agent worker。

### C7 键位可发现性

部分成立。`src/ui/events/input/mod.rs:789` 的 Ctrl+D 在非空输入/任务面板可见时只吞键；帮助仅写 Exit。`src/ui/events/input/mod.rs:1709/1716` 有 Ctrl+Z 和 Ctrl+Y 行为，但帮助未列，且 Ctrl+Y 有 kill-ring yank 与 redo 两种语义。`?:1527` 仅空输入打开帮助，文案 `ui.help.other.toggle` 未说明该条件。全局键注册表已经存在于 `src/ui/events/keymap.rs`，但没有覆盖所有上下文内键。

“Shell workflows”列 JSON 并非必然错误：JSON 格式的 command 步骤同样通过 bash 执行。这项更适合澄清术语，不能作为正确性缺陷。

### C9 交互模式 API Key

占位符启动属实；缺少请求校验不成立。`src/main.rs:326` 有意允许交互界面先启动，headless 另行拒绝缺密钥。`src/llm/client.rs:1009` 在流请求前按 provider 的 requires_api_key 检查有效密钥并返回配置提示；普通 chat 也有同类检查。免密钥 provider 不应被统一启动拦截。首次输入前提示是否足够及时属于 UX 问题，不能据此声称占位密钥未经校验直发。

## 安全部分独立核查

### A1 参数注入内联 shell

确认存在于自定义 agent 路径。`src/agent/tools/skill.rs:265` 将调用参数放入 SubTask，`src/agent/skills/custom.rs:145` 起读取参数，`:169` 替换正文，`:173` 执行替换后正文中的内联命令。`src/agent/skills/loader.rs:381` 的参数替换不转义，`:549` 起直接调用系统 shell，未经过 Bash 的逐命令权限检查。外层 Skill 工具仍过权限门，因此不是完全无授权；不可信外部内容还需通过模型形成工具参数，未证实未经模型的直接输入流。建议高优先级 P1，而非无条件 P0。

普通 SKILL.md 路径 `src/agent/tools/skill.rs:1012` 先 render，再在下一行替换参数，不受同一顺序缺陷影响。运行时加载未发现来源信任强制检查；marketplace 内嵌资源与远程 git 来源应分开描述。先执行模板内联命令再替换参数能阻止参数引入新命令块，但不能替代来源信任或完整 shell 权限治理。

### A2 Keychain argv

确认存在，限 macOS。`src/core/secure_storage/keychain.rs:31` 起的 store 使用 `security add-generic-password -w <value>`，秘密在子进程存活期间进入 argv；可见性取决于本机进程检查权限。当前生产构造经 SecureStorageManager 到 `/local-vault`（`src/commands/parity/memory.rs:289`），不能泛称 provider API key 必经该路径。非 macOS 返回 Unsupported。建议 P1；使用系统 API 或经确认支持的安全输入方式，不能未经核实就假定 security 子命令支持 stdin。

### A3 Tokio worker 阻塞

确认存在。`custom.rs:173` 在 async execute 内同步调用 loader，`loader.rs:549` 起用 std::process，`:604` 每 50ms thread::sleep。单命令通常有 5s 限制，不代表永久占死整个 runtime；多条命令累积阻塞。退出后才同步读 stdout/stderr，也有大输出导致管道阻塞、继承管道的后代使读取无期限等待的风险。宜使用异步子进程、并行排空输出和全生命周期取消/超时；spawn_blocking 只能隔离线程阻塞，不能自动提供取消。

### B4 错误吞噬

- model_list：`src/agent/model_list.rs:279` 的 store.load().await.ok() 确认丢失配置错误；其他 API 失败部分有 debug 日志，但常返回 Ok 的空列表或当前模型合成条目。外层还有超时错误提示，不能称所有失败都静默。
- MCP：`src/core/mcp/manager.rs:106` 的 refresh_tools 错误被忽略，可能把刷新失败误报成 Tool not found；不能外推为整个 MCP 初始化错误都被吞，discover_all 和上层有错误表与提示。
- Keychain：`src/core/secure_storage/keychain.rs:59` 起 get 将所有非零状态归为 Ok(None)，权限拒绝与不存在不可区分；delete 也将非零状态泛化为 NotFound。store 保留错误。上层 vault 的 get 因此会误报条目不存在。

## 审批与 UI 核查

### A4 Shift+Tab 直接批准

**后续生产路由回归纠正：原“直接批准可达”结论过强。** `src/ui/events/input/tool_approval.rs` 的 BackTab 分支确实包含直接发送 ProceedOnce 的代码；但 `handle_key_event` 在跳过前置模式切换守卫后，仍经过 `dispatch_global_binding`，其中 TogglePlanMode 又无条件调用 cycle_approval_mode，因此当前正常主入口提前切换权限模式，并未到达该快速批准分支。新增端到端按键回归发现了这一点。实际缺陷是审批上下文的键盘归属错误，另有潜在直接批准分支。修复已同时给全局分发补守卫，并移除审批中的直接批准代码；验证要求保持确认待处理、权限模式不变且不发批准消息，只有 Enter 才提交。审批提示此前也未列 Shift+Tab。

### C1 流消息唤醒与绘制顺序

部分成立。`src/ui/app/runtime/mod.rs:599` 先画，`:620` 后取 stream，`:672` 只等待 key_rx，流消息到达不能唤醒该等待。空闲无动画时 wait 最多接近 1s；处理中或流式输出时 `is_animating` 为真，等待使用帧预算，不能说每次首 token 固定延迟 1s。即使已取到消息，本轮末尾仍可能等待后才进入下轮绘制；animating 是处理 stream 之前取的快照，状态由空闲变活跃时仍可能使用旧等待预算。应将流通道加入唤醒条件，并使已收到的状态变更及时绘制。未做时延实测。

### C2 帮助弹窗无滚动

确认存在。`src/ui/components/help_popup.rs:137` 将高度限制为终端高度减 4，`:148` 的 Paragraph 没有 scroll。`src/ui/events/input/mod.rs:1048` 的 Help 路由只处理关闭键，其余键直接返回，PgUp/PgDn 无法访问超出高度的条目。24 行终端最多给弹窗 20 行（含边框），后半内容不可达。

### C3 超过 100 行的输入显示

确认存在于展开输入。`src/ui/components/chat_input.rs:126` 以 input_line_count > 100 标记 truncated；没有粘贴引用且未折叠时，`:169` 转入 Paragraph，`:332` 只取前 100 行，不再使用原生 textarea 的光标滚动绘制，也没有省略数量提示或 set_cursor_position。正文仍保留，问题是显示和编辑反馈失效，不是提交内容被删除。折叠预览和粘贴块另有分支；正常不超过 100 行的原生 textarea 路径支持滚动，不能概括成所有输入都没有滚动。

### C4 Ctrl+L 清屏

确认存在清屏和同步光标查询风险，固定两秒与闪屏程度未实测。`src/ui/app/runtime/mod.rs:602` 调用 terminal.clear。Cargo.lock 锁定的 ratatui-core 0.1.2 在 `src/terminal/buffers.rs:147` 的 clear 中先 backend.get_cursor_position，再清视口并重置后缓冲；ratatui-crossterm 0.1.2 的该方法调用 crossterm::cursor::position。因此该风险有依赖源码依据，不只来自项目注释。与键盘读取线程竞争及终端响应情况会影响结果，不能把“约 2s”当成已测结论。替换为帧内 Clear 时仍需确保物理屏幕重绘语义，不能只画一层随后被覆盖的 Clear 就宣称等价。

### C5 Up/Down 误认滚轮

确认存在。`src/ui/events/input/mod.rs:1587/1652` 共享 pending_scroll_time，相邻 Up/Down 小于 50ms 时直接调用 scroll_chat，跳过历史或 textarea 导航。真实键盘快速连按、自动重复乃至混合方向都可能进入该条件，不依赖 Event::Mouse。原报告“约半数”未经统计，不作为结论。移除该猜测前可另核对是否还需支持把滚轮编码成方向键的终端兼容场景。

### C6 EscapeGuard 文档与实现

确认不一致。`src/ui/app/runtime/key.rs:13/15` 写 250/500ms，`:33/34` 实际为 20/100ms。拆分序列中 Esc 到 '['/'O' 的间隔超过触发窗时不会被此 guard 吸收，这是静态条件；WSL/SSH 上发生频率未测。不能只为匹配旧注释就直接放宽窗口，需结合误吞正常输入的权衡校正文档和实现；环境变量化属于可选实现方案。

### C7 补充：审批字母键

`src/ui/events/input/tool_approval.rs:166` 起支持 y/s/a/d/n，但仅更改选中项，仍需 Enter，不能与 Shift+Tab 的立即批准混为一谈。审批卡片提示只列数字选择，字母别名未展示，可发现性问题成立。统一注册表应在已有全局表基础上扩展上下文键，而不是重复建立另一份真相源。

### C8 窄终端弹窗越界

部分成立，24 列示例不成立。真实调用来自已接线的 provider/session 选择浮层，`src/ui/components/command_suggestions.rs:136` 使用 68%，`:159` 强制至少 20 列，却以扩宽前的宽度计算 x。24 列时计算宽 16、左边距 4，最终 20 列刚好到右边界；22 列时计算宽 14、左边距 4，最终右边界 24，已超出 area。高度同样有 max(3) 超出极矮区域的风险。应先将最终尺寸限制到 area，再计算居中位置；不能据此直接断言发生 panic，具体裁切由 widget 实现决定。

## 最终确认结论表

| 原编号 | 结论 | 判定边界 |
|---|---|---|
| A1 | 确认存在 | 自定义 agent 参数先替换后执行内联 shell；普通 SKILL.md 顺序相反 |
| A2 | 确认存在 | 限 macOS vault Keychain 路径，argv 可见性依赖本机权限 |
| A3 | 确认存在 | 同步子进程占当前 Tokio worker；不等于整个 UI 必然永久卡死 |
| A4 | 部分成立（回归纠正） | 主入口提前切换权限模式；直接批准代码存在但该路由未到达。两层行为均已修改，待本轮验证 |
| B1 | 部分成立 | 共享锁污染可导致后续 panic；全进程崩溃未证实 |
| B2 | 部分成立 | 取消未接通、无内部超时；外层有默认 1800s watchdog |
| B3 | 未证实 | 有 expect，未找到可达的并发取空路径；外层捕获工具 panic |
| B4 | 确认存在 | 点名的三类错误丢失存在，其他部分错误仍有日志或 UI 提示 |
| C1 | 部分成立 | stream 无法唤醒键盘等待，且先画后处理；不是固定 1s 首 token 延迟 |
| C2 | 确认存在 | 帮助内容超过终端可用高度后不可滚动访问 |
| C3 | 确认存在 | 展开输入超过 100 行只画前 100 行，失去光标滚动；正文未删除 |
| C4 | 部分成立 | 清屏同步查询光标有源码依据；2s 和实际闪屏未测 |
| C5 | 确认存在 | 50ms 启发式会把真实方向键转成聊天滚动；半数比例未测 |
| C6 | 部分成立 | 文档与常量矛盾确认；跨环境漏判频率未测 |
| C7 | 部分成立 | 条件与别名提示不全；已有全局注册表；Shell workflows 标题不是必然错误 |
| C8 | 部分成立 | 更窄终端可越界，24 列当前刚好容纳；未证明 panic |
| C9 | 部分成立 | 允许占位符启动属实；请求前已按 provider 校验密钥 |

建议先处理 A1/A2/A3/A4、Workflow 取消及错误保真，然后处理输入显示和事件唤醒。A 组不能统一按 P0，B3 不宜按已证实缺陷修补。修复须分别验证权限边界、进程回收和交互行为，不能用原有 workflow/i18n 测试代替。以上表格描述核查基线；后续在用户持续目标下开始实施修复，当前状态以 docs/ccb-detail-alignment.md 的本轮记录为准。没有提交或推送。
