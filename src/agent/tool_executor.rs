use crate::agent::loop_engineering::StructuredError;
use crate::core::tools::ToolRegistry;
use crate::llm::client::StarClient;
use crate::types::{StarTool, StarToolCall, ToolResult};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use crate::core::config::Config;
use crate::core::confirmation_bus::types::{
    Message, MessageBusType, ToolConfirmationRequest, ToolConfirmationResponse, ToolFinished,
    ToolPolicyRejection, ToolStarted,
};
use crate::core::confirmation_bus::MessageBus;
use crate::core::permissions::{PermissionHit, SessionPermissionManager};
use crate::core::policy::{FunctionCall, PolicyDecision};
use crate::core::state::CachedToolResult;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

// 默认确认超时 (毫秒)
const DEFAULT_CONFIRM_TIMEOUT_MS: u64 = 600_000; // 10分钟（给用户充足确认时间）

// 默认工具执行超时 (毫秒)
const DEFAULT_TOOL_TIMEOUT_MS: u64 = 600_000; // 10分钟

/// 从 panic payload 里取出可读消息
///
/// `panic!("...")` 的 payload 是 `&str` 或 `String`（`format!` 参数化的那种）；
/// 切片越界这类由 std 抛的也是 `String`。其它类型只能给个占位。
fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic payload>".to_string())
}

pub struct ToolExecutor {
    tool_registry: Arc<ToolRegistry>,
    config: Arc<Config>,
    message_bus: Option<Arc<MessageBus>>,
    cached_tool_definitions: RwLock<Option<(u64, Vec<StarTool>)>>,
    permission_manager: Arc<SessionPermissionManager>,
    /// 工具确认超时 (毫秒)
    confirm_timeout_ms: u64,
    /// 工具执行超时 (毫秒)
    tool_timeout_ms: u64,
}

impl ToolExecutor {
    /// Cache TTL in milliseconds (5 minutes)
    const CACHE_TTL_MS: u128 = 300_000;
    /// Maximum number of cached entries before eviction
    const MAX_CACHE_ENTRIES: usize = 200;

    pub fn new(
        _client: StarClient,
        tool_registry: Arc<ToolRegistry>,
        message_bus: Option<Arc<MessageBus>>,
        config: Arc<Config>,
    ) -> Self {
        let confirm_timeout_ms = std::env::var("STAR_CONFIRM_TIMEOUT")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_CONFIRM_TIMEOUT_MS);

        let tool_timeout_ms = std::env::var("STAR_TOOL_TIMEOUT")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_TOOL_TIMEOUT_MS);

        let permission_manager = Arc::new(SessionPermissionManager::with_persistence(
            config.storage().project_permissions_path(),
        ));

        Self {
            tool_registry,
            config,
            message_bus,
            cached_tool_definitions: RwLock::new(None),
            permission_manager,
            confirm_timeout_ms,
            tool_timeout_ms,
        }
    }

    pub fn get_tool_definitions(&self) -> Vec<StarTool> {
        let generation = self.tool_registry.generation();
        if let Ok(cache) = self.cached_tool_definitions.read() {
            if let Some((cached_generation, tools)) = cache.as_ref() {
                if *cached_generation == generation {
                    return tools.clone();
                }
            }
        }

        let mut tools = Vec::new();
        let hide_skill = self.config.recursion_depth > 0;

        // Add tools from registry
        for f in self.tool_registry.get_function_declarations() {
            // Tool description single source of truth: tool-description-*.md
            // (external dir → embedded). Falls back to the tool's default
            // description when no .md file is registered for it.
            let description =
                crate::core::prompts::tool_descriptions::resolve_tool_description(&f.name)
                    .unwrap_or_else(|| f.description.clone());
            tools.push(StarTool {
                tool_type: "function".to_string(),
                function: crate::types::StarToolFunction {
                    name: f.name,
                    description,
                    parameters: crate::types::StarToolParameters {
                        param_type: "object".to_string(),
                        properties: f
                            .parameters
                            .get("properties")
                            .cloned()
                            .unwrap_or(serde_json::json!({}))
                            .as_object()
                            .cloned()
                            .unwrap_or_default()
                            .into_iter()
                            .collect(),
                        required: f
                            .parameters
                            .get("required")
                            .cloned()
                            .unwrap_or(serde_json::json!([]))
                            .as_array()
                            .cloned()
                            .unwrap_or_default()
                            .iter()
                            .map(|v| v.as_str().unwrap_or_default().to_string())
                            .collect(),
                    },
                },
            });
        }

        if hide_skill {
            tools.retain(|t| t.function.name != "skill");
        }

        // MCP management tools are injected here so the agent can always orchestrate MCP safely.
        tools.extend(mcp_management_tools());

        let compacted = tools
            .into_iter()
            .map(compact_tool_definition)
            .collect::<Vec<_>>();

        if let Ok(mut cache) = self.cached_tool_definitions.write() {
            *cache = Some((generation, compacted.clone()));
        }

        compacted
    }

    /// 执行一个工具调用。
    ///
    /// # 为什么套一层 `catch_unwind`
    ///
    /// 工具是在 agent worker 那个 tokio 任务里**直接** await 的（`execute_batch` 用
    /// `join_all`，没有 per-tool spawn）。于是任何一个工具 panic，展开的是 worker 自己：
    /// UI→Agent 通道从此静默失效，用户看到的是"消息发出去了，没有任何反应"，而且没有
    /// 任何错误提示。
    ///
    /// 已知触发过这条路径的：`WebSearch` 按字节切中文网页正文（`&content[..2000]` 落在
    /// 汉字中间）。那些切片已经逐一改成按字符，但工具还会继续加，第三方 crate 也会自己
    /// panic（`readability-rust` 就是现成的例子），所以这里留一道兜底 —— panic 变成一条
    /// 普通的工具错误：模型看得到、能换个做法，worker 活着。
    ///
    /// panic hook 仍会先跑一遍，但 `ui::app::runtime` 的 hook 只在渲染线程上拆终端，
    /// 后台线程的 panic 只落日志，不会花屏。
    pub async fn execute(
        &self,
        tool_call: &StarToolCall,
        update_output: Option<Arc<dyn Fn(String) + Send + Sync>>,
        abort_signal: Option<tokio_util::sync::CancellationToken>,
    ) -> ToolResult {
        let guarded = std::panic::AssertUnwindSafe(self.execute_inner(
            tool_call,
            update_output,
            abort_signal,
        ));

        match futures::FutureExt::catch_unwind(guarded).await {
            Ok(result) => result,
            Err(payload) => {
                let detail = panic_payload_message(payload.as_ref());
                let tool_name = tool_call.function.name.as_str();

                crate::utils::logging::append_debug_log_line(&format!(
                    "[ToolExecutor] tool '{}' panicked ({}) — surfaced as a tool error instead of \
                     unwinding the agent worker",
                    tool_name, detail
                ));

                // 正常路径末尾会发 ToolFinished；panic 跳过了那里，这里补一条，
                // 否则 UI 上这个工具会一直转圈。
                if let Some(bus) = &self.message_bus {
                    let _ = bus
                        .publish(Message::ToolFinished(ToolFinished {
                            message_type: MessageBusType::ToolFinished,
                            tool_call_id: tool_call.id.clone(),
                            tool_name: tool_name.to_string(),
                            success: false,
                        }))
                        .await;
                }

                ToolResult {
                    success: false,
                    output: Some(format!(
                        "Tool `{}` crashed while running ({}). This is a defect in the tool itself, \
                         not in the request. The session is unaffected — try a different approach \
                         or a different tool.",
                        tool_name, detail
                    )),
                    error: Some(format!("tool panicked: {}", detail)),
                    data: None,
                }
            }
        }
    }

    async fn execute_inner(
        &self,
        tool_call: &StarToolCall,
        update_output: Option<Arc<dyn Fn(String) + Send + Sync>>,
        abort_signal: Option<tokio_util::sync::CancellationToken>,
    ) -> ToolResult {
        let original_tool_name = tool_call.function.name.as_str();
        let resolved_tool_name = Self::canonical_tool_name(original_tool_name);
        let name = resolved_tool_name.as_str();
        if self.config.recursion_depth > 0 && name == "skill" {
            return Self::error_result(
                "Skill tool is disabled for sub-agents to prevent recursion."
                    .to_string()
                    .into(),
            );
        }
        let mut args: Value =
            serde_json::from_str(&tool_call.function.arguments).unwrap_or(Value::Null);
        // LLM may send null or empty arguments; convert to empty object so
        // deserialization into tool params structs doesn't fail.
        if args.is_null() {
            args = Value::Object(serde_json::Map::new());
        }
        let args = Self::normalize_tool_args(original_tool_name, name, args);

        // Check for cancellation before starting
        if let Some(ref token) = abort_signal {
            if token.is_cancelled() {
                return Self::error_result("Tool execution cancelled before start.".into());
            }
        }

        // --- Cache Logic: Read ---
        let is_readonly = self.is_tool_read_only(name);
        let global_state = self.tool_registry.get_config().runtime_global_state();
        let cache_key = if is_readonly {
            Some(format!("{}:{}", name, args.to_string()))
        } else {
            None
        };

        if let Some(key) = &cache_key {
            if let Some(state) = &global_state {
                let cache = state.tool_cache.read().await;
                if let Some(cached) = cache.get(key) {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis();
                    let age_ms = now.saturating_sub(cached.timestamp);
                    if age_ms < Self::CACHE_TTL_MS {
                        crate::utils::logging::append_debug_log_line(&format!(
                            "🚀 Cache Hit: {} (age: {}ms)",
                            name, age_ms
                        ));

                        if let Some(bus) = &self.message_bus {
                            let _ = bus
                                .publish(Message::ToolStarted(ToolStarted {
                                    message_type: MessageBusType::ToolStarted,
                                    tool_call_id: tool_call.id.clone(),
                                    tool_name: name.to_string(),
                                }))
                                .await;
                        }

                        if let Some(bus) = &self.message_bus {
                            let _ = bus
                                .publish(Message::ToolFinished(ToolFinished {
                                    message_type: MessageBusType::ToolFinished,
                                    tool_call_id: tool_call.id.clone(),
                                    tool_name: name.to_string(),
                                    success: cached.result.success,
                                }))
                                .await;
                        }

                        return cached.result.clone();
                    }
                    // else: stale — don't use, fall through to re-execute
                }
                drop(cache);
            }
        }
        // -------------------------

        // Emit ToolStarted
        if let Some(bus) = &self.message_bus {
            let _ = bus
                .publish(Message::ToolStarted(ToolStarted {
                    message_type: MessageBusType::ToolStarted,
                    tool_call_id: tool_call.id.clone(),
                    tool_name: name.to_string(),
                }))
                .await;
        }

        // Try registry
        let result = if let Some(tool) = self.tool_registry.get_tool(name) {
            let permission_identity = tool.permission_cache_identity();
            match tool.create_invocation(args.clone()) {
                Ok(invocation) => {
                    // 1. Check Policy / Confirmation
                    let mut approved = false;

                    if let Some(bus) = &self.message_bus {
                        let check = bus
                            .check_tool_policy(
                                &FunctionCall {
                                    name: name.to_string(),
                                    args: Some(args.clone()),
                                },
                                None,
                            )
                            .await;

                        match check.decision {
                            PolicyDecision::Deny => {
                                let _ = bus
                                    .publish(Message::ToolPolicyRejection(ToolPolicyRejection {
                                        message_type: MessageBusType::ToolPolicyRejection,
                                        tool_call: FunctionCall {
                                            name: name.to_string(),
                                            args: Some(args.clone()),
                                        },
                                    }))
                                    .await;
                                return Self::error_result(
                                    "Tool execution denied by policy.".to_string().into(),
                                );
                            }
                            PolicyDecision::DenyWithReason(reason) => {
                                let _ = bus
                                    .publish(Message::ToolPolicyRejection(ToolPolicyRejection {
                                        message_type: MessageBusType::ToolPolicyRejection,
                                        tool_call: FunctionCall {
                                            name: name.to_string(),
                                            args: Some(args.clone()),
                                        },
                                    }))
                                    .await;
                                return Self::error_result(
                                    format!("Tool execution denied: {}", reason).into(),
                                );
                            }
                            PolicyDecision::Allow => {
                                approved = true;
                            }
                            PolicyDecision::AskUser => {}
                        }
                    }

                    if !approved {
                        let permission_hit = self.permission_manager.check_allowed_with_identity(
                            name,
                            &args,
                            permission_identity.as_deref(),
                        );
                        if permission_hit != PermissionHit::None {
                            let source = match permission_hit {
                                PermissionHit::ToolSession => "tool_session",
                                PermissionHit::ToolPersisted => "tool_persisted",
                                PermissionHit::SignatureSession => "signature_session",
                                PermissionHit::SignaturePersisted => "signature_persisted",
                                PermissionHit::None => "none",
                            };
                            crate::utils::logging::append_debug_log_line(&format!(
                                "✅ Permission: Allowing {} without prompt (source={})",
                                name, source
                            ));
                            approved = true;
                        }
                    }

                    if !approved {
                        if let Some(bus) = &self.message_bus {
                            // Pass cancellation token to confirmation check if needed (though it's usually fast)
                            let should_confirm = invocation
                                .should_confirm_execute(abort_signal.as_ref())
                                .await;

                            match should_confirm {
                                Ok(Some(details)) => {
                                    let request = ToolConfirmationRequest {
                                        message_type: MessageBusType::ToolConfirmationRequest,
                                        tool_call: FunctionCall {
                                            name: name.to_string(),
                                            args: Some(args.clone()),
                                        },
                                        correlation_id: "".to_string(), // Will be filled by bus.request
                                        server_name: None,
                                        title: Some(details.title),
                                        prompt: Some(details.prompt),
                                    };

                                    // Use bus.request to wait for response (Policy Check + User Confirmation if needed)
                                    // Timeout from config (default 1 minute)
                                    let response_result: Result<ToolConfirmationResponse, _> = bus
                                        .request(
                                            request,
                                            MessageBusType::ToolConfirmationResponse,
                                            self.confirm_timeout_ms,
                                        )
                                        .await;

                                    match response_result {
                                        Ok(resp) => {
                                            if resp.confirmed {
                                                approved = true;

                                                // Trigger callback (e.g. for trusted folders)
                                                let requested_outcome = resp.outcome.clone().unwrap_or(
                                                    crate::types::ToolConfirmationOutcome::ProceedOnce,
                                                );
                                                let outcome_val = tool
                                                    .normalize_confirmation_outcome(
                                                        requested_outcome.clone(),
                                                    );
                                                if outcome_val != requested_outcome {
                                                    crate::utils::logging::append_debug_log_line(&format!(
                                                        "🔐 Permission outcome normalized for {}: {:?} -> {:?}",
                                                        name, requested_outcome, outcome_val
                                                    ));
                                                }
                                                (details.on_confirm)(outcome_val.clone());

                                                if resp.outcome.is_some() {
                                                    match outcome_val {
                                                        crate::types::ToolConfirmationOutcome::AllowSession => {
                                                            self.permission_manager.allow_tool_session_with_identity(
                                                                name,
                                                                permission_identity.as_deref(),
                                                            );
                                                            crate::utils::logging::append_debug_log_line(&format!(
                                                                "🔐 Permission Granted: Allowing tool {} for session",
                                                                name
                                                            ));
                                                        }
                                                        crate::types::ToolConfirmationOutcome::ProceedAlways => {
                                                            self.permission_manager.allow_action_with_identity(
                                                                name,
                                                                &args,
                                                                permission_identity.as_deref(),
                                                            );
                                                            crate::utils::logging::append_debug_log_line(&format!(
                                                                "🔐 Permission Granted: Allowing {} for session (signature)",
                                                                name
                                                            ));
                                                        }
                                                        crate::types::ToolConfirmationOutcome::ProceedAlwaysAndSave => {
                                                            self.permission_manager.allow_tool_persisted_with_identity(
                                                                name,
                                                                permission_identity.as_deref(),
                                                            );
                                                            crate::utils::logging::append_debug_log_line(&format!(
                                                                "🔐 Permission Granted: Persisted allow for tool {}",
                                                                name
                                                            ));
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            return Self::error_result(
                                                format!("Confirmation timeout or error: {}", e)
                                                    .into(),
                                            )
                                        }
                                    }
                                }
                                Ok(None) => {
                                    // No confirmation needed
                                    approved = true;
                                }
                                Err(e) => return Self::error_result(e),
                            }
                        } else {
                            // No message bus (legacy/test mode), assume allowed unless tool invocation fails later
                            approved = true;
                        }
                    }

                    if approved {
                        // First execution
                        let first_result = match invocation
                            .execute(abort_signal.as_ref(), update_output.clone())
                            .await
                        {
                            Ok(core_result) => {
                                let success = core_result.error.is_none();
                                let error_msg =
                                    core_result.error.clone().map(|e| e.message.clone());
                                let output = core_result
                                    .return_display
                                    .clone()
                                    .unwrap_or_else(|| core_result.output.clone());

                                // Enrich error with structured error info
                                if !success {
                                    if let Some(ref err) = error_msg {
                                        let structured =
                                            StructuredError::from_tool_output(name, &output, err);
                                        let mut data = core_result
                                            .data
                                            .clone()
                                            .unwrap_or_else(|| serde_json::json!({}));
                                        if let Some(obj) = data.as_object_mut() {
                                            obj.insert(
                                                "structured_error".to_string(),
                                                structured.to_json_value(),
                                            );
                                        }
                                        ToolResult {
                                            success: false,
                                            output: Some(structured.format_display()),
                                            error: error_msg,
                                            data: Some(data),
                                        }
                                    } else {
                                        ToolResult {
                                            success: false,
                                            output: Some(output),
                                            error: error_msg,
                                            data: core_result.data.clone(),
                                        }
                                    }
                                } else {
                                    ToolResult {
                                        success: true,
                                        output: Some(output),
                                        error: None,
                                        data: core_result.data.clone(),
                                    }
                                }
                            }
                            Err(e) => return Self::error_result(e),
                        };

                        // Check for sandbox restriction and auto-retry without sandbox
                        if !first_result.success {
                            if let Some(ref data) = first_result.data {
                                if let Some(should_retry) = data.get("should_retry_without_sandbox")
                                {
                                    if should_retry.as_bool().unwrap_or(false) {
                                        crate::utils::logging::append_debug_log_line(
                                             &format!("🔄 Sandbox restriction detected, retrying without sandbox for {}", name)
                                         );

                                        // Re-execute with sandbox disabled
                                        let retry_args = if name == "Bash" {
                                            let mut map =
                                                args.as_object().cloned().unwrap_or_default();
                                            map.insert(
                                                "dangerously_disable_sandbox".to_string(),
                                                serde_json::json!(true),
                                            );
                                            serde_json::Value::Object(map)
                                        } else {
                                            args.clone()
                                        };

                                        // Create new invocation with modified args
                                        if let Some(tool) = self.tool_registry.get_tool(name) {
                                            if let Ok(retry_invocation) =
                                                tool.create_invocation(retry_args)
                                            {
                                                match retry_invocation
                                                    .execute(abort_signal.as_ref(), update_output)
                                                    .await
                                                {
                                                    Ok(retry_result) => {
                                                        return ToolResult {
                                                            success: retry_result.error.is_none(),
                                                            output: Some(
                                                                retry_result
                                                                    .return_display
                                                                    .unwrap_or(retry_result.output),
                                                            ),
                                                            error: retry_result
                                                                .error
                                                                .map(|e| e.message),
                                                            data: retry_result.data,
                                                        };
                                                    }
                                                    Err(e) => {
                                                        return Self::error_result(e);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        first_result
                    } else {
                        Self::error_result("Tool execution denied.".into())
                    }
                }
                Err(e) => Self::error_result(e),
            }
        } else if is_mcp_management_tool(name) {
            execute_mcp_management_tool(&self.tool_registry, name, &args).await
        } else if name.starts_with("mcp__") {
            execute_mcp_dynamic_tool(&self.tool_registry, name, &args).await
        } else {
            Self::error_result(format!("Unknown tool: {}", name).into())
        };

        // --- Cache Logic: Write/Invalidate ---
        if let Some(state) = &global_state {
            if let Some(key) = cache_key {
                // Is read-only, save result if successful
                if result.success {
                    let mut cache = state.tool_cache.write().await;
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis();
                    cache.insert(
                        key,
                        CachedToolResult {
                            result: result.clone(),
                            timestamp: now,
                        },
                    );
                    // Evict oldest entries if over limit
                    if cache.len() > Self::MAX_CACHE_ENTRIES {
                        let to_remove: usize = cache.len() - Self::MAX_CACHE_ENTRIES;
                        let mut entries: Vec<_> = cache.iter().collect();
                        entries.sort_by_key(|(_, v)| v.timestamp);
                        let keys_to_remove: Vec<String> = entries
                            .into_iter()
                            .take(to_remove)
                            .map(|(k, _)| k.clone())
                            .collect();
                        for k in &keys_to_remove {
                            cache.remove(k);
                        }
                    }
                }
            } else {
                // Write operation -> Invalidate
                let mut cache = state.tool_cache.write().await;
                if !cache.is_empty() {
                    cache.clear();
                    crate::utils::logging::append_debug_log_line(
                        "🧹 Cache invalidated due to write operation",
                    );
                }
            }
        }
        // -------------------------------------

        // Emit ToolFinished
        if let Some(bus) = &self.message_bus {
            let _ = bus
                .publish(Message::ToolFinished(ToolFinished {
                    message_type: MessageBusType::ToolFinished,
                    tool_call_id: tool_call.id.clone(),
                    tool_name: name.to_string(),
                    success: result.success,
                }))
                .await;
        }

        result
    }
    fn error_result(e: Box<dyn std::error::Error>) -> ToolResult {
        let error_msg = e.to_string();
        let structured = StructuredError::from_tool_output("unknown", "", &error_msg);
        let mut data = serde_json::Map::new();
        data.insert("structured_error".to_string(), structured.to_json_value());
        ToolResult {
            success: false,
            output: Some(structured.format_display()),
            error: Some(error_msg),
            data: Some(serde_json::Value::Object(data)),
        }
    }

    pub fn is_tool_read_only(&self, name: &str) -> bool {
        let canonical_name = Self::canonical_tool_name(name);
        let name = canonical_name.as_str();
        if matches!(
            name,
            "mcp_list_servers" | "mcp_list_tools" | "mcp_tool_info" | "mcp_search_tools"
        ) {
            return true;
        }
        if let Some(tool) = self.tool_registry.get_tool(name) {
            tool.is_read_only()
        } else {
            false
        }
    }

    fn canonical_tool_name(name: &str) -> String {
        crate::core::tools::constants::canonical_tool_name(name)
    }

    fn normalize_tool_args(original_name: &str, canonical_name: &str, args: Value) -> Value {
        // 先对所有工具做通用修复：
        // 1. 把字符串数字转为真正的数字
        // 2. 把 camelCase 参数名转为 snake_case
        let args = Self::fix_string_numbers(args);
        let args = Self::camel_to_snake_keys(args);
        match canonical_name {
            "Edit" => Self::normalize_replace_args(args),
            "multi_edit" => Self::normalize_multi_edit_args(args),
            "Grep" => Self::normalize_search_args(original_name, args),
            "Read" | "view_file" => Self::normalize_read_file_args(args),
            "Agent" => Self::normalize_agent_args(args),
            _ => args,
        }
    }

    /// 递归遍历 JSON args，把 camelCase 的 key 转为 snake_case
    /// 例如: filePath → file_path, oldString → old_string
    fn camel_to_snake_keys(value: Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut new_map = serde_json::Map::new();
                for (k, v) in map {
                    let snake_key = Self::camel_to_snake(&k);
                    new_map.insert(snake_key, Self::camel_to_snake_keys(v));
                }
                Value::Object(new_map)
            }
            Value::Array(arr) => {
                Value::Array(arr.into_iter().map(Self::camel_to_snake_keys).collect())
            }
            other => other,
        }
    }

    /// 把 camelCase 字符串转为 snake_case
    fn camel_to_snake(s: &str) -> String {
        // 如果已经是 snake_case 或全小写，直接返回
        if !s.chars().any(|c| c.is_uppercase()) {
            return s.to_string();
        }
        let mut result = String::with_capacity(s.len() + 4);
        for (i, ch) in s.chars().enumerate() {
            if ch.is_uppercase() {
                if i > 0 {
                    result.push('_');
                }
                result.push(ch.to_lowercase().next().unwrap_or(ch));
            } else {
                result.push(ch);
            }
        }
        result
    }

    /// 归一化 read_file 参数，处理 LLM 常见的参数格式错误
    fn normalize_read_file_args(mut args: Value) -> Value {
        let Some(obj) = args.as_object_mut() else {
            return args;
        };
        // 兼容各种参数名
        Self::copy_string_arg(
            obj,
            "file_path",
            &["file_path", "path", "filePath", "filename", "file"],
        );
        args
    }

    /// 归一化 Agent 参数，处理 subagent_type 大小写问题
    fn normalize_agent_args(mut args: Value) -> Value {
        let Some(obj) = args.as_object_mut() else {
            return args;
        };
        // subagent_type: 兼容 camelCase 和 PascalCase
        if let Some(val) = obj
            .get("subagent_type")
            .or(obj.get("subagentType"))
            .cloned()
        {
            if let Some(s) = val.as_str() {
                let lower = s.to_lowercase();
                let normalized = match lower.as_str() {
                    "general_purpose" | "generalpurpose" | "general" => "general_purpose",
                    "explorer" | "explore" => "explorer",
                    "analyzer" | "analyze" => "analyzer",
                    "editor" | "edit" => "editor",
                    "code_reviewer" | "codereviewer" | "reviewer" => "code_reviewer",
                    other => other,
                };
                obj.insert(
                    "subagent_type".to_string(),
                    Value::String(normalized.to_string()),
                );
                obj.remove("subagentType");
            }
        }
        args
    }

    /// 递归遍历 JSON args，把字符串数字转为真正的数字类型
    fn fix_string_numbers(value: Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut new_map = serde_json::Map::new();
                for (k, v) in map {
                    new_map.insert(k, Self::fix_string_numbers(v));
                }
                Value::Object(new_map)
            }
            Value::Array(arr) => {
                Value::Array(arr.into_iter().map(Self::fix_string_numbers).collect())
            }
            Value::String(s) => {
                // 尝试解析为 u64/i64/f64，成功后返回数字类型
                if let Ok(n) = s.parse::<i64>() {
                    Value::Number(serde_json::Number::from(n))
                } else if let Ok(n) = s.parse::<u64>() {
                    Value::Number(serde_json::Number::from(n))
                } else if let Ok(n) = s.parse::<f64>() {
                    if let Some(num) = serde_json::Number::from_f64(n) {
                        Value::Number(num)
                    } else {
                        Value::String(s)
                    }
                } else {
                    Value::String(s)
                }
            }
            other => other,
        }
    }

    fn normalize_replace_args(mut args: Value) -> Value {
        let Some(obj) = args.as_object_mut() else {
            return args;
        };

        Self::copy_string_arg(
            obj,
            "file_path",
            &["file_path", "path", "target_file", "file"],
        );
        Self::copy_string_arg(
            obj,
            "old_string",
            &["old_string", "old_str", "old", "oldString", "old_text"],
        );
        Self::copy_string_arg(
            obj,
            "new_string",
            &["new_string", "new_str", "new", "newString", "new_text"],
        );

        args
    }

    /// 归一化 multi_edit 参数，处理 LLM 常见的参数格式错误
    fn normalize_multi_edit_args(mut args: Value) -> Value {
        // 如果 LLM 错误地使用了 flat 格式（跟 replace 一样），自动包装成 edits 数组
        if args.as_object().map_or(false, |obj| {
            obj.contains_key("file_path") && !obj.contains_key("edits")
        }) {
            let obj = args.as_object_mut().unwrap();
            // 提取 file_path, old_string, new_string
            let file_path =
                Self::first_string_arg(obj, &["file_path", "path", "target_file", "file"]);
            let old_string = Self::first_string_arg(
                obj,
                &["old_string", "old_str", "old", "oldString", "old_text"],
            );
            let new_string = Self::first_string_arg(
                obj,
                &["new_string", "new_str", "new", "newString", "new_text"],
            );

            if let (Some(file_path), Some(old_string), Some(new_string)) =
                (file_path, old_string, new_string)
            {
                let edit = serde_json::json!({
                    "file_path": file_path,
                    "old_string": old_string,
                    "new_string": new_string,
                });
                args = serde_json::json!({ "edits": [edit] });
            }
            return args;
        }

        // 归一化 edits 数组中每个元素的字段名
        let Some(obj) = args.as_object_mut() else {
            return args;
        };

        if let Some(Value::Array(edits)) = obj.get_mut("edits") {
            for edit in edits.iter_mut() {
                if let Some(edit_obj) = edit.as_object_mut() {
                    Self::copy_string_arg(
                        edit_obj,
                        "file_path",
                        &["file_path", "path", "target_file", "file"],
                    );
                    Self::copy_string_arg(
                        edit_obj,
                        "old_string",
                        &["old_string", "old_str", "old", "oldString", "old_text"],
                    );
                    Self::copy_string_arg(
                        edit_obj,
                        "new_string",
                        &["new_string", "new_str", "new", "newString", "new_text"],
                    );
                }
            }
        }

        args
    }

    fn normalize_search_args(original_name: &str, mut args: Value) -> Value {
        let Some(obj) = args.as_object_mut() else {
            return args;
        };

        Self::copy_string_arg(obj, "query", &["query", "pattern"]);

        if obj.get("search_type").is_none()
            && matches!(
                original_name,
                "search_file_content" | "grep_search" | "Grep"
            )
        {
            obj.insert("search_type".to_string(), Value::String("text".to_string()));
        }

        if obj.get("regex").is_none() && matches!(original_name, "grep_search" | "Grep") {
            obj.insert("regex".to_string(), Value::Bool(true));
        }

        if obj.get("include_pattern").is_none() {
            let legacy_path = Self::first_string_arg(obj, &["path", "dir_path"]);
            let legacy_file_pattern =
                Self::first_string_arg(obj, &["file_pattern", "include", "Glob"]);
            if let Some(include_pattern) = Self::build_legacy_search_include_pattern(
                legacy_path.as_deref(),
                legacy_file_pattern.as_deref(),
            ) {
                obj.insert(
                    "include_pattern".to_string(),
                    Value::String(include_pattern),
                );
            }
        }

        args
    }

    fn copy_string_arg(
        obj: &mut serde_json::Map<String, Value>,
        target_key: &str,
        candidate_keys: &[&str],
    ) {
        if obj.get(target_key).and_then(|v| v.as_str()).is_some() {
            return;
        }

        if let Some(value) = Self::first_string_arg(obj, candidate_keys) {
            obj.insert(target_key.to_string(), Value::String(value));
        }
    }

    fn first_string_arg(
        obj: &serde_json::Map<String, Value>,
        candidate_keys: &[&str],
    ) -> Option<String> {
        candidate_keys.iter().find_map(|key| {
            obj.get(*key)
                .and_then(|value| value.as_str())
                .map(|value| value.to_string())
                .filter(|value| !value.trim().is_empty())
        })
    }

    fn build_legacy_search_include_pattern(
        legacy_path: Option<&str>,
        legacy_file_pattern: Option<&str>,
    ) -> Option<String> {
        let normalized_path = legacy_path
            .map(str::trim)
            .filter(|value| !value.is_empty() && *value != ".")
            .map(Self::normalize_search_scope);
        let normalized_file_pattern = legacy_file_pattern
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.replace('\\', "/"));

        match (normalized_path, normalized_file_pattern) {
            (None, None) => None,
            (None, Some(pattern)) => Some(pattern),
            (Some(path), None) => {
                if Self::looks_like_glob(&path) || std::path::Path::new(&path).extension().is_some()
                {
                    Some(path)
                } else {
                    Some(format!("{}/**", path.trim_end_matches('/')))
                }
            }
            (Some(path), Some(pattern)) => {
                let path = path.trim_end_matches('/');
                let pattern = pattern.trim_start_matches('/');
                if Self::looks_like_glob(pattern) && pattern.starts_with(path) {
                    Some(pattern.to_string())
                } else if pattern.contains('/') {
                    Some(format!("{}/{}", path, pattern))
                } else {
                    Some(format!("{}/**/{}", path, pattern))
                }
            }
        }
    }

    fn normalize_search_scope(raw: &str) -> String {
        let path = std::path::Path::new(raw);
        if path.is_absolute() {
            let cwd = crate::core::utils::paths::current_dir_cached();
            if let Ok(relative) = path.strip_prefix(cwd) {
                return relative.to_string_lossy().replace('\\', "/");
            }
        }

        raw.trim_start_matches("./").replace('\\', "/")
    }

    fn looks_like_glob(value: &str) -> bool {
        value.contains('*') || value.contains('?') || value.contains('[') || value.contains('{')
    }

    /// 获取最大工具并发数（从环境变量读取，默认10）
    fn max_tool_concurrency() -> usize {
        std::env::var("STAR_MAX_TOOL_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(10)
            .max(1) // 至少1个并发
    }

    pub async fn execute_batch(
        &self,
        tool_calls: Vec<StarToolCall>,
        update_output: Option<Arc<dyn Fn(String) + Send + Sync>>,
        abort_signal: Option<tokio_util::sync::CancellationToken>,
    ) -> Vec<ToolResult> {
        let mut tool_calls_with_index: Vec<(usize, StarToolCall)> =
            tool_calls.into_iter().enumerate().collect();
        let mut groups: Vec<Vec<(usize, StarToolCall)>> = Vec::new();

        if tool_calls_with_index.is_empty() {
            return Vec::new();
        }

        let first = tool_calls_with_index.remove(0);
        let mut current_group = vec![first];
        let mut is_current_group_readonly =
            self.is_tool_read_only(&current_group[0].1.function.name);

        for (index, call) in tool_calls_with_index {
            let is_readonly = self.is_tool_read_only(&call.function.name);

            if is_readonly == is_current_group_readonly {
                current_group.push((index, call));
            } else {
                groups.push(current_group);
                current_group = vec![(index, call)];
                is_current_group_readonly = is_readonly;
            }
        }
        groups.push(current_group);

        let max_concurrency = Self::max_tool_concurrency();
        let mut final_results: Vec<(usize, ToolResult)> = Vec::new();

        for group in groups {
            if group.is_empty() {
                continue;
            }
            let is_readonly = self.is_tool_read_only(&group[0].1.function.name);

            if is_readonly && group.len() > 1 {
                // Parallel execution for read-only tools with concurrency limit
                use futures::future::join_all;

                // Split into chunks of max_concurrency size
                for chunk in group.chunks(max_concurrency) {
                    let futures: Vec<_> = chunk
                        .iter()
                        .map(|(index, call)| {
                            let uo = update_output.clone();
                            let sig = abort_signal.clone();
                            let call_clone = call.clone();
                            let index_clone = *index;
                            async move {
                                let result = self.execute(&call_clone, uo, sig).await;
                                (index_clone, result)
                            }
                        })
                        .collect();
                    let results = join_all(futures).await;
                    final_results.extend(results);
                }
            } else {
                // Sequential execution for write tools
                for (index, call) in group {
                    let result = self
                        .execute(&call, update_output.clone(), abort_signal.clone())
                        .await;
                    final_results.push((index, result));
                }
            }
        }

        final_results.sort_by_key(|k| k.0);
        final_results.into_iter().map(|(_, v)| v).collect()
    }
}

// ── Schema pruning helpers ───────────────────────────────────────────

const DEFAULT_TOOL_DESCRIPTION_MAX_CHARS: usize = 160;
const DEFAULT_TOOL_PARAM_DESCRIPTION_MAX_CHARS: usize = 120;
const DEFAULT_TOOL_SCHEMA_DEPTH: usize = 4;

fn normalize_tool_text(input: &str, max_chars: usize) -> String {
    let compact = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }

    let mut out = compact.chars().take(max_chars).collect::<String>();
    out.push_str("...");
    out
}

fn tool_description_max_chars() -> usize {
    std::env::var("STAR_TOOL_DESCRIPTION_MAX_CHARS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_TOOL_DESCRIPTION_MAX_CHARS)
        .max(48)
}

fn tool_param_description_max_chars() -> usize {
    std::env::var("STAR_TOOL_PARAM_DESCRIPTION_MAX_CHARS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_TOOL_PARAM_DESCRIPTION_MAX_CHARS)
        .max(32)
}

fn prune_schema_value(value: &Value, depth: usize, max_desc_chars: usize) -> Value {
    if depth == 0 {
        return match value {
            Value::Object(map) => {
                let mut out = serde_json::Map::new();
                if let Some(value_type) = map.get("type") {
                    out.insert("type".to_string(), value_type.clone());
                }
                if let Some(description) = map.get("description").and_then(|v| v.as_str()) {
                    out.insert(
                        "description".to_string(),
                        Value::String(normalize_tool_text(description, max_desc_chars)),
                    );
                }
                Value::Object(out)
            }
            _ => value.clone(),
        };
    }

    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();

            if let Some(value_type) = map.get("type") {
                out.insert("type".to_string(), value_type.clone());
            }

            if let Some(description) = map.get("description").and_then(|v| v.as_str()) {
                let normalized = normalize_tool_text(description, max_desc_chars);
                if !normalized.is_empty() {
                    out.insert("description".to_string(), Value::String(normalized));
                }
            }

            if let Some(format) = map.get("format").and_then(|v| v.as_str()) {
                out.insert("format".to_string(), Value::String(format.to_string()));
            }

            if let Some(enum_values) = map.get("enum").and_then(|v| v.as_array()) {
                out.insert(
                    "enum".to_string(),
                    Value::Array(enum_values.iter().take(16).cloned().collect()),
                );
            }

            if let Some(items) = map.get("items") {
                out.insert(
                    "items".to_string(),
                    prune_schema_value(items, depth.saturating_sub(1), max_desc_chars),
                );
            }

            if let Some(properties) = map.get("properties").and_then(|v| v.as_object()) {
                let mut pruned_properties = serde_json::Map::new();
                for (key, property_schema) in properties {
                    pruned_properties.insert(
                        key.clone(),
                        prune_schema_value(
                            property_schema,
                            depth.saturating_sub(1),
                            max_desc_chars,
                        ),
                    );
                }
                if !pruned_properties.is_empty() {
                    out.insert("properties".to_string(), Value::Object(pruned_properties));
                }
            }

            if let Some(required) = map.get("required").and_then(|v| v.as_array()) {
                out.insert("required".to_string(), Value::Array(required.clone()));
            }

            if let Some(additional_properties) = map.get("additionalProperties") {
                let pruned = match additional_properties {
                    Value::Bool(value) => Value::Bool(*value),
                    Value::Object(_) => prune_schema_value(
                        additional_properties,
                        depth.saturating_sub(1),
                        max_desc_chars,
                    ),
                    _ => Value::Bool(true),
                };
                out.insert("additionalProperties".to_string(), pruned);
            }

            for key in ["anyOf", "oneOf", "allOf"] {
                if let Some(variants) = map.get(key).and_then(|v| v.as_array()) {
                    out.insert(
                        key.to_string(),
                        Value::Array(
                            variants
                                .iter()
                                .take(4)
                                .map(|variant| {
                                    prune_schema_value(
                                        variant,
                                        depth.saturating_sub(1),
                                        max_desc_chars,
                                    )
                                })
                                .collect(),
                        ),
                    );
                }
            }

            Value::Object(out)
        }
        Value::Array(values) => Value::Array(
            values
                .iter()
                .take(16)
                .map(|value| prune_schema_value(value, depth.saturating_sub(1), max_desc_chars))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn compact_tool_definition(mut tool: StarTool) -> StarTool {
    let tool_description_limit = tool_description_max_chars();
    let param_description_limit = tool_param_description_max_chars();

    tool.function.description =
        normalize_tool_text(&tool.function.description, tool_description_limit);
    tool.function.parameters.properties = tool
        .function
        .parameters
        .properties
        .into_iter()
        .map(|(key, value)| {
            (
                key,
                prune_schema_value(&value, DEFAULT_TOOL_SCHEMA_DEPTH, param_description_limit),
            )
        })
        .collect();
    tool
}

// ── MCP management tools ─────────────────────────────────────────────

fn mcp_management_tools() -> [StarTool; 10] {
    [
        StarTool {
            tool_type: "function".to_string(),
            function: crate::types::StarToolFunction {
                name: "mcp_list_servers".to_string(),
                description: "List MCP servers currently configured".to_string(),
                parameters: crate::types::StarToolParameters {
                    param_type: "object".to_string(),
                    properties: HashMap::new(),
                    required: vec![],
                },
            },
        },
        StarTool {
            tool_type: "function".to_string(),
            function: crate::types::StarToolFunction {
                name: "mcp_list_tools".to_string(),
                description: "List tools for an MCP server (returns mcp__<server>__<tool>)"
                    .to_string(),
                parameters: crate::types::StarToolParameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert(
                            "server".to_string(),
                            serde_json::json!({
                                "type": "string",
                                "description": "MCP server name"
                            }),
                        );
                        props
                    },
                    required: vec!["server".to_string()],
                },
            },
        },
        StarTool {
            tool_type: "function".to_string(),
            function: crate::types::StarToolFunction {
                name: "mcp_tool_info".to_string(),
                description: "Get schema/description for a tool in an MCP server".to_string(),
                parameters: crate::types::StarToolParameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert(
                            "server".to_string(),
                            serde_json::json!({
                                "type": "string",
                                "description": "MCP server name"
                            }),
                        );
                        props.insert(
                            "tool".to_string(),
                            serde_json::json!({
                                "type": "string",
                                "description": "Tool name in server (without mcp__ prefix)"
                            }),
                        );
                        props
                    },
                    required: vec!["server".to_string(), "tool".to_string()],
                },
            },
        },
        StarTool {
            tool_type: "function".to_string(),
            function: crate::types::StarToolFunction {
                name: "mcp_search_tools".to_string(),
                description: "Search MCP tools by keyword across servers".to_string(),
                parameters: crate::types::StarToolParameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert(
                            "query".to_string(),
                            serde_json::json!({
                                "type": "string",
                                "description": "Keyword"
                            }),
                        );
                        props.insert(
                            "server".to_string(),
                            serde_json::json!({
                                "type": "string",
                                "description": "Optional server filter"
                            }),
                        );
                        props
                    },
                    required: vec!["query".to_string()],
                },
            },
        },
        StarTool {
            tool_type: "function".to_string(),
            function: crate::types::StarToolFunction {
                name: "mcp_restart_server".to_string(),
                description: "Restart one MCP server and refresh tools".to_string(),
                parameters: crate::types::StarToolParameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert(
                            "server".to_string(),
                            serde_json::json!({
                                "type": "string",
                                "description": "MCP server name"
                            }),
                        );
                        props
                    },
                    required: vec!["server".to_string()],
                },
            },
        },
        StarTool {
            tool_type: "function".to_string(),
            function: crate::types::StarToolFunction {
                name: "mcp_refresh".to_string(),
                description: "Re-discover MCP servers and refresh tool cache".to_string(),
                parameters: crate::types::StarToolParameters {
                    param_type: "object".to_string(),
                    properties: HashMap::new(),
                    required: vec![],
                },
            },
        },
        StarTool {
            tool_type: "function".to_string(),
            function: crate::types::StarToolFunction {
                name: "mcp_list_resources".to_string(),
                description: "列出 MCP 服务器的资源列表 (List resources for an MCP server)"
                    .to_string(),
                parameters: crate::types::StarToolParameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert(
                            "server".to_string(),
                            serde_json::json!({
                                "type": "string",
                                "description": "MCP server name"
                            }),
                        );
                        props
                    },
                    required: vec!["server".to_string()],
                },
            },
        },
        StarTool {
            tool_type: "function".to_string(),
            function: crate::types::StarToolFunction {
                name: "mcp_read_resource".to_string(),
                description: "读取 MCP 服务器上的资源内容 (Read a resource from an MCP server)"
                    .to_string(),
                parameters: crate::types::StarToolParameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert(
                            "server".to_string(),
                            serde_json::json!({
                                "type": "string",
                                "description": "MCP server name"
                            }),
                        );
                        props.insert(
                            "uri".to_string(),
                            serde_json::json!({
                                "type": "string",
                                "description": "Resource URI to read"
                            }),
                        );
                        props
                    },
                    required: vec!["server".to_string(), "uri".to_string()],
                },
            },
        },
        StarTool {
            tool_type: "function".to_string(),
            function: crate::types::StarToolFunction {
                name: "mcp_list_prompts".to_string(),
                description: "列出 MCP 服务器的提示模板列表 (List prompt templates for an MCP server)"
                    .to_string(),
                parameters: crate::types::StarToolParameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert(
                            "server".to_string(),
                            serde_json::json!({
                                "type": "string",
                                "description": "MCP server name"
                            }),
                        );
                        props
                    },
                    required: vec!["server".to_string()],
                },
            },
        },
        StarTool {
            tool_type: "function".to_string(),
            function: crate::types::StarToolFunction {
                name: "mcp_get_prompt".to_string(),
                description:
                    "获取 MCP 服务器的提示模板 (Get a prompt template from an MCP server with optional arguments)"
                        .to_string(),
                parameters: crate::types::StarToolParameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert(
                            "server".to_string(),
                            serde_json::json!({
                                "type": "string",
                                "description": "MCP server name"
                            }),
                        );
                        props.insert(
                            "prompt".to_string(),
                            serde_json::json!({
                                "type": "string",
                                "description": "Prompt template name"
                            }),
                        );
                        props.insert(
                            "arguments".to_string(),
                            serde_json::json!({
                                "type": "object",
                                "description": "Optional arguments for the prompt template"
                            }),
                        );
                        props
                    },
                    required: vec!["server".to_string(), "prompt".to_string()],
                },
            },
        },
    ]
}

fn is_mcp_management_tool(name: &str) -> bool {
    matches!(
        name,
        "mcp_list_servers"
            | "mcp_list_tools"
            | "mcp_tool_info"
            | "mcp_search_tools"
            | "mcp_restart_server"
            | "mcp_refresh"
            | "mcp_list_resources"
            | "mcp_read_resource"
            | "mcp_list_prompts"
            | "mcp_get_prompt"
    )
}

async fn execute_mcp_management_tool(
    tool_registry: &Arc<ToolRegistry>,
    name: &str,
    args: &Value,
) -> ToolResult {
    let config = tool_registry.get_config();
    let Some(mcp_manager) = config.runtime_mcp_manager() else {
        return ToolResult {
            success: false,
            output: None,
            error: Some("MCP not initialized".to_string()),
            data: None,
        };
    };

    let require_str = |key: &str| -> Result<String, ToolResult> {
        let value = args
            .get(key)
            .and_then(|v| v.as_str())
            .map(|v| v.trim().to_string())
            .unwrap_or_default();
        if value.is_empty() {
            Err(ToolResult {
                success: false,
                output: None,
                error: Some(format!("Missing required argument: {}", key)),
                data: None,
            })
        } else {
            Ok(value)
        }
    };

    match name {
        "mcp_list_servers" => {
            let servers = mcp_manager.list_server_names().await;
            ToolResult {
                success: true,
                output: Some(servers.join("\n")),
                error: None,
                data: Some(serde_json::json!(servers)),
            }
        }
        "mcp_list_tools" => {
            let server = match require_str("server") {
                Ok(v) => v,
                Err(err) => return err,
            };
            match mcp_manager.list_tools(&server).await {
                Ok(tools) => {
                    let names: Vec<String> = tools
                        .iter()
                        .map(|t| format!("mcp__{}__{}", server, t.name))
                        .collect();
                    ToolResult {
                        success: true,
                        output: Some(names.join("\n")),
                        error: None,
                        data: Some(serde_json::json!(tools)),
                    }
                }
                Err(e) => ToolResult {
                    success: false,
                    output: None,
                    error: Some(e.to_string()),
                    data: None,
                },
            }
        }
        "mcp_tool_info" => {
            let server = match require_str("server") {
                Ok(v) => v,
                Err(err) => return err,
            };
            let tool = match require_str("tool") {
                Ok(v) => v,
                Err(err) => return err,
            };
            match mcp_manager.list_tools(&server).await {
                Ok(tools) => match tools.into_iter().find(|t| t.name == tool) {
                    Some(t) => {
                        let payload = serde_json::json!({
                            "server": server,
                            "tool": t.name,
                            "description": t.description,
                            "inputSchema": t.input_schema
                        });
                        ToolResult {
                            success: true,
                            output: Some(
                                serde_json::to_string_pretty(&payload)
                                    .unwrap_or_else(|_| payload.to_string()),
                            ),
                            error: None,
                            data: Some(payload),
                        }
                    }
                    None => ToolResult {
                        success: false,
                        output: None,
                        error: Some(format!("Tool {} not found in server {}", tool, server)),
                        data: None,
                    },
                },
                Err(e) => ToolResult {
                    success: false,
                    output: None,
                    error: Some(e.to_string()),
                    data: None,
                },
            }
        }
        "mcp_search_tools" => {
            let query = match require_str("query") {
                Ok(v) => v.to_lowercase(),
                Err(err) => return err,
            };
            let server_filter = args
                .get("server")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let mut hits = Vec::new();
            let mut errors = Vec::new();
            let servers = mcp_manager.list_server_names().await;
            for server in servers {
                if let Some(ref filter) = server_filter {
                    if !server.contains(filter) {
                        continue;
                    }
                }

                match mcp_manager.list_tools(&server).await {
                    Ok(tools) => {
                        for tool in tools {
                            let n = tool.name.to_lowercase();
                            let d = tool.description.to_lowercase();
                            if n.contains(&query) || d.contains(&query) {
                                hits.push(serde_json::json!({
                                    "server": server,
                                    "tool": tool.name,
                                    "description": tool.description,
                                    "fullName": format!("mcp__{}__{}", server, tool.name),
                                }));
                            }
                        }
                    }
                    Err(e) => {
                        errors.push(serde_json::json!({
                            "server": server,
                            "error": e.to_string(),
                        }));
                    }
                }
            }

            let payload = serde_json::json!({
                "query": query,
                "hits": hits,
                "errors": errors,
            });
            ToolResult {
                success: true,
                output: Some(
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string()),
                ),
                error: None,
                data: Some(payload),
            }
        }
        "mcp_restart_server" => {
            let server = match require_str("server") {
                Ok(v) => v,
                Err(err) => return err,
            };
            match mcp_manager.restart_server(&server).await {
                Ok(()) => {
                    let tools = mcp_manager
                        .get_cached_tools(&server)
                        .await
                        .unwrap_or_default();
                    let names: Vec<String> = tools
                        .iter()
                        .map(|t| format!("mcp__{}__{}", server, t.name))
                        .collect();
                    let payload = serde_json::json!({
                        "server": server,
                        "toolCount": names.len(),
                        "tools": names,
                    });
                    ToolResult {
                        success: true,
                        output: Some(
                            serde_json::to_string_pretty(&payload)
                                .unwrap_or_else(|_| payload.to_string()),
                        ),
                        error: None,
                        data: Some(payload),
                    }
                }
                Err(e) => ToolResult {
                    success: false,
                    output: None,
                    error: Some(e.to_string()),
                    data: None,
                },
            }
        }
        "mcp_refresh" => {
            if let Err(e) = mcp_manager.initialize_mcp_servers().await {
                return ToolResult {
                    success: false,
                    output: None,
                    error: Some(e.to_string()),
                    data: None,
                };
            }

            let errors = mcp_manager.discover_all().await;
            let server_count = mcp_manager.list_server_names().await.len();
            let payload = serde_json::json!({
                "serverCount": server_count,
                "errors": errors,
            });
            ToolResult {
                success: payload["errors"]
                    .as_object()
                    .map(|m| m.is_empty())
                    .unwrap_or(true),
                output: Some(
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string()),
                ),
                error: None,
                data: Some(payload),
            }
        }
        "mcp_list_resources" => {
            let server = match require_str("server") {
                Ok(v) => v,
                Err(err) => return err,
            };
            match mcp_manager.list_resources(&server).await {
                Ok(resources) => {
                    let text = resources
                        .iter()
                        .map(|r| format!("{} ({})", r.uri, r.name))
                        .collect::<Vec<_>>()
                        .join("\n");
                    ToolResult {
                        success: true,
                        output: Some(if text.is_empty() {
                            "(no resources)".to_string()
                        } else {
                            text
                        }),
                        error: None,
                        data: Some(serde_json::json!(resources)),
                    }
                }
                Err(e) => ToolResult {
                    success: false,
                    output: None,
                    error: Some(e.to_string()),
                    data: None,
                },
            }
        }
        "mcp_read_resource" => {
            let server = match require_str("server") {
                Ok(v) => v,
                Err(err) => return err,
            };
            let uri = match require_str("uri") {
                Ok(v) => v,
                Err(err) => return err,
            };
            match mcp_manager.read_resource(&server, &uri).await {
                Ok(contents) => {
                    let text = contents
                        .iter()
                        .filter_map(|c| c.text.as_deref())
                        .collect::<Vec<_>>()
                        .join("\n");
                    ToolResult {
                        success: true,
                        output: Some(text),
                        error: None,
                        data: Some(serde_json::json!(contents)),
                    }
                }
                Err(e) => ToolResult {
                    success: false,
                    output: None,
                    error: Some(e.to_string()),
                    data: None,
                },
            }
        }
        "mcp_list_prompts" => {
            let server = match require_str("server") {
                Ok(v) => v,
                Err(err) => return err,
            };
            match mcp_manager.list_prompts(&server).await {
                Ok(prompts) => {
                    let text = prompts
                        .iter()
                        .map(|p| {
                            let desc = p.description.as_deref().unwrap_or("");
                            if desc.is_empty() {
                                p.name.clone()
                            } else {
                                format!("{}: {}", p.name, desc)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    ToolResult {
                        success: true,
                        output: Some(if text.is_empty() {
                            "(no prompts)".to_string()
                        } else {
                            text
                        }),
                        error: None,
                        data: Some(serde_json::json!(prompts)),
                    }
                }
                Err(e) => ToolResult {
                    success: false,
                    output: None,
                    error: Some(e.to_string()),
                    data: None,
                },
            }
        }
        "mcp_get_prompt" => {
            let server = match require_str("server") {
                Ok(v) => v,
                Err(err) => return err,
            };
            let prompt = match require_str("prompt") {
                Ok(v) => v,
                Err(err) => return err,
            };
            let prompt_args = args.get("arguments").cloned();
            match mcp_manager.get_prompt(&server, &prompt, prompt_args).await {
                Ok(result) => {
                    let text = serde_json::to_string_pretty(&result)
                        .unwrap_or_else(|_| result.to_string());
                    ToolResult {
                        success: true,
                        output: Some(text),
                        error: None,
                        data: Some(result),
                    }
                }
                Err(e) => ToolResult {
                    success: false,
                    output: None,
                    error: Some(e.to_string()),
                    data: None,
                },
            }
        }
        _ => ToolResult {
            success: false,
            output: None,
            error: Some(format!("Unknown MCP management tool: {}", name)),
            data: None,
        },
    }
}

async fn execute_mcp_dynamic_tool(
    tool_registry: &Arc<ToolRegistry>,
    name: &str,
    args: &Value,
) -> ToolResult {
    let suffix = name.strip_prefix("mcp__").unwrap_or(name);
    let mut parts = suffix.splitn(2, "__");
    let server = parts.next().unwrap_or("").trim();
    let tool = parts.next().unwrap_or("").trim();

    if server.is_empty() || tool.is_empty() {
        return ToolResult {
            success: false,
            output: None,
            error: Some(format!("Invalid MCP tool name: {}", name)),
            data: None,
        };
    }

    let config = tool_registry.get_config();
    let Some(mcp_manager) = config.runtime_mcp_manager() else {
        return ToolResult {
            success: false,
            output: None,
            error: Some("MCP not initialized".to_string()),
            data: None,
        };
    };

    let arg_payload = if args.is_null() {
        serde_json::json!({})
    } else {
        args.clone()
    };

    match mcp_manager.call_tool(server, tool, arg_payload).await {
        Ok(value) => ToolResult {
            success: true,
            output: Some(
                serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()),
            ),
            error: None,
            data: Some(value),
        },
        Err(e) => ToolResult {
            success: false,
            output: None,
            error: Some(e.to_string()),
            data: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 静音 panic hook 跑一段代码：被捕获的 panic 不该往测试输出里打噪音。
    fn without_panic_output<T>(f: impl FnOnce() -> T) -> T {
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let out = f();
        std::panic::set_hook(hook);
        out
    }

    #[test]
    fn panic_messages_are_extracted_from_both_payload_shapes() {
        // `panic!("literal")` → &str
        assert_eq!(panic_payload_message(&"boom"), "boom");
        // `panic!("{}", x)` 和 std 自己抛的（比如切片越界）→ String
        assert_eq!(panic_payload_message(&"boom".to_string()), "boom");
        // 其它类型只能给占位，但不能崩
        assert_eq!(panic_payload_message(&42u8), "<non-string panic payload>");
    }

    /// `execute` 用的就是这套组合子。这里验证机制本身：一个会 panic 的 async 块被
    /// 捕获成 `Err(payload)`，而不是把调用方（真实场景里就是 agent worker）带走。
    #[test]
    fn a_panicking_future_is_contained_by_the_same_guard_execute_uses() {
        let caught = without_panic_output(|| {
            futures::executor::block_on(async {
                let boom = async {
                    // 复现真实成因：按字节切一段中文。
                    let content = "上网找的知识点";
                    let _ = &content[..2];
                    "unreachable"
                };
                futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(boom)).await
            })
        });

        let payload = caught.expect_err("byte-slicing CJK should panic");
        let message = panic_payload_message(payload.as_ref());
        assert!(
            message.contains("byte index") || message.contains("char boundary"),
            "unexpected panic message: {}",
            message
        );
    }
}
