# 会话浏览器 j/k 搜索回归（2026-11-22）

## 范围

修复 `src/ui/events/input/bg_agent.rs::handle_log_selector_key`：移除会话浏览器的裸 `j/k` 导航别名，让两个字符进入已有的搜索输入分支。↑/↓ 导航及循环选择不变；主题选择器的 j/k 导航不变。未增加搜索模式、会话 ID 搜索或远端检索，不改恢复协议、权限或会话存储。

依据是本项目该处理器的“↑↓ 选择、输入过滤”约定，以及本地参考 `.code/openclaude-main/src/components/LogSelector.tsx` 的直接输入进入搜索行为。参考仅用于交互对照，不作为官方架构依据。

## 先失败的离线回归

通过真实 `handle_key_event` 入口和本地 mpsc 通道测试，不调用模型、不恢复真实会话。

任务 `byl5bqsfu` 退出码 101：1 passed、1 failed、2610 filtered out。

- `log_selector_types_jk_and_navigates_filtered_rows_with_arrows`：输入 `j` 后，搜索词实际为空，预期为 `j`，失败位置为 `bg_agent.rs:282`。
- `log_selector_loading_blocks_search_and_navigation_but_allows_escape`：通过。

第一项检查连续输入 j/k 后精确过滤到两项、首项选择、方向键在过滤结果中循环、不改底层草稿、不发送 agent 请求且不排队消息。第二项检查加载时字符、方向键和 Enter 不改变查询或触发恢复，Esc 仍关闭模态且保留草稿。

## 修复后验证

任务 `b5kdgr7i7` 退出码 0，串行验证结果：

- 选择器定向测试：6 passed、0 failed、2606 filtered out，包含新增两项回归和既有恢复排队测试。
- `ui::services::session_list`：4 passed、0 failed、2608 filtered out。
- `cargo fmt --all -- --check` 与 `git diff --check`：通过。
- `cargo check --workspace --all-targets --locked`：通过。

Cargo 使用 `CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1`，lib 测试使用 `--test-threads=1`。本次未重跑全量 lib 或 Clippy；此前记录的全量及真实 TUI 结果不覆盖本次生产改动。

## 真实 TUI 验证入口

任务 `bqdgkxr1q` 退出码 0：`cargo build --locked --bin starcode-cli` 成功。

新增 `scripts/test-session-browser-tui.py`，运行方式：

```bash
python3 -I scripts/test-session-browser-tui.py target/debug/starcode-cli
```

驱动使用私有 tmux socket、临时 HOME/配置/git 项目及无路由且回环关闭的 user/network namespace。准备三个空正文的本地会话夹具，以文件名作为标题；保留主输入草稿，通过 Ctrl+P 命令面板打开会话浏览器，依次检查 j、jk 过滤及方向键选中项，最后 Esc 关闭并追加字符确认草稿和键盘通路。不会在浏览器内按 Enter 恢复会话，也不会提交正文。保存逐步截图、二进制 SHA256、日志与 result.json。

第一次真实交互任务 `b42gnwcez` 退出码 1，证据 `/tmp/starcode-session-search-9wxfw963`：搜索 j/jk 与方向键步骤已通过，但驱动向 tmux 发送 `Esc` 时被按普通文本输入，查询变为 `jkEsc`，关闭步骤超时。将驱动键名改为 tmux 的 `Escape`，未因此修改应用退出逻辑。

重跑任务 `bzky7iku3` 退出码 0，完整交互全部通过，证据 `/tmp/starcode-session-search-xru5qz4a`。被测二进制 SHA256：`845b558a0631c95677cc5151a85f641bc70568ff66bd68ce58f0bd18599463dd`。已人工读取搜索和导航截图：`/jk` 仅显示两个匹配项，Down 将选择从 jk-first 移至 jk-second；退出后的截图无浏览器，草稿追加字符后为 `untouched draftq`。测试依赖 Linux、Python 3.11+、tmux、unshare、ip 和 git；临时证据不会随仓库迁移，复验须重跑脚本。

## 后续：未绑定修饰键污染查询

真实 `handle_key_event` 入口失败回归任务 `b3lklspru` 退出码 101：3 passed、1 failed、2610 filtered out。未绑定 Ctrl+G 将查询从 `jk` 改为 `jkg`，普通字符、Shift 大写和中文正对照通过。

最小修复在会话选择器的字符分支之前消费带 Control/Alt 的字符事件，包括带修饰键的 `/`，避免改写或清空查询。全局绑定仍由已有上层路由处理；未修改全局按键表。采用工程现有文本输入的 Control/Alt 过滤策略；某些平台的 AltGr 编码兼容性未在此验收。

新增回归检查四种未绑定组合键保持查询、选中项和底层草稿，且不发送或排队消息。真实 TUI 驱动增加 Ctrl+G/Alt+X 后的普通字符握手，同时将草稿断言加强为精确读取输入区域。任务 `bmrnt6rva` 退出码 0：选择器定向测试 8 passed、2606 filtered out；后台会话列表测试 4 passed、2610 filtered out；格式/diff、workspace all-targets locked 编译及新二进制构建均通过。真实 TUI 复验任务 `bas4xd9f5` 退出码 0，证据 `/tmp/starcode-session-search-qv9dsvn6`，最新二进制 SHA256 `256db77ec66c9d19562ebeb5ab745609688169baf0969dd6dfd6e21c95992dd2`。搜索、方向键导航、Ctrl+G/Alt+X 不污染查询、退格、Escape 关闭和草稿精确保留均通过。已人工读取握手后的查询 `/jkq`（无 g/x）及关闭后的精确正文 `untouched draftq`。本轮未重跑全量 lib 或 Clippy；先前二进制指纹仅作为历史记录。

## 边界

本轮不提交、不推送、不调用付费模型。未修改用户配置或共享工作区其他已有改动。上述真实 TUI 仅覆盖本地夹具的搜索、导航和退出；没有验收远端模型、真实会话恢复、其他平台或终端，也不据此宣称全工程无 bug。
