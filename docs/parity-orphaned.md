# 孤立模块 / 未接线子系统

> 这些子系统编译通过、代码可读，但从未被构造或调用。
> 构建在它们之上前，**必须先 grep 构造函数确认有实际调用者**。

---

## 已确认孤立 (CLAUDE.md 记录)

### 1. ModelFallbackManager

**位置**: `src/agent/model_fallback.rs`

**问题**: 仅在 `#[cfg(test)]` 模块中构造。生产代码中无调用者。

**影响**: `STAR_MODEL_FALLBACK_*` 环境变量被忽略。模型回退逻辑不存在。

**接线方案**:
- 在 `src/core/config/runtime_bootstrap/agent_runtime.rs` 中构造
- 注入到 `Agent` 或 `StreamingSession`
- 需要确定触发回退的条件: 错误类型、超时、限流

---

### 2. PolicyEngine::load_permission_rules

**位置**: `src/core/policy/policy_engine.rs:52`

**问题**: 零调用者。`.star/permissions.json` 规则文件仅为建议，从未被加载。

**影响**: 用户配置的权限规则不生效。`PolicyEngine::check` 使用的是硬编码逻辑。

**接线方案**:
- 在 `Config::initialize()` 或 `PolicyEngine::new()` 中调用 `load_permission_rules`
- 需要处理: 文件不存在、JSON 解析失败、规则冲突
- 路径: 通过 `Storage` 获取 `.star/permissions.json`

---

### 3. WebBrowserTool

**位置**: 实现了 `BaseDeclarativeTool` trait

**问题**: `WebBrowserTool::new()` 从未被调用。工具注册中没有它。

**影响**: 模型无法浏览网页。`WebFetch` 工具可能已替代了此功能。

**评估**:
- 如果 `WebFetch` 已满足需求, 可删除 `WebBrowserTool`
- 如果需要 headless browser 能力, 需要在 `runtime_bootstrap/agent_runtime.rs` 中注册

---

### 4. Analytics HTTP Sink

**位置**: analytics 模块

**问题**: 日志输出 `"[Analytics HTTP] Would send ..."` 而非实际发送。

**影响**: 使用数据不上传。对本地使用无影响。

**接线方案** (如果需要):
- 实现 HTTP POST 到分析端点
- 需要: 端点 URL、认证 token、数据格式
- 建议: 用 `STAR_ANALYTICS_ENDPOINT` 环境变量控制

---

## 存在 TODO 的骨架模块

### 5. Bridge (Web UI / WebSocket)

**位置**: `src/core/bridge/`

**未实现**:
| TODO | 文件 | 说明 |
|------|------|------|
| command/query 处理 | — | 接收 Web UI 命令 |
| 重连逻辑 | — | WebSocket 断线重连 |
| WebSocket 发送/广播 | — | 向连接的客户端推送 |
| Pong 响应 | — | 心跳保活 |
| Web UI server | — | 静态文件服务 + WebSocket |

**现状**: 骨架代码存在, 核心逻辑为 TODO。

---

### 6. SSH

**位置**: `src/core/ssh/`

**未实现**:
| TODO | 文件 | 说明 |
|------|------|------|
| 文件/目录部署 | `deploy.rs` | SCP/SFTP 上传 |
| 认证方法 | `auth.rs` | 密码/密钥/agent forwarding |
| 连接/执行逻辑 | `session.rs` | SSH session 管理 |

**现状**: 数据结构定义完整, 实现全部为 TODO。

---

### 7. Voice (语音输入)

**位置**: `src/core/voice/`

**未实现**:
| TODO | 文件 | 说明 |
|------|------|------|
| Anthropic STT API | `stt.rs` | 语音转文字 (Anthropic 风格) |
| 流式转录 | `stt.rs` | 边录边转 |
| Doubao STT API | `stt.rs` | 字节跳动语音 API |
| 实际音频捕获 | `capture.rs` | 麦克风输入 |

**现状**: API 接口定义完整, 底层实现全部为 TODO。

---

### 8. Secure Storage (钥匙链)

**位置**: `src/core/secure_storage/keychain.rs`

**未实现**: 系统钥匙链存储 (macOS Keychain / Linux Secret Service / Windows Credential Manager)

**当前替代**: API key 存储在 settings.json 明文或环境变量。

---

### 9. LSP Instance

**位置**: `src/core/lsp/instance.rs`

**未实现**: 实际启动 LSP server 进程

**当前替代**: 可能通过外部 LSP 或无 LSP 支持。

---

### 10. Context Selection

**位置**: `src/core/context/selection.rs`

**未实现**: 解析 Cargo.toml 依赖

**影响**: 自动上下文选择可能不完整。

---

### 11. SDK

**位置**: `src/sdk/mod.rs`

**未实现**: chat/tool/session 请求处理

**注意**: `src/sdk/` 是孤立目录, 未被 `lib.rs` 或 `main.rs` 引用。

---

## 孤立目录

| 目录 | 问题 |
|------|------|
| `src/constants/` | 未被 `lib.rs` 或 `main.rs` 声明 |
| `src/sdk/` | 未被 `lib.rs` 或 `main.rs` 声明 |

编辑这些目录中的文件不会影响编译产物。

---

## 其他已知问题

| 问题 | 说明 |
|------|------|
| 根目录 `config.toml` | 无代码读取。实际配置: `~/.star/settings.json`, `./.star/settings.json`, 环境变量 |
| `tests/` 目录 | 大部分不可编译。仅 `tests/lib.rs` + `tests/eval_harness_live.rs` 有效 |
| `.gitignore` `**/test_*.rs` | 阻止测试文件被 git 跟踪 |
| `Cargo.lock` 被 gitignore | 二进制 crate 不应如此, 导致依赖版本不一致 |

---

## 接线优先级建议

| 优先级 | 模块 | 理由 |
|--------|------|------|
| P0 | `PolicyEngine::load_permission_rules` | 权限规则不生效, 用户配置无效 |
| P1 | `ModelFallbackManager` | 模型回退是可靠性关键 |
| P2 | `WebBrowserTool` | 评估是否需要, 或删除 |
| P2 | LSP Instance | IDE 级代码智能 |
| P3 | Bridge | Web UI 是高级功能 |
| P3 | Voice | 语音输入是锦上添花 |
| P3 | SSH | 远程执行场景 |
| P3 | Secure Storage | 改善安全性但非关键 |
| P3 | Analytics | 可选功能 |
| P3 | SDK | 孤立目录, 需要重新设计 |
