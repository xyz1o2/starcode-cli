# Agent 间消息互通架构设计（对标 Claude Code）

> 状态：Phase 1–6 全部实现（hub 接线 / SendMessage 走 hub / 局域网互通 / 唤醒联动 /
> `/agent-mail` 命令）。已知边界见 §8。
> 对标来源：CCB coordinator-and-swarm / SendMessageTool / teammateMailbox 机制；
> 本仓库既有实现（NotificationQueue / AsyncSubagentRunner / cross_agent）。

---

## 1. Claude Code 的四层通信 = 本仓库对应物

| 层 | Claude Code 机制 | 本仓库对应物 | 现状 |
|---|---|---|---|
| ① UI ↔ Agent | 双 async queue（入向 input/steering/interrupt，出向 stream events）| `AgentRequest::SendMessage`（steering）+ `StreamMessage`/`AgentEvent`（runtime.rs 两条 mpsc-100）| ✅ 健康 |
| ② Lead ↔ SubAgent | 一次性 prompt + `<task-notification>` user 注入 | `NotificationQueue` + `agent_loop` drain_for_next_turn | ✅ 健康 |
| ③ Teammates 相互通信 | SendMessage + 每 agent mailbox + turn 开头 drain + 唤醒 | `cross_agent::SendMessageTool` → 死信文件；worker 禁发 | ❌ **本设计目标** |
| ④ 外部 Harness | inbox.jsonl / outbox.jsonl / stream-json / mcp serve | 同机制（inbox/outbox 已实现）| ✅ 不在范围 |

## 2. 现状断点清单（为什么"相互"不成立）

1. **死信链路**：`AsyncSubagentRunner::deliver_message` 写 `.star/messages/{target}_{ts}.json`，
   全仓库无消费者（`poll_messages`/`read_unread_messages` 无调用点）。消息发出去即消失。
2. **worker 禁言**：`coordinator/tool_filter.rs` 的 `WORKER_DISALLOWED_TOOLS` 排除了
   `SendMessage` —— worker 无法主动汇报/求助/回消息，通信单向。
3. **三处重复实现，只接了一处**：
   - `agent::messaging::router::MessageRouter`（内存 mailbox + agent registry）——孤儿，无调用点；
   - `core::swarm::mailbox::MailboxManager + SendMessageTool`（文件 inbox、原子写、清理策略）——孤儿，
     bootstrap 注册的是另一个版本；
   - `core::tools::cross_agent::SendMessageTool` —— 已注册，但 deliver 是死信文件。
4. **注册表分家**：`register_agent_runtime_tools` 与 `register_skill_tool` 各自新建
   `AsyncSubagentRunner`（注释自称"与 AgentTool 用同一个 runner"，实际是两个实例）。
   skill fork 出来的 agent 在 SendMessage 的 name registry 里查不到。
5. **lead 无身份**：没有 "lead" 这样的可寻址目标，worker 想回话没有收件人。

## 3. 设计目标

никогда违背三条约束：
- **不新增注入通路**：复用 `NotificationQueue` 的既有接线模式（共享状态挂 `RuntimeServices`
  → 子 config 经 `Arc` clone 自动继承 → `agent_loop` 每轮 turn 开头 drain 注入 user 消息）。
- **同一 loop 双端复用**：lead 与 worker 跑的是同一份 `agent_loop.rs`，drain 注入写一次，
  两个身份同时受益 —— worker 自己的每个 turn 开头也会消费别人发给它的消息。
- **无死信**：投递要么成功（对账：from/to/送达数），要么显式 `UnknownTarget`，
  绝不静默写文件假装成功。

## 4. 目标架构

```
                ┌───────────────────────────────────────────────────────────┐
   │       AgentRouter Hub                      │
   │  (进程级单例，挂 RuntimeServices，子 config 经 Arc 继承；       │
   │   agent_runtime 与 skill_tool 不再各自造 runner 注册表)         │
   │                                                              │
   │  registry: 身份键 → AgentEntry { agent_id, name, status }      │
   │  mailboxes: 身份键 → VecDeque<AgentMessage> (上限 MAX_MAILBOX) │
   │                                                              │
   │  deliver(to, msg) → Delivered{count} | UnknownTarget          │
   │  drain(identity) → Vec<AgentMessage>                          │
   │  register/unregister（生命周期挂钩）                             │
   └────┬──────────────────────────┬───────────────────┬─────────┘
       │ spawn: register(id,name)  │ SendMessage 工具     │ 每轮 turn 开头 drain
       │ 终态: unregister           │ (lead / worker 通用； │ 注入 user-role XML +
       │                            │  worker 不再禁发)      │ UI assistant note
 AsyncSubagentRunner            cross_agent.rs         agent_loop.rs
                             cross_agent.rs         agent_loop.rs
                              (runner.rs run_with_progress
                               里 attach_agent_inbox)
```

寻址语义（对标 SendMessageTool 三路路由，收敛为两路 + 广播）：
- **by name / agent_id**：registry 命中 → 投递到对应 mailbox；
- **`"lead"`**：主 agent 固定身份，worker 回话的默认收件人；
- **`"*"`**：广播到所有**已注册**身份（不含发送者自己）。

## 5. 消息形状

```xml
<agent-message from="researcher" to="lead" type="plain_text" summary="auth 问题找到了">
...content...
</agent-message>
```

- 与既有 `<task-notification>` 同风格（user-role 注入、模型侧纯文本）；
- `type` 复用协议分类：`plain_text` / `task_assignment` / `shutdown_request` /
  `plan_approval_request` / `permission_request`（与 swarm/mailbox 的枚举对齐）；
- UI 侧：`agent_loop` 注入时同时 `emit_direct_chunk(assistant_note)`，
  形如 `[message from researcher] summary`。

## 6. 实施步骤

| Phase | 内容 | 文件 | 状态 |
|---|---|---|---|
| 1 | `AgentMessage` + `AgentRouter`（registry/mailbox/deliver/drain/broadcast + 单测） | `src/agent/messaging/hub.rs`（新增）、`mod.rs` | ✅ |
| 2 | RuntimeServices `agent_hub` cell；bootstrap 创建并装配；`AsyncSubagentRunner` spawn 注册/终态注销、`deliver_message` 委托 hub | `runtime_services.rs` / `runtime_bootstrap.rs` / `subagent/runner.rs` | ✅ |
| 3 | `SubAgentRequest` 增 `identity`、`Config` 增 `agent_identity`（发件人同源）；子 agent `set_agent_hub(hub, identity)`；`agent_loop` drain 注入 user-role XML + assistant note；lead 固定身份 `"lead"` | `core/agents/mod.rs` / `config.rs` / `subagent/runner.rs` / `agent_core.rs` / `agent_loop.rs` | ✅ |
| 4 | `cross_agent::SendMessageTool`：from 取 config 身份（lead=None→`"lead"`）、deliver 全走 hub、`UnknownTarget` 列出已注册 agent 显式报错；文件死信路径删除；worker 工具池放行 `SendMessage`（对标 CC：只禁 TeamCreate/Delete/synthetic_output） | `cross_agent.rs` / `coordinator/tool_filter.rs` | ✅ |
| 5 | 局域网互通（RemoteBridge：进程内 hub → HTTP 出站/入站、well-known 发现、peer 列表、共享密钥） | 见 §7 | ✅ |
| 6 | 孤儿 `MessageRouter` 删除（`messaging/router.rs` 零调用点，hub 是唯一实现，`MessageType` 导出同步移除）；IdleNotification 唤醒：`MailWakeupSignal`（tokio Notify）+ `AgentMailbox.deliver` 投给 lead/广播时 notify + 入站 listener 携带唤醒 + UI 空闲等待醒来注入消费提示（同 `apply_loop_tasks` 路径） | `messaging/hub.rs` / `remote.rs` / `ui/app/runtime/mod.rs` | ✅ |

## 7. 局域网互通（Phase 5 · 已实现，剩 peer 自动发现同 §7.3 标注）

跨进程/跨机消息原先列在「明确不做」，现在单独立项：本仓库第④层已有
`inbox.jsonl`（外部 harness → 本进程），但**本机多个 StarCode 进程之间**、
以及**局域网内多台机器的 StarCode 之间**没有互通通道。设计如下。

### 7.1 现有标准对标（2025–2026 生态调研结论）

| 标准 | 形态 | 取什么 | 留什么 |
|---|---|---|---|
| **A2A**（Google → Linux Foundation，v1.0 于 2025-11 批准） | Agent Card 发现 + HTTP/JSON-RPC + 长任务 + webhook push | 发现（Agent Card）、寻址 URL、任务状态回购回 | 它的 task 生命周期（本仓库已有 notification 语义，不重造） |
| **MCP** | 客户端↔服务器工具协议 | stdio/TCP 传输层的成帧（LSP 风格 Content-Length 头） | 客户端角色（本仓库是 agent↔agent，不是宿主↔工具） |
| **社区 swarm 编排**（claude-swarm 等） | git 分支 / 文件信箱 / tmux+SSH | 「文件即邮箱」的降级路径 | 无协议、无对账——正是 §2 断点 1 的死信反模式 |

结论：**不整体拥抱 A2A**（它的 Agent Card / OAuth / 签名链对局域网可信
Neighbors 场景过重），而是取它的三样骨架 —— `/.well-known` 发现、
HTTP+JSON 承载、回执语义 —— 包在本仓库已有的 `AgentMessage` 形状外面。

### 7.2 架构

```
 StarCode 进程 A（机器1）                StarCode 进程 B（机器2）
 ┌─────────────────────┐               ┌─────────────────────┐
 │ AgentRouter（进程内） │               │ AgentRouter          │
 │   ├─ lead mailbox    │               │   ├─ lead mailbox    │
 │   └─ worker mailboxes│               │   └─ worker mailboxes│
 │         ▲  drain      │               │         ▲ drain      │
 │         │             │               │         │            │
 │   RemoteBridge ───────┼── HTTP POST ──┼→ /agent-mail/inbox   │
 │   （出站投递 + 入站    │←── 202 回执 ──┼─  （只收 POST、      │
 │     HTTP listener）   │               │    开 127.0.0.1 回环 │
 └─────────────────────┘               └─────────────────────┘
              免配置模式：只开回环 —— 单机多进程互通；
              局域网模式：显式 --agent-mail-host 0.0.0.0 + 共享密钥
```

要点：

1. **进程内 hub 是唯一的语义层**，RemoteBridge 只是一条投递管道。远端
   消息落进本地 mailbox 后，走现成的「turn 开头 drain → user-role XML」
   消费路径，两个身份（lead/worker）同时受益的结论原样成立。
2. **发现**：对标 A2A，`GET /.well-known/star-agent.json` 返回
   `{ identity, mail_inbox_url, version, capabilities }`。局域网内
   显式配置对端列表（`STAR_AGENT_PEERS="http://10.0.0.5:7901,…"`），
   不做 mDNS 自动发现（v1 保持可审计）。
3. **寻址延续两路 + 广播**：`to=name` 先查进程内 registry，未命中且
   配置了 peers → 查各 peer 的 well-known 卡片转发；`to="lead"` 是
   远端进程主 agent 的固定身份；`"*"` 只在**本进程**内广播，跨机广播
   显式用 `"*all"`（避免误伤不同项目目录的进程）。
4. **投递语义 = HTTP 202 + 进程内 deliver**：接收方把消息写进自己
   mailbox 后回 `{status:"queued", mailbox:"<identity>"}`；不回 ack 的
   peer 记为 `unreachable`，发送方显式报 UnknownTarget（与 §3 「无死信」
   约束一致 —— 网络失败不是静默成功）。
5. **安全**（局域网不可信前提）：入站只接受 `Authorization: Bearer
   <SECREThn shared_key>`（`STAR_AGENT_MAIL_KEY` 环境变量配对）；
   默认绑定 `127.0.0.1`，暴露到局域网需要显式 `--agent-mail-host`；
   消息体上限（默认 64 KB，对齐 MAX_MAILBOX_MESSAGES 的节流思路）。
6. **复用既有表面积**：listener 生命周期挂在 `RuntimeServices`（与
   `agent_hub` cell 同一层）；headless 模式天然适配（第④层 harness 已
   验证了「无 UI 也能收指令」的路径）。

### 7.3 分步落地

| 步骤 | 内容 | 状态 |
|---|---|---|
| 5.1 | 出站：runner.deliver_message 本地 UnknownTarget 时按 `STAR_AGENT_PEERS` 转发 + 逐对端对账（全败显式报错） | ✅ `messaging/remote.rs` PeerMailClient |
| 5.2 | 入站 HTTP listener（hyper 1.x 轻量 server）：`POST /agent-mail/inbox` 写本地 mailbox，404 unknown_target；Bearer 共享密钥；非回环绑定无密钥拒绝启动 | ✅ `messaging/remote.rs` spawn_inbound_listener |
| 5.4a | listener 挂生产装配：`AgentRequest::InitAgentMail`（UI 启动时发一次）→ worker 读 `STAR_AGENT_MAIL_*` env → `spawn_inbound_listener`；默认关闭，失败折 `[AgentMail]` 日志 | ✅ |
| 5.3 | `/.well-known/star-agent.json` 发现 + `/agent-mail probe` 逐对端探测卡片（kind/version/inbox/auth 对账）；不做 mDNS（§7.2 第 2 条：保持可审计） | ✅ |
| 5.4b | `/agent-mail` 状态命令：listener/peers 配置真相（env 读数）+ `probe` 子命令探测对端；`every_top_level_command_lands_in_a_help_category` 守住 /help 可见性 | ✅ |

E2E（`two_process_cross_forward_lands_in_peer_mailbox`）：两个 hub + 两个 listener
模拟两个 StarCode 进程，本地未命中 → HTTP 转发 → 对端 mailbox 落箱 → drain 消费，
全程真 HTTP 往返，测试内验证。

## 8. 对标 CC agent teams 的差距账（2026-11-08 调研 changelog 2.1.32→2.1.319）

CC 的 agent teams 本体（同机）机制：隐式 team（2.1.178 起 `Agent(name=…)` 即注册，
TeamCreate/Delete 已删）、`name@team` 双字段寻址、teammate 双形态（in-process /
tmux·iTerm2 pane）、idle notification 携带结题正文、stuck teammate 被 SendMessage
即唤醒重试、resume 后按 name 找回（compaction 前的要 agent id）、权限/模型全链继承。
**CC 没有 LAN 互通**（跨机靠 cloud sessions）—— 本仓库 §7 的 well-known + HTTP
是对 A2A 的自研方向，不是 CC 的对标面。

| CC 语义 | 本仓库状态 |
|---|---|
| 隐式 team：`Agent(name=…)` 即可寻址 | ✅ 命名后台 spawn `with_identity(name)`（`agent_tool.rs` AsyncAgent / CoordinatorWorker / ForkAgent 全路径），SendMessage name 寻址、worker 发件人、hub 注册键三者同源 |
| stuck teammate 被 SendMessage 即唤醒重试 | ✅ `agent_loop::hub_has_pending_mail`：turn 收尾邮箱有未读 → 不收工，续跑消费（测试 `wakeup_retry_gates_on_identity_and_pending_mail`） |
| ListAgents 列 live teammates | ✅ `/agent-mail agents`：spawn/终态各推 `AgentTeamSnapshot` chunk（progress::emit_to_ui），UI 存 `agent_team_snapshot` |
| idle notification 带结题正文 | 部分：`<task-notification>` 带 summary/result（原有），「回话给 lead」的语义走 SendMessage 提示 |
| 消息早达（running agent 的 fg command 转 background） | 未做（量在 ToolExecutor，后置） |
| 死后 mailbox 持久化（跨重启 resume 寻址） | 未做（进程内 mailbox，重启即无——CC 靠 `~/.claude/teammates/` 文件） |
| 权限/model 继承 | 大半既有（sub_config clone 带 allowed_tools/policy mode）；缺 org 策略家族降级（非互通缺口） |

## 9. 明确不做

- 跨机消息不做 A2A 全家桶（Agent Card 签名链 / OAuth2 / webhook push）——
  局域网可信场景用共享密钥 + 回环默认绑定就够；标准演进到需要时再接；
- 不做消息加密/鉴权之外的信用模型（不搞 reputation / capability 协商）；
- 不把 `swarm/teams.rs` 的 TeamCreate/TeamDelete 顺手实现 —— 那是另一条
  工作流，本设计只负责消息互通。
