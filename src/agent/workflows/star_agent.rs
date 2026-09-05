use crate::agent::messaging::queue::AsyncMessageQueue;
use crate::agent::messaging::AgentEvent;
use crate::agent::Agent;
use crate::core::config::Config;
use crate::core::state::{FileSnapshot, ReadFileState};
use crate::llm::client::StarClient;
use crate::types::ApprovalMode;
use crate::types::StarToolCall;
use crate::types::{ChatEntry, StarUsage, StreamingChunk, StreamingChunkType, ToolResult};
use futures::Stream;
use sha2::{Digest, Sha256};
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use tokio::sync::Notify;

pub struct StarAgent {
    inner: Agent,
    abort_flag: Arc<AtomicBool>,
    client: StarClient,
    approval_mode_lock: Arc<Mutex<ApprovalMode>>,
    fallback_mcp_manager: Option<Arc<crate::core::mcp::MCPManager>>,
    is_mcp_initialized: Arc<AtomicBool>,
    steering_queue: Option<Arc<AsyncMessageQueue<(u64, String)>>>,
    steering_signal: Option<Arc<Notify>>,
}

impl StarAgent {
    fn initial_approval_mode(config: &Config) -> ApprovalMode {
        match config.approval_mode() {
            crate::core::policy::ApprovalMode::Default => ApprovalMode::Default,
            crate::core::policy::ApprovalMode::Plan => ApprovalMode::Plan,
            crate::core::policy::ApprovalMode::Yolo => ApprovalMode::Yolo,
        }
    }

    const MAX_RECURSION_DEPTH: usize = 3;

    pub async fn new(
        api_key: &str,
        model: Option<String>,
        base_url: impl Into<Option<String>>,
        max_subagent_rounds: Option<u32>,
        is_openai_compatible: Option<bool>,
        config: Option<Arc<Config>>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let config = config.ok_or_else(|| "Config not initialized".to_string())?;

        if config.recursion_depth >= Self::MAX_RECURSION_DEPTH {
            return Err(format!(
                "Maximum recursion depth reached ({}). Cannot create nested agent.",
                Self::MAX_RECURSION_DEPTH
            )
            .into());
        }

        // AgentTool 的 max_rounds 参数在此生效：覆盖该 agent 的循环轮次上限。
        // 之前这个参数被忽略，导致子 Agent 永远跑主会话的 max_session_turns。
        let config = match max_subagent_rounds {
            Some(rounds) if rounds > 0 => {
                let mut cloned = (*config).clone();
                cloned.set_max_session_turns(rounds as i32);
                Arc::new(cloned)
            }
            _ => config,
        };

        let client = StarClient::new(api_key, model, base_url.into(), is_openai_compatible, None);
        let initial_mode = Self::initial_approval_mode(config.as_ref());
        let abort_flag = Arc::new(AtomicBool::new(false));

        crate::utils::logging::append_agent_log_line("[INIT] Creating Agent::new...");
        let mut inner = Agent::new(client.clone(), config);
        crate::utils::logging::append_agent_log_line("[INIT] Agent::new completed");

        inner.set_abort_flag(abort_flag.clone());
        inner.set_approval_mode(initial_mode.clone());

        // refresh_plugin_tools 延迟到首次使用时调用（在 lazy_init 中）
        crate::utils::logging::append_agent_log_line(
            "[INIT] StarAgent::new completed (plugin tools deferred)",
        );

        Ok(Self {
            inner,
            abort_flag,
            client,
            approval_mode_lock: Arc::new(Mutex::new(initial_mode.clone())),
            fallback_mcp_manager: None,
            is_mcp_initialized: Arc::new(AtomicBool::new(false)),
            steering_queue: None,
            steering_signal: None,
        })
    }

    pub async fn set_model(&mut self, model: &str) {
        self.inner.set_model(model);
        self.client = self.inner.get_client();
    }

    pub async fn set_model_with_provider(&mut self, model: &str, provider_id: Option<&str>) {
        if let Some(pid) = provider_id {
            let store = crate::core::config::provider_store::ProviderStore::new();
            let config = store.load().await.unwrap_or_default();

            // Try exact match first, then case-insensitive fallback
            let settings = config.providers.get(pid).or_else(|| {
                config.providers.iter().find_map(|(k, v)| {
                    if k.eq_ignore_ascii_case(pid) {
                        Some(v)
                    } else {
                        None
                    }
                })
            });
            let (base_url, api_key) = if let Some(settings) = settings {
                // Use the provider's configured base_url, falling back to the
                // current client's URL rather than a hardcoded default. This
                // prevents OpenAI's URL from being injected for custom providers.
                let url = settings
                    .base_url
                    .clone()
                    .or_else(|| {
                        crate::core::config::providers::get_provider_by_id(pid)
                            .and_then(|m| m.default_base_url)
                            .map(|s| s.to_string())
                    })
                    .unwrap_or_else(|| self.client.base_url.clone());
                let key = crate::core::config::providers::resolve_runtime_api_key(
                    Some(pid),
                    settings.api_key.clone(),
                )
                .unwrap_or_else(|| self.client.api_key.clone());
                (url, key)
            } else if let Some(meta) = crate::core::config::providers::get_provider_by_id(pid) {
                let url = meta
                    .default_base_url
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| self.client.base_url.clone());
                let key = crate::core::config::providers::resolve_runtime_api_key(Some(pid), None)
                    .unwrap_or_else(|| self.client.api_key.clone());
                (url, key)
            } else {
                self.inner.set_model(model);
                return;
            };

            crate::utils::logging::append_debug_log_line(&format!(
                "[SetModel] Switching to model={}, provider={}, base_url={}",
                model, pid, base_url
            ));

            let is_openai_compatible =
                crate::core::config::providers::provider_openai_compatible_mode(pid);
            self.inner.switch_provider(
                model,
                &base_url,
                &api_key,
                is_openai_compatible,
                Some(pid.to_string()),
            );
            self.client = self.inner.get_client();
        } else {
            self.inner.set_model(model);
        }
    }

    pub async fn process_user_message(
        &mut self,
        prompt: &str,
    ) -> Result<Vec<ChatEntry>, Box<dyn std::error::Error + Send + Sync>> {
        let text = self.inner.run(prompt).await?;
        Ok(vec![ChatEntry::assistant(text)])
    }

    pub async fn execute_tool(
        &mut self,
        tool_call: &StarToolCall,
    ) -> Result<ToolResult, Box<dyn std::error::Error + Send + Sync>> {
        let results = self.inner.execute_tool_calls(vec![tool_call.clone()]).await;
        Ok(results.into_iter().next().unwrap_or(ToolResult {
            success: false,
            output: None,
            error: Some("tool executor returned no result".to_string()),
            data: None,
        }))
    }

    pub fn append_tool_result_message(
        &mut self,
        tool_call: &StarToolCall,
        tool_result: &ToolResult,
    ) {
        self.inner
            .append_external_tool_result(tool_call, tool_result);
    }

    pub async fn compress_context(
        &mut self,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let result = self.inner.force_compress_session_messages().await?;
        if result.messages.is_empty() {
            return Ok("Session context is empty; no compression needed.".to_string());
        }
        if !result.was_compacted {
            return Ok(format!(
                "No compression needed: current context is about {} tokens.",
                result.new_token_count
            ));
        }
        Ok(format!(
            "Context compressed: tokens {} -> {}, message count {}.",
            result.original_token_count,
            result.new_token_count,
            result.messages.len()
        ))
    }

    /// /summary、/recap、/btw：对当前会话消息做一次旁路 LLM 生成（对标 Claude Code
    /// 手动 Session Memory 提取与 away-summary）。
    /// 不修改 session_messages —— 结果仅回显给用户。
    pub async fn generate_note(
        &mut self,
        kind: crate::runtime::messages::NoteKind,
        question: Option<String>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        use crate::runtime::messages::NoteKind;

        let messages = &self.inner.session_messages;
        // 只统计有实际内容的 user/assistant 消息
        let has_content = messages.iter().any(|m| {
            (m.role == "user" || m.role == "assistant")
                && m.content
                    .as_deref()
                    .map(|c| !c.trim().is_empty())
                    .unwrap_or(false)
        });
        // /btw 是独立提问，空会话也应该能回答
        if !has_content && kind != NoteKind::Aside {
            return Err("Nothing to summarize yet — send a message first.".into());
        }

        // 压缩 transcript：跳过空内容与超长 tool 输出
        let mut transcript = String::new();
        for msg in messages {
            let role = msg.role.as_str();
            if role != "user" && role != "assistant" && role != "tool" {
                continue;
            }
            let content = msg.content.as_deref().unwrap_or("[tool calls]").trim();
            if content.is_empty() {
                continue;
            }
            let safe_end = content
                .char_indices()
                .nth(2000)
                .map(|(i, _)| i)
                .unwrap_or(content.len());
            let body = if safe_end < content.len() {
                format!("{}…[truncated]", &content[..safe_end])
            } else {
                content.to_string()
            };
            transcript.push_str(&format!("{}: {}\n", role, body));
        }

        let prompt = match kind {
            NoteKind::Summary => format!(
                "Summarize the conversation below for the user.\n\
                 Cover: (1) the user's goal, (2) key decisions made, (3) work completed \
                 (files changed, commands run), (4) current state and any pending next steps.\n\
                 Be factual and concise. Use short markdown sections or bullets. \
                 Write in the same language as the conversation.\n\n\
                 ## Conversation:\n{}",
                transcript
            ),
            NoteKind::Recap => format!(
                "Produce a ONE-sentence recap (max 40 words, plain text, no markdown) \
                 of the conversation below: what the user is trying to accomplish, \
                 what just happened, and what happens next. \
                 Write in the same language as the conversation.\n\n\
                 ## Conversation:\n{}",
                transcript
            ),
            NoteKind::Aside => {
                let q = question.unwrap_or_default();
                if transcript.trim().is_empty() {
                    format!(
                        "Answer the following question directly and concisely. \
                         Write in the same language as the question.\n\n\
                         ## Question:\n{}",
                        q
                    )
                } else {
                    format!(
                        "Answer the user's side question directly and concisely. It is a detour \
                         from the ongoing work — do not restart or continue that work, and do not \
                         propose changes to it unless the question asks for them. The conversation \
                         is background only; use it just to resolve references in the question. \
                         Write in the same language as the question.\n\n\
                         ## Question:\n{}\n\n## Conversation (background):\n{}",
                        q, transcript
                    )
                }
            }
        };

        self.client.chat_completion_simple(&prompt).await
    }

    pub async fn list_checkpoints(
        &mut self,
    ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        // New file-history API: return snapshot ids (newest last).
        // Legacy list_checkpoints() walked tmp/checkpoints/*.json which was
        // never populated (create_checkpoint had 0 callers).
        Ok(crate::utils::checkpoint_manager::list_snapshots(None)
            .await?
            .into_iter()
            .map(|s| s.snapshot_id)
            .collect())
    }

    pub async fn restore_checkpoint(
        &mut self,
        id: &str,
    ) -> Result<(Vec<ChatEntry>, String), Box<dyn std::error::Error + Send + Sync>> {
        // New file-history API: rewind to snapshot by id.
        // The new API is file-only (does not restore chat history), so we
        // return an empty chat_history to signal "no chat restore" — callers
        // that relied on chat_history restore (e.g. /undo via runtime channel)
        // will see an empty history and not overwrite the current one.
        let changed = crate::utils::checkpoint_manager::rewind(id, None).await?;
        let summary = if changed.is_empty() {
            format!(
                "Snapshot `{}` restored. No files changed (already at this state).",
                id
            )
        } else {
            format!(
                "Snapshot `{}` restored. Restored {} file(s):\n{}",
                id,
                changed.len(),
                changed.join("\n")
            )
        };
        Ok((Vec::new(), summary))
    }

    pub async fn list_models(&self) -> Result<Vec<crate::types::ModelInfo>, String> {
        // Use the full model list implementation that fetches from API
        crate::agent::model_list::list_models(&self.client).await
    }

    /// 带来源信息的模型列表。`force = true` 时跳过缓存并扇出到所有已配置
    /// provider（面板里的显式刷新）；否则走缓存优先的便宜路径。
    pub async fn list_models_cached(
        &self,
        force: bool,
    ) -> Result<crate::agent::model_list::ModelListResult, String> {
        crate::agent::model_list::list_models_with_mode(&self.client, force).await
    }

    /// Expose the runtime GlobalState so external callers (e.g. the streaming
    /// session worker) can set `current_message_id` for file-history
    /// checkpoint association before tool execution begins.
    pub fn runtime_global_state(&self) -> Option<Arc<crate::core::state::GlobalState>> {
        self.inner.runtime_global_state()
    }

    pub fn abort(&self) {
        crate::agent::abort::abort(&self.abort_flag);
    }

    pub fn abort_handle(&self) -> Arc<AtomicBool> {
        self.abort_flag.clone()
    }

    pub fn reset_abort(&self) {
        crate::agent::abort::reset_abort(&self.abort_flag);
    }

    pub fn clear_session_context(&mut self) {
        self.inner.clear_session_messages();
        self.abort_flag.store(false, Ordering::SeqCst);
    }

    /// 把 `!command` 的输出攒进上下文，等下一条用户消息一起发出去
    ///
    /// 给 `!command` 用：命令输出得进上下文，模型下一轮才看得见；
    /// 又不该因为跑了个 `git status` 就触发一次回答。
    pub fn append_session_context(&mut self, content: String) {
        self.inner.pending_local_context.push(content);
    }

    pub async fn process_user_message_stream(
        &mut self,
        prompt: &str,
    ) -> Result<
        Pin<
            Box<
                dyn Stream<Item = Result<StreamingChunk, Box<dyn std::error::Error + Send + Sync>>>
                    + Send
                    + '_,
            >,
        >,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        // Create a direct-to-UI channel so that thinking / text deltas
        // emitted during the LLM stream reach the UI in real-time, without
        // waiting for the agent loop to finish.
        let (stream_tx, stream_rx) = tokio::sync::mpsc::unbounded_channel::<StreamingChunk>();
        self.inner.stream_tx = Some(stream_tx.clone());

        // 把顶层会话的 chunk 发送端登记为全局 UI sink，供 AgentTool 在
        // 同步/后台路径中推送子 Agent 进度（对标参考实现的 AsyncLocalStorage
        // 透传 UI 回调）。只有 depth==0 才注册，否则子 Agent 会覆盖父级 sink。
        if self.inner.recursion_depth() == 0 {
            crate::agent::subagent::progress::set_ui_sink(stream_tx);
        }

        let event_stream = self.inner.run_stream(prompt.to_string());

        // Branch A: AgentEvent → StreamingChunk (tool calls, final messages, etc.)
        // Branch B is always active (we just set stream_tx above), so skip
        // TextDelta and ReasoningDelta here because Branch B already sent them
        // in real-time via emit_direct_chunk(). This prevents duplicate text.
        let event_chunks = Box::pin(async_stream::try_stream! {
            use futures::StreamExt;
            let mut stream = event_stream;
            let mut streamed_text_buf = String::new();

            while let Some(event_result) = stream.next().await {
                let event = event_result?;
                let chunk = match event {
                    AgentEvent::TextDelta(content) => {
                        streamed_text_buf.push_str(&content);
                        // Branch B already sent this via emit_direct_chunk — skip
                        None
                    }
                    AgentEvent::ReasoningDelta(_content) => {
                        // Branch B already sent this via emit_direct_chunk — skip
                        None
                    }
                    AgentEvent::Trace { event, payload } => {
                        Some(StreamingChunk::trace_event(event, payload))
                    }
                    AgentEvent::Message(content) => {
                        let is_duplicate_final =
                            !streamed_text_buf.is_empty() && streamed_text_buf == content;
                        streamed_text_buf.clear();
                        if is_duplicate_final {
                            None
                        } else {
                            Some(StreamingChunk::content(content))
                        }
                    }
                    AgentEvent::ToolStarted { tool_call } => {
                        streamed_text_buf.clear();
                        Some(StreamingChunk::tool_calls(vec![tool_call]))
                    }
                    AgentEvent::ToolFinished { tool_call, result } => {
                        Some(StreamingChunk {
                            chunk_type: StreamingChunkType::ToolResult,
                            tool_call: Some(tool_call),
                            tool_result: Some(result),
                            ..Default::default()
                        })
                    }
                    AgentEvent::ToolProgress {
                        tool_name,
                        tool_call_id,
                        status,
                        message,
                        current,
                        total,
                    } => Some(StreamingChunk::tool_progress(crate::types::ToolProgress {
                        tool_name,
                        tool_call_id,
                        status,
                        message,
                        current,
                        total,
                    })),
                    AgentEvent::Error(err) => {
                        // 走 Error 通道而不是伪装成一段助手文本 —— 只有这一路
                        // 才会触发 UI 的错误分类、清理 processing 状态、
                        // 并按可重试性决定要不要开重试浮层。
                        streamed_text_buf.clear();
                        Some(StreamingChunk::error(err))
                    }
                    AgentEvent::TurnFinished | AgentEvent::Done => {
                        streamed_text_buf.clear();
                        Some(StreamingChunk::done())
                    }
                    AgentEvent::StatsUpdate { token_usage } => {
                        let tokens = token_usage.as_ref().map(|u: &StarUsage| u.total_tokens as u32).unwrap_or(0);
                        Some(StreamingChunk {
                            chunk_type: StreamingChunkType::TokenCount,
                            token_count: Some(tokens),
                            token_usage,
                            ..Default::default()
                        })
                    }
                };
                if let Some(chunk) = chunk {
                    yield chunk;
                }
            }
        });

        // Branch B: real-time thinking / text deltas emitted directly from
        // call_llm() via emit_direct_chunk().  These bypass the AgentEvent
        // stream so they reach the UI concurrently with the agent loop.
        use tokio_stream::wrappers::UnboundedReceiverStream;
        let direct_stream = tokio_stream::StreamExt::map(
            UnboundedReceiverStream::new(stream_rx),
            |c: StreamingChunk| -> Result<StreamingChunk, Box<dyn std::error::Error + Send + Sync>> { Ok(c) },
        );

        // Merge: whichever branch produces first gets yielded next.
        // Both are Pin<Box<...>> (Unpin) so select() is happy.
        Ok(Box::pin(futures::stream::select(
            event_chunks,
            Box::pin(direct_stream),
        )))
    }

    pub async fn initialize_mcp(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(manager) = self.inner.runtime_mcp_manager() {
            manager.initialize_mcp_servers().await?;
            let errs = manager.discover_all().await;
            self.inner.refresh_mcp_tools().await;
            self.is_mcp_initialized.store(true, Ordering::SeqCst);
            if errs.is_empty() {
                return Ok(());
            }
            let details = errs
                .into_iter()
                .map(|(server, err)| format!("{}: {}", server, err))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(format!("MCP discover failed: {}", details).into());
        }

        if self.fallback_mcp_manager.is_none() {
            let manager = Arc::new(crate::core::mcp::MCPManager::new());
            manager.initialize_mcp_servers().await?;
            self.fallback_mcp_manager = Some(manager);
        }

        if let Some(manager) = &self.fallback_mcp_manager {
            let errs = manager.discover_all().await;
            self.is_mcp_initialized.store(true, Ordering::SeqCst);
            self.inner.refresh_mcp_tools().await;
            if errs.is_empty() {
                Ok(())
            } else {
                let details = errs
                    .into_iter()
                    .map(|(server, err)| format!("{}: {}", server, err))
                    .collect::<Vec<_>>()
                    .join("; ");
                Err(format!("MCP discover failed: {}", details).into())
            }
        } else {
            Err("MCP initialization failed".into())
        }
    }

    pub fn is_mcp_ready(&self) -> bool {
        self.is_mcp_initialized.load(Ordering::SeqCst)
    }

    pub async fn mcp_list_servers(&self) -> Vec<String> {
        if let Some(manager) = self.inner.runtime_mcp_manager() {
            return manager.list_server_names().await;
        }
        if let Some(manager) = &self.fallback_mcp_manager {
            return manager.list_server_names().await;
        }
        Vec::new()
    }

    pub async fn mcp_list_tools(
        &self,
        server: &str,
    ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        if let Some(manager) = self.inner.runtime_mcp_manager() {
            let tools = manager.list_tools(server).await?;
            return Ok(tools.into_iter().map(|t| t.name).collect());
        }
        if let Some(manager) = &self.fallback_mcp_manager {
            let tools = manager.list_tools(server).await?;
            return Ok(tools.into_iter().map(|t| t.name).collect());
        }
        Err("MCP not initialized".into())
    }

    pub fn update_provider_config(
        &mut self,
        provider_id: Option<String>,
        api_key: Option<String>,
        base_url: Option<String>,
        is_openai_compatible: Option<bool>,
        model: Option<String>,
    ) {
        let next_api_key = match api_key {
            Some(k) => k,
            None => self.client.api_key.clone(),
        };
        let next_base_url = base_url.unwrap_or_else(|| self.client.base_url.clone());
        let next_is_openai_compatible =
            is_openai_compatible.unwrap_or(self.client.is_openai_compatible);
        let next_model = model.unwrap_or_else(|| self.inner.model());

        // Keep the outer client and the agent's inner client in sync. Chat requests
        // go through `inner`, so updating only `self.client` causes stale API key errors.
        self.inner.switch_provider(
            &next_model,
            &next_base_url,
            &next_api_key,
            Some(next_is_openai_compatible),
            provider_id,
        );
        self.client = self.inner.get_client();
    }

    pub async fn mark_file_as_read(&mut self, path: &str) {
        let Some(global_state) = self.inner.runtime_global_state() else {
            return;
        };

        let resolved = match std::fs::canonicalize(path) {
            Ok(p) => p,
            Err(_) => {
                let p = PathBuf::from(path);
                if p.is_absolute() {
                    p
                } else {
                    match std::env::current_dir() {
                        Ok(cwd) => cwd.join(p),
                        Err(_) => p,
                    }
                }
            }
        };
        let abs_path = resolved.to_string_lossy().to_string();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let fs_ts = std::fs::metadata(&resolved)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis())
            .unwrap_or(now_ms);

        let content = tokio::fs::read_to_string(&resolved)
            .await
            .unwrap_or_default();
        let hash = {
            let mut hasher = Sha256::new();
            hasher.update(content.as_bytes());
            format!("{:x}", hasher.finalize())
        };

        {
            let mut read_state = global_state.read_file_state.write().await;
            read_state.insert(
                abs_path.clone(),
                ReadFileState {
                    content: content.clone(),
                    timestamp: now_ms,
                    file_system_timestamp: fs_ts,
                },
            );
        }

        {
            let mut exec_state = global_state.execution_state.write().await;
            exec_state.mark_file_read(
                abs_path,
                FileSnapshot {
                    content,
                    timestamp: now_ms,
                    hash,
                },
            );
        }
    }

    pub fn toggle_yolo_mode(&mut self) -> crate::types::ApprovalMode {
        if self.inner.is_yolo_mode_disabled() {
            {
                let mut mode = self.approval_mode_lock.lock().unwrap();
                *mode = ApprovalMode::Default;
            }
            self.inner.set_approval_mode(ApprovalMode::Default);
            if let Some(bus) = self.inner.runtime_message_bus() {
                crate::agent::approval::set_approval_mode(
                    ApprovalMode::Default,
                    &self.approval_mode_lock,
                    &bus,
                );
            }
            return ApprovalMode::Default;
        }

        if let Some(bus) = self.inner.runtime_message_bus() {
            let mode = crate::agent::approval::toggle_yolo_mode(&self.approval_mode_lock, &bus);
            self.inner.set_approval_mode(mode.clone());
            return mode;
        }

        let mut mode = self.approval_mode_lock.lock().unwrap();
        let new_mode = if *mode == ApprovalMode::Yolo {
            ApprovalMode::Default
        } else {
            ApprovalMode::Yolo
        };
        *mode = new_mode.clone();
        self.inner.set_approval_mode(new_mode.clone());
        new_mode
    }

    pub fn set_approval_mode(&mut self, mode: crate::types::ApprovalMode) {
        let safe_mode = if self.inner.is_yolo_mode_disabled() && mode == ApprovalMode::Yolo {
            ApprovalMode::Default
        } else {
            mode
        };

        // Sync mode to inner agent for plan mode reminder injection
        self.inner.set_approval_mode(safe_mode.clone());

        if let Some(bus) = self.inner.runtime_message_bus() {
            crate::agent::approval::set_approval_mode(safe_mode, &self.approval_mode_lock, &bus);
            return;
        }

        let mut m = self.approval_mode_lock.lock().unwrap();
        *m = safe_mode;
    }

    /// Get the current approval mode
    pub fn get_approval_mode(&self) -> crate::types::ApprovalMode {
        let mode = self.approval_mode_lock.lock().unwrap().clone();
        crate::utils::logging::append_debug_log_line(&format!(
            "[STAR_AGENT] get_approval_mode: {:?}",
            mode
        ));
        mode
    }

    pub fn set_steering_queue(&mut self, queue: Arc<AsyncMessageQueue<(u64, String)>>) {
        self.steering_queue = Some(queue);
    }

    pub fn set_steering_signal(&mut self, signal: Arc<Notify>) {
        self.steering_signal = Some(signal);
    }
}

impl Deref for StarAgent {
    type Target = Agent;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for StarAgent {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
