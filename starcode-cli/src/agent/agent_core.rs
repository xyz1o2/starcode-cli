use crate::agent::messaging::AgentEvent;
use crate::agent::subagent::notification::NotificationQueue;
use crate::agent::tool_executor::ToolExecutor;
use crate::agent::workflows::context_compression::ContextCompressor;
use crate::core::config::Config;
use crate::llm::client::StarClient;
use crate::types::{StarMessage, StreamingChunk};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct Agent {
    pub(crate) client: StarClient,
    pub(crate) config: Arc<Config>,
    pub(crate) tool_executor: Arc<ToolExecutor>,
    pub(crate) streaming_executor: crate::agent::streaming_executor::StreamingToolExecutor,
    pub(crate) context_compressor: ContextCompressor,
    pub(crate) compact_manager: crate::agent::compact::CompactManager,
    pub(crate) context_engine: crate::core::context::engine::ContextEngine,
    pub(crate) session_messages: Vec<StarMessage>,
    pub(crate) abort_flag: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub(crate) abort_token: Option<tokio_util::sync::CancellationToken>,
    pub(crate) approval_mode: crate::types::ApprovalMode,
    /// Optional event sender — when set, streaming events (TextDelta, ReasoningDelta,
    /// ToolStarted, ToolFinished) are sent through this channel during the agent loop.
    pub(crate) event_tx: Option<tokio::sync::mpsc::UnboundedSender<AgentEvent>>,
    /// Direct-to-UI streaming sender — bypasses the AgentEvent→StreamingChunk
    /// conversion. Used for real-time thinking/content deltas during the LLM stream
    /// so the UI updates without waiting for the agent loop to finish.
    pub(crate) stream_tx: Option<tokio::sync::mpsc::UnboundedSender<StreamingChunk>>,
    /// Whether content was streamed in real-time during the last call_llm().
    /// Set in agent_loop.rs; checked in agent_run.rs to prevent duplicate emission.
    pub(crate) last_content_streamed: bool,
    /// Whether reasoning was streamed in real-time during the last call_llm().
    pub(crate) last_reasoning_streamed: bool,
    /// Lazy cache for tool schema JSON lengths (tool_name → bytes),
    /// avoiding repeated serde_json::to_string per turn for token estimation.
    pub(crate) tool_schema_len_cache: HashMap<String, usize>,
    /// Session-scoped denial tracker: detects same-tool consecutive denials
    /// and injects system nudge messages to break infinite retry loops.
    /// Mirrors Claude Code's denialTracking.ts (≥3 consecutive → auto-inject).
    pub(crate) denial_tracker: crate::core::permission_rules::deny_log::DenialTracker,
    /// 异步 SubAgent 通知队列：每轮 turn 开始前消费后台 agent 的完成通知。
    pub(crate) async_notification_queue: Option<Arc<Mutex<NotificationQueue>>>,
    /// Current task complexity level (Simple/Medium/Complex) set by the router.
    /// Used to dynamically adjust thinking limits and other parameters.
    pub(crate) task_complexity: crate::core::routing::RequestComplexity,
    /// Memory manager for persisting project and user knowledge
    pub(crate) memory_manager: crate::core::memory::MemoryManager,
    /// ContextEngine 延迟初始化标记
    pub(crate) context_engine_initialized: bool,
    /// Token预算跟踪器
    pub(crate) token_budget_tracker: crate::agent::token_budget::TokenBudgetTracker,
    /// 流式停滞检测器
    pub(crate) stream_stall_detector: crate::agent::stream_stall::StreamStallDetector,
    /// Reactive Compact管理器
    pub(crate) reactive_compact_manager: crate::agent::compact::reactive_compact::ReactiveCompactManager,
}

impl Agent {
    pub fn new(client: StarClient, config: Arc<Config>) -> Self {
        crate::utils::logging::append_agent_log_line("[INIT] Agent::new starting...");
        
        crate::utils::logging::append_agent_log_line("[INIT] build_agent_runtime_artifacts...");
        let runtime =
            crate::core::config::runtime_bootstrap::build_agent_runtime_artifacts(&client, &config);
        crate::utils::logging::append_agent_log_line("[INIT] build_agent_runtime_artifacts completed");

        let tool_registry = runtime.tool_registry.clone();

        crate::utils::logging::append_agent_log_line("[INIT] ToolExecutor::new...");
        let tool_executor = ToolExecutor::new(
            client.clone(),
            runtime.tool_registry,
            Some(runtime.message_bus),
            config.clone(),
        );
        crate::utils::logging::append_agent_log_line("[INIT] ToolExecutor::new completed");

        // ContextEngine: 延迟初始化，不在构造函数中做任何 I/O
        let context_engine = crate::core::context::engine::ContextEngine::new();

        let context_window = config.context_window();
        let tool_executor_arc = Arc::new(tool_executor);
        let streaming_executor = crate::agent::streaming_executor::StreamingToolExecutor::new(tool_executor_arc.clone(), 4);

        let compact_manager = crate::agent::compact::CompactManager::from_env();
        let compact_config = compact_manager.config().clone();
        
        let mut agent = Self {
            client,
            config,
            tool_executor: tool_executor_arc,
            streaming_executor,
            context_compressor: ContextCompressor::new(Some(context_window)),
            compact_manager,
            context_engine,
            session_messages: Vec::new(),
            abort_flag: None,
            abort_token: None,
            approval_mode: crate::types::ApprovalMode::Default,
            event_tx: None,
            stream_tx: None,
            last_content_streamed: false,
            last_reasoning_streamed: false,
            tool_schema_len_cache: HashMap::new(),
            denial_tracker: crate::core::permission_rules::deny_log::DenialTracker::new(),
            async_notification_queue: None,
            task_complexity: crate::core::routing::RequestComplexity::Medium,
            memory_manager: crate::core::memory::MemoryManager::new(
                &std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            ),
            context_engine_initialized: false,
            token_budget_tracker: crate::agent::token_budget::TokenBudgetTracker::new(),
            stream_stall_detector: crate::agent::stream_stall::StreamStallDetector::new(),
            reactive_compact_manager: crate::agent::compact::reactive_compact::ReactiveCompactManager::new(
                compact_config
            ),
        };
        agent.load_persisted_session_messages();
        crate::utils::logging::append_agent_log_line("[INIT] Agent::new completed");

        // 连接后台 SubAgent 通知队列（若 runtime 已初始化）
        if let Some(queue) = agent.config.runtime_notification_queue() {
            agent.set_notification_queue(queue);
        }

        agent
    }

    /// 延迟初始化：在首次实际使用时调用，不在构造函数中阻塞
    pub async fn lazy_init(&mut self) {
        if self.context_engine_initialized {
            return;
        }
        self.context_engine_initialized = true;

        crate::utils::logging::append_agent_log_line("[INIT-LAZY] starting lazy initialization...");

        // 1. refresh_plugin_tools（之前在 StarAgent::new 中同步调用）
        crate::utils::logging::append_agent_log_line("[INIT-LAZY] refresh_plugin_tools...");
        self.refresh_plugin_tools().await;

        // 2. ContextEngine 初始化
        if let Ok(cwd) = std::env::current_dir() {
            crate::utils::logging::append_agent_log_line("[INIT-LAZY] init_project_components...");
            self.context_engine.init_project_components(&cwd);
            
            if self.context_engine.has_dynamic_context_candidates(&cwd) {
                crate::utils::logging::append_agent_log_line("[INIT-LAZY] prewarm_index_cache...");
                self.context_engine.prewarm_index_cache();
            }

            if let Some(tool_registry) = self.runtime_tool_registry() {
                let cached_tool = crate::core::tools::semantic_search::SemanticSearchTool::with_cache(
                    self.config.clone(),
                    self.context_engine.search_cache.clone(),
                );
                tool_registry.register_tool(Arc::new(cached_tool));
            }

            crate::core::tools::project_map::spawn_project_map_prewarm(cwd);
        }

        crate::utils::logging::append_agent_log_line("[INIT-LAZY] completed");
    }

    pub fn set_abort_flag(&mut self, flag: Arc<std::sync::atomic::AtomicBool>) {
        let token = tokio_util::sync::CancellationToken::new();
        let child = token.clone();
        let f = flag.clone();
        tokio::spawn(async move {
            loop {
                if f.load(std::sync::atomic::Ordering::SeqCst) {
                    child.cancel();
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        });
        self.abort_token = Some(token);
        self.abort_flag = Some(flag);
    }

    /// Emit an event through the event channel if configured.
    /// Silently ignores send errors (channel may be closed / UI disconnected).
    pub(crate) fn emit_event(&self, event: AgentEvent) {
        if let Some(ref tx) = self.event_tx {
            let _ = tx.send(event);
        }
    }

    /// Emit a StreamingChunk directly to the UI bypassing the AgentEvent
    /// stream conversion. Used during the LLM stream for real-time
    /// thinking / content updates.
    pub(crate) fn emit_direct_chunk(&self, chunk: StreamingChunk) {
        if let Some(ref tx) = self.stream_tx {
            let _ = tx.send(chunk);
        }
    }

    /// 直接通过 stream_tx 发送工具事件，实现实时显示
    /// 绕过 event_tx 的缓冲机制
    pub(crate) fn emit_tool_started(&self, tool_call: &crate::types::StarToolCall) {
        self.emit_direct_chunk(StreamingChunk::tool_calls(vec![tool_call.clone()]));
    }

    pub(crate) fn emit_tool_finished(&self, tool_call: &crate::types::StarToolCall, result: &crate::types::ToolResult) {
        self.emit_direct_chunk(StreamingChunk::tool_result(tool_call.clone(), result.clone()));
    }

    /// 注入异步通知队列（启用后台 SubAgent 完成通知的回流）
    pub fn set_notification_queue(&mut self, queue: Arc<Mutex<NotificationQueue>>) {
        self.async_notification_queue = Some(queue);
    }

    pub fn set_approval_mode(&mut self, mode: crate::types::ApprovalMode) {
        crate::utils::logging::append_debug_log_line(&format!(
            "[AGENT] Approval mode changed: {:?}",
            mode
        ));
        self.approval_mode = mode;
    }

    pub fn get_approval_mode(&self) -> crate::types::ApprovalMode {
        self.approval_mode.clone()
    }

    pub async fn refresh_mcp_tools(&self) {
        let Some(mcp_manager) = self.runtime_mcp_manager() else {
            return;
        };
        let Some(tool_registry) = self.runtime_tool_registry() else {
            return;
        };

        let servers = mcp_manager.list_server_names().await;
        for server_name in servers {
            match mcp_manager.list_tools(&server_name).await {
                Ok(tools) => {
                    for tool in tools {
                        let registered_name =
                            format!("mcp__{}__{}", server_name, tool.name.as_str());
                        tool_registry.register_tool(Arc::new(
                            crate::tools::mcp_tool::McpToolWrapper::new(
                                mcp_manager.clone(),
                                server_name.clone(),
                                tool,
                                registered_name,
                            ),
                        ));
                    }
                }
                Err(e) => {
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[MCP] Failed to load tools for server {}: {}",
                        server_name, e
                    ));
                }
            }
        }
    }

    pub async fn refresh_plugin_tools(&self) {
        let Some(tool_registry) = self.runtime_tool_registry() else {
            return;
        };

        match crate::core::plugins::discover_plugin_tools(self.config.target_dir()).await {
            Ok(resolved_tools) => {
                let declarative_tools =
                    crate::core::plugins::build_plugin_declarative_tools(resolved_tools);
                let skipped = tool_registry.sync_plugin_tools(declarative_tools);
                crate::utils::logging::append_debug_log_line(&format!(
                    "[PluginTools] Synced plugin tools (skipped={})",
                    skipped.len()
                ));
            }
            Err(error) => {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[PluginTools] Failed to refresh plugin tools: {}",
                    error
                ));
            }
        }
    }

    pub fn model(&self) -> String {
        self.client.model.clone()
    }

    pub(crate) fn runtime_tool_registry(&self) -> Option<Arc<crate::core::tools::ToolRegistry>> {
        self.config.runtime_tool_registry()
    }

    pub(crate) fn runtime_message_bus(
        &self,
    ) -> Option<Arc<crate::core::confirmation_bus::MessageBus>> {
        self.config.runtime_message_bus()
    }

    pub(crate) fn runtime_mcp_manager(&self) -> Option<Arc<crate::core::mcp::MCPManager>> {
        self.config.runtime_mcp_manager()
    }

    pub(crate) fn runtime_global_state(&self) -> Option<Arc<crate::core::state::GlobalState>> {
        self.config.runtime_global_state()
    }

    pub fn set_model(&mut self, model: &str) {
        self.client.set_model(model);
    }

    pub fn switch_provider(
        &mut self,
        model: &str,
        base_url: &str,
        api_key: &str,
        is_openai_compatible: Option<bool>,
        provider_id: Option<String>,
    ) {
        self.client
            .switch_provider(model, base_url, api_key, is_openai_compatible, provider_id);
    }

    pub fn get_client(&self) -> StarClient {
        self.client.clone()
    }

    /// 获取压缩管理器的引用
    pub fn compact_manager(&self) -> &crate::agent::compact::CompactManager {
        &self.compact_manager
    }

    pub async fn execute_tool_calls(
        &mut self,
        tool_calls: Vec<crate::types::StarToolCall>,
    ) -> Vec<crate::types::ToolResult> {
        if tool_calls.len() > 1 {
            self.streaming_executor
                .execute_partitioned(tool_calls, None)
                .await
        } else {
            self.tool_executor
                .execute_batch(tool_calls, None, None)
                .await
        }
    }

    pub fn is_yolo_mode_disabled(&self) -> bool {
        self.config.is_yolo_mode_disabled()
    }
}
