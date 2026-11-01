# 外部 harness 集成指南

这份文档描述**外部进程**（CI 脚本、编排器、IDE 插件、另一个 agent）如何驱动
StarCode：拿到完整过程、双向控制会话、读回结果。

对标的是 Claude Code 的 `--output-format stream-json` 与 headless 模式。

---

## 1. 三种能力

| 能力 | 入口 | 状态 |
|---|---|---|
| **单向取结果** | `starcode-cli -p "<prompt>" --output-format stream-json` | ✅ |
| **双向控制会话** | `.star/remote/inbox.jsonl` + `.star/remote/outbox.jsonl` | ✅ |
| **把 StarCode 当 MCP server** | `starcode-cli mcp serve` | ⚠️ 只暴露 2 个代码检索工具 |

---

## 2. 单向取结果：`--output-format stream-json`

```bash
starcode-cli -p "解释一下 Rust 的所有权" --output-format stream-json
```

每行一个 JSON 对象。

### 2.1 事件行

`type` 取值（**对外契约**，改名会让已集成的 harness 静默失效）：

| `type` | 含义 |
|---|---|
| `text_delta` | 模型正文的增量（`content` 字段） |
| `reasoning_delta` | 思考内容的增量 |
| `trace` | 阶段轨迹（`event` + `payload`）—— 任务分类、上下文解析、自动计划决策等 |
| `tool_started` | 模型决定调工具（`tool_call`） |
| `tool_finished` | 工具执行完（`tool_call` + `result`） |
| `tool_progress` | 工具执行进度 |
| `message` | 最终助手正文（**只在有工具调用的回合出现**） |
| `error` | 出错（`content` 是错误文本） |
| `turn_finished` / `done` | 回合结束 / 整体结束 |
| `stats_update` | token 用量（`token_usage`） |

> **纯文本回答没有 `message` 事件** —— 那个只在有工具调用时发。正文要从
> `text_delta` 累积。

### 2.2 `result` 终态行

**总是最后一行**。外部 harness 靠它判断"跑完了"，不需要猜"最后一个事件是不是结束"。

```json
{
  "type": "result",
  "is_error": false,
  "duration_ms": 26561,
  "cost_usd": 0.0020297400000000004,
  "session_id": "headless-1792877758",
  "usage": {
    "prompt_tokens": 14052,
    "completion_tokens": 863,
    "total_tokens": 14915,
    "cache_read_tokens": 13568,
    "cache_creation_tokens": 0,
    "cache_telemetry_reported": true
  }
}
```

- `session_id` —— `null` 表示没落盘（见 §4）；非 null 时可用来 `--resume`。
- `usage` —— **整次运行**的总量，包含远程消息触发的那几轮。

### 2.3 对比其它格式

| 格式 | 内容 | 什么时候用 |
|---|---|---|
| `jsonl`（默认） | 每行一条最终对话条目（`ChatEntry`） | 只想拿助手正文 |
| `text` | 纯文本 | 人看 |
| `stream-json` | 完整事件流 + 终态行 | **外部 harness** |

`jsonl` 只有助手正文 —— 工具轨迹、token 用量、成本全都没有。

---

## 3. 双向控制：remote inbox 协议

### 3.1 文件

| 方向 | 文件 | 语义 |
|---|---|---|
| harness → StarCode | `<project>/.star/remote/inbox.jsonl` | 每行一个 JSON，追加写 |
| StarCode → harness | `<project>/.star/remote/outbox.jsonl` | 每行一个 JSON，追加写 |

**StarCode 在主流程结束后还会收几轮 inbox**（默认 3 轮 × 1 秒，`STAR_REMOTE_DRAIN_SECS`
可调，`0` 关闭）。所以「启动 headless → 写一条批准 → 等 agent 继续」是可行的。

### 3.2 请求

```json
{
  "protocol": "starcode.remote.v2",
  "action": "send_message",
  "source": "ci-bot",
  "request_id": "r-1",
  "message": "请检查 main 分支构建失败原因"
}
```

| 字段 | 必填 | 含义 |
|---|---|---|
| `protocol` | 否 | `starcode.remote.v2`（默认）。`starcode.remote.v1` 仍接受，但只有 `send_message` |
| `action` | 否 | 见下表。默认 `send_message` |
| `message` | 看动作 | `send_message` 是消息正文；`approve_tool` 是 tool_call_id |
| `source` | 否 | 请求方标识，响应里回显 |
| `request_id` | 否 | 响应原样带回，harness 靠它配对 |
| `approved` | 否 | `approve_tool` 用：批准还是拒绝（默认批准） |
| `always_allow` | 否 | `approve_tool` 用：是否「本会话总是允许」 |

### 3.3 动作

| 动作 | `message` | 效果 |
|---|---|---|
| `send_message` | 消息正文 | 往会话里追加一轮对话，**接在当前会话后面**（不是另起一段） |
| `approve_tool` | tool_call_id | 批准 / 拒绝一次挂起的工具确认 |
| `cancel` | — | 中止当前回合 |
| `ping` | — | 探活，回一条 `pong`，不动会话 |

### 3.4 响应

```json
{
  "protocol": "starcode.remote.v2",
  "request_id": "r-1",
  "action": "send_message",
  "status": "ok",
  "detail": "我在呢",
  "created_at": 1792877758
}
```

`status` 是 `ok` / `error`。`request_id` 与请求一致（请求没带就没有）。

### 3.5 完整例子

```bash
# 启动前排一条
echo '{"protocol":"starcode.remote.v2","action":"ping","request_id":"r-1"}' \
  > .star/remote/inbox.jsonl

starcode-cli -p "解释索引" --output-format stream-json &

# 稍后读响应
cat .star/remote/outbox.jsonl
```

### 3.6 批准工具的注意事项

- `approve_tool` 的 `message` 填的是**确认请求的 id**（UI 上那张卡挂着的那个）。
  纯文本回答没有确认卡；只有需要人工确认的工具调用才有。
- headless 默认**不**等人工确认（策略层把 `AskUser` 判成 `Deny`）。挂了外部 harness
  之后总线的"有人能回答"标志会被打开，等待层才愿意等。
- 若 agent 跑得比你的批准快，批准就送不到了 —— 工具已经执行或已被拒。这种情况
  应该用 TUI（人在旁边）或 `--permission-mode acceptEdits`。

---

## 4. Hook 事件（外部脚本的挂钩点）

hook 是外部 harness 最常用的集成面：在 `.star/settings.json` 的 `hooks` 段里按事件名挂
shell 脚本，脚本通过 `STAR_HOOK_*` 环境变量拿到上下文。

### 4.1 支持的事件

| 事件 | 触发时机 |
|---|---|
| `SessionStart` / `SessionEnd` | 会话开始 / 结束 |
| `UserPromptSubmit` | 用户提交提示 |
| `PreToolUse` / `PostToolUse` | 工具执行前 / 后 |
| `PostToolUseFailure` | 工具执行**失败** |
| `PermissionRequest` | 弹出工具确认**之前** |
| `PermissionDenied` | 工具被**策略**拒绝 |
| `Stop` / `StopFailure` | 停止 / 停不下来 |
| `PreCompact` / `PostCompact` | 上下文压缩前 / 后 |
| `SubagentStart` / `SubagentStop` | 子代理开始 / 结束 |
| `BeforeModel` / `AfterModel` | 模型调用前 / 后 |
| `BeforeToolSelection` | 工具短名单选定前 |
| `Notification` | 内部失败上报 |
| `BeforeAgent` / `AfterAgent` | agent 回合前 / 后 |

`PostToolUseFailure`、`PermissionRequest`、`PermissionDenied`、`StopFailure`、
`SubagentStart`、`PostCompact` 是 2026-10-25 补齐的 Claude Code 兼容事件。
**补齐之前它们只存在于一个从未被任何 `mod` 声明引用、因而从不参与编译的文件里** ——
看得到、用不着。

### 4.2 `PermissionRequest` 与远程批准配合

外部 harness 挂 `PermissionRequest` 就能知道"有工具在等人批准"：

```json
{
  "hooks": {
    "PermissionRequest": [
      { "hooks": [{ "type": "command", "command": "handle-approval.sh" }] }
    ]
  }
}
```

脚本从 `STAR_HOOK_TOOL_NAME` / `STAR_HOOK_TOOL_ARGUMENTS` 拿到工具信息，
把 tool_call_id 写进 `.star/remote/inbox.jsonl` 的 `approve_tool` 动作即可应答。

### 4.3 限制

- handler 只支持 `"type": "command"`（shell 脚本）。`http` / `prompt` / `agent` /
  `mcp_tool` 四种类型**静默跳过** —— 代码在
  `core/hooks/executors/`，但那个目录**没被 `mod` 声明引用**，同样不参与编译。
- `args` 会被拼成一条 shell 命令串（`sh -c`），不是 Claude Code 那种不过 shell 的
  exec form —— 路径含空格或引号会破。
- 默认超时 600 秒（Claude Code 是 60 秒）。
- `disableAllHooks` / `allowManagedHooksOnly` 两个开关**没有对应实现**。

---

## 5. 会话持久化

| 场景 | 是否落盘 |
|---|---|
| `-p`（不带 `--resume`） | **默认不落盘** —— 免得 headless 的临时会话混进用户的会话列表，让 `/continue` 捡到 |
| `-p` + `STAR_HEADLESS_SESSION=1` | 落盘到 `.star/sessions/headless-<ts>.json` |
| `-p --resume <id>` | 写回该会话（用户点名要续的） |
| TUI | 照旧落盘 |

落盘后的会话可以 `--resume <id>` 接着聊。`result` 行的 `session_id` 就是那个 id。

---

## 6. 把 StarCode 当 MCP server

```bash
starcode-cli mcp serve
```

stdio 传输，MCP 协议版本 `2024-11-05`，server 名 `starcode-context-engine`。

**只暴露 2 个工具**：

| 工具 | 参数 | 作用 |
|---|---|---|
| `search_code` | `query`（必填）、`root_path` | 语义代码检索 |
| `trace_call_chain` | `name_hint`（必填）、`root_path` | 调用链追踪 |

这是**代码检索能力**，不是完整的 agent 工具集 —— 外部 agent 拿不到 StarCode 的
Read/Write/Bash 等工具。想要那些能力，用 `--output-format stream-json` 驱动它。

反向（StarCode 作为 MCP **客户端**）是完整的：stdio / SSE / streamable_http 三种
transport，工具、资源、prompt 全支持。

---

## 7. 事件流的可观测性

`.star/transcript.jsonl`（默认开，`STAR_TRANSCRIPT=0` 关）记录 UI 层事件，每行带
`schema_version` / `run_id` / `seq` / `ts`。事件类型：`user` / `tool_call` /
`tool_result` / `start` / `done` / `stats_update` / `error` / `recovery_notice` 等。

**它不含助手正文** —— 能读到「问了什么、调了哪些工具、工具成功没、token 多少」，
读不到「模型答了什么」。要正文用 `--output-format stream-json`。

---

## 8. 还没做的

- **没有 WebSocket / IPC 控制面**。`core/bridge/` 有一份完整的实现（1447 行），
  但**没有接线** —— 外部零引用。文件协议是当前唯一可用的跨进程通道。
- **没有公开的 SDK**。`src/sdk/` 是空壳且未被 `mod` 声明引用，不参与编译。
  `src/lib.rs` 导出了 9 个模块，但没有整理过的稳定 API 契约。
- **headless 模式不能交互批准工具**，除非用外部 harness 走 §3.3 的 `approve_tool`。
- **子代理没有面向外部的接口** —— 结果只能从 `.star/subagent_outputs/<id>.txt` 读，
  是纯文本不是结构化数据。
