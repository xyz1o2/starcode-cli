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
    /// `!command` 跑出来的输出，等下一条用户消息一起发出去
    ///
    /// 不直接 push 成一条独立的 user 消息：Anthropic 要求 user/assistant 交替，
    /// 连续两条 user 会被拒。所以攒在这里，`run_stream` 组装当轮 user 消息时带上。
    pub(crate) pending_local_context: Vec<String>,
    pub(crate) abort_flag: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub(crate) abort_token: Option<tokio_util::sync::CancellationToken>,
    /// 本回合 token 轮询任务的句柄。`CancellationToken::cancel()` 不可撤销，
    /// 所以 token 必须每回合换新；上一回合没撞上中断的轮询任务靠它回收。
    pub(crate) abort_watcher: Option<tokio::task::JoinHandle<()>>,
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
    /// 运行中插入的需求（steering）。
    ///
    /// 用户在 agent 还在跑的时候又提了一条要求，这条要进**当前这一轮**的对话，
    /// 而不是等本轮跑完才发出去 —— 否则 agent 可能已经沿着错误的方向跑完了，
    /// 用户还得再等一整轮才看得到自己的补充被采纳。
    ///
    /// runtime 侧（`runtime/session.rs`）把回合期间收到的 `SendMessage` 收进这个
    /// 队列；这里在每轮 turn 开始时消费，把它们作为 user 消息插进对话。
    pub(crate) steering_queue:
        Option<Arc<crate::agent::messaging::AsyncMessageQueue<(u64, String)>>>,
    /// 运行中切模型的交接槽（见 `agent::model_switch`）。
    ///
    /// 用户在 agent 还在跑的时候切模型：改动落在**下一次请求**上（Claude Code
    /// / OpenCode 的做法），不是等整轮结束。运行时把用户选中的那对
    /// (model, provider) 放进这个槽，回合在每次发请求前取走它。
    ///
    /// 长寿命 agent 与每回合派生的 fresh agent **共用同一个槽** —— 运行时够不到
    /// 任何一个（回合跑起来之后 `StarAgent` 被独占借出），槽就是唯一的通道。
    pub(crate) model_switch_cell: crate::agent::model_switch::ModelSwitchCell,
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
    pub(crate) reactive_compact_manager:
        crate::agent::compact::reactive_compact::ReactiveCompactManager,
    /// 当前逻辑回合冻结的上下文策略。由顶层 `StarAgent` 在开始用户回合前安装。
    pub(crate) context_policy: crate::core::context_policy::ResolvedContextPolicy,
    /// 当前逻辑回合冻结的思考力度。后续 LLM request-options 迁移会消费它。
    pub(crate) thinking_effort: crate::types::ThinkingEffort,
    /// 模型回退管理器：跨 turn 存活，处理 STAR_FALLBACK_MODELS / STAR_FALLBACK_BASE_URLS
    /// 等列表式回退配置（单值 STAR_FALLBACK_MODEL / STAR_FALLBACK_BASE_URL 仍由
    /// agent_llm.rs 优先生效）。
    pub(crate) model_fallback: crate::agent::model_fallback::ModelFallbackManager,
    /// 会话级日期（YYYY-MM-DD），首次 `run_stream` 时固定，之后整场会话不再变。
    ///
    /// 它进 system prompt，而 system 数组整体就是 Anthropic 的缓存前缀。若每轮重取
    /// `Local::now()`，一场跨零点的会话会在 00:00 把整段前缀（约 4 万字符）按原价
    /// 重算一次 —— 为了一行日期赔上整个缓存。空串 = 尚未固定。
    pub(crate) session_today: String,
}

impl Agent {
    pub fn new(client: StarClient, config: Arc<Config>) -> Self {
        crate::utils::logging::append_agent_log_line("[INIT] Agent::new starting...");

        crate::utils::logging::append_agent_log_line("[INIT] build_agent_runtime_artifacts...");
        let runtime =
            crate::core::config::runtime_bootstrap::build_agent_runtime_artifacts(&client, &config);
        crate::utils::logging::append_agent_log_line(
            "[INIT] build_agent_runtime_artifacts completed",
        );

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

        let context_policy = crate::core::context_policy::ResolvedContextPolicy::from_evidence(
            config.configured_context_window_selection(),
            crate::core::context_policy::ContextWindowEvidence::default(),
        );
        let tool_executor_arc = Arc::new(tool_executor);
        let streaming_executor = crate::agent::streaming_executor::StreamingToolExecutor::new(
            tool_executor_arc.clone(),
            4,
        );

        let compact_manager = crate::agent::compact::CompactManager::new(
            crate::agent::compact::CompactConfig::from_context_policy(&context_policy),
        );
        let compact_config = compact_manager.config().clone();

        let mut agent = Self {
            client,
            config,
            tool_executor: tool_executor_arc,
            streaming_executor,
            context_compressor: ContextCompressor::from_context_policy(&context_policy),
            compact_manager,
            context_engine,
            session_messages: Vec::new(),
            pending_local_context: Vec::new(),
            abort_flag: None,
            abort_token: None,
            abort_watcher: None,
            approval_mode: crate::types::ApprovalMode::Default,
            event_tx: None,
            stream_tx: None,
            last_content_streamed: false,
            last_reasoning_streamed: false,
            tool_schema_len_cache: HashMap::new(),
            denial_tracker: crate::core::permission_rules::deny_log::DenialTracker::new(),
            async_notification_queue: None,
            steering_queue: None,
            model_switch_cell: crate::agent::model_switch::new_cell(),
            task_complexity: crate::core::routing::RequestComplexity::Medium,
            memory_manager: crate::core::memory::MemoryManager::new(
                &std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            ),
            context_engine_initialized: false,
            token_budget_tracker: crate::agent::token_budget::TokenBudgetTracker::new(),
            stream_stall_detector: crate::agent::stream_stall::StreamStallDetector::new(),
            reactive_compact_manager:
                crate::agent::compact::reactive_compact::ReactiveCompactManager::new(compact_config),
            context_policy,
            thinking_effort: crate::types::ThinkingEffort::default(),
            model_fallback: crate::agent::model_fallback::ModelFallbackManager::new(),
            session_today: String::new(),
        };
        // 原生会话由启动解析器或 UI→worker RestoreSession 显式安装。
        // 不在构造期间读取旧 sidecar，避免它覆盖用户明确选择的 canonical JSON 会话。
        crate::utils::logging::append_agent_log_line(
            "[INIT] Session context starts empty pending explicit restoration",
        );
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

            // 后台索引协调器：watcher + 低优先级增量重建（免阻塞，见 watcher.rs）。
            // 注入语义引擎缓存后启动，Edit/Write 的脏标记与文件监听统一由此驱动。
            // 注入**进程级共享**的缓存：watcher 把建好的索引写进它，
            // `CodebaseSearchTool` 读的是同一个对象。
            //
            // 以前这里注入的是 `self.context_engine.search_cache` —— 而工具在
            // runtime_bootstrap 里**独立**创建了自己的一份。两边不是同一个
            // 对象，于是 watcher 建好的索引工具永远读不到，每次检索都退回
            // 同步全量重建。
            crate::core::context::watcher::set_engine_cache(
                crate::core::context::search_cache::shared_search_cache(),
            );

            // 装 watcher + 请求首轮重建**必须**离开这条关键路径。
            //
            // `watcher::ensure_started` 里的
            // `notify::Watcher::watch(.., RecursiveMode::Recursive)` 是**同步**的：
            // 它要把整棵目录树逐个登记一遍。而这段代码位于 `lazy_init`，也就是
            // 第一条用户消息在发出第一个 LLM 请求**之前**必须走完的路。
            //
            // 在 WSL 的 `/mnt/d`（Windows 盘）上实测 83.7 秒 —— 也就是用户看到的
            // 「首次特别慢」：首轮白等 80 多秒，而且整个 agent worker 都被占住，
            // Esc 只能排在后面。而 `git status` 的轮询在同一时间还在抢同一块慢
            // I/O（见 `core::services::git_service`）。
            //
            // 放后台没有任何语义损失：重建本来就是后台增量做的，早 10 秒开始
            // 只早 10 秒就绪；真用到索引时（CodebaseSearch）它要么已经建好，
            // 要么如实返回 "index is building"，与同步等待的结果一致。
            let root = cwd.clone();
            let _ = std::thread::Builder::new()
                .name("star-index-watch-setup".into())
                .spawn(move || {
                    crate::core::context::watcher::ensure_started(&root);
                    // 仅启动 watcher 不会触发首轮构建（ensure_started 不插脏标记）。
                    // 冷启动时必须主动请求一次，否则首次 CodebaseSearch 只能拿到
                    // 空引擎并返回 "index is building"，且本轮不会有任何重建发生。
                    crate::core::context::watcher::request_refresh(&root);
                });

            if self.context_engine.has_dynamic_context_candidates(&cwd) {
                crate::utils::logging::append_agent_log_line("[INIT-LAZY] prewarm_index_cache...");
                self.context_engine.prewarm_index_cache();
            }

            if let Some(tool_registry) = self.runtime_tool_registry() {
                let cached_tool =
                    crate::core::tools::codebase_search::CodebaseSearchTool::with_cache(
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
        self.abort_flag = Some(flag);
        self.arm_abort_token();
    }

    /// 挂一个尚未取消的中断 token，供本回合的 LLM 流和工具执行协作取消。
    ///
    /// `CancellationToken::cancel()` 是**永久**的，而 `set_abort_flag` 只在
    /// `StarAgent::new` 调一次。若跨回合复用同一个 token，会话里第一次 ESC
    /// 就会把它永远留在 cancelled 状态：
    /// - `agent_llm.rs` 的流式 select 会在第一个 chunk 之前立刻命中中断分支，
    ///   响应还没开始就被掐断，之后每条消息都回空内容；
    /// - `tool_executor.rs` 的 `is_cancelled()` 预检会让之后所有工具直接失败。
    ///
    /// 所以每个用户回合开始前必须换一个新 token，并顺手回收上一回合没退出的
    /// 轮询任务（否则每个回合攒一个永不结束的 sleep 循环）。
    pub(crate) fn arm_abort_token(&mut self) {
        if let Some(watcher) = self.abort_watcher.take() {
            watcher.abort();
        }
        let Some(flag) = self.abort_flag.clone() else {
            self.abort_token = None;
            return;
        };
        let token = tokio_util::sync::CancellationToken::new();
        let child = token.clone();
        // 只活到本回合：撞上中断就 cancel 并退出，下一回合的 arm 会把还没退出的
        // 旧任务 abort 掉，不会每个回合攒一个常驻 sleep 循环。
        let watcher = tokio::spawn(async move {
            loop {
                if flag.load(std::sync::atomic::Ordering::SeqCst) {
                    child.cancel();
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            }
        });
        self.abort_token = Some(token);
        self.abort_watcher = Some(watcher);
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

    pub(crate) fn emit_tool_finished(
        &self,
        tool_call: &crate::types::StarToolCall,
        result: &crate::types::ToolResult,
    ) {
        self.emit_direct_chunk(StreamingChunk::tool_result(
            tool_call.clone(),
            result.clone(),
        ));
    }

    /// 注入异步通知队列（启用后台 SubAgent 完成通知的回流）
    pub fn set_notification_queue(&mut self, queue: Arc<Mutex<NotificationQueue>>) {
        self.async_notification_queue = Some(queue);
    }

    /// 注入 steering 队列：运行中插入的需求从这里进对话。
    pub fn set_steering_queue(
        &mut self,
        queue: Arc<crate::agent::messaging::AsyncMessageQueue<(u64, String)>>,
    ) {
        self.steering_queue = Some(queue);
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

    /// Move the agent to a new project root after the UI has validated the
    /// path. This runs on the worker thread and updates all process-scoped and
    /// agent-scoped state in one place.
    pub async fn change_working_directory(&mut self, path: &std::path::Path) -> Result<(), String> {
        let actual = crate::core::utils::paths::set_current_dir(path)?;
        crate::agent::hooks::clear_project_root_override();

        // Memory and project context are resolved per project. Reset the
        // lazy-init marker so the next turn rebuilds the watcher, plugin tools
        // and context components against the new root.
        self.memory_manager = crate::core::memory::MemoryManager::new(&actual);
        self.context_engine_initialized = false;
        self.tool_schema_len_cache.clear();
        self.refresh_plugin_tools().await;

        crate::utils::logging::append_agent_log_line(&format!(
            "[CWD] Session moved to {}",
            actual.display()
        ));
        Ok(())
    }

    pub fn model(&self) -> String {
        self.client.model.clone()
    }

    /// 当前 Agent 的嵌套深度（0 = 顶层会话）
    pub fn recursion_depth(&self) -> usize {
        self.config.recursion_depth
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

    /// 用 worker 已解析的会话策略重新配置所有上下文敏感组件。
    /// 此方法只在逻辑用户回合的边界调用，因此不会改变正在进行的工具循环。
    pub fn apply_runtime_snapshot(
        &mut self,
        context_policy: crate::core::context_policy::ResolvedContextPolicy,
        thinking_effort: crate::types::ThinkingEffort,
    ) {
        let compact_config =
            crate::agent::compact::CompactConfig::from_context_policy(&context_policy);
        self.context_compressor = ContextCompressor::from_context_policy(&context_policy);
        self.compact_manager = crate::agent::compact::CompactManager::new(compact_config.clone());
        self.reactive_compact_manager =
            crate::agent::compact::reactive_compact::ReactiveCompactManager::new(compact_config);
        self.context_policy = context_policy;
        self.thinking_effort = thinking_effort;
    }

    pub fn context_policy(&self) -> &crate::core::context_policy::ResolvedContextPolicy {
        &self.context_policy
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
