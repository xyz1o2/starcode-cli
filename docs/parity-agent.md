# Agent 内部机制对标分析

> 基于 `src/agent/` 目录下的对标注释和实现

## 状态: 🟡 大部分核心机制已对标实现，部分模块有 TODO

---

## 1. Compact 策略 (7 模块)

| 模块 | 功能 | 状态 |
|------|------|------|
| grouping | 消息分组压缩 | ✅ |
| cached micro-compact | 缓存微压缩 | ✅ |
| compact warning hook | 压缩预警钩子 | ✅ |
| reactive compact | 响应式压缩 | ✅ |
| post-compact cleanup | 压缩后清理 | ✅ |
| time-based config | 时间配置 | ✅ |
| session memory compact | 会话记忆压缩 | ✅ |

**相关环境变量**: `STAR_AUTO_COMPACT`

---

## 2. Streaming Executor

| 功能 | 状态 | 说明 |
|------|------|------|
| 工具编排 | ✅ | 并行/串行工具执行调度 |
| 流式工具执行 | ✅ | 边接收边执行 |

---

## 3. Enhanced Executor (10+ 模块)

| 模块 | 功能 | 状态 |
|------|------|------|
| withheld mechanism | 消息暂扣机制 | ✅ |
| post-sampling hooks | 采样后钩子 | ✅ |
| bash classifier | Bash 命令分类器 | ✅ |
| simulated sed edit stripping | 模拟 sed 编辑剥离 | ✅ |
| tool span management | 工具 span 管理 | ✅ |
| tool attributes | 工具属性 | ✅ |
| input sanitization | 输入净化 | ✅ |
| streaming tool execution | 流式工具执行 | ✅ |
| tool execution | 工具执行 | ✅ |
| tool use summary | 工具使用摘要 | ✅ |
| structured output capture | 结构化输出捕获 | ✅ |

---

## 4. Command Queue

| 功能 | 状态 |
|------|------|
| queue command system | ✅ |
| attachment messages | ✅ |
| tool refresh | ✅ |
| periodic task summaries | ✅ |

---

## 5. MCP Permissions

| 功能 | 状态 | 说明 |
|------|------|------|
| server type | ✅ | MCP 服务器类型识别 |
| server management | ✅ | 服务器生命周期管理 |
| decision reason mapping | ✅ | 决策原因映射 |
| image paste ID | ✅ | 图片粘贴 ID |
| tool error classification | ✅ | 工具错误分类 |

---

## 6. Message Processing

| 功能 | 状态 |
|------|------|
| message constants | ✅ |
| message factory | ✅ |
| message lookups | ✅ |
| system reminder sibling compression | ✅ |
| error tool result content sanitization | ✅ |

---

## 7. Query Loop

CCB 对标: 完整的 continue/stop 条件系统。

| 条件类型 | 数量 | 状态 |
|----------|------|------|
| Continue conditions | 8 | ✅ |
| Stop conditions | 11 | ✅ |

---

## 8. Coordinator

| 状态机 | 状态 |
|--------|------|
| tool filter 状态机 | ✅ |
| coordinator mode 状态机 | ✅ |
| prompt 状态机 | ✅ |

---

## 9. Messaging / Router

| 功能 | 状态 |
|------|------|
| SendMessageTool parity (Router) | ✅ |

---

## 10. Subagent

| 功能 | 状态 |
|------|------|
| fork subagent isolation | ✅ |

---

## 11. 待完善 / 有 TODO 的部分

以下模块有对标注释但包含未完成的 TODO:

| 模块 | 位置 | TODO 内容 |
|------|------|-----------|
| bridge | `src/core/bridge/` | command/query 处理, 重连逻辑, WebSocket 发送/广播, Pong 响应, Web UI server |
| SSH | `src/core/ssh/` | 文件/目录部署, 认证方法, 连接/执行逻辑 |
| voice | `src/core/voice/` | STT API (Anthropic + Doubao), 流式转录, 音频捕获 |
| secure storage | `src/core/secure_storage/keychain.rs` | 系统钥匙链存储 |
| LSP | `src/core/lsp/instance.rs` | LSP server 进程启动 |
| context | `src/core/context/selection.rs` | Cargo.toml 依赖解析 |
| SDK | `src/sdk/mod.rs` | chat/tool/session 请求处理 |

---

## 12. 已确认未接线的子系统

| 子系统 | 位置 | 问题 |
|--------|------|------|
| `ModelFallbackManager` | `src/agent/model_fallback.rs` | 仅 test 中构造; `STAR_MODEL_FALLBACK_*` 被忽略 |
| `PolicyEngine::load_permission_rules` | `policy_engine.rs:52` | 零调用者; `.star/permissions.json` 仅为建议 |
| `WebBrowserTool` | — | 实现了 trait 但 `new` 从未被调用 |
| Analytics HTTP sink | — | 日志代替发送 |

**教训**: 编译通过 ≠ 功能生效。构建在任何子系统之上前，先 grep 其构造函数确认有实际调用者。
