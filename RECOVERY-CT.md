[COMPACTION-RECOVERY]
Route: ct-attack-defense（本仓实为 Rust 工程接线任务）
Active target: /mnt/h/test/starcode（branch sc-dev-m）
Goal: CC 三件套（name-spawn 寻址 / worker 唤醒重试 / /agent-mail agents）落地完毕；等全量回归拍板
Exact identifiers/evidence: agent_tool.rs(AsyncAgent with_identity / ForkAgent)/ agent_loop.rs(hub_has_pending_mail + 内联 Done 分支)/ runner.rs(emit_team_snapshot async)/ hub.rs(registered_entries)/ types/mod.rs(AgentTeamSnapshot)/ stream_chunks.rs(解 JSON → AgentTeamUpdate)/ stream/mod.rs(AgentTeamUpdate 分支)/ worker.rs(bg 转发放行 AgentTeamSnapshot)/ store/mod.rs(agent_team_snapshot 字段)/ parity.rs(agents 子命令)/ system.rs(声明)
Bound slots: 无占位符
Technical scope: local | repo-only
Deliverable checkpoints:
  - (1) name-spawn: ✅ 命名后台 spawn 以 name 注册（/agent-mail 语义 = 回话用 SendMessage target_agent='lead'）
  - (3) 唤醒重试: ✅ hub_has_pending_mail 四象限测试绿
  - (7) /agent-mail agents: ✅ 快照链 spawn→chunk→StreamMessage→ChatState 全接通；声明测试含 3 个子命令
  - 文档: ✅ §8 差距账（CC changelog 2.1.32→2.1.319 调研）+ Phase5 标题改「已实现」
  - fmt/form check 前置已做；全量回归 bm5orcb4d 运行中
Observed ledger: 编译绿; 三条新测试各自单独跑全绿
Next action: 读 bm5orcb4d（期待 ~2082 passed / 0 failed）→ 汇报三件套 + 差距账；后续若用户要继续 → 剩余差距: idle notification 结题正文 / 消息早达（ToolExecutor）/ 死后 mailbox 持久化 / org 模型降级
Continuation cues: continue / 继续 / 照做
