# CCB 细节对标跟踪

日期：2026-11-18。参考：`.code/claude-code-main`，只读使用；架构决策依据 `docs/claude-code-alignment.md` 与当前生产调用链。参考副本为反编译后修改版本，其文档和功能声明不能替代源码证据。

## 验收方法

按输入/schema → 配置或请求 → 真实执行消费者 → 输出/生命周期 → 回归测试逐项验收。仅声明字段、增加提示或编译通过不算完成。当前范围为多 Agent、Explore、fork、消息与任务生命周期、自定义定义、相关提示，以及用户追加的问题清单；不把“完备”解释为整个参考仓库的产品功能全部移植。没有证据的能力不标为已实现。用户最新方向为功能/UI细节可模仿借鉴、以符合用户操作为准：优先设置真正生效、任务状态准确、键盘与输入顺畅，不追求参考仓库的全部架构移植。

共享工作区存在外部提交：当前记录基线 aa19fd5；部分本轮改动已被外部会话提交。本助手未提交或推送，不恢复或重复实施已进入 HEAD 的改动。

## 多 Agent 差距

| 项目 | 状态 | 证据及下一步 |
|---|---|---|
| Explore 专用提示与工具集合门 | 已实现，补验见验证记录 | prompt_builder 与 tool_executor 同时限制 schema 和执行；已从白名单移除 Bash，执行器回归显式覆盖 Bash 拒绝 |
| fork 结构化消息 | 上一轮完成 | 保留完成的消息与工具配对，排除当前未完成批次；不承诺相同系统提示或精确工具前缀 |
| poor 状态与文案 | 上一轮完成 | 管理本地建议偏好和持久化；自动建议生成尚无生产调用，不宣称降低模型费用 |
| 调用级 model | 构造级及 runner 到 HTTP 回归通过 | AgentToolParams → FullInput → SubAgentRequest → runner；空值/inherit 继承，显式模型只覆盖当前 provider 的模型。执行器每次注入真实父客户端，避免共享 runner 使用注册时旧快照。本机 HTTP 请求检查原父模型、更新后继承与显式子模型；测试直接调用子客户端，不覆盖真实执行器注入到最终 HTTP 的完整链 |
| 后台邮箱身份 | 完整门禁通过 | 启动返回前用任务 ID 注册邮箱；名称只是别名。同名并发隔离，前台 async-runner、后台、fork/subtask 统一入口；替换 hub 同时更新注册与消费侧 |
| 终态与广播 | 完整门禁通过 | 广播/可寻址集合只包括 Running；先注销再发布终态，旧 ID 收尾不影响同名新任务 |
| 子代理消息发件人 | 完整门禁通过 | 执行器向每个 invocation 注入当前 identity；实际投递及 lead mailbox 消费回归覆盖 |
| 自定义 tools/disallowedTools | 已接通，定向回归通过 | `SubagentType::Custom` 解析名称/别名；定义的 tools 与 disallowedTools 均和父 core/exclude 工具集合求交，只收紧不授权；schema 与直接执行共享同一注册表/执行门。frontmatter 支持行内、空格/逗号和 YAML 列表，camelCase `disallowedTools` 也覆盖。 |
| 自定义类型与 background | 已接通，定向回归通过 | `AgentTool` 接受自定义名称并在运行前解析定义；调用方显式 model/effort/max_rounds/background 优先，未提供时才采用定义默认值；未知名称在启动前报错。 |
| 自定义 memory/hooks | 已接通，定向回归通过 | user/project/local memory 从各自 `.star/agent-memory*` 读取并注入上下文；定义 hooks 保存在子代理 Config，不写回项目配置；项目/用户 hooks 与定义 hooks 均执行，定义级 hook 只增加约束。 |
| 调用级 effort | 实现接通，分层回归及完整门禁通过 | AgentToolParams → FullInput → SubAgentRequest → child 回合快照；执行器注入实际调用方的冻结值，显式 Off 优先。agent_llm 显式传 LlmRequestOptions，StarClient 冻结一次并供入口流式失败后的非流式 fallback 共用，RigAdapter 请求构造不再读全局。LLM 94 项通过，含 wire JSON、并发本机 HTTP low/high 与受控 fallback；这些并发 HTTP 直接调用适配器，尚非父执行器启动两个完整子代理的全链证明。Off 仍沿用 provider 方言，OpenAI 方言省略字段，不承诺服务端关闭推理。Skill 本轮 effort 改为经成功工具结果交回当前 Agent，不写进程全局值；真实加载 Skill 后的本地消费隔离回归 1 项和 frontmatter 8 项通过。普通内联 Skill 已补生产 run_agentic_loop → 本机 HTTP 两次请求回归：首请求 Low，模型调用真实 Skill 后第二次请求 High，包含 Skill 正文，全局 Max 不变，重装回合快照可恢复 Low；不覆盖 UI 或 StarAgent 外层入口。Skill context: fork 的 effort/model 和参数展开已接通，执行器每次刷新真实父客户端及冻结 effort；真实文件 → 执行器 → 子代理循环 → 本机 HTTP 回归通过，含前后台继承、显式覆盖、Off、并发 high/low、父设置不变及不继承父历史。工具限制、自定义 agent 解析元数据已由本轮定向测试覆盖 |
| worktree | 已接通，临时 Git 回归通过；非沙箱 | `AgentIsolation::Worktree` 实际执行 `git worktree add --detach`，子配置重建 Storage/RuntimeServices 并把文件、搜索、Shell、Notebook 等工具绑定到工作树；会话快照仍写发起项目。clean 且 HEAD 未变化自动移除，dirty/新提交/清理错误保留并回传路径。回归覆盖 clean 移除、dirty 保留、committed 保留；隔离 Shell 与管理命令清除继承的 Git 仓库定位环境变量，实际 Shell 相对写入不会修改父仓库文件。Git worktree 不是 OS 沙箱；绝对路径、symlink、主动脱离 Git 的 shell 子进程仍不作强隔离保证。 |
| 后台停止、输出查询、恢复 | 停止、历史输出查询与有效原生上下文恢复完整门禁通过；交接边界静态核查完成 | RuntimeServices 共享任务记录，runner 登记启动及最终输出，TaskOutput 查询同一实例并兼容旧 JSON 输出；路径经 Storage 解析，task_id 拒绝路径分隔符。新增版本化终态元数据，正文写入后用同目录临时文件和rename发布；当前进程记录优先，未命中从项目磁盘查询，元数据不携带输出路径，损坏、版本不支持、ID不匹配与读取错误明确报错。仅持久化终态，不把旧进程任务显示为仍在运行。跨全新运行时和独立进程查询均通过，完整门禁通过。新任务ID使用完整UUID，兼容既有短ID查询。新增 TaskStop 通过共享标志取消实际 agent 循环，保持外层消费到退出后收尾；初始化前、请求头等待与 SSE 期间停止回归通过；连接释放与审批取消边界进入完整库补验。新增Agent的resume参数引用项目内原生快照；本次创建新任务ID/邮箱/取消控制，使用当前客户端、权限模式和cwd，不覆盖来源快照。同步任务ID与快照ID已统一，runner在普通失败及协作取消后抓取有效上下文，快照错误显式返回。恢复不承诺压缩前完整历史或崩溃当下状态，SendMessage仍不恢复终态任务。首轮编译因Storage导入路径错误失败、已修；随后HTTP断言失败于无工具请求下rig将历史调用转为文本，已拆分为原生快照保留ID/配对与HTTP保留历史内容两类断言，HTTP定向通过。省略恢复类型在路由前继承，显式冲突拒绝；快照发布不覆盖既有来源。存储失败夹具曾被Storage向上解析影响，已改独立项目；bpinc51n9恢复过滤6项、原生快照过滤2项通过（均含仓库中既有同名匹配测试）。随后 b59rntctw 真实工具调用触及轮次上限后恢复1项通过；btpmchgl7 请求头前取消后恢复1项通过，旧停止标志保留，新执行使用独立控制。b7j99ddhq 当前权限拒绝回归通过：来源结束后实时总线切Plan，恢复的模型Write调用被实际执行器拒绝，目标文件不存在，失败结果含新任务ID。后台停止保留执行/快照保存错误原因。btem6a1i2 完整门禁通过：全目标locked编译、串行库2340通过/5忽略、文档0通过/3忽略、fmt/diff通过。其后仅调整快照发布后临时文件清理失败的处理，避免完整文件已发布却报保存失败，bb7ti05vu 原生快照过滤2项及fmt/diff补验通过。快照可见表示执行循环已交接，不承诺跨进程完成通知/邮箱注销的原子性 |
| hooks 生命周期 | 已接通，定向回归通过 | 每次执行尝试生成/保留独立任务 ID，开始与结束使用同一 ID；成功、初始化校验失败、执行失败、协作取消统一收尾。按调用方当前会话目录加载配置，每次任务冻结 Start/Stop 共用的目录；matcher 在生命周期事件中兼容类型展示名与 slug，普通工具 matcher 不受影响。未配置 working_dir 时在项目目录执行。后台控制入口在 Stop hook 前关闭，防止 hook 与最终通知状态冲突。两项目并发隔离、fork 前后台成功及恢复前失败/取消回归通过；本次新增目录切换与停止竞态回归的结果见验证记录。不保证进程退出或外层任务强制 abort。定义内 hooks 与项目/用户 hooks 叠加执行 |
| memory | 已接通，范围受限 | 自定义子代理支持 user/project/local memory 目录和 `MEMORY.md` 注入；读取不赋予权限，写入必须经正常文件工具和权限门。 |
| Explore Bash 强只读 | 采用禁用 Bash 的限制方案 | Explore 的 schema 白名单、执行门和提示词均排除 Bash，改用专用文件/搜索/导航工具；未实现通用 Bash 参数沙箱 |
| Dynamic Workflow | 已接通，真实执行链 13 项定向通过 | 支持 JavaScript `agent`、`parallel`、`pipeline`、`phase`、`log`、schema 校验、并发/调用上限、超时、取消、原子 journal 和条件 resume；JSON/SH 工作流保持兼容。Node → ToolExecutor → AgentTool → StarAgent → 本机 HTTP 回归覆盖父客户端继承、并行模型/effort、schema、resume 不重复请求和显式拒绝不发请求。已修子代理覆盖父工具注册映射、非 Auto 模式误调用分类器，以及失败清理误记 cancelled；真实外部取消仍记 cancelled。Node 执行环境不提供安全沙箱。 |

## 本轮修复

| 项目 | 已落地行为 | 验证与限制 |
|---|---|---|
| Bash 审批与执行状态 | 批准卡使用中性色并写明允许范围；工具调用行按实际结果显示成功/失败；失败正文合并错误摘要及详细输出，迟到开始事件不复活终态工具 | 新增带颜色的截图快照和双通道重复结果/迟到开始回归；最终测试结果见验证记录 |
| 子代理注册表 | 每个子代理分离注册映射与 schema 缓存，按当前配置重装 core/agent 工具；父级与兄弟的 runner、模型和工作目录不再被覆盖；共享审批、任务和消息句柄保持一致 | 新增父/子/兄弟映射与模型槽断言、自定义工具限制跨装配回归，最终结果见验证记录 |
| 非 Auto 审批 | 已安装 Auto 分类器也不能改变 Default/AcceptEdits 的审批语义 | 真实工作流恢复测试暴露 Default 意外调用分类器，增加模式守卫和零分类调用断言；不改用户权限配置 |
| 运行中补充消息 | 普通需求进入当前回合 steering 队列，独立消息 ID；审批等待不自动批准工具 | 新增 UI 发出与 runtime 入队/唤醒回归，复用 agent 消费一次和 Markdown 安全边界测试；分层测试不等于真实终端操作验收 |
| AgentGroup 汇总状态 | 后台计作运行，失败/拒绝与停止分别计数，纯停止和部分停止不显示成功勾；头部按当前主题取色并折行 | 3 项组件回归通过，含深浅主题、混合终态、1/2 列宽度上限及 24 列带颜色 snapshot；未做跨终端实测 |
| shell Workflow 路径与失败 | 列表/执行统一 Storage::project_workflows_dir，列出 JSON/sh/js；继承项目根 cwd；预校验完整 JSON，首个非零退出立即停止，保留步骤名/退出码/stderr；拒绝目录名称 | 前轮 23 项 workflow、4 项 i18n 及全目标编译通过 |
| Workflow 命令入口 | run 拒绝多余参数；create/edit 实际发送创建/编辑请求，格式说明改为 JSON/sh | 请求分发不等于文件已创建或脚本已执行；正常工具权限门仍生效 |
| Workflow 取消与时限 | 普通批处理/公开工具执行传入本回合 token；整工作流默认 300s、最大 1800s，审批等待不计入；Unix 独立进程组在取消/超时/future 丢弃时终止 | 先并行排空输出再回收组长，避免正常收尸后按旧 PID 发信号。Windows 仅保证直接子进程；主动脱离组的进程不在保证内。已补取消、实际后代停止和多步共享截止时间回归 |
| 审批 Shift+Tab | 全局分发在等待确认时不切权限模式，审批分支不直接批准；Enter 才提交 | 首次端到端回归纠正原核查：正常主入口原先提前切模式，未进入直接批准分支。详见核查报告 A4 |
| 长输入与方向键 | 展开输入超过 100 行继续用原生 textarea 滚动/光标；删除 50ms 滚轮猜测，滚轮走真实鼠标事件 | 120 行末尾可见、正文完整及连续历史导航回归已补 |
| 流消息与绘制 | 空闲等待同时接键盘/stream；处理消息后再绘制，动画流保留帧率门控 | 已补无键盘流唤醒和帧率门控回归；未做终端性能实测 |
| Ctrl+L | 在同步输出范围内清后端并更新差分空屏基线，不查询光标 DSR | 已补连续清屏、相同内容、中文/emoji/空白/样式重绘回归；未做终端闪屏实测 |
| 帮助与窄终端 | 帮助软换行、滚动条、方向/PgUp/PgDn/Home/End；提示退出条件、撤销/重做与审批字母别名；选择浮层尺寸先限制再居中 | 已补帮助末行可达、按键不改草稿、极窄/矮区域边界回归。EscapeGuard 修正文档以匹配20/100ms，不无依据放宽窗口 |
| MCP 调用错误保真 | refresh_tools 失败直接返回原错；manager 与 server 内层重连失败均保留首次调用和重连原因，共用错误合并函数 | MCP 16 项回归通过，包含 server 真实调用边界：模拟工具传输失败后重连配置错误/重试耗尽，最终错误同时保留两段原因 |
| Keychain 写入/查询/删除 | macOS 写入、查询和删除均走 security-framework 系统 API，保留 service/account 与新增或更新语义，秘密不进子进程 argv；只有 item-not-found 映射为缺失，其余系统错误保留 | Linux 回归通过；macOS CI 已接一次性托管 runner 的临时钥匙串测试，覆盖新增/更新/删除、旧命令生成条目互通及锁定库错误，详见 [平台验收](keychain-validation.md)。本机只有 Linux/WSL2，无包含本次变更的 macOS CI 结果；系统授权弹窗与任意既有登录库 ACL 的人工验收仍无法在本机完成 |
| 模型列表加载错误 | 传播 ProviderStore::load 的 Err，进入现有 ModelsError 提示 | ProviderStore 全局/项目/旧版坏JSON改为返回路径错误，不输出原始配置内容；启动入口移除unwrap_or_default；save拒绝覆盖坏配置；旧版迁移在保存成功后才归档。测试legacy路径也隔离到临时项目。by7j2lda9全目标编译和ProviderStore18项通过；其停止保存失败回归因夹具缺父目录失败，已修。bde8l83g3完整门禁通过：全目标locked编译、ProviderStore18项、后台停止3项、串行库2343通过/5忽略、文档0通过/3忽略、fmt/diff通过。随后移除会吞掉字段类型错误的二次解析（旧版字段已为统一设置子集），去重ID及编辑表单保留key的读取错误向上传播，新增类型错误与旧版别名兼容回归；bfzvj4g0k定向补验通过：全目标locked编译、ProviderStore19项、provider_form7项、fmt/diff通过 |

完整问题核查见 [2026-11-17 核查报告](archive/2026-11-17-ccb-issue-verification.md)。A1 参数注入、A3 同步技能 shell 阻塞、自定义工具限制涉及此前受拒绝的 custom.rs 编辑，当前未换路径执行；A2 Keychain store argv 已改为系统 API，macOS 平台验收尚缺；B1 索引锁污染已改为明确错误及调用图内存降级，不强行恢复受损状态；bniezsi42 的 indexer 13 项及 call_graph_db 18 项通过，真实进程级 token/sync/index-IO 锁在专用子进程污染后重复查询/调用验证通过，未覆盖 watcher 线程自身 panic 的自动重启。B3 shell 竞态未证实，不凭 expect 直接改成已修复。

## 验证记录

- 2026-11-18 真实终端补验：`b2l73wwm8` 全目标 locked 编译、串行库 2391 通过 / 7 忽略 / 0 失败、文档 0 通过 / 3 忽略及 fmt/diff 通过。其后通过独立 tmux 和本机 HTTP mock 运行真实 CLI，确认批准卡为中性的 Allowed once、工具行为失败符号；现场发现执行器失败封装丢失 stdout 和模型正文，以及仅产出工具调用时复杂度分析状态残留，已修。`b5exg9y7i` 的 Bash 失败过滤 2 项、无正文工具调用状态 1 项通过并成功构建 CLI；新终端实际键盘批准后可见独立的 `TUI fixture diagnostic` 诊断行，审批时无旧复杂度分析状态。该终端验收早于后续工具工作目录绑定改动，最终门禁以新增记录为准。

- 2026-11-18 工作目录与缓存隔离补验：全目标 `cargo check --workspace --all-targets --locked`、串行库 `2413` 通过 / `7` 忽略、文档测试 `0` 通过 / `3` 忽略、`git diff --check` 与 `cargo fmt --all -- --check` 通过。新增回归确认 RunTests、LS、NextEdit、ReadManyFiles、ProjectMap、RepoMap、CodeGraph、Search、Memory、GitInsight、GitHub PR、LSP、SmartEdit 及反思记忆均使用当前 agent worktree 根，Git 子命令清除继承的仓库定位环境；只读工具缓存按规范化 worktree 分桶，缓存命中仍重新走当前权限门，拒绝调用不会先广播成功的 ToolFinished。ExitWorktree 经真实 invocation 拒绝删除当前 worktree；共享权限管理器按 `permission_grants.json` 路径分桶，父/子 worktree 不复用持久授权。监听器仍是进程级单根；非托管搜索根回退同步构建，未宣称多 worktree watcher 隔离。测试只使用临时目录、本机命令与本机 mock，不运行参考仓库脚本或真实外部模型。

- 2026-11-18 截图与真实 Workflow 链补验：截图回归、steering 分层回归及 worktree 早版定向通过后，完整库曾为 2383 通过 / 5 失败 / 7 忽略。三项独立进程探针启动时报 ENOENT，发生在多个 Cargo 任务同时重链期间；另两项真实失败为共享工具注册表深度污染和普通错误误记 cancelled。修复后工作流首次重跑 12 通过 / 1 失败，定位到 Default 模式也调用已安装 Auto 分类器，增加模式守卫。bmcdx7bep 顺序补验通过：Workflow 13 项、非 Auto 分类器隔离 1 项、父子注册表/模型槽隔离 1 项。后续完整门禁以新增记录为准；不把早版定向结果当作最新全库结果。

- 2026-11-18 追加隔离/自定义代理/动态 Workflow 补验：全目标 locked 编译通过；自定义 frontmatter 14 项、子代理进度去重 6 项、Agent 工具处理 12 项、临时 Git worktree clean/dirty/committed 1 项通过；`disallowedTools` camelCase、定义 hooks 多项缩进解析和全局/定义 hooks 叠加行为已修。子代理同一工具经 direct chunk 与 AgentEvent 双通道到达时按 tool_call_id 幂等，不再重复展示。macOS Keychain 仍只有 Linux 回归和 CI 验收入口，无本机 Darwin 结果。

- 2026-11-18 隔离与生命周期补验：本轮策略按 agent identity 隔离，确认请求携带 identity 供总线再次判定；lead 保留共享模型切换槽，每个 child 独立建槽。父批次仅包含 Skill 时也刷新当前客户端。生命周期 matcher 的展示名/slug 归一仅作用于 SubagentStart/Stop；任务启动时冻结会话 hook 根。后台停止入口提前到 Stop hook 前关闭，避免 hook 与最终任务状态不一致。新增权限隔离、确认消息序列化/复核、模型槽独立性、hook 目录冻结、hook 执行期间停止请求的回归。bxi9jreq4 定向门通过：全目标 locked 编译、确认复核 1 项、runner 25 项、core hooks 17 项、内联 Skill HTTP 1 项、fork Skill HTTP 1 项。Clippy --lib --locked 完成（533 条既有警告，变更文件所见警告均在未修改行）；bfalm8h6n 最终门禁通过：串行库 2374 通过 / 7 忽略 / 0 失败；文档 0 通过 / 3 忽略；全目标 locked 编译、fmt/diff 检查通过。最初隔离测试错误地期待默认 Auto 对 Read 返回 AskUser，改用 WebFetch 并保留双向及兄弟隔离断言后通过；未更改生产默认权限。

- 索引锁污染与 worktree 创建补充门禁 bon7zs9rv：串行库 2367 通过 / 7 忽略；文档 0 通过 / 3 忽略；fmt/diff 通过。新增的两项忽略测试是仅由父测试显式启动的隔离子进程探针；父测试实际验证 token/sync/index-IO 全局锁污染，不把全局污染留给其余库测试。bniezsi42 全目标 locked 编译通过；buo8j1cuq worktree 真实 Git 回归 2 项及全目标 locked 编译通过。随后新增的内联 Skill HTTP 测试首次编译失败于误用 Config getter 名，已按真实 runtime_tool_registry 接口修正；第二次因模拟结论过短触发生产结论补问而耗尽两轮，保持生产规则并补足模拟结论。by3b04j41 补验通过：内联 Skill 模型循环 HTTP 回归 1 项、全目标 locked 编译及 fmt/diff。该回归验证首请求 Low、加载真实 Skill 后下一请求 High、全局 Max 不变及快照重装恢复 Low，未覆盖 UI/StarAgent 外层入口。

- 2026-11-17 fork Skill / 通用子代理 hooks / Keychain 阶段门禁 bha6fvbye：全目标 locked 编译通过；串行库 2361 通过 / 5 忽略；文档 0 通过 / 3 忽略；fmt/diff 通过。bksd4icb7 定向覆盖 fork 执行器到 HTTP 1 项、生命周期跨项目隔离 1 项、失败/取消后恢复 2 项及 core hooks 16 项。首次 fork HTTP 断言因夹具模型名未命中推理能力判断而失败，改用 gpt-5 测试模型后保留请求字段断言通过。Keychain 12 项含禁止 store 子进程的源码门；没有 macOS target 或实机，系统互通回归已提供为 macOS 显式忽略测试，未执行。Clippy --lib --locked 成功，变更文件所见警告为原有 unwrap_or 和 Default 建议，未宣称警告清零。fork 的此项测试从执行器进入真实子代理循环，不覆盖父模型自行生成 fork Skill 调用；普通内联 Skill 的生产模型循环到后续 HTTP 请求已由 by3b04j41 单独补验通过。

- 2026-11-17 调用级 effort / AgentGroup 汇总最终门禁 basi8jmfo：全目标 locked 编译通过；串行库 2358 通过 / 5 忽略；文档 0 通过 / 3 忽略；fmt/diff 通过。先前完整库曾因提示词中反引号 effort 被工具名引用门拦下，改为普通引号后 4 项引用检查通过。Skill 本轮 effort 不再写全局变量，成功结果在普通及自动触发路径交回当前 Agent，Skill 作为执行 barrier 保证后续同批 Agent 启动前消费设置；测试覆盖真实 Skill 加载后的局部消费与分段顺序，未将其称为完整模型循环到 HTTP 的端到端证明。Clippy advisory 成功完成，历史警告未清零。

- 多 Agent 前轮完整门禁：全目标 locked 编译通过；串行库 2287通过/4忽略；文档0通过/3忽略；fmt/diff通过。
- 本轮阶段结果：全目标编译通过；审批/方向键各1项、输入组件18项、workflow27项、输入路由64项通过。后续新增回归需以最终门禁为准。
- 第一轮完整库：2324通过、1失败、4忽略。唯一失败为提示词引用检查把 timeout_secs/steps/command 的反引号认作工具名；已改用引号，未扩白名单。
- 第二轮完整门禁 b8fgkcgy6 成功：全目标 locked 编译；库2324通过/4忽略；文档0通过/3忽略；fmt/diff通过，HEAD为ae54486加工作区。运行期间补入的少量后续回归与Workflow显式收尸调整，由下面的最终定向补验覆盖。
- 最终定向补验 bkskv3d60 成功：全目标 locked 编译；Workflow 7项、UI运行循环9项、帮助1项、公开工具批处理取消令牌1项全部通过；fmt/diff通过。覆盖取消和执行future丢弃后的后代停止、多步共享截止时间、帮助换行末行、stream唤醒与帧率。
- HTTP 模型回归代理未交付代码后已停止。主线程随后补入 runner 到本机 HTTP 回归：bvjoi99xf、bpaatgaf0 各通过1项，后者包含异常清理守卫；bpaatgaf0 后续 fmt 因并发加入的 MCP 改动未格式化而失败，不能称整个命令成功。格式化后统一补验 be3yiyv0e 成功：全目标 locked 编译；MCP 16项、HTTP 1项通过；fmt/diff通过。
- 后台输出首轮 b7qcviu78 成功：全目标 locked 编译；输出2项、runner收尾1项、环境登记2项通过；fmt/diff通过。随后将记录终态发布移到邮箱注销后，并扩充 runner → 通知消费 → TaskOutput 查询回归；最终补验 bsv0tmgu6 成功：全目标 locked 编译；runner跨入口1项、输出2项、提示词引用门4项通过；fmt/diff通过。
- 后台停止实现定向通过：TaskStop 按精确任务 ID 请求取消，与实际子代理循环共享中断标志；初始化前请求保留，收尾后才发布 Killed，UI 独立显示“已停止”。停止与完成在注册表锁内裁决。新增本机回归覆盖 SSE 未结束及响应头未返回时停止、重复请求、通知唯一、邮箱注销和停止后查询；恢复及重启后查询仍未实现。
- 停止首轮 bzq4r6udk 在编译阶段发现3处 Killed 显示分支遗漏，测试未运行；已补齐。一次重试因自动审批服务超时未启动；随后 b0p5hsn0p 成功：全目标 locked 编译；停止2项、初始化前1项、提示词门4项、词典门4项；fmt/diff通过。进一步补入取消后服务端连接关闭且不重试、审批等待取消令牌不执行工具、Killed 后迟到进度不复活且耗时冻结；完整串行门禁 bhjse4njs：编译通过，库2330通过/2失败/4忽略；两项停止测试均发现模型连接未释放，后续doc/fmt未运行。发现 rig_adapter 的独立驱动任务在等待网络时不会观察接收端关闭，已给共享驱动入口增加 tx.closed 与整个驱动生命周期竞争，OpenAI兼容及Anthropic共用。bzus5f6pf 定向仍为2失败；直接丢弃适配层流及直接 reqwest 也失败，后确认环境代理绕过列表缺少127.0.0.1，代理继续持有到模拟服务的连接，不能把这些失败全部归为客户端泄漏。禁用代理的直接 reqwest 回归通过；bzqxug14y 在测试子进程绕过回环代理后，适配层1项及后台停止2项均通过，保持原连接关闭和不重试断言。最终在 build_http_client 的 cfg(test) 分支禁用环境代理，未修改生产代理行为，也未在 Tokio 测试中写进程环境；移除临时诊断输出及直接 reqwest 探针，保留适配层取消回归。bf8iemrq0 完整门禁成功：适配层正常流及取消4项、后台停止2项、rig适配2项通过；全目标locked编译通过；串行库2333通过/4忽略；文档0通过/3忽略；fmt/diff通过。此前一次定向启动因自动审批服务超时未执行。审批测试覆盖执行器取消等待，尚非模型调用 TaskStop 到子代理审批的全链测试。

- 历史输出首轮 btxhllrn2 成功：全目标locked编译；输出3项、runner启动/收尾到全新运行时查询1项、停止2项、提示词门4项；fmt/diff通过。随后改完整UUID避免长期记录短ID碰撞，增加独立子进程读取completed/failed/killed记录，持久化失败追加到完成通知；bgk8rz4hd 成功：独立进程查询父测试1项（内部启动忽略的专用读进程测试并验证三种终态）、子代理模块37项；全目标locked编译通过；串行库2334通过/5忽略（新增1个仅由父测试显式启动的子进程入口）；文档0通过/3忽略；fmt/diff通过。读取历史输出不代表恢复原transcript。

- 原生上下文恢复 btem6a1i2 完整门禁通过：全目标locked编译；串行库2340通过/5忽略；文档0通过/3忽略；fmt/diff通过。覆盖正常恢复、轮次耗尽且工具调用返回拒绝结果后恢复、请求头前取消后恢复、当前Plan拒绝模型Write。原生快照保留工具ID及结果配对；无工具schema时HTTP历史允许rig按其契约转为文本，不要求线格式保留ID。没有证明成功执行Read后恢复或内部任务panic恢复。发布后的临时清理错误处理随后由bb7ti05vu原生快照过滤2项和fmt/diff补验通过。跨进程活动检测及完成通知/邮箱注销的原子性不在当前保证内，错误与取消交接独立静态审查完成：合并流等待两个分支结束，内部result_rx先安装会话再关闭direct发送端，当前普通失败和协作取消均完成交接；内部任务panic导致oneshot关闭时曾可能沿用旧上下文；新增每轮交接标记，缺失交接时发送明确错误并禁止发布本次快照，b9vy90r05定向补验通过：全目标locked编译、受控内部panic到后台失败/邮箱注销/无新快照1项、恢复过滤8项、后台停止3项、fmt/diff通过。这里只保证异常交接明确失败且不发布过期快照，不恢复panic前尚未交回的历史。最终完整门禁首次启动因自动审批服务故障未执行，待重试。新增停止同时快照目录不可写的回归已在bde8l83g3后台停止3项及完整库中通过，错误保留在通知及TaskOutput正文，output_error仍专指输出文件持久化错误。

未运行参考项目脚本、真实外部模型请求或计费测试。已运行本项目在临时目录中的受控 shell 回归；这与未执行攻击载荷、未枚举真实密钥相互独立。固定时延、误触比例、跨终端表现未测，不引用为事实。
