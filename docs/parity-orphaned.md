# 孤立模块 / 未接线子系统

> 这些子系统编译通过、代码可读，但从未被构造或调用。
> 构建在它们之上前，**必须先 grep 构造函数确认有实际调用者**。

---

## 一、已确认孤立 (CLAUDE.md 记录)

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

---

## 二、完全桩实现的子系统 (方法为空操作或返回占位符)

### 5. Bridge (Web UI / WebSocket)

**位置**: `src/core/bridge/{mod.rs, api.rs, session.rs, transport.rs, web_ui}`

**桩方法**:
| 方法 | 行为 | 说明 |
|------|------|------|
| `BridgeApi::execute_command()` | 返回 `Value::Null` | TODO: 发送命令并等待响应 |
| `BridgeApi::query_status()` | 返回 `Value::Null` | TODO: 发送查询并等待响应 |
| `BridgeManager::send_message()` | `Ok(())` 空操作 | TODO: WebSocket 消息发送 |
| `BridgeManager::broadcast_message()` | `Ok(())` 空操作 | TODO: 消息广播 |
| `SessionManager::handle_command()` | TODO | 命令处理逻辑 |
| `SessionManager::handle_query()` | TODO | 查询处理逻辑 |
| `WebSocketTransport::reconnect()` | 设 `connected=true` 但无实际重连 | TODO: 重连逻辑 |
| `WebUiServer::start()` | 打印消息后返回 | TODO: Web UI 服务器 |

**相关死环境变量**: `STAR_BRIDGE_ENABLED`, `STAR_BRIDGE_PORT`, `STAR_BRIDGE_JWT_SECRET`, `STAR_BRIDGE_AUTH_TOKEN`, `STAR_BRIDGE_HEARTBEAT_INTERVAL`, `STAR_BRIDGE_MAX_CONNECTIONS`, `STAR_BRIDGE_SESSION_TIMEOUT`, `STAR_BRIDGE_WEB_UI_ENABLED`, `STAR_BRIDGE_WEB_UI_PORT`

---

### 6. SSH

**位置**: `src/core/ssh/{deploy.rs, auth.rs, session.rs}`

**桩方法**:
| 方法 | 行为 | 说明 |
|------|------|------|
| `SSHDeploy::deploy_file()` | `Ok(())` 空操作 | TODO: 文件部署 |
| `SSHDeploy::deploy_directory()` | `Ok(())` 空操作 | TODO: 目录部署 |
| `SSHAuthProxy::get_auth_method()` | 始终返回 `None` | TODO: 认证方式获取 |
| `SSHAuthProxy::save_auth()` | `Ok(())` 空操作 | TODO: 认证信息保存 |
| `SSHSession::connect()` | 设状态为 `Connected` 但无实际连接 | TODO: 连接逻辑 |
| `SSHSession::execute()` | 返回空 `SSHExecResult` | TODO: 执行逻辑 |

---

### 7. Voice (语音输入)

**位置**: `src/core/voice/{stt.rs, capture.rs}`

**桩方法**:
| 方法 | 行为 | 说明 |
|------|------|------|
| `AnthropicSttProvider::transcribe()` | 返回硬编码 `"Transcription placeholder"` | TODO: Anthropic STT API |
| `AnthropicSttProvider::transcribe_stream()` | 返回 `NotSupported` 错误 | 未实现 |
| `DoubaoSttProvider::transcribe()` | 返回硬编码 `"豆包转录占位符"` | TODO: Doubao STT API |
| `DoubaoSttProvider::transcribe_stream()` | 返回 `NotSupported` 错误 | 未实现 |
| `AudioCapture::start()` | TODO | 实际音频捕获 |
| `AudioCapture::get_frame()` | 返回全零假数据 | TODO: 实际音频帧获取 |

**Feature flag**: `voice_mode` 在 `feature_flags.rs` 中注册但 `rollout_percentage: 0` (禁用)

**相关死环境变量**: `STAR_VOICE_MODE`, `STAR_VOICE_LANGUAGE`, `STAR_VOICE_SAMPLE_RATE`, `STAR_VOICE_STT_PROVIDER`, `STAR_VOICE_STT_API_KEY`, `STAR_VOICE_STT_API_ENDPOINT`

---

### 8. SDK

**位置**: `src/sdk/mod.rs`

**桩方法**:
| 方法 | 行为 | 说明 |
|------|------|------|
| `SDKManager::handle_chat_request()` | 返回 `{"message": "Chat request received"}` | TODO |
| `SDKManager::handle_tool_request()` | 返回 `{"message": "Tool request received"}` | TODO |
| `SDKManager::handle_session_request()` | 返回 `{"message": "Session request received"}` | TODO |

**注意**: `src/sdk/` 是孤立目录, 未被 `lib.rs` 或 `main.rs` 引用。

**相关死环境变量**: `STAR_SDK_ENABLED`, `STAR_SDK_PORT`, `STAR_SDK_API_KEY`

---

### 9. LSP Instance

**位置**: `src/core/lsp/instance.rs`

**问题**: `LspServerInstance::start()` 设状态为 `Running` 但未启动任何实际 LSP 进程。

---

### 10. Workflow Engine (部分桩)

**位置**: `src/core/workflow/mod.rs`

**桩行为**:
| StepType | 行为 |
|----------|------|
| `Tool` | 返回 `"Tool step executed (placeholder)"` |
| `Prompt` | 返回 `"Prompt step executed (placeholder)"` |
| `Condition` | 返回 `"completed (type not implemented)"` |
| `Loop` | 返回 `"completed (type not implemented)"` |
| `Parallel` | 返回 `"completed (type not implemented)"` |
| `SubWorkflow` | 返回 `"completed (type not implemented)"` |
| `Shell` | **唯一有实际实现的步骤类型** |

`evaluate_condition()` 对任何非空条件字符串始终返回 `true` (无实际评估)。

---

### 11. Context Store (空壳)

**位置**: `src/core/context/storage.rs`

**问题**: `ContextStore` 是完全空的 struct — 无字段、无方法。注释: "Placeholder for persistent storage (e.g. SQLite or JSON files)"

**影响**: 上下文持久化不存在。所有上下文仅在内存中。

---

### 12. Skill Tool (占位响应)

**位置**: `src/core/tools/skill.rs:62`

**问题**: `execute()` 返回硬编码成功消息, 不实际查找或执行任何 skill。注释: "For now, return a placeholder response"

**影响**: 模型调用 skill 工具得到的是假响应。

---

### 13. Edit Tool Trust Logic

**位置**: `src/core/tools/edit.rs:916`

**问题**: 不受信任文件夹的编辑路径中, 信任逻辑为占位符。注释: "Placeholder for trust logic"

**影响**: 不受信任文件夹的安全检查可能不完整。

---

### 14. Chrome DevTools (大部分动作桩)

**位置**: `src/core/chrome/mod.rs`

**已实现**: connect + list targets

**桩方法** (8 个): navigate, click, type, screenshot, execute_script, get_network_requests, get_console_logs, wait_for_element — 均返回桩结果或错误

**相关环境变量**: `STAR_CHROME_DEBUG_PORT` (读取但大部分功能不可用)

---

### 15. Keychain Storage (仅 macOS, 非 macOS 桩)

**位置**: `src/core/secure_storage/keychain.rs`

**问题**:
- `is_available()` 在非 macOS 平台返回 `false`
- `store()`, `get()`, `delete()` 在非 macOS 返回 `Err(StorageError::Unsupported)`
- `list()` 始终返回空 `Vec` (即使在 macOS 上)
- macOS 实际集成也不完整 (TODO 注释)

---

## 三、占位符 flush / 发送 (内存累积但从不发送)

### 16. Langfuse Observer

**位置**: `src/agent/tool_enhanced.rs:289`

**问题**: `flush()` 为占位符。数据在内存中累积但从不发送到 Langfuse 服务器。

---

### 17. OTLP Logger

**位置**: `src/agent/tool_enhanced.rs:464`

**问题**: `flush()` 为占位符。数据在内存中累积但从不发送到 OTLP 服务器。

---

### 18. Analytics HTTP Sink

同 §1.4 — 日志代替发送。

---

## 四、解析/索引占位符

### 19. Structure Index (正则占位)

**位置**: `src/core/context/structure_index.rs:71`

**问题**: 使用正则表达式解析代码结构。注释: "NB: regex-based parsing is a placeholder -- real parsing should use tree-sitter."

**影响**: 代码结构索引精度低于 tree-sitter 解析。

---

### 20. Context Selection (Cargo.toml 解析)

**位置**: `src/core/context/selection.rs`

**问题**: 解析 Cargo.toml 依赖的逻辑为 TODO。

---

## 五、Eval / 测试桩

### 21. Eval Harness E2E

**位置**: `src/agent/eval_harness.rs`

**问题**: 所有 E2E 测试返回 `executed: false`。`evaluate_e2e_task_stub()` 函数标记所有 E2E 标准为 `passed: false`, 详情: "E2E not executed: requires API key and git worktree isolation"

---

### 22. Mock LLM Client

**位置**: `src/llm/mock.rs`

**说明**: 完整的 `MockClient` 实现, 返回 "This is a mock response." — 注册为 `LlmProvider::Mock`。这是有意为之的测试工具, 不是 bug。

---

## 六、死环境变量 (读取但喂入桩子系统)

| 子系统 | 环境变量 | 状态 |
|--------|---------|------|
| Voice | `STAR_VOICE_MODE` `STAR_VOICE_LANGUAGE` `STAR_VOICE_SAMPLE_RATE` `STAR_VOICE_STT_PROVIDER` `STAR_VOICE_STT_API_KEY` `STAR_VOICE_STT_API_ENDPOINT` | 读取但 STT 全桩 |
| Bridge | `STAR_BRIDGE_ENABLED` `STAR_BRIDGE_PORT` `STAR_BRIDGE_JWT_SECRET` `STAR_BRIDGE_AUTH_TOKEN` `STAR_BRIDGE_HEARTBEAT_INTERVAL` `STAR_BRIDGE_MAX_CONNECTIONS` `STAR_BRIDGE_SESSION_TIMEOUT` `STAR_BRIDGE_WEB_UI_ENABLED` `STAR_BRIDGE_WEB_UI_PORT` | 读取但 Bridge 全桩 |
| SDK | `STAR_SDK_ENABLED` `STAR_SDK_PORT` `STAR_SDK_API_KEY` | 读取但 SDK 全桩 |
| Daemon | `STAR_DAEMON_ENABLED` `STAR_DAEMON_PID_FILE` `STAR_DAEMON_LOG_FILE` `STAR_DAEMON_MAX_RUNTIME` `STAR_DAEMON_HEALTH_CHECK_INTERVAL` `STAR_DAEMON_MAX_WORKERS` | 读取但功能有限 |
| Chrome | `STAR_CHROME_DEBUG_PORT` | 读取但 8/10 动作桩 |

---

## 七、Feature Flags

**位置**: `src/core/feature_flags.rs`

| Flag | 状态 | 说明 |
|------|------|------|
| `vim_mode` | ✅ enabled, 100% rollout | 正常 |
| `voice_mode` | ❌ **disabled, 0% rollout** | 对应全桩 Voice 子系统 |
| `proactive_suggestions` | ✅ enabled, 50% rollout | 正常 |
| `context_collapse` | ✅ enabled, 100% rollout | 正常 |
| `auto_compact` | ✅ enabled, 100% rollout | 正常 |

---

## 八、孤立目录

| 目录 | 问题 |
|------|------|
| `src/constants/` | 未被 `lib.rs` 或 `main.rs` 声明 |
| `src/sdk/` | 未被 `lib.rs` 或 `main.rs` 声明 |

编辑这些目录中的文件不会影响编译产物。

---

## 九、其他已知问题

| 问题 | 说明 |
|------|------|
| 根目录 `config.toml` | 无代码读取。实际配置: `~/.star/settings.json`, `./.star/settings.json`, 环境变量 |
| `tests/` 目录 | 大部分不可编译。仅 `tests/lib.rs` + `tests/eval_harness_live.rs` 有效 |
| `.gitignore` `**/test_*.rs` | 阻止测试文件被 git 跟踪 |
| `Cargo.lock` 被 gitignore | 二进制 crate 不应如此, 导致依赖版本不一致 |
| README "Cross-Platform" 声明 | secure storage 仅 macOS, 其他平台返回 `Unsupported` |

---

## 十、接线优先级建议

| 优先级 | 模块 | 理由 |
|--------|------|------|
| **P0** | `PolicyEngine::load_permission_rules` | 权限规则不生效, 用户配置无效 |
| **P0** | Edit trust logic | 安全检查缺失 |
| **P1** | `ModelFallbackManager` | 模型回退是可靠性关键 |
| **P1** | Structure Index → tree-sitter | 代码索引精度 |
| **P1** | Context Store | 上下文持久化 |
| **P2** | Skill Tool | 模型得到假响应 |
| **P2** | Workflow Engine (非 Shell 步骤) | 工作流能力受限 |
| **P2** | LSP Instance | IDE 级代码智能 |
| **P2** | `WebBrowserTool` | 评估或删除 |
| **P2** | Langfuse / OTLP flush | 可观测性 |
| **P3** | Bridge | Web UI 高级功能 |
| **P3** | Voice | 语音输入锦上添花 |
| **P3** | SSH | 远程执行场景 |
| **P3** | Chrome DevTools | 浏览器自动化 |
| **P3** | Secure Storage | 改善安全性 |
| **P3** | SDK | 孤立目录需重新设计 |
| **P3** | Analytics | 可选功能 |
| **P3** | Eval E2E | 测试基础设施 |
