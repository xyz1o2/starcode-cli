# 快速打开输入同步调查（2026-11-22）

## 范围

调查 `modal_input::handle_quick_open` 的 Enter、Tab、Shift+Tab 接受路径。当前代码直接追加 `state.input`，尚需确认是否导致 textarea 与提交正文不同步。不改变搜索算法、fd 参数、路径格式或已有按键含义。

## 先建离线回归

新增 `quick_open_acceptance_keeps_editor_text_intent_and_attachments_in_sync`，通过真实 `handle_key_event` 入口注入文件结果，不启动 fd、模型或 shell。三种接受键分别覆盖空输入、已有草稿、带图片的 Literal 草稿，共九组组合；检查 input、textarea、prepared_input、输入意图及图片一致，关闭模态后追加字符不丢路径且不发送消息。

任务 `b5qt402gg` 退出码 101：0 passed、1 failed、2617 filtered out。首组 Enter + 空输入即失败，textarea 仍为空，预期为 `src/example.rs `；后续组合尚未执行。

生产修复合并三种接受键的追加逻辑，保留 Enter/Shift+Tab 纯路径、Tab 带 @ 的含义。先取消选择区并将光标移到末尾，向 textarea 插入，再用既有 sync_input_from_textarea 同步 input、行数和草稿；不调用会清除附件或重置意图的 setter。任务 `bwy2a2wa2` 正在串行验证九组组合、相邻模态回归、格式/diff 及 workspace all-targets locked 编译。

## 边界

不提交、不推送、不调用付费模型、不并行运行 Cargo，不覆盖共享工作区已有历史搜索修改。此前其他功能的绿色门禁不覆盖本次调查。
