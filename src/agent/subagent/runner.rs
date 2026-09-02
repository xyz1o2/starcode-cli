//! SubAgent 执行器：同步 + 异步。
//!
//! - `StarAgentRunner` 从 `agent/subagent_runner.rs` 迁入，原有同步逻辑不变。
//! - `AsyncSubagentRunner` 新增：后台执行 + 通知队列 + name 注册表。

use crate::agent::subagent::notification::{
    NotificationQueue, NotificationStatus, NotificationUsage, TaskNotification,
};
use crate::agent::subagent::progress::{AgentProgressTracker, SubAgentProgressSink};
use crate::core::agents::{
    SharedSubAgentRunner, SubAgentError, SubAgentErrorKind, SubAgentRequest, SubAgentResult,
};
use crate::core::config::Config;
use crate::core::tools::tools::ToolResult;
use crate::llm::client::StarClient;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

// ── StarAgentRunner（迁入，逻辑不变）──────────────────────────────────────

/// 同步 SubAgent 执行器
pub struct StarAgentRunner {
    client: StarClient,
    config: Arc<Config>,
}

impl StarAgentRunner {
    pub fn new(client: StarClient, config: Arc<Config>) -> Self {
        Self { client, config }
    }

    /// 创建共享引用（供 AgentTool 使用）
    pub fn shared(client: StarClient, config: Arc<Config>) -> SharedSubAgentRunner {
        Arc::new(Self::new(client, config))
    }

    /// 同步执行 SubAgent（无进度回流）
    pub async fn run(&self, request: SubAgentRequest) -> Result<SubAgentResult, SubAgentError> {
        self.run_with_progress(request, None).await
    }

    /// 同步执行 SubAgent，并把实时进度推给 `sink`。
    ///
    /// 对标 Claude Code 里 AgentTool 消费子 Agent 的 message 流并调用
    /// `setToolUseProgress` 的做法：这里消费 `process_user_message_stream`
    /// 产出的 chunk，边累加统计边回调。
    ///
    /// 之所以走 stream 而不是 `process_user_message`：后者只返回一条
    /// assistant 文本，工具调用与 token 用量全部丢失。
    pub async fn run_with_progress(
        &self,
        request: SubAgentRequest,
        sink: Option<SubAgentProgressSink>,
    ) -> Result<SubAgentResult, SubAgentError> {
        if self.config.recursion_depth >= 3 {
            return Err(SubAgentError::new(
                SubAgentErrorKind::RecursionLimitExceeded,
                "Maximum SubAgent recursion depth reached (3). Cannot start another SubAgent.",
            ));
        }

        let mut sub_config = (*self.config).clone();
        sub_config.recursion_depth = sub_config.recursion_depth.saturating_add(1);

        let mut agent = crate::agent::StarAgent::new(
            &self.client.api_key,
            Some(self.client.model.clone()),
            Some(self.client.base_url.clone()),
            request.max_rounds,
            Some(self.client.is_openai_compatible),
            Some(Arc::new(sub_config)),
        )
        .await
        .map_err(|e| {
            SubAgentError::new(
                SubAgentErrorKind::InitializationFailed,
                format!("Failed to create SubAgent: {}", e),
            )
        })?;

        let mut tracker = AgentProgressTracker::new(sink);
        let mut final_text = String::new();

        {
            use crate::types::StreamingChunkType;
            use futures::StreamExt;

            let mut stream = agent
                .process_user_message_stream(&request.prompt)
                .await
                .map_err(|e| {
                    SubAgentError::new(
                        SubAgentErrorKind::ExecutionFailed,
                        format!("SubAgent execution error: {}", e),
                    )
                })?;

            while let Some(item) = stream.next().await {
                let chunk = item.map_err(|e| {
                    SubAgentError::new(
                        SubAgentErrorKind::ExecutionFailed,
                        format!("SubAgent execution error: {}", e),
                    )
                })?;

                match chunk.chunk_type {
                    StreamingChunkType::ToolCalls => {
                        for tc in chunk.tool_calls.iter().flatten() {
                            tracker.on_tool_started(tc);
                        }
                    }
                    StreamingChunkType::ToolResult => {
                        if let (Some(tc), Some(tr)) = (&chunk.tool_call, &chunk.tool_result) {
                            tracker.on_tool_finished(tc, tr);
                        }
                    }
                    StreamingChunkType::TokenCount => {
                        if let Some(tokens) = chunk.token_count {
                            tracker.on_tokens(tokens);
                        }
                    }
                    StreamingChunkType::Content => {
                        if let Some(text) = chunk.content.as_ref() {
                            if !text.trim().is_empty() {
                                final_text = text.clone();
                                tracker.on_assistant_text(text.clone());
                            }
                        }
                    }
                    // TextDelta / ReasoningDelta 是增量文本，子 Agent 的中间思考
                    // 不回流父级 UI（对标参考实现只回流 progress messages）
                    _ => {}
                }
            }
        }

        let tool_use_count = tracker.tool_use_count();
        let total_tokens = tracker.tokens();
        let last_tool_info = tracker.last_tool_info();
        let entries = tracker.into_entries();

        let output = if final_text.trim().is_empty() {
            entries
                .iter()
                .filter(|entry| matches!(entry.entry_type, crate::types::ChatEntryType::Assistant))
                .last()
                .map(|entry| entry.content.clone())
                .unwrap_or_else(|| {
                    "SubAgent completed but returned no specific output.".to_string()
                })
        } else {
            final_text
        };

        Ok(SubAgentResult {
            output,
            entries,
            tool_use_count,
            total_tokens,
            last_tool_info,
        })
    }
}

// ── AsyncSubagentRunner ──────────────────────────────────────────────────

/// 后台 SubAgent 启动结果
#[derive(Debug, Clone)]
pub struct AsyncLaunchResult {
    pub agent_id: String,
    pub output_file: PathBuf,
    pub description: String,
}

/// 异步 SubAgent 执行器
pub struct AsyncSubagentRunner {
    /// 内部的同步 runner（clone 后用于后台 tokio::spawn）
    sync_runner: Arc<StarAgentRunner>,
    /// 全局通知队列
    notification_queue: Arc<Mutex<NotificationQueue>>,
    /// agent_name → agent_id 注册表，供 SendMessage 按 name 寻址
    name_registry: Arc<Mutex<HashMap<String, String>>>,
}

impl AsyncSubagentRunner {
    /// 创建异步 runner，需要同步 runner + 通知队列
    pub fn new(
        sync_runner: Arc<StarAgentRunner>,
        notification_queue: Arc<Mutex<NotificationQueue>>,
    ) -> Self {
        Self {
            sync_runner,
            notification_queue,
            name_registry: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 后台启动 SubAgent，立即返回 AsyncLaunchResult
    ///
    /// `agent_type_label` 是用户可见的类型标签（对标 `userFacingName`），
    /// 会随通知回到 UI，避免 UI 侧再硬编码 "general-purpose"。
    pub fn spawn_background(
        &self,
        request: SubAgentRequest,
        name: Option<String>,
        description: String,
        agent_type_label: String,
    ) -> AsyncLaunchResult {
        let agent_id: String = {
            let id = uuid::Uuid::new_v4().to_string();
            format!("agent-{}", &id[..8])
        };

        let output_file = PathBuf::from(format!(".star/subagent_outputs/{}.txt", agent_id));
        let runner = self.sync_runner.clone();
        let queue = self.notification_queue.clone();
        let registry = self.name_registry.clone();
        let desc = description.clone();
        let aid = agent_id.clone();
        let agent_name = name.clone();
        let type_label = agent_type_label.clone();

        tokio::spawn(async move {
            // 注册 name → agent_id 映射。放进 spawn 内用 async lock，
            // 避免在 async 上下文里调用 blocking_lock 触发 panic。
            if let Some(ref n) = agent_name {
                registry.lock().await.insert(n.clone(), aid.clone());
            }

            let start = std::time::Instant::now();

            // 后台 agent 的进度同样回流到 UI（AgentTaskUpdate chunk）
            let progress_id = aid.clone();
            let progress_desc = desc.clone();
            let progress_type = type_label.clone();
            let progress_name = agent_name.clone();
            let sink: SubAgentProgressSink = Arc::new(move |p| {
                crate::agent::subagent::progress::emit_to_ui(
                    crate::types::StreamingChunk::agent_task_update(
                        crate::types::AgentTaskUpdatePayload::new(
                            progress_id.clone(),
                            progress_type.clone(),
                        )
                        .with_description(progress_desc.clone())
                        .with_status(crate::types::AgentTaskStatus::Running)
                        .with_stats(p.tool_use_count, p.tokens)
                        .with_async(true)
                        .with_last_tool_info(p.last_tool_info.clone())
                        .with_name(progress_name.clone())
                        .with_task_description(Some(progress_desc.clone()))
                        .with_sub_entries(p.new_entries.clone()),
                    ),
                );
            });

            let result = runner.run_with_progress(request, Some(sink)).await;

            let (status, output, entries, tokens, tool_uses) = match result {
                Ok(r) => (
                    NotificationStatus::Completed,
                    r.output,
                    r.entries,
                    r.total_tokens as u64,
                    r.tool_use_count as u64,
                ),
                Err(e) => (
                    NotificationStatus::Failed,
                    format!("SubAgent failed: {}", e),
                    Vec::new(),
                    0,
                    0,
                ),
            };

            let mut q = queue.lock().await;
            q.enqueue(TaskNotification {
                task_id: aid,
                tool_use_id: None,
                status,
                summary: desc,
                result: output,
                usage: NotificationUsage {
                    total_tokens: tokens,
                    tool_uses,
                    duration_ms: start.elapsed().as_millis() as u64,
                },
                entries,
                agent_type: Some(type_label),
                name: agent_name,
            });
        });

        AsyncLaunchResult {
            agent_id,
            output_file,
            description,
        }
    }

    /// 按 name 查找 agent_id
    pub async fn resolve_name(&self, name: &str) -> Option<String> {
        self.name_registry.lock().await.get(name).cloned()
    }

    /// 列出所有已注册 agent name（供 broadcast 使用）
    pub async fn list_agent_names(&self) -> Vec<String> {
        self.name_registry.lock().await.keys().cloned().collect()
    }

    /// 投递消息到目标 agent（供 SendMessage 委托）
    ///
    /// - running → 写入文件 inbox（排队到 agent 下一轮处理）
    /// - stopped → 尝试从 sidechain 恢复后台运行
    pub async fn deliver_message(
        &self,
        _target_name: &str,
        _message: &str,
        _summary: &str,
        _is_protocol: bool,
    ) -> Result<ToolResult, Box<dyn std::error::Error>> {
        // 基础实现：写文件 inbox
        let inbox_dir = std::env::current_dir()
            .unwrap_or_default()
            .join(".star")
            .join("messages");

        if !inbox_dir.exists() {
            tokio::fs::create_dir_all(&inbox_dir).await?;
        }

        let timestamp = chrono::Utc::now().timestamp_millis();
        let filename = format!("{}_{}.json", _target_name, timestamp);
        let msg_path = inbox_dir.join(&filename);

        let msg = serde_json::json!({
            "from": "agent",
            "to": _target_name,
            "message": _message,
            "summary": _summary,
            "is_protocol": _is_protocol,
            "timestamp": timestamp,
        });

        tokio::fs::write(&msg_path, serde_json::to_string_pretty(&msg)?).await?;

        Ok(ToolResult {
            llm_content: Some(format!("Message delivered to {}", _target_name)),
            return_display: None,
            output: String::new(),
            error: None,
            data: None,
        })
    }
}
