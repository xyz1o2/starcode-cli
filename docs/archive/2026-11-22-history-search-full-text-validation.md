# 历史搜索原文恢复调查（2026-11-22）

## 基线与范围

当前共享基线已更新到 `2ccf6e5`；该提交不是本助手执行的提交。既有会话浏览器修复已在基线内，本次不还原或重写它们。

调查 `src/ui/events/modal_input.rs::handle_history_search` 的 Enter 路径：列表由 `search_history` 生成，其中 content 截断到 100 字符；用户条目按 index 从原始 chat_history 恢复，但助手条目回退到列表摘要。暂不改变搜索范围、列表显示或历史存储。

## 离线回归

新增 `history_search_restores_full_text_instead_of_the_display_summary`，经真实 `handle_key_event` 入口选择超过 100 字符、含 Unicode 和唯一末尾标记的历史内容。覆盖用户与助手条目，要求恢复全文、不提交消息；用户保留原有输入意图，助手保持 Literal，防止命令前缀获得本地执行资格。

任务 `b4yn33d1d` 退出码 101：1 passed、1 failed、2613 filtered out，既有图片恢复正对照通过。新增用例在打开历史搜索时先触发 Unicode 摘要截断 panic：`history.rs:180` 的字节索引 97 落在汉字内部。尚未到达 Enter，不能据此认定助手条目的原文恢复问题已由失败回归证实。

为这个已确认的崩溃另加两个摘要截断测试，覆盖长中文、emoji、ASCII、不截断的短文本，以及 max 为 0～3 的边界。摘要长度将明确为 Unicode 标量值数（不声称是终端显示列数）；保留既有 ASCII 的省略号输出。任务 `bbjqe14dm` 退出码 101：0 passed、2 failed、2615 filtered out，分别确认 `max - 3` 下溢和 UTF-8 非字符边界 panic。

生产修复仅将该摘要 helper 改为按 Unicode 标量值计数、取前缀，并为上限 0～3 返回对应长度的省略点。任务 `bgvbnaeyd` 的格式/diff 与两个摘要回归通过（2 passed）；原文恢复测试随后确认 Assistant 分支只恢复摘要，末尾标记丢失（1 passed、1 failed，退出码 101）。随后修复助手条目为按原始索引取全文并转为 Literal，用户图片和意图不变。任务 `bomxh3kmz` 退出码 0：全文恢复测试 2 passed；摘要截断测试 2 passed；模态测试模块 3 passed（包含上述两项恢复测试，不重复计为新增覆盖）。格式/diff、workspace all-targets locked 编译及新二进制构建均通过。共覆盖 5 项不同测试，本轮未重跑全量 lib 或 Clippy。

## 真实 TUI 验收

```bash
python3 -I scripts/test-history-search-tui.py target/debug/starcode-cli
```

驱动使用独立 tmux socket、临时 HOME/配置/git 项目及无路由、回环关闭的 user/network namespace。仅恢复本地合成夹具，使用 Plan 模式及不可访问的模型端点；不加载真实用户历史，不提交正文或调用模型。

任务 `bvt7245x0` 退出码 0，证据 `/tmp/starcode-history-search-m_ucaa10`，二进制 SHA256 `c8569129b467646e6c8018edd95a60caf5c037f449366fd8797d453d2adfa9d3`。真实 Ctrl+R 打开包含长中文和 emoji 的历史，按摘要之外的末尾标记过滤到唯一助手条目，Enter 后精确检查输入区域等于完整原文，并追加 q 确认仍可编辑。已人工读取过滤及恢复截图，恢复文本末尾包含 `END-OF-ORIGINAL`。不声称验收了其他平台、终端或真实模型交互。临时证据不会随仓库迁移，脚本依赖 Linux、Python 3.11+、tmux、unshare、ip 和 git。

## 约束

不提交、不推送、不调用付费模型；不修改用户配置，不并行运行 Cargo。此前会话浏览器验收不覆盖本次新增测试或后续修复。
