# 在 TUI 的 input 框里测 harness

本文是**可操作的测试手册** —— 不需要写任何 Rust 代码,全靠 TUI 的 input 框
和几个文件就能把 harness 的每条路径走一遍。

前置:先读 `docs/harness-integration.md` 了解协议本身。

---

## 0. 准备

```bash
cd /你的项目目录          # 建议用一个干净目录，别拿真实项目试
mkdir -p .star/remote
```

TUI 的 harness 轮询器每 **1 秒**扫一次 `.star/remote/inbox.jsonl`
（`runtime/background.rs` 的 `spawn_schedule_pollers`），所以任何写进这个文件的
请求，一秒内就会被 TUI 消费。

**关键认知**：在 TUI 里输入 `/remote send ...` 写进 inbox 的消息，会被**同一个
TUI 进程**的轮询器消费掉 —— 也就是「自己给自己发消息」。这正是最省事的测法：
不用开第二个进程。

---

## 1. 测 `/remote` 命令本身（input 框）

在 input 框输入：

```
/remote protocol
```

聊天区应出现协议说明：inbox / outbox 两个路径、四个动作（`send_message` /
`approve_tool` / `cancel` / `ping`）、协议版本 `starcode.remote.v2`。

```
/remote status
```

应看到 `📡 Remote Control 状态`、`- inbox: <路径>`、`- queued: 0`。

```
/remote send 测试一下远程投递
```

应看到 `✅ 已写入 remote inbox`、`- message: 测试一下远程投递`、`- queued: 1`。

**紧接着（约 1 秒后）**，聊天区应该出现一条 `[remote:...] 测试一下远程投递`
的 user 消息，agent 开始回应它 —— 这就是远程消息被消费的全过程。

如果 `/remote status` 显示 `queued: 0` 但没有消息出现，说明消息**已经被消费**了
（正常）；如果一直显示 `queued: 1`，说明轮询器没在跑，检查 `.star/logs/starcode_debug.log`
里的 `[REMOTE] poll failed`。

---

## 2. 测双向协议（ping / cancel）

用另一个终端观察响应：

```bash
# 终端 A：启动 TUI
starcode-cli

# 终端 B：往 inbox 写 ping
echo '{"protocol":"starcode.remote.v2","action":"ping","source":"tester","request_id":"r-1"}' \
  >> .star/remote/inbox.jsonl

# 终端 B：读响应
cat .star/remote/outbox.jsonl
```

期望：

```json
{"protocol":"starcode.remote.v2","request_id":"r-1","action":"ping","status":"ok","detail":"pong","created_at":...}
```

`request_id` 必须**原样带回** —— 这是 harness 配对响应的依据。

### 测 cancel

```bash
# 先让 TUI 跑一个长任务（让它 busy），然后：
echo '{"protocol":"starcode.remote.v2","action":"cancel","request_id":"r-2"}' \
  >> .star/remote/inbox.jsonl
```

当前回合应被中止，outbox 出现 `action=cancel, status=ok`。

---

## 3. 测 hook 事件（外部脚本挂钩）

hook 是外部 harness 最常用的集成面。

### 3.1 建一个 hook

在 `.star/settings.json` 里（没有就新建）：

```json
{
  "hooks": {
    "PermissionRequest": [
      { "hooks": [{ "type": "command", "command": "/tmp/hook.sh" }] }
    ],
    "PostToolUse": [
      { "hooks": [{ "type": "command", "command": "/tmp/hook.sh" }] }
    ]
  }
}
```

`/tmp/hook.sh`：

```bash
#!/bin/sh
{
  echo "event=$STAR_HOOK_EVENT"
  echo "tool=$STAR_HOOK_TOOL_NAME"
  echo "args=$STAR_HOOK_TOOL_ARGUMENTS"
  echo "call_id=$STAR_HOOK_TOOL_CALL_ID"
  echo "status=$STAR_HOOK_STATUS"
  echo "success=$STAR_HOOK_TOOL_SUCCESS"
} >> /tmp/hook.log
```

```bash
chmod +x /tmp/hook.sh
```

### 3.2 触发

在 TUI 的 input 框输入一个会触发工具调用的请求，例如：

```
读一下 README.md 的前 10 行
```

然后：

```bash
cat /tmp/hook.log
```

期望能看到 `event=PostToolUse`、`tool=Read` 等。

### 3.3 测 `PermissionRequest` + 远程批准（完整闭环）

这是 harness 最典型的用法：agent 要批准一个工具，外部脚本替人批。

1. 挂上 `PermissionRequest` hook，脚本里把 tool_call_id 写进 inbox：

```bash
#!/bin/sh
cat > .star/remote/inbox.jsonl <<EOF
{"protocol":"starcode.remote.v2","action":"approve_tool","request_id":"approve-1","message":"$STAR_HOOK_TOOL_CALL_ID","approved":true}
EOF
```

2. 在 TUI 里输入一个**需要确认**的工具调用，例如写文件到未信任目录：

```
在 /tmp/harness-test/ 写一个 hello.txt
```

3. 期望：hook 触发 → 脚本写入 inbox → TUI 轮询到 → 批准 → 工具执行。

4. `PermissionRequest` 的 `message` 字段填的是**确认请求的 id**
   （`STAR_HOOK_TOOL_CALL_ID` 那个），不是模型侧的 tool_call id。这两个是不同
   的命名空间。

> 注意：只挂 `command` 类型的 handler。`http` / `prompt` / `agent` / `mcp_tool`
> 四种类型会被静默跳过（执行器代码在 `core/hooks/executors/`，但那个目录没被
> `mod` 声明引用，不参与编译）。

---

## 4. 测 headless 的 `stream-json`

TUI 之外,headless 才是 harness 真正会用的入口。

```bash
starcode-cli -p "解释一下 Rust 的所有权" --output-format stream-json
```

每行一个 JSON。**重点看这几行**：

```bash
starcode-cli -p "读一下 README.md" --output-format stream-json \
  | grep -E '"type":"(tool_started|tool_finished|result)"'
```

期望能看到 `tool_started` / `tool_finished`（工具轨迹）和最后的 `result`
（带 `usage` / `cost_usd` / `is_error`）。

想只看用量：

```bash
starcode-cli -p "hi" --output-format stream-json | tail -1 | python3 -m json.tool
```

---

## 5. 测远程消息的完整往返

headless 下也能用远程控制（主流程跑完后会再收几轮 inbox）：

```bash
# 预先排队
mkdir -p .star/remote
echo '{"protocol":"starcode.remote.v2","action":"send_message","source":"ci","request_id":"r-1","message":"总结一下这个项目"}' \
  > .star/remote/inbox.jsonl

starcode-cli -p "解释索引" --output-format stream-json

# 响应
cat .star/remote/outbox.jsonl
```

期望 outbox 里有 `action=send_message, status=ok`，`detail` 是 agent 的实际回复。

`STAR_REMOTE_DRAIN_SECS` 控制收尾窗口（默认 3 秒，`0` 关闭）。agent 跑得比
你的批准快时，批准就送不到了 —— 那种情况用 TUI 或 `--permission-mode acceptEdits`。

---

## 6. 排障

| 现象 | 查哪 |
|---|---|
| inbox 里的消息没被消费 | `.star/logs/starcode_debug.log` 里的 `[REMOTE] poll failed` |
| 消息被消费了但没回应 | 看 chat_history 里有没有 `[remote:<source>]` 开头的 user 消息 |
| 响应写到了别的目录 | `/cd` 之后 cwd 变了 —— 现在两边都用 `current_dir_cached()`，应该一致；不一致就是 bug |
| hook 脚本没跑 | hook 事件名拼错（大小写/下划线）；`disableAllHooks` 未实现，所以不该拦 |
| hook 脚本跑了但没输出 | `type` 不是 `command`；或命令在 `sh -c` 下没找到 |
| `approve_tool` 报错 "没有挂起的确认请求" | id 填错了 —— 要 bus 的 correlation id，不是模型侧 tool_call id |
| `result` 行的 usage 是 0 | 这次运行没触发任何模型调用 |

---

## 7. 一页速查

| 想测什么 | 在 input 框输入 / 执行 |
|---|---|
| 协议说明 | `/remote protocol` |
| inbox 状态 | `/remote status` |
| 远程投递（自发自收） | `/remote send <消息>` |
| 外部 ping | `echo '{"protocol":"starcode.remote.v2","action":"ping","request_id":"r1"}' >> .star/remote/inbox.jsonl` |
| 外部取消 | `echo '{"protocol":"starcode.remote.v2","action":"cancel","request_id":"r2"}' >> .star/remote/inbox.jsonl` |
| 读响应 | `cat .star/remote/outbox.jsonl` |
| hook 是否触发 | `cat /tmp/hook.log` |
| headless 事件流 | `starcode-cli -p "..." --output-format stream-json` |
| headless 用量 | `starcode-cli -p "hi" --output-format stream-json \| tail -1` |
