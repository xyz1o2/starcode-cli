# MCP 与命令补全离线验收（2026-11-21）

## 范围与边界

基线：`1dd19d1`。本轮不提交、不推送，不调用付费模型；Cargo 单实例、单作业，lib 测试串行。未更改生产代理策略、远端 MCP schema、重放策略或参数归一化规则。

本轮修改：修复 MCP 执行器测试缺失的 `StarToolCallFunction` 导入；补充 JSON 参数空值、数组、标量边界断言；修正关于字符串 `"null"` 的错误注释；增加 `scripts/test-command-completion-tui.py` 真实 TUI 验收入口。

## Rust 验证

回环 HTTP 夹具测试进程显式设置 `NO_PROXY=127.0.0.1,localhost` 和小写等值，避免继承的代理配置干扰本机请求；不修改生产 HTTP client。

- MCP 定向测试：29 passed，0 failed。
- MCP 服务端测试：3 passed，0 failed，覆盖 JSON-RPC 错误保留 code/message/data 且不重连重放、响应丢失后重连但不重放副作用调用、调用与重连失败诊断保留。
- `cargo test --workspace --locked --lib -- --test-threads=1`：2588 passed，0 failed，7 ignored。
- `cargo check --workspace --all-targets --locked`：通过。
- `cargo build --locked --bin starcode-cli`：通过。
- `cargo test --workspace --locked --doc`：0 passed，0 failed，3 ignored。
- `cargo clippy --workspace --all-targets --locked`：退出码 0；0 errors、618 warnings。此处不是 `-D warnings` 零警告验收。
- `cargo fmt --all -- --check` 与 `git diff --check`：最终检查通过。

上述 Cargo 命令使用 `CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1`。

## 真实 TUI 入口

依赖：Linux 的 user/network namespace、`unshare`、`ip`、`tmux`、`git`，Python 3.11+。

```bash
python3 -I scripts/test-command-completion-tui.py target/debug/starcode-cli
```

脚本以当前构建二进制运行，保存二进制 SHA256、逐例截图、日志及 `result.json`；每例独立 HOME、配置和 git 项目，防止草稿恢复和祖先配置串扰。tmux socket 位于新建的私有临时目录内，不复用默认服务端。服务端及 pane 在回环关闭、无路由的独立网络命名空间中运行；模型地址仅指向不可访问的回环端口，密钥是无效测试占位符。

覆盖：

1. `/zzzzzzzzzz` + Tab：完整输入不变、不提交。
2. `/zzzzzzzzzz` + Enter：原文提交并显示 Unknown command。
3. `/mcp zzzzzzzzzz` + Tab：完整输入不变、不提交。
4. `/mcp zzzzzzzzzz` + Enter：显示 unrecognized subcommand 与 Usage。
5. `/resum` + Tab：补全为 `/resume`，不提交。
6. `/resum` + Enter：只接受补全，不提交。

Tab 负例后再输入字符确认事件循环仍工作，并精确检查输入区域。启动阶段用字符输入及删除握手确认键盘线程接管后才开始用例。

### 实际执行记录

- 第一次运行 `/tmp/starcode-completion-probe-0rudt580`：首例通过，第二例因脚本共用 HOME 恢复了首例草稿而失败。已改为每例独立 HOME/项目，不属于应用补全失败。
- 第二次运行 `/tmp/starcode-completion-probe-4u2s5p96`：四个未知输入用例、`/resum` Tab 通过；最后一例发送 Enter 前输入首字符缺失，未进入 Enter 验证。已加强就绪判据，加入键盘握手，不将此结果计为 Enter 失败或通过。
- 带握手的最终运行 `/tmp/starcode-completion-probe-jun5wtm8`：退出码 0，六例全部通过。`result.json` 记录每例通过状态，`case0`～`case5` 的 before/after 文本及日志保留于该临时目录；Tab 负例另有 synced 截图。已人工读取顶层未知命令 Enter、MCP 未知子命令 Enter、正常补全 Enter 的最终界面，分别确认错误提示、Usage 和 `/resume` 仍留在输入框。临时目录不会随仓库迁移，复验应运行上述脚本重新生成证据。

测试运行输出位置（本机临时文件）：

- 定向 MCP：任务 `bvme9bx9z`。
- 全量 lib/check/build：任务 `b9f6nmuue`。
- doc/Clippy/fmt/diff：任务 `b07nwv5cx`。
- 最终真实 TUI：任务 `bcihsjtpv`。

任务输出目录：`/tmp/claude-1000/-mnt-d-ai-workspace-starcode-cli/8b3ac380-13d2-4066-b569-798a5e2ef36a/tasks/`。

被测二进制 SHA256：`99562621e854d1908102230be00aa69567a65e981333ef7bb1671ab64faf4f7b`。

## 后续发现：启动输入最终快照未同步

上述六例通过后追查首字符缺失，发现 `runtime/mod.rs` 在初始化结束时消费 `early_input_text`，却未使用它恢复输入框；原来只有 200ms 定时预览负责同步，初始化结束前最后一帧之后的输入可能丢失。现已将最终快照同步提取为 `restore_startup_input` 并接入启动路径：显式 `--message` 优先，否则先同步最终捕获，再恢复草稿。

新增三个无时钟、无终端依赖的回归：空／过期／已完整预览均以最终快照为准；草稿在前、最终多行输入在后且无重复；显式初始消息覆盖捕获并保留未消费草稿。此修复之后的复验任务为 `bmtm583ga`。三个新增测试均通过；全量 lib 为 2591 passed、0 failed、7 ignored，`cargo check --workspace --all-targets --locked` 通过。新二进制构建通过，构建后的六组真实 TUI 复验全部通过（任务 `btajjawx2`，证据 `/tmp/starcode-completion-probe-jfqebb1e`，二进制 SHA256 `c38ce3fda10e0d9953529390820187893bb73bfc965a9270114b6e4f4507d50b`）。修复后的 doc/Clippy/fmt/diff 门禁（任务 `b3kpbjetg`）全部退出码 0：doc 为 0 passed、0 failed、3 ignored；Clippy 为 0 errors、618 warnings；最终格式和 diff 检查通过。

### 最终快照补充边界

后续只扩展测试数据，未再改生产代码：加入最后一次预览后退格至空串、Unicode 与多行快照，验证空快照会清除旧预览，且 `input`、textarea、行数三者一致。定向复验任务 `br10lt01s` 退出码 0：3 passed，2595 filtered out；其中包含新增的清空与 Unicode 多行数据。该次格式与 diff 检查也通过。未重复执行全量门禁，上面的全量计数对应此前完整运行。

## 后续发现：早期 Ctrl+C 自连接与回收锁

`early_input.rs` 的捕获线程原来在 Ctrl+C 分支调用 `stop_capturing()`，后者拿出捕获线程自身句柄并 join；此外句柄锁在 join 期间仍被持有。现将按键处理返回为 `ControlFlow`，Ctrl+C 只返回退出意图，由捕获线程发布停止状态、清理终端并以 130 退出，不再等待自己。外部停止路径先取出句柄并释放锁，再 join。

新增离线测试覆盖 Ctrl+C 判定不改正文／不退出测试进程、回收线程时句柄锁可用、Unicode／退格／快捷键过滤。定向任务 `bg2mbsjs9` 退出码 0：3 passed，2598 filtered out。完整串行 lib/check/build/doc/Clippy/fmt/diff 任务 `bi7gbl2ly` 退出码 0：2594 passed、0 failed、7 ignored；check/build 通过；doc 为 0 passed、3 ignored；Clippy 为 0 errors、620 warnings；fmt/diff 检查通过。随后新构建真实 TUI 六例全部通过（任务 `bq0ebb4ff`，证据 `/tmp/starcode-completion-probe-q4x9rik_`，二进制 SHA256 `c6d7dad0acb46646224f41234e570683da6107998cea8258ab80b78a761f8c3e`）。

Clippy 比此前多两条：新增的嵌套 if、测试模块后的函数定义。已做语义不变整理（合并 if、将终端清理函数移到测试模块前），任务 `bhyhevdpd` 退出码 0：定向测试 3 passed、2598 filtered out；Clippy 0 errors、618 warnings，回到整理前的原有警告数量；格式及 diff 检查通过。上述完整门禁和二进制指纹对应整理前版本，不冒充整理后的重新构建验收。

### 受控 PTY 的早期 Ctrl+C 回归

新增 Linux 专用测试 `ctrl_c_exits_capture_process_and_restores_terminal`：用 Python 标准库 PTY 启动当前 Rust 测试二进制的隔离子进程，真实开启 raw mode、alternate screen 和早期捕获线程。普通字符 `q` 被捕获并确认后，父进程才发送 Ctrl+C 字节，不靠固定等待猜测线程是否就绪。父进程检查限定时间内退出码 130、termios 完全恢复、退出备用屏幕及恢复光标的转义序列，失败时仅清理自身子进程组。驱动为 `scripts/test-early-input-pty.py`，需要 Linux 与 Python 3 标准库，不启动完整应用或模型。

该测试与其他早期输入回归的定向运行任务 `bnbz5b8dt` 退出码 0：4 passed、2598 filtered out（0.47s）。真实捕获线程的 Ctrl+C 路径已在 PTY 子进程验证退出码和终端恢复；本轮仅新增测试，没有再改生产逻辑。最终格式、diff 与 Clippy 任务 `bxdv8cgt3` 退出码 0：格式及 diff 检查通过；Clippy 0 errors、618 warnings，没有新增警告。Linux 专用 PTY 测试依赖 `python3` 和仓库中的驱动脚本；非 Linux 不编译该测试。此前 2594 项全量运行不含新增 PTY 测试，该测试目前为定向通过。

## 后续发现：启动期 bracketed paste 被忽略

参考实现将 bracketed paste 作为独立正文事件；本项目在终端启用该模式后，早期捕获却只处理 `Event::Key`，忽略 `Event::Paste`。先建立真实 PTY 失败回归（任务 `b0412f91d`）：普通字符握手通过，随后 `/help`、中文、多行及 Ctrl+C 正文字节的整段粘贴未进入缓冲，出现 `bracketed paste was lost` 并退出 101。

修复将早期缓冲从纯 String 改为现有 `UserInput`，处理粘贴并规范换行、保留 `Literal` 意图；预览与最终恢复都传递这一意图，避免 `/` 或 `!` 开头的粘贴正文获得本地命令资格。键盘清空缓冲后才恢复命令资格；显式初始消息优先行为不变。PTY 子进程标志也改为精确匹配测试名称，避免仅存在环境变量就误进子进程路径。

新增启动恢复的命令前缀正文回归。修复后定向任务 `b03hnjeg7` 退出码 0：早期输入测试 5 passed、2599 filtered out（含真实 PTY 粘贴、Ctrl+C 及终端恢复）；启动恢复测试 4 passed、2600 filtered out（含命令前缀仍为 Literal）。完整串行 fmt/diff/lib/check/build/doc/Clippy 与新二进制六例交互任务 `b47zknsho` 退出码 0：lib 为 2597 passed、0 failed、7 ignored；check/build 通过；doc 为 0 passed、0 failed、3 ignored；Clippy 为 0 errors、615 warnings（仍非零警告门禁）；格式与 diff 检查通过。当前全量运行包含新增 PTY 测试。新二进制六组真实 TUI 交互全部通过，证据 `/tmp/starcode-completion-probe-mtzo3vzj`，二进制 SHA256 `325b6480d88da5e4e2436dacd4c1c853b30702e8b0aa925a302fce701d6a3b46`。此前结果保留作历史，最终验收以本次记录为准。

## 后续发现：正文仍生成命令补全

`refresh_command_hints` 原来只看 `/` 前缀与粘贴短保护，不看 InputIntent。失败回归任务 `bl5695hay` 退出 101：Literal `/resum` 仍打开命令候选；Literal 的 `@Cargo` 文件提示正对照通过。问题属于正文被补全改写／首次 Enter 被消耗，不据此声称获得本地命令执行资格。

现仅在命令候选生成处检查 CommandEligible，并在 Enter/Tab 接受前清除 Literal 的陈旧命令候选；不禁用文件提示，不改变显式会话／供应商菜单。定向 suggestions 测试任务 `b65h6nicl` 退出码 0：14 passed、2592 filtered out。真实 TUI 脚本增加第七例：bracketed paste `/resum`，越过粘贴保护期后 Tab 保持原文，并追加字符确认输入通路仍工作。完整串行 fmt/diff/lib/check/build/doc/Clippy 与七例交互任务 `bsdc2zigl` 退出码 0：lib 为 2599 passed、0 failed、7 ignored；check/build 通过；doc 为 0 passed、0 failed、3 ignored；Clippy 为 0 errors、615 warnings；格式与 diff 检查通过。新构建七组真实 TUI 交互全部通过，证据 `/tmp/starcode-completion-probe-4fqhwv6r`，二进制 SHA256 `c6a32de3e6b6e3087c5378ab912e7bbb728e4d0b0bdf7e9c857d05de2ac5dfeb`。已读取第七例 Tab 后及追加字符后的界面，分别确认输入为 `/resum` 和 `/resumq`，没有接受 `/resume`。

### 主按键 Enter 提交补充回归

新增 `literal_command_prefix_enter_submits_once_without_accepting_stale_hints`，通过真实 `handle_key_event` 入口与本地 mpsc，检查 Literal `/resum`、`/help`、无害 `!printf literal-only` 在正常和陈旧候选两种状态下一次 Enter 原文发送一次、不打开会话菜单、不接受补全。测试不运行模型或 shell，使用临时项目与纯 fixture。定向任务 `bu4pg37j6` 退出码 0：1 passed、2606 filtered out（0.22s），其中循环覆盖三种正文与两种候选状态。格式和 diff 检查通过。本轮仅新增测试，前述 2599 passed 的全量结果不含这项新增用例，未重复执行全量门禁。

后续加固仅将该测试的 ChatState 设为 Plan：若 Literal 保护失效而误入直接 shell 分流，也只能被拒绝，不能执行测试命令；正确正文仍须向本地通道发送一次。不修改实际会话设置或生产权限。加固后的定向任务 `b4gskup7j` 退出码 0：1 passed、2609 filtered out（0.18s），六种正文／候选组合均通过；格式及 diff 检查通过。仅测试状态发生变化，未再运行全量门禁。

### 后台候选刷新与清空正文的补充回归（2026-11-22）

仅新增三项测试，未改生产逻辑：后台插件候选不能重新激活 Literal 的行内命令补全；Literal 草稿不影响显式命令面板的插件候选；部分删除保持 Literal，完整清空后重新手动输入恢复命令补全且不能撤销回旧正文。串行任务 `b0c6jtso6` 退出码 0：suggestions 模块 15 passed、2595 filtered out；plugin_commands 模块 14 passed、2596 filtered out。格式和 diff 检查通过。未重新运行全量测试，前述 2599 passed 的全量结果不包含这些后补测试。

## 结论限制

以上是列明场景的离线回归，不是“全工程无 bug”的证明。真实 MCP 服务、远端模型交互、其他终端和平台均未以此验收。启动输入最终快照同步由三个确定性回归覆盖，且修复后的全量门禁和就绪后补全交互均通过；但尚未用可控的真实 PTY 同步点直接复现启动交接窗口，不能推断所有启动交接问题均已解决。
