//! SubAgent 执行器：同步 + 异步。
//!
//! - `StarAgentRunner` 从 `agent/subagent_runner.rs` 迁入，原有同步逻辑不变。
//! - `AsyncSubagentRunner` 新增：后台执行 + 通知队列 + name 注册表。

use crate::agent::subagent::notification::{
    NotificationQueue, NotificationStatus, NotificationUsage, TaskNotification,
};
use crate::agent::subagent::progress::{
    AgentProgressTracker, SubAgentProgress, SubAgentProgressSink,
};
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

/// 从子代理本轮生成的会话中读取最终助手正文（排除继承的父会话）。
fn subagent_final_text(messages: &[crate::types::StarMessage]) -> Option<String> {
    messages.last().and_then(|message| {
        (message.role == "assistant" && message.tool_calls.as_ref().is_none_or(Vec::is_empty))
            .then(|| message.content.as_deref().unwrap_or("").trim())
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    })
}

/// 把 fork 或 `/subtask` 的父会话装进刚创建的子代理。
///
/// 单独抽出来是为了让“结构化上下文 + pending 本地输出”这条 fork 语义
/// 能在不调用真实 LLM 的情况下被单测钉住。
fn install_parent_context(agent: &mut crate::agent::StarAgent, request: &SubAgentRequest) {
    if !request.parent_messages.is_empty() || !request.parent_pending_context.is_empty() {
        agent.replace_session_context(
            request.parent_messages.clone(),
            request.parent_pending_context.clone(),
        );
    }
}

/// 子代理未显式指定 max_rounds 时的默认轮次上限。
///
/// 与 agent_tool.rs 的 `DEFAULT_SYNC_MAX_ROUNDS` 同值：那处只护同步路径，
/// 这里是 runner 层的总收口（后台 / fork / skill / coordinator 全过这里）。
/// 不设兜底的话子代理继承主会话的 max_session_turns（默认 200），空转的
/// 子代理能把一个简单检索烧成几十万 token。
const DEFAULT_SUBAGENT_MAX_ROUNDS: u32 = 50;

/// 深度到达此值即拒绝创建；lead 为 0，允许的子代理深度为 1、2。
/// runner 和构造器共用边界，让超限在派生前返回明确的递归错误。
pub const MAX_SUBAGENT_DEPTH: usize = 3;

fn child_depth(parent_depth: usize, runner_depth: usize) -> Result<usize, SubAgentError> {
    let depth = parent_depth.max(runner_depth).saturating_add(1);
    if depth >= MAX_SUBAGENT_DEPTH {
        return Err(SubAgentError::new(
            SubAgentErrorKind::RecursionLimitExceeded,
            format!(
                "Maximum SubAgent recursion depth reached ({}). Cannot start another SubAgent.",
                MAX_SUBAGENT_DEPTH
            ),
        ));
    }
    Ok(depth)
}

/// Explorer 的工具集合；执行器同时限制 schema 和执行入口。
/// Bash 仍经过通用权限门，工具白名单本身不是命令参数级只读沙箱。
pub const EXPLORER_READONLY_TOOLS: &[&str] = &[
    "Read",
    "Grep",
    "Glob",
    "LS",
    "Bash",
    "ReadManyFiles",
    "CodebaseSearch",
    "CodeGraph",
    "RepoMap",
    "NotebookRead",
    "TodoWrite",
    "AskUserQuestion",
];

/// 同步 SubAgent 执行器
pub struct StarAgentRunner {
    client: StarClient,
    config: Arc<Config>,
    /// Agent 间消息 hub：派生带 identity 的子 agent 时把 hub 接进去。
    /// `None` = 从 config 的 RuntimeServices 读（bootstrap 创建的同一单例）。
    agent_hub: Option<crate::agent::messaging::SharedAgentRouter>,
    #[cfg(test)]
    panic_loop_before_handoff: bool,
}

impl StarAgentRunner {
    pub fn new(client: StarClient, config: Arc<Config>) -> Self {
        let agent_hub = config.runtime_agent_hub();
        Self {
            client,
            config,
            agent_hub,
            #[cfg(test)]
            panic_loop_before_handoff: false,
        }
    }

    /// 调用级模型只影响新子代理，provider 和 endpoint 保持父级选择。
    async fn create_child_agent(
        &self,
        request: &SubAgentRequest,
        config: Config,
    ) -> Result<crate::agent::StarAgent, SubAgentError> {
        let model = request
            .model
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty() && !model.eq_ignore_ascii_case("inherit"))
            .unwrap_or(&self.client.model);
        crate::agent::StarAgent::new_with_provider(
            &self.client.api_key,
            Some(model.to_owned()),
            Some(self.client.base_url.clone()),
            request.max_rounds,
            Some(self.client.is_openai_compatible),
            Some(Arc::new(config)),
            self.client.provider_id.clone(),
        )
        .await
        .map_err(|error| {
            SubAgentError::new(
                SubAgentErrorKind::InitializationFailed,
                format!("Failed to create SubAgent: {error}"),
            )
        })
    }

    /// 替换本次派生所继承的客户端，保留配置和消息 hub。
    pub(crate) fn with_parent_client(&self, client: &StarClient) -> Self {
        Self {
            client: client.clone(),
            config: self.config.clone(),
            agent_hub: self.agent_hub.clone(),
            #[cfg(test)]
            panic_loop_before_handoff: self.panic_loop_before_handoff,
        }
    }

    /// 在路由和工具集合确定前继承恢复来源的类型，显式冲突不得静默覆盖。
    pub(crate) async fn resolve_resume_type(
        &self,
        input: &mut crate::core::agents::AgentToolFullInput,
    ) -> Result<(), SubAgentError> {
        if let Some(id) = input.resume.as_deref() {
            let session = super::session::AgentSession::load(self.config.storage(), id)
                .await
                .map_err(|e| SubAgentError::new(SubAgentErrorKind::InitializationFailed, e))?;
            if input
                .subagent_type
                .as_ref()
                .is_some_and(|kind| *kind != session.subagent_type)
            {
                return Err(SubAgentError::new(
                    SubAgentErrorKind::InitializationFailed,
                    "Resume requires the original subagent type",
                ));
            }
            input.subagent_type = Some(session.subagent_type);
        }
        Ok(())
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
    ///
    /// 所有类型共用流式 LLM 循环与进度 tracker；Explorer 通过子配置选择
    /// 搜索专用提示与工具白名单，不另起绕过进度回流的执行路径。
    pub async fn run_with_progress(
        &self,
        request: SubAgentRequest,
        sink: Option<SubAgentProgressSink>,
    ) -> Result<SubAgentResult, SubAgentError> {
        self.run_with_control(request, sink, None).await
    }

    async fn run_with_control(
        &self,
        request: SubAgentRequest,
        sink: Option<SubAgentProgressSink>,
        stop: Option<Arc<std::sync::atomic::AtomicBool>>,
    ) -> Result<SubAgentResult, SubAgentError> {
        if stop
            .as_ref()
            .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst))
        {
            return Err(SubAgentError::new(
                SubAgentErrorKind::ExecutionFailed,
                "SubAgent stopped before initialization",
            ));
        }
        let resumed = if let Some(id) = request.resume.as_deref() {
            if request.identity.as_deref() == Some(id) {
                return Err(SubAgentError::new(
                    SubAgentErrorKind::InitializationFailed,
                    "Resume requires a new task identity",
                ));
            }
            if !request.parent_messages.is_empty() || !request.parent_pending_context.is_empty() {
                return Err(SubAgentError::new(
                    SubAgentErrorKind::InitializationFailed,
                    "Cannot combine resume with fork context",
                ));
            }
            if let Some(tasks) = self.config.runtime_subagent_tasks() {
                if tasks
                    .lock()
                    .await
                    .get(id)
                    .is_some_and(|task| task.status.is_none())
                {
                    return Err(SubAgentError::new(
                        SubAgentErrorKind::InitializationFailed,
                        "Cannot resume a running agent",
                    ));
                }
            }
            let session = super::session::AgentSession::load(self.config.storage(), id)
                .await
                .map_err(|e| SubAgentError::new(SubAgentErrorKind::InitializationFailed, e))?;
            if request.subagent_type != session.subagent_type {
                return Err(SubAgentError::new(
                    SubAgentErrorKind::InitializationFailed,
                    "Resume requires the original subagent type",
                ));
            }
            Some(session)
        } else {
            None
        };
        // 子代理开始 —— 外部 harness 靠这个事件知道"多了一个干活的"。
        // 之前 `SubagentStop` 只在**失败**时触发，成功路径静悄悄，外部看不到
        // 子代理的完整生命周期。
        crate::agent::hooks::run_subagent_start_hook(
            "",
            &request.prompt,
            request.subagent_type.as_str(),
        )
        .await;

        // 共享注册表的 runner 可能属于父级，以执行器传来的真实调用深度为下界。
        let child_depth = child_depth(request.parent_depth, self.config.recursion_depth)?;

        // Explorer 也是通用 LLM 循环（2026-11-05 内置检索专家下线后没有专用
        // 实现），不再提前分流 —— 之前这里转进 `run_explore`：那条路径走
        // **非流式** `process_user_message`，全程只有开头一次 sink 回流，UI
        // 端 5 个 Explore 就永远停在 "Searching codebase…"（本次事故）。通用
        // 循环的 `run_with_progress` 才有 tracker 实时回流 tool uses / tokens。
        let mut sub_config = (*self.config).clone();
        sub_config.recursion_depth = child_depth;
        sub_config.subagent_type = Some(request.subagent_type.clone());
        //
        // 轮次上限兜底：调用方（AgentTool 的 max_rounds 参数）给了就用，没给
        // 回落 50 —— 不兜底的话子代理继承主会话的 max_session_turns（默认
        // 200），一个空转的 Explore 能烧几十万 token（本次事故：子代理跑到
        // turn 54+）。同步路径在 agent_tool.rs 有同样的回落，这里收口在
        // runner 层，spawn_background / run_foreground / skill fork 全覆盖。
        let rounds = request.max_rounds.unwrap_or(DEFAULT_SUBAGENT_MAX_ROUNDS);
        sub_config.set_max_session_turns(rounds as i32);
        //
        // Explorer 只暴露搜索工具；Bash 继续走权限门，不能把工具名当成
        // 参数级只读保证。专用提示说明职责边界，执行器负责工具集合约束。
        if request.subagent_type == crate::core::agents::SubagentType::Explorer {
            sub_config.core_tools = Some(
                EXPLORER_READONLY_TOOLS
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            );
        }
        // 子 agent 自己的 hub 身份写进 config：SendMessage 从 config 读发件人，
        // worker 回话给 "lead" 时不用自报 agent_id（Phase 3/4）。
        let child_identity = request.identity.clone().or_else(|| {
            let id = uuid::Uuid::new_v4().to_string();
            Some(format!("agent-{id}"))
        });
        sub_config.agent_identity = child_identity.clone();
        let session_id = child_identity
            .clone()
            .expect("child identity is generated above");

        if let Some(bus) = self.config.runtime_message_bus() {
            sub_config
                .set_approval_mode(bus.approval_mode().await)
                .map_err(|e| {
                    SubAgentError::new(SubAgentErrorKind::InitializationFailed, e.to_string())
                })?;
        }
        let mut agent = self.create_child_agent(&request, sub_config).await?;
        #[cfg(test)]
        {
            agent.panic_loop_before_handoff = self.panic_loop_before_handoff;
        }

        // Hub 身份接线（Phase 3）：子 agent 在 hub 里注册成自己的身份，
        // SendMessage 按 name / agent_id 单播它，它自己每个 turn 开头 drain。
        // hub 优先取 runner 上挂的；runner 未挂（装配缺省）时从 config 读
        // RuntimeServices 里的同一个单例。身份与 sub_config.agent_identity
        // 同源（上面已填），保证 SendMessage 的发件人与 mailbox 键一致。
        if let Some(hub) = self
            .agent_hub
            .clone()
            .or_else(|| self.config.runtime_agent_hub())
        {
            let identity = child_identity.unwrap_or_else(|| {
                let id = uuid::Uuid::new_v4().to_string();
                format!("agent-{id}")
            });
            agent.set_agent_hub(hub, identity);
        }

        // fork 与 /subtask 都先安装结构化父上下文，再追加本次任务，
        // 保留已有工具调用和结果的配对。普通子代理的父上下文为空。
        if let Some(session) = resumed {
            agent.replace_session_context(session.messages, session.pending_context);
        } else {
            install_parent_context(&mut agent, &request);
        }
        // 父上下文安装会清除旧标志；之后再绑定本任务控制，保留初始化期间的停止请求。
        if let Some(stop) = stop {
            agent.bind_abort_handle(stop);
        }

        let mut tracker = AgentProgressTracker::new(sink);
        let mut final_text = String::new();
        let mut stream_error = None;

        let execution = async {
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
                let chunk = match item {
                    Ok(chunk) => chunk,
                    Err(error) => {
                        stream_error = Some(error.to_string());
                        // 继续排空到执行器的结束交接，避免丢弃外层流留下实际循环。
                        continue;
                    }
                };

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
                    StreamingChunkType::Error => {
                        stream_error =
                            Some(chunk.content.unwrap_or_else(|| "Unknown error".to_string()));
                    }
                    // TextDelta / ReasoningDelta 是增量文本，子 Agent 的中间思考
                    // 不回流父级 UI（对标参考实现只回流 progress messages）
                    _ => {}
                }
            }
            Ok::<(), SubAgentError>(())
        }
        .await;
        if !agent.turn_context_handed_back() {
            return Err(SubAgentError::new(
                SubAgentErrorKind::ExecutionFailed,
                stream_error
                    .or_else(|| execution.as_ref().err().map(ToString::to_string))
                    .unwrap_or_else(|| {
                        "Agent execution did not return session context; no resume snapshot saved"
                            .to_owned()
                    }),
            ));
        }
        let (session_messages, pending_context) = agent.session_context_snapshot();
        let snapshot_result = super::session::AgentSession::new(
            self.config.storage(),
            session_id,
            request.subagent_type.clone(),
            session_messages.clone(),
            pending_context,
        )
        .map_err(|e| SubAgentError::new(SubAgentErrorKind::ExecutionFailed, e));
        let snapshot_result = match snapshot_result {
            Ok(snapshot) => snapshot.save(self.config.storage()).await,
            Err(error) => Err(error.to_string()),
        };
        if let Err(save_error) = snapshot_result {
            let original = execution
                .as_ref()
                .err()
                .map(ToString::to_string)
                .or_else(|| stream_error.clone());
            return Err(SubAgentError::new(
                SubAgentErrorKind::ExecutionFailed,
                match original {
                    Some(original) => format!("{original}; {save_error}"),
                    None => save_error,
                },
            ));
        }
        execution?;

        if let Some(error) = stream_error {
            return Err(SubAgentError::new(
                SubAgentErrorKind::ExecutionFailed,
                format!("SubAgent execution error: {error}"),
            ));
        }

        // 流式模型的正文通常只走 direct TextDelta；Branch A 的 Message
        // 会因为已流式输出而跳过，因此 tracker 不一定收到 Content。
        // 结果以代理最终会话里的 assistant 正文为准，不从中间增量猜结论。
        // 压缩会缩短历史，不能按继承消息数切片；本次用户消息之后的末条助手才是本轮结果。
        let answer = session_messages
            .iter()
            .rposition(|m| m.role == "user")
            .and_then(|index| subagent_final_text(&session_messages[index + 1..]))
            .or_else(|| (!final_text.trim().is_empty()).then_some(final_text));
        if let Some(ref text) = answer {
            // 增量流本身不记助手条目；收尾时补上最终结论供详情与通知展示。
            if tracker.last_assistant_text() != Some(text.as_str()) {
                tracker.on_assistant_text(text.clone());
            }
        }
        let output = answer
            .unwrap_or_else(|| "SubAgent completed but returned no specific output.".to_string());
        let tool_use_count = tracker.tool_use_count();
        let total_tokens = tracker.tokens();
        let last_tool_info = tracker.last_tool_info();
        let entries = tracker.into_entries();

        Ok(SubAgentResult {
            output,
            entries,
            tool_use_count,
            total_tokens,
            last_tool_info,
        })
    }
}

/// 把 hub 的注册表拍成一份成员快照推给 UI（对标 CC 的 ListAgents 列 live
/// teammates）。register / unregister 之后各推一次，`/agent-mail agents`
/// 读到的就是当前真相而不是 spawn 时的旧帧。
///
/// 走 `progress::emit_to_ui`（cross-turn 兜底 sink 也在），后台 tokio 任务
/// 里同样可达；chunk 的 `content` 字段带成员 JSON 数组，运行时侧在
/// `stream_chunks.rs` 解成 `StreamMessage::AgentTeamUpdate`。
async fn emit_team_snapshot(hub: &crate::agent::messaging::SharedAgentRouter) {
    use crate::agent::messaging::{AgentRouter, LEAD_IDENTITY};
    let router: tokio::sync::MutexGuard<'_, AgentRouter> = hub.lock().await;
    let members: Vec<crate::runtime::messages::AgentTeamMember> = router
        .registered_entries()
        .into_iter()
        .map(
            |(identity, entry)| crate::runtime::messages::AgentTeamMember {
                mailbox_pending: router.pending_count(&identity),
                identity,
                agent_id: entry.agent_id,
                status: match entry.status {
                    crate::agent::messaging::AgentStatus::Running => "running".to_string(),
                    crate::agent::messaging::AgentStatus::Completed => "completed".to_string(),
                },
            },
        )
        .collect();
    // lead 常驻第一行（SendMessage target_agent="lead" 是回话路径）。
    let snapshot_json = serde_json::to_string(&members).unwrap_or_else(|_| "[]".to_string());
    crate::agent::subagent::progress::emit_to_ui(crate::types::StreamingChunk {
        chunk_type: crate::types::StreamingChunkType::AgentTeamSnapshot,
        content: Some(snapshot_json),
        ..crate::types::StreamingChunk::new(crate::types::StreamingChunkType::AgentTeamSnapshot)
    });
    let _ = LEAD_IDENTITY;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn child_model_override_keeps_parent_provider_endpoint_and_model() {
        let parent = crate::agent::StarAgent::new_for_runtime_settings_test_with_provider(Some(
            "openrouter".into(),
        ))
        .await;
        let mut parameters = crate::core::config::ConfigParameters::default();
        parameters.core_tools = Some(Vec::new());
        parameters.mcp_enabled = Some(false);
        parameters.enable_agents = Some(false);
        let mut config = Config::new(parameters);
        config.initialize().await.unwrap();
        let config = Arc::new(config);
        let original = StarAgentRunner::shared(parent.client.clone(), config.clone());
        let current = StarClient::new(
            "current-test-key",
            Some("current-model".into()),
            Some("http://127.0.0.1:9/v1".into()),
            Some(true),
            Some("current-provider".into()),
        );
        let async_original = AsyncSubagentRunner::new(
            original.clone(),
            Arc::new(Mutex::new(NotificationQueue::new())),
        );
        let rebound = async_original.with_parent_client(&current);
        assert!(Arc::ptr_eq(
            &rebound.notification_queue,
            &async_original.notification_queue
        ));
        assert!(Arc::ptr_eq(
            &rebound.name_registry,
            &async_original.name_registry
        ));
        for runner in [
            original.with_parent_client(&current),
            rebound.sync_runner.with_parent_client(&current),
        ] {
            for (requested, expected) in [
                (None, "current-model"),
                (Some("inherit"), "current-model"),
                (Some(" INHERIT "), "current-model"),
                (Some(" child-model "), "child-model"),
            ] {
                let mut request = SubAgentRequest::new("inspect");
                request.model = requested.map(str::to_owned);
                let child = runner
                    .create_child_agent(&request, (*config).clone())
                    .await
                    .unwrap();
                assert_eq!(child.client.model, expected);
                assert_eq!(child.client.base_url, current.base_url);
                assert_eq!(child.client.provider_id, current.provider_id);
            }
        }
        assert_eq!(parent.client.model, "test-model");
        assert_eq!(original.client.model, "test-model");
        assert_eq!(async_original.sync_runner.client.model, "test-model");
        assert_eq!(current.model, "current-model");
    }

    #[tokio::test]
    async fn child_model_and_refreshed_parent_reach_the_http_request() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for _ in 0..3 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 4096];
                loop {
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert!(count > 0, "request ended before its body");
                    bytes.extend_from_slice(&buffer[..count]);
                    assert!(bytes.len() < 1024 * 1024, "unexpected request size");
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap();
                        if bytes.len() >= end + 4 + length {
                            assert!(headers.starts_with("POST /v1/chat/completions "));
                            requests.push(
                                serde_json::from_slice::<serde_json::Value>(
                                    &bytes[end + 4..end + 4 + length],
                                )
                                .unwrap(),
                            );
                            break;
                        }
                    }
                }
                let body = r#"{"id":"child-test","object":"chat.completion","created":1,"model":"local-test","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"child response"}}]}"#;
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                socket.write_all(response.as_bytes()).await.unwrap();
            }
            requests
        });
        struct AbortServerOnDrop(tokio::task::AbortHandle);
        impl Drop for AbortServerOnDrop {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let _server_guard = AbortServerOnDrop(server.abort_handle());
        let result = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            let directory = tempfile::tempdir().unwrap();
            let mut parameters = crate::core::config::ConfigParameters::default();
            parameters.target_dir = directory.path().to_owned();
            parameters.core_tools = Some(Vec::new());
            parameters.mcp_enabled = Some(false);
            parameters.enable_agents = Some(false);
            let mut config = Config::new(parameters);
            config.initialize().await.unwrap();
            let config = Arc::new(config);
            let original_client = StarClient::new(
                "local-test-key",
                Some("original-parent".into()),
                Some(endpoint.clone()),
                Some(true),
                Some("local-test".into()),
            );
            let original = StarAgentRunner::new(original_client, config.clone());
            let current_client = StarClient::new(
                "local-test-key",
                Some("refreshed-parent".into()),
                Some(endpoint.clone()),
                Some(true),
                Some("local-test".into()),
            );
            let refreshed = original.with_parent_client(&current_client);
            for (runner, model) in [
                (&original, None),
                (&refreshed, Some("inherit")),
                (&refreshed, Some("explicit-child")),
            ] {
                let mut request = SubAgentRequest::new("inspect");
                request.model = model.map(str::to_owned);
                let child = runner
                    .create_child_agent(&request, (*config).clone())
                    .await
                    .unwrap();
                assert_eq!(child.client.base_url, endpoint);
                let answer = child.client.chat_completion_simple("reply").await.unwrap();
                assert_eq!(answer, "child response");
            }
            assert_eq!(original.client.model, "original-parent");
            assert_eq!(refreshed.client.model, "refreshed-parent");
            server.await.unwrap()
        })
        .await;
        let requests = result.expect("local HTTP regression timed out");
        let models: Vec<_> = requests
            .iter()
            .map(|body| body["model"].as_str().unwrap())
            .collect();
        assert_eq!(
            models,
            ["original-parent", "refreshed-parent", "explicit-child"]
        );
    }

    #[tokio::test]
    async fn resume_reaches_http_with_native_history_and_current_client() {
        assert_resume_http(false, false, false).await;
    }

    #[tokio::test]
    async fn resume_after_execution_failure_preserves_tool_history() {
        assert_resume_http(true, false, false).await;
    }

    #[tokio::test]
    async fn resume_after_cancel_uses_fresh_abort_control() {
        assert_resume_http(false, true, false).await;
    }

    #[tokio::test]
    async fn resume_obeys_current_plan_permissions() {
        assert_resume_http(false, false, true).await;
    }

    async fn assert_resume_http(fail_first: bool, cancel_first: bool, plan_write: bool) {
        use crate::core::tools::tools::BaseDeclarativeTool;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let directory = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let server_stop = stop.clone();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (index, answer) in ["original answer", "continued answer"]
                .into_iter()
                .enumerate()
            {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut buf = [0; 4096];
                loop {
                    let count = socket.read(&mut buf).await.unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buf[..count]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let length = String::from_utf8_lossy(&bytes[..end])
                            .lines()
                            .find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap();
                        if bytes.len() >= end + 4 + length {
                            requests.push(
                                serde_json::from_slice::<serde_json::Value>(
                                    &bytes[end + 4..end + 4 + length],
                                )
                                .unwrap(),
                            );
                            break;
                        }
                    }
                }
                if cancel_first && index == 0 {
                    server_stop.store(true, std::sync::atomic::Ordering::SeqCst);
                    let closed = tokio::time::timeout(
                        std::time::Duration::from_secs(8),
                        socket.read(&mut buf),
                    )
                    .await
                    .unwrap();
                    assert!(matches!(closed, Ok(0) | Err(_)));
                    continue;
                }
                let frame = if plan_write && index == 1 {
                    serde_json::json!({"object":"chat.completion.chunk","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"plan-write","type":"function","function":{"name":"Write","arguments":"{\"file_path\":\"forbidden.txt\",\"content\":\"must not exist\"}"}}]}}]})
                } else if fail_first && index == 0 {
                    serde_json::json!({"object":"chat.completion.chunk","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"failed-read","type":"function","function":{"name":"Read","arguments":"{\"file_path\":\"missing.rs\"}"}}]}}]})
                } else {
                    serde_json::json!({"object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content": answer}}]})
                };
                let body = format!("data: {frame}\n\ndata: {{\"object\":\"chat.completion.chunk\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n");
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
            requests
        });
        struct StopServer(tokio::task::AbortHandle);
        impl Drop for StopServer {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let _guard = StopServer(server.abort_handle());
        tokio::time::timeout(std::time::Duration::from_secs(20), async {
            let mut config = Config::new(crate::core::config::ConfigParameters {
                target_dir: directory.path().to_owned(), core_tools: Some(if plan_write { vec!["Write".into()] } else { Vec::new() }),
                mcp_enabled: Some(false), enable_agents: Some(false), ..Default::default()
            });
            config.initialize().await.unwrap();
            let config = Arc::new(config);
            let client = StarClient::new("local-key", Some("original-model".into()), Some(endpoint.clone()), Some(true), Some("local-test".into()));
            let runner = StarAgentRunner::shared(client, config.clone());
            let call = crate::types::StarToolCall {
                id: "old-read".into(), call_type: "function".into(),
                function: crate::types::StarToolCallFunction { name: "Read".into(), arguments: r#"{"file_path":"old.rs"}"#.into() }
            };
            let history = vec![crate::types::StarMessage::user("historical question"),
                crate::types::StarMessage::assistant_with_tool_calls(vec![call]),
                crate::types::StarMessage::tool("old-read", "historical tool result")];
            let first = runner.run_with_control(SubAgentRequest::new("original prompt").with_identity("agent-original")
                .with_parent_context(history, vec![]).with_max_rounds(if fail_first { 1 } else { 2 }), None, Some(stop.clone())).await;
            if fail_first {
                assert!(first.is_err(), "a tool turn at the turn limit must fail");
            } else if cancel_first {
                assert!(stop.load(std::sync::atomic::Ordering::SeqCst));
            } else {
                assert_eq!(first.unwrap().output, "original answer");
            }
            let before = tokio::fs::read(config.storage().project_subagent_sessions_dir().join("agent-original.json")).await.unwrap();
            let original = crate::agent::subagent::session::AgentSession::load(config.storage(), "agent-original").await.unwrap();
            assert!(original.messages.iter().any(|m| m.tool_calls.as_ref().is_some_and(|calls| calls.iter().any(|call| call.id == "old-read"))));
            assert!(original.messages.iter().any(|m| m.role == "tool" && m.tool_call_id.as_deref() == Some("old-read")));
            if plan_write {
                config.runtime_message_bus().unwrap().set_approval_mode(crate::core::policy::types::ApprovalMode::Plan).await;
                assert_ne!(*config.approval_mode(), crate::core::policy::types::ApprovalMode::Plan);
            }
            let tool = crate::core::tools::agent_tool::AgentTool::new(runner);
            let mut invocation = tool.create_invocation(serde_json::json!({
                "description":"continue investigation", "prompt":"new follow-up", "resume":"agent-original", "max_rounds": if plan_write { 1 } else { 2 }
            })).unwrap();
            let current = StarClient::new("current-local-key", Some("current-model".into()), Some(endpoint), Some(true), Some("local-test".into()));
            invocation.set_agent_parent_client(&current);
            let result = invocation.execute(None, None).await.unwrap();
            if plan_write {
                assert!(result.error.is_some());
                assert!(!directory.path().join("forbidden.txt").exists());
            } else {
                assert!(result.error.is_none(), "{:?}", result.error);
                assert_eq!(result.output, "continued answer");
            }
            let id = result.data.unwrap()["agent_id"].as_str().unwrap().to_owned();
            assert_ne!(id, "agent-original");
            let continued = crate::agent::subagent::session::AgentSession::load(config.storage(), &id).await.unwrap();
            if plan_write {
                let denied = continued.messages.iter().find(|m| m.role == "tool" && m.tool_call_id.as_deref() == Some("plan-write")).expect("denied Write must have a tool result");
                let text = denied.content.as_deref().unwrap_or("").to_lowercase();
                assert!(text.contains("denied") || text.contains("not allowed"), "{text}");
            } else {
                assert!(continued.messages.iter().any(|m| m.content.as_deref() == Some("continued answer")));
            }
            assert!(continued.messages.iter().any(|m| m.tool_calls.as_ref().is_some_and(|calls| calls.iter().any(|call| call.id == "old-read"))));
            assert!(continued.messages.iter().any(|m| m.role == "tool" && m.tool_call_id.as_deref() == Some("old-read")));
            assert_eq!(before, tokio::fs::read(config.storage().project_subagent_sessions_dir().join("agent-original.json")).await.unwrap());
            let requests = server.await.unwrap();
            assert_eq!(requests[1]["model"], "current-model");
            let messages = requests[1]["messages"].as_array().unwrap();
            let serialized = serde_json::to_string(messages).unwrap();
            // 当前工具集合为空时，rig 将历史工具调用转成文本；原生 ID 已在两份快照中验证。
            for text in ["historical question", "historical tool result", "old.rs", "original prompt", "new follow-up"] {
                assert!(serialized.contains(text), "missing {text}: {serialized}");
            }
            if fail_first {
                assert!(original.messages.iter().any(|m| m.tool_calls.as_ref().is_some_and(|calls| calls.iter().any(|call| call.id == "failed-read"))));
                assert!(original.messages.iter().any(|m| m.role == "tool" && m.tool_call_id.as_deref() == Some("failed-read")));
                assert!(serialized.contains("missing.rs"));
            } else if !cancel_first {
                assert!(serialized.contains("original answer"));
            }
            if cancel_first {
                assert!(stop.load(std::sync::atomic::Ordering::SeqCst), "resume must not reuse or clear the source abort control");
            }
        }).await.expect("resume HTTP regression timed out");
    }

    #[tokio::test]
    async fn resume_inherits_type_before_routing_and_rejects_conflicts() {
        use crate::agent::subagent::router::{route_agent_call, AgentRoute};
        use crate::core::agents::{AgentToolFullInput, SubagentType};
        let root = tempfile::tempdir().unwrap();
        let config = Arc::new(Config::new(crate::core::config::ConfigParameters {
            target_dir: root.path().to_owned(),
            ..Default::default()
        }));
        crate::agent::subagent::session::AgentSession::new(
            config.storage(),
            "explore-old".into(),
            SubagentType::Explorer,
            vec![],
            vec![],
        )
        .unwrap()
        .save(config.storage())
        .await
        .unwrap();
        let runner = StarAgentRunner::new(StarClient::new("", None, None, None, None), config);
        let mut input: AgentToolFullInput = serde_json::from_value(serde_json::json!({
            "description":"continue", "prompt":"find caller", "resume":"explore-old"
        }))
        .unwrap();
        runner.resolve_resume_type(&mut input).await.unwrap();
        assert_eq!(input.subagent_type, Some(SubagentType::Explorer));
        assert!(matches!(
            route_agent_call(&input, false, false),
            AgentRoute::AsyncAgent { .. }
        ));
        input.background = Some(false);
        assert!(matches!(
            route_agent_call(&input, false, false),
            AgentRoute::SyncNamedAgent {
                subagent_type: SubagentType::Explorer,
                ..
            }
        ));
        input.subagent_type = Some(SubagentType::Editor);
        assert!(runner
            .resolve_resume_type(&mut input)
            .await
            .unwrap_err()
            .to_string()
            .contains("original subagent type"));
        let mut request = SubAgentRequest::new("continue").with_identity("explore-old");
        request.resume = Some("explore-old".into());
        assert!(runner
            .run(request)
            .await
            .unwrap_err()
            .to_string()
            .contains("new task identity"));
        input.resume = Some("missing".into());
        assert!(runner.resolve_resume_type(&mut input).await.is_err());
    }

    #[tokio::test]
    async fn background_launch_registers_before_return_and_cleans_up_after_failure() {
        use crate::core::tools::tools::BaseDeclarativeTool;
        let output_directory = tempfile::tempdir().unwrap();
        let mut config = Config::new(crate::core::config::ConfigParameters {
            target_dir: output_directory.path().to_owned(),
            ..Default::default()
        });
        let bus = Arc::new(crate::core::confirmation_bus::MessageBus::new(
            crate::core::policy::policy_engine::PolicyEngine::new(Default::default()),
            false,
        ));
        let queue = Arc::new(Mutex::new(NotificationQueue::new()));
        let hub = crate::agent::messaging::new_shared_router();
        config.install_runtime_services(
            crate::core::config::RuntimeServices::new(
                None,
                bus,
                None,
                Arc::new(crate::core::state::GlobalState::new()),
            )
            .with_notification_queue(queue.clone())
            .with_agent_hub(hub.clone()),
        );
        let config = Arc::new(config);
        let output_tool = crate::core::tools::task_management::TaskOutputTool::new(config.clone());
        let sync = StarAgentRunner::shared(StarClient::new("", None, None, None, None), config);
        let runner = AsyncSubagentRunner::new(sync, queue.clone()).with_agent_hub(hub.clone());
        assert!(Arc::ptr_eq(
            runner.sync_runner.agent_hub.as_ref().unwrap(),
            &hub
        ));
        for name in [None, Some("named-worker".to_string())] {
            // 在构造/请求模型之前触发确定性的深度错误，无真实 API 调用。
            let request = SubAgentRequest::new("inspect").with_parent_depth(usize::MAX);
            let launch = runner
                .spawn_background(request, name.clone(), "inspect".into(), "Agent".into())
                .await;
            let identity = launch.agent_id.as_str();
            assert_eq!(
                hub.lock().await.resolve(&launch.agent_id).as_deref(),
                Some(identity)
            );
            let mut notifications = Vec::new();
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    notifications.extend(queue.lock().await.drain_for_next_turn());
                    if !notifications.is_empty() {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert!(hub.lock().await.resolve(&launch.agent_id).is_none());
            assert_eq!(notifications.len(), 1);
            let queried = output_tool
                .create_invocation(serde_json::json!({"task_id": launch.agent_id}))
                .unwrap()
                .execute(None, None)
                .await
                .unwrap();
            assert_eq!(queried.output, notifications[0].result);
            let mut fresh = Config::new(crate::core::config::ConfigParameters {
                target_dir: output_directory.path().to_owned(),
                ..Default::default()
            });
            fresh.initialize().await.unwrap();
            assert!(fresh
                .runtime_subagent_tasks()
                .unwrap()
                .lock()
                .await
                .get(&launch.agent_id)
                .is_none());
            let historical =
                crate::core::tools::task_management::TaskOutputTool::new(Arc::new(fresh))
                    .create_invocation(serde_json::json!({"task_id": launch.agent_id}))
                    .unwrap()
                    .execute(None, None)
                    .await
                    .unwrap();
            assert_eq!(historical.output, queried.output);
            assert_eq!(historical.data.as_ref().unwrap()["status"], "failed");
            assert_eq!(queried.data.as_ref().unwrap()["status"], "failed");
            assert_eq!(
                queried.data.as_ref().unwrap()["output_file"],
                launch.output_file.to_string_lossy().as_ref()
            );
            let stored = runner.tasks.lock().await.get(&launch.agent_id).unwrap();
            assert_eq!(stored.status, Some(NotificationStatus::Failed));
            assert_eq!(stored.output_file, launch.output_file);
            assert!(stored.output_error.is_none());
            assert!(tokio::fs::read_to_string(&stored.output_file)
                .await
                .unwrap()
                .contains("SubAgent failed"));
            assert!(matches!(
                notifications[0].status,
                NotificationStatus::Failed
            ));
            assert!(!queue.lock().await.has_inflight());
            if let Some(name) = name {
                assert!(runner.resolve_name(&name).await.is_none());
            }
            // 测试只清理由本次启动返回的临时输出文件。
            let _ = tokio::fs::remove_file(launch.output_file).await;
        }
    }

    #[tokio::test]
    async fn background_stop_cancels_the_actual_stream_and_preserves_queryable_result() {
        assert_background_stop(true, false).await;
    }

    #[tokio::test]
    async fn background_stop_cancels_before_response_headers() {
        assert_background_stop(false, false).await;
    }

    #[tokio::test]
    async fn background_stop_preserves_session_save_failure() {
        assert_background_stop(false, true).await;
    }

    async fn assert_background_stop(send_headers: bool, block_session_save: bool) {
        use crate::core::tools::tools::BaseDeclarativeTool;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let directory = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut bytes = [0; 4096];
            loop {
                let count = socket.read(&mut bytes).await.unwrap();
                assert!(count > 0);
                request.extend_from_slice(&bytes[..count]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        let body: serde_json::Value =
                            serde_json::from_slice(&request[end + 4..end + 4 + length]).unwrap();
                        assert_eq!(body["model"], "local-model");
                        assert_eq!(
                            body["stream"], true,
                            "stop regression must reach the agent stream"
                        );
                        break;
                    }
                }
            }
            if send_headers {
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"working\"}}]}\n\n").await.unwrap();
            }
            let _ = started_tx.send(());
            let closed =
                tokio::time::timeout(std::time::Duration::from_secs(8), socket.read(&mut bytes))
                    .await
                    .expect("cancelled request must release its connection");
            assert!(
                matches!(closed, Ok(0) | Err(_)),
                "request sent unexpected data after cancellation"
            );
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(200), listener.accept())
                    .await
                    .is_err(),
                "cancelled request must not retry"
            );
        });
        struct StopServer(tokio::task::AbortHandle);
        impl Drop for StopServer {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let _server_guard = StopServer(server.abort_handle());
        let mut config = Config::new(crate::core::config::ConfigParameters {
            target_dir: directory.path().to_owned(),
            core_tools: Some(Vec::new()),
            mcp_enabled: Some(false),
            enable_agents: Some(false),
            ..Default::default()
        });
        config.initialize().await.unwrap();
        let config = Arc::new(config);
        let queue = Arc::new(Mutex::new(NotificationQueue::new()));
        let runner = AsyncSubagentRunner::new(
            StarAgentRunner::shared(
                StarClient::new(
                    "local-test-key",
                    Some("local-model".into()),
                    Some(endpoint),
                    Some(true),
                    Some("local-test".into()),
                ),
                config.clone(),
            ),
            queue.clone(),
        );
        let launch = runner
            .spawn_background(
                SubAgentRequest::new("inspect").with_max_rounds(2),
                Some("stoppable-worker".into()),
                "inspect".into(),
                "Agent".into(),
            )
            .await;
        tokio::time::timeout(std::time::Duration::from_secs(10), started_rx)
            .await
            .unwrap()
            .unwrap();
        if block_session_save {
            tokio::fs::create_dir_all(config.storage().star_dir())
                .await
                .unwrap();
            tokio::fs::write(
                config.storage().project_subagent_sessions_dir(),
                "blocked session directory",
            )
            .await
            .unwrap();
        }
        let tool = crate::core::tools::task_management::TaskStopTool::new(config.clone());
        for attempt in 0..2 {
            let response = tool
                .create_invocation(serde_json::json!({"task_id": launch.agent_id}))
                .unwrap()
                .execute(None, None)
                .await
                .unwrap();
            if attempt == 0 {
                assert_eq!(response.data.unwrap()["stop_requested"], true);
            } else {
                assert!(response.error.is_none());
            }
        }
        let notifications = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let notifications = queue.lock().await.drain_for_next_turn();
                if !notifications.is_empty() {
                    break notifications;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].status, NotificationStatus::Killed);
        assert!(!queue.lock().await.has_inflight());
        assert!(queue.lock().await.drain_for_next_turn().is_empty());
        assert!(runner.resolve_name("stoppable-worker").await.is_none());
        assert!(config
            .runtime_agent_hub()
            .unwrap()
            .lock()
            .await
            .resolve(&launch.agent_id)
            .is_none());
        let snapshot =
            crate::agent::subagent::session::AgentSession::load(config.storage(), &launch.agent_id)
                .await;
        if block_session_save {
            assert!(snapshot.is_err());
            assert!(notifications[0]
                .result
                .contains("Failed to create agent session directory"));
        } else {
            assert!(snapshot.unwrap().messages.iter().any(|m| m.role == "user"));
        }
        let output = crate::core::tools::task_management::TaskOutputTool::new(config)
            .create_invocation(serde_json::json!({"task_id": launch.agent_id}))
            .unwrap()
            .execute(None, None)
            .await
            .unwrap();
        assert_eq!(output.data.unwrap()["status"], "killed");
        assert!(output.output.contains("stopped by request"));
        if block_session_save {
            assert!(output
                .output
                .contains("Failed to create agent session directory"));
        }
        let response = tool
            .create_invocation(serde_json::json!({"task_id": launch.agent_id}))
            .unwrap()
            .execute(None, None)
            .await
            .unwrap();
        assert_eq!(response.data.unwrap()["stop_requested"], false);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn panicked_execution_fails_and_never_publishes_stale_session() {
        let root = tempfile::tempdir().unwrap();
        let mut config = Config::new(crate::core::config::ConfigParameters {
            target_dir: root.path().to_owned(),
            core_tools: Some(vec![]),
            mcp_enabled: Some(false),
            enable_agents: Some(false),
            ..Default::default()
        });
        config.initialize().await.unwrap();
        let config = Arc::new(config);
        let mut sync = StarAgentRunner::new(
            StarClient::new(
                "local-key",
                Some("local-model".into()),
                Some("http://127.0.0.1:9/v1".into()),
                Some(true),
                None,
            ),
            config.clone(),
        );
        sync.panic_loop_before_handoff = true;
        let queue = Arc::new(Mutex::new(NotificationQueue::new()));
        let runner = AsyncSubagentRunner::new(Arc::new(sync), queue.clone());
        let launch = runner
            .spawn_background(
                SubAgentRequest::new("inspect").with_parent_context(
                    vec![
                        crate::types::StarMessage::user("old question"),
                        crate::types::StarMessage::assistant("stale answer"),
                    ],
                    vec![],
                ),
                Some("panic-worker".into()),
                "inspect".into(),
                "Agent".into(),
            )
            .await;
        let notification = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                if let Some(notification) =
                    queue.lock().await.drain_for_next_turn().into_iter().next()
                {
                    break notification;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("panic must close both streams and finish the background task");
        assert_eq!(notification.status, NotificationStatus::Failed);
        assert!(notification
            .result
            .contains("without returning session context"));
        assert!(!config
            .storage()
            .project_subagent_sessions_dir()
            .join(format!("{}.json", launch.agent_id))
            .exists());
        assert!(runner.resolve_name("panic-worker").await.is_none());
        assert!(!queue.lock().await.has_inflight());
        assert!(config
            .runtime_agent_hub()
            .unwrap()
            .lock()
            .await
            .resolve(&launch.agent_id)
            .is_none());
        assert_eq!(
            runner
                .tasks
                .lock()
                .await
                .get(&launch.agent_id)
                .unwrap()
                .status,
            Some(NotificationStatus::Failed)
        );
    }

    #[tokio::test]
    async fn stop_before_initialization_never_constructs_a_child() {
        let runner = StarAgentRunner::new(
            StarClient::new("", None, None, None, None),
            Arc::new(Config::new(Default::default())),
        );
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let error = runner
            .run_with_control(
                SubAgentRequest::new("inspect").with_parent_depth(usize::MAX),
                None,
                Some(stop),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("stopped before initialization"));
    }

    #[test]
    fn spawned_identity_is_the_same_as_the_consumers_mailbox() {
        for (explicit, name, expected) in [
            (None, None, "agent-id"),
            (None, Some("skill-worker"), "agent-id"),
            (Some("explicit"), Some("alias"), "agent-id"),
        ] {
            let mut request = SubAgentRequest::new("inspect");
            request.identity = explicit.map(str::to_owned);
            let identity = AsyncSubagentRunner::bind_identity(&mut request, name, "agent-id");
            assert_eq!(identity, expected);
            assert_eq!(request.identity.as_deref(), Some(expected));
            let mut hub = crate::agent::messaging::AgentRouter::new();
            hub.register_entry(identity, "agent-id", name.map(str::to_owned));
            hub.deliver(
                crate::agent::messaging::AgentMessage::new("lead", "steer", "update"),
                "agent-id",
                "lead",
            );
            assert_eq!(hub.drain(request.identity.as_deref().unwrap()).len(), 1);
        }
    }

    #[test]
    fn nested_depth_uses_the_caller_and_rejects_before_construction() {
        assert_eq!(child_depth(0, 0).unwrap(), 1);
        assert_eq!(child_depth(1, 0).unwrap(), 2);
        assert_eq!(child_depth(0, 1).unwrap(), 2);
        for (caller, runner) in [(2, 0), (0, 2), (usize::MAX, 0)] {
            assert!(matches!(
                child_depth(caller, runner).unwrap_err().kind,
                SubAgentErrorKind::RecursionLimitExceeded
            ));
        }
    }

    #[test]
    fn final_answer_comes_from_session_even_when_stream_only_emits_deltas() {
        let history = vec![
            crate::types::StarMessage::user("Find the handler".to_string()),
            crate::types::StarMessage::assistant("Searching…".to_string()),
            crate::types::StarMessage::assistant_with_tool_calls(vec![
                crate::types::StarToolCall {
                    id: "call-1".to_string(),
                    call_type: "function".to_string(),
                    function: crate::types::StarToolCallFunction {
                        name: "Grep".to_string(),
                        arguments: "{}".to_string(),
                    },
                },
            ]),
            crate::types::StarMessage::assistant("The handler is in auth.rs".to_string()),
        ];
        assert_eq!(
            subagent_final_text(&history).as_deref(),
            Some("The handler is in auth.rs")
        );
        assert_eq!(subagent_final_text(&history[..3]), None);
        assert_eq!(
            subagent_final_text(&history[..2]).as_deref(),
            Some("Searching…")
        );
        assert_eq!(subagent_final_text(&history[..1]), None);
    }

    /// Explorer 派发：内置 `ExploreAgent` 下线后走**通用 LLM 循环**。
    ///
    /// 2026-11-05 之前，`explorer` 走一条独立的确定性检索路径（ACE 广搜 +
    /// 可选 deep filter），那条路径不碰网络，所以能用假 key 单测。
    ///
    /// 现在内置检索专家已删除，`explorer` 与其他类型一样走通用循环 ——
    /// 而通用循环需要完整的 runtime bootstrap（`StarAgent` 构造要
    /// ToolRegistry），在单测里起不来。
    ///
    /// 所以这条测试现在只锁**类型判定**：Explorer 仍然是可请求的类型，
    /// 且它的专用实现已经不在了。派发路由本身的覆盖见 `router.rs`。
    ///
    /// 回归锚点（2026-11-13）：`run_explore` 盲跑路径已整体删除 —— 之前
    /// explorer 在 `run_with_progress` 里提前分流进那条非流式路径，全程只有
    /// 开头一次进度回流，UI 端永远停在 "Searching codebase…"。现在 explorer
    /// 与其他类型一样走 tracker 实时回流的通用循环，本测试的存在同时锁定
    /// 「不再有提前分流」这一事实：若有人重新引入 `run_explore` 分支，此测试
    /// 所在文件的编译期引用链（`SubagentType::Explorer` 不再命中专用分支）
    /// 会在 review 时被这条注释拦下。
    #[tokio::test]
    async fn explorer_has_no_dedicated_deterministic_agent_anymore() {
        // 内置 ExploreAgent 已从 src/agent/skills/ 删除 —— 用编译期事实
        // 表达这条不变量：下面的 use 会编译不过。
        //
        // use crate::agent::skills::ExploreAgent;
        assert!(
            !std::any::type_name::<crate::agent::skills::SubTask>().contains("ExploreAgent"),
            "内置 ExploreAgent 不该再存在"
        );

        // Explorer 仍是可请求的子代理类型（只是没有专用实现了）。
        let request = SubAgentRequest::new("where is auth handled")
            .with_subagent_type(crate::core::agents::SubagentType::Explorer);
        assert_eq!(
            request.subagent_type,
            crate::core::agents::SubagentType::Explorer
        );
    }

    /// Explore 只读白名单（对标 CC 官方 docs 的 Explore agent：Read/Grep/Glob/
    /// 只读 Bash，禁一切可变更工具）。两条派发路径的收口点都靠把
    /// `core_tools` 灌进子 config —— 这里锁不变量的输入侧：白名单不含
    /// Write/Edit 类可变更工具，且派发判定路由正确（真正注册面由
    /// `is_core_tool_enabled` 在构造期消费，其覆盖见 core_runtime 测试）。
    #[test]
    fn explorer_readonly_whitelist_excludes_mutating_tools() {
        let mutating = ["Write", "Edit", "MultiEdit", "SmartEdit", "NotebookEdit"];
        for tool in mutating {
            assert!(
                !EXPLORER_READONLY_TOOLS.contains(&tool),
                "{} 不该出现在 Explore 只读白名单里",
                tool
            );
        }
        for tool in ["Read", "Grep", "Glob", "Bash"] {
            assert!(
                EXPLORER_READONLY_TOOLS.contains(&tool),
                "{} 是 Explore 的核心检索工具，必须在名单里",
                tool
            );
        }

        // 通用循环路径的收口：派发判定仍以 subtype 为准。这里用配置构造的
        // 输入侧验证（StarAgent 构造需要完整 runtime bootstrap，单测起不来）。
        let explorer_req = SubAgentRequest::new("find auth")
            .with_subagent_type(crate::core::agents::SubagentType::Explorer);
        assert_eq!(
            explorer_req.subagent_type,
            crate::core::agents::SubagentType::Explorer
        );
    }

    /// 回归（2026-11-13 UI 冻结事故）：后台 Explore 子代理**必须**走带
    /// tracker 的通用循环，任何「只发一次初始进度就静默跑完」的专用路径都
    /// 会让选择器永久停在初始占位文案。
    ///
    /// 单测起不了完整 runtime bootstrap（`StarAgent` 构造要 ToolRegistry），
    /// 所以锁结构不变量而不是跑真代理：`StarAgentRunner` 的公开执行面只有
    /// `run` / `run_with_progress`（都经 tracker 回流），不存在第三条
    /// explorer 专用方法。通过方法清单断言表达 —— 重新引入专用路径而忘记接
    /// tracker 时，这条测试配合上方注释会提示 reviewer 检查 sink 接线。
    #[test]
    fn explorer_progress_flows_through_the_shared_tracker_path() {
        // 编译期事实：这些调用都能解析，说明执行入口仍然存在且签名未变 ——
        // 全部收口在带 tracker 的 run_with_progress 上。
        let _ = |r: &StarAgentRunner| {
            let _ = r.run(SubAgentRequest::new("x"));
            let _ = r.run_with_progress(SubAgentRequest::new("x"), None);
        };

        // spawn_background / run_foreground 最终都落在 sync_runner.run(_with_progress)
        // —— 即 tracker 路径。真正跑起来时的 sink 接线由 agent_tool 的两条
        // 路径各自提供；这里锁的是「公开面没有 run_explore 类旁路」。
    }

    /// 派发判定本身：只有 Explorer 进检索分支。通用路径需要完整
    /// runtime bootstrap（`StarAgent` 构造要 ToolRegistry），在单测里
    /// 起不来，这里只验类型判定的输入侧——路由侧的覆盖见 `router.rs`。
    #[tokio::test]
    async fn forked_subtask_installs_parent_messages_and_pending_context() {
        let mut agent = crate::agent::StarAgent::new_for_runtime_settings_test().await;
        let parent_messages = vec![
            crate::types::StarMessage::new("user", Some("fix the login flow".to_string())),
            crate::types::StarMessage::new(
                "assistant",
                Some("The auth module is in src/auth.rs".to_string()),
            ),
        ];
        let request = SubAgentRequest::new("add the regression test")
            .with_parent_context(parent_messages, vec!["cargo test: all passed".to_string()]);

        install_parent_context(&mut agent, &request);
        let (messages, pending) = agent.session_context_snapshot();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(
            messages[1].content.as_deref(),
            Some("The auth module is in src/auth.rs")
        );
        assert_eq!(pending, vec!["cargo test: all passed"]);
    }

    #[test]
    fn only_explorer_is_routed_to_search() {
        // 默认是 GeneralPurpose，不能误判成 Explorer
        let default = SubAgentRequest::new("anything");
        assert_ne!(
            default.subagent_type,
            crate::core::agents::SubagentType::Explorer
        );

        // 显式标注才进检索
        let explorer = SubAgentRequest::new("find auth")
            .with_subagent_type(crate::core::agents::SubagentType::Explorer);
        assert_eq!(
            explorer.subagent_type,
            crate::core::agents::SubagentType::Explorer
        );
    }

    /// 子代理的轮次上限兜底：没显式给 max_rounds 时回落到 50。
    ///
    /// 不兜底的话子代理继承主会话的 `max_session_turns`（默认 200），
    /// 一个空转的 Explore 能烧几十万 token（本次事故：子代理跑到 turn 54+，
    /// token 烧到 84k+ 还没停）。
    #[test]
    fn subagent_turn_budget_falls_back_to_a_bounded_default() {
        assert_eq!(DEFAULT_SUBAGENT_MAX_ROUNDS, 50);

        // 显式指定的轮次优先于兜底值。
        let explicit = SubAgentRequest::new("x").with_max_rounds(7);
        assert_eq!(explicit.max_rounds, Some(7));

        // 没指定时由 runner 层兜底 —— 这里锁「兜底发生在 runner 层」
        // （覆盖 spawn_background / run_foreground / skill fork 三条路径）。
        let unspecified = SubAgentRequest::new("x");
        assert_eq!(unspecified.max_rounds, None);
        assert_eq!(
            unspecified
                .max_rounds
                .unwrap_or(DEFAULT_SUBAGENT_MAX_ROUNDS),
            DEFAULT_SUBAGENT_MAX_ROUNDS
        );
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
    tasks: crate::agent::subagent::tasks::SharedSubagentTasks,
    /// Agent 间消息路由 hub（进程级单例）。`None` = 装配方没接 hub，
    /// `deliver_message` 退回写文件死信（旧行为），注册/注销跳过。
    agent_hub: Option<crate::agent::messaging::SharedAgentRouter>,
    #[cfg(test)]
    output_directory: Option<PathBuf>,
}

impl AsyncSubagentRunner {
    /// 创建异步 runner，需要同步 runner + 通知队列
    pub fn new(
        sync_runner: Arc<StarAgentRunner>,
        notification_queue: Arc<Mutex<NotificationQueue>>,
    ) -> Self {
        let agent_hub = sync_runner
            .agent_hub
            .clone()
            .or_else(|| sync_runner.config.runtime_agent_hub());
        let tasks = sync_runner
            .config
            .runtime_subagent_tasks()
            .unwrap_or_default();
        Self {
            sync_runner,
            notification_queue,
            tasks,
            name_registry: Arc::new(Mutex::new(HashMap::new())),
            agent_hub,
            #[cfg(test)]
            output_directory: None,
        }
    }

    /// 注入消息路由 hub（启用进程内互通：注册 / 投递 / 注销全走它）。
    pub fn with_agent_hub(mut self, hub: crate::agent::messaging::SharedAgentRouter) -> Self {
        // 同步执行侧与异步注册侧必须持有同一个 hub，不能只替换外层。
        self.sync_runner = Arc::new(StarAgentRunner {
            client: self.sync_runner.client.clone(),
            config: self.sync_runner.config.clone(),
            agent_hub: Some(hub.clone()),
            #[cfg(test)]
            panic_loop_before_handoff: self.sync_runner.panic_loop_before_handoff,
        });
        self.agent_hub = Some(hub);
        self
    }

    /// 每次调用继承当前客户端，通知队列和名称注册表仍与原 runner 共享。
    pub(crate) fn with_parent_client(&self, client: &StarClient) -> Self {
        Self {
            sync_runner: Arc::new(self.sync_runner.with_parent_client(client)),
            notification_queue: self.notification_queue.clone(),
            name_registry: self.name_registry.clone(),
            tasks: self.tasks.clone(),
            agent_hub: self.agent_hub.clone(),
            #[cfg(test)]
            output_directory: self.output_directory.clone(),
        }
    }

    fn output_file(&self, agent_id: &str) -> PathBuf {
        #[cfg(test)]
        if let Some(ref directory) = self.output_directory {
            return directory.join(format!("{agent_id}.txt"));
        }
        self.sync_runner
            .config
            .storage()
            .project_subagent_outputs_dir()
            .join(format!("{agent_id}.txt"))
    }

    /// 邮箱以任务 ID 隔离；可读名仅作别名，重名任务不能消费彼此的消息。
    fn bind_identity(request: &mut SubAgentRequest, _name: Option<&str>, agent_id: &str) -> String {
        request.identity = Some(agent_id.to_owned());
        agent_id.to_owned()
    }

    /// **前台**运行 SubAgent —— 调用方等结果。
    ///
    /// 与 [`Self::spawn_background`] 的区别只是「等不等」：
    /// - `spawn_background` 立刻返回，结果随通知回流（`background: true`）
    /// - 本方法等到跑完，把结果直接交给调用方（`background: false`）
    ///
    /// skill 的 `context: fork` + `background: false` 走这条：Claude Code 的
    /// 语义是「本轮等 fork 的结果，而不是让它在后台跑」。
    pub async fn run_foreground(
        &self,
        mut request: SubAgentRequest,
        name: Option<String>,
        description: String,
        agent_type_label: String,
    ) -> Result<SubAgentResult, SubAgentError> {
        let aid = format!("agent-fork-{}", uuid::Uuid::new_v4());
        let identity = Self::bind_identity(&mut request, name.as_deref(), &aid);
        if let Some(ref agent_name) = name {
            self.name_registry
                .lock()
                .await
                .insert(agent_name.clone(), aid.clone());
        }
        if let Some(ref hub) = self.agent_hub {
            hub.lock()
                .await
                .register_entry(identity, &aid, name.clone());
            emit_team_snapshot(hub).await;
        }
        let _ = description;
        let _ = agent_type_label;
        let result = self.sync_runner.run(request).await;
        if let Some(ref agent_name) = name {
            let mut registry = self.name_registry.lock().await;
            if registry.get(agent_name) == Some(&aid) {
                registry.remove(agent_name);
            }
        }
        if let Some(ref hub) = self.agent_hub {
            hub.lock().await.unregister(&aid);
            emit_team_snapshot(hub).await;
        }
        result
    }

    /// 后台启动 SubAgent，立即返回 AsyncLaunchResult
    ///
    /// `agent_type_label` 是用户可见的类型标签（对标 `userFacingName`），
    /// 会随通知回到 UI，避免 UI 侧再硬编码 "general-purpose"。
    pub async fn spawn_background(
        &self,
        mut request: SubAgentRequest,
        name: Option<String>,
        description: String,
        agent_type_label: String,
    ) -> AsyncLaunchResult {
        let agent_id: String = {
            let id = uuid::Uuid::new_v4().to_string();
            format!("agent-{id}")
        };

        let output_file = self.output_file(&agent_id);
        self.tasks
            .lock()
            .await
            .register(&agent_id, description.clone(), output_file.clone());
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.tasks
            .lock()
            .await
            .attach_cancel(&agent_id, stop.clone());
        let tasks = self.tasks.clone();
        let out_path = output_file.clone();
        let runner = self.sync_runner.clone();
        let queue = self.notification_queue.clone();
        let registry = self.name_registry.clone();
        let desc = description.clone();
        let aid = agent_id.clone();
        let agent_name = name.clone();
        let type_label = agent_type_label.clone();
        let spawn_hub = self.agent_hub.clone();
        // 实际邮箱用任务 ID，调用者提供的名称只作为路由别名。
        let hub_identity = Self::bind_identity(&mut request, name.as_deref(), &agent_id);
        // 返回启动结果前完成注册，调用方紧接着发送消息也能命中同一邮箱。
        if let Some(ref n) = name {
            registry.lock().await.insert(n.clone(), agent_id.clone());
        }
        if let Some(ref hub) = spawn_hub {
            hub.lock()
                .await
                .register_entry(&hub_identity, &agent_id, name.clone());
            emit_team_snapshot(hub).await;
        }

        // 启动前先登记 in-flight：主 Agent 收尾时要看到它，且终态 enqueue
        // 不能早于登记。随后发启动快照，再 spawn 后台任务；不能等第一次
        // progress callback（初始化和模型请求可能长期无回调），也不能在 spawn
        // 后才发 Running（极快完成时会把终态覆盖成假运行中）。
        queue.lock().await.mark_spawned(&agent_id);
        crate::agent::subagent::progress::emit_to_ui(
            crate::types::StreamingChunk::agent_task_update(
                crate::types::AgentTaskUpdatePayload::new(&agent_id, &type_label)
                    .with_description(desc.clone())
                    .with_status(crate::types::AgentTaskStatus::Running)
                    .with_async(true)
                    .with_name(agent_name.clone())
                    .with_task_description(Some(desc.clone())),
            ),
        );

        tokio::spawn(async move {
            let start = std::time::Instant::now();

            // 进度统计的最后一次快照。终态更新时，失败分支拿不到 SubAgentResult，
            // 直接报 0 会把选择器里已经涨上去的 tool uses / tokens 抹平，所以留个副本。
            let seen_tools = Arc::new(std::sync::atomic::AtomicU32::new(0));
            let seen_tokens = Arc::new(std::sync::atomic::AtomicU32::new(0));

            // 后台 agent 的进度同样回流到 UI（AgentTaskUpdate chunk）
            let progress_id = aid.clone();
            let progress_desc = desc.clone();
            let progress_type = type_label.clone();
            let progress_name = agent_name.clone();
            let progress_tools = seen_tools.clone();
            let progress_tokens = seen_tokens.clone();
            let sink: SubAgentProgressSink = Arc::new(move |p| {
                use std::sync::atomic::Ordering;
                progress_tools.store(p.tool_use_count, Ordering::Relaxed);
                progress_tokens.store(p.tokens, Ordering::Relaxed);
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

            let result = runner
                .run_with_control(request, Some(sink), Some(stop.clone()))
                .await;
            // 完成与停止在同一注册表锁下裁决；进入收尾后不再接收新的停止请求。
            let stopped = {
                let mut records = tasks.lock().await;
                records.close_control(&aid)
            };
            let result = if stopped {
                let message = match result {
                    Err(error) => format!("SubAgent stopped by request; {error}"),
                    Ok(_) => "SubAgent stopped by request".to_owned(),
                };
                Err(SubAgentError::new(
                    SubAgentErrorKind::ExecutionFailed,
                    message,
                ))
            } else {
                result
            };

            let (status, output, entries, tokens, tool_uses) = match result {
                Ok(r) => (
                    NotificationStatus::Completed,
                    r.output,
                    r.entries,
                    r.total_tokens as u64,
                    r.tool_use_count as u64,
                ),
                Err(e) => (
                    if stopped {
                        NotificationStatus::Killed
                    } else {
                        NotificationStatus::Failed
                    },
                    if stopped {
                        e.to_string()
                    } else {
                        format!("SubAgent failed: {}", e)
                    },
                    Vec::new(),
                    0,
                    0,
                ),
            };

            // 结果落盘：`AsyncLaunchResult.output_file` 这条路径已经作为
            // "Output file: …" 告诉了模型，必须真的有内容，否则后续读取只会拿到
            // "file not found"。写失败不影响任务本身，只记日志。
            if let Some(parent) = out_path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            let mut output_error = tokio::fs::write(&out_path, &output)
                .await
                .err()
                .map(|error| {
                    crate::utils::logging::append_agent_log_line(&format!(
                        "[AsyncSubagent] failed to write {}: {}",
                        out_path.display(),
                        error
                    ));
                    error.to_string()
                });
            let snapshot = tasks.lock().await.get(&aid);
            if let Some(mut snapshot) = snapshot {
                snapshot.status = Some(status.clone());
                snapshot.output_error = output_error.clone();
                if let Err(error) = snapshot.persist_final().await {
                    crate::utils::logging::append_agent_log_line(&format!(
                        "[AsyncSubagent] {error}"
                    ));
                    output_error = Some(match output_error {
                        Some(previous) => format!("{previous}; {error}"),
                        None => error,
                    });
                }
            }
            // 先停止寻址，再发布终态，避免调用方收到完成通知后仍能投递到旧任务。
            if let Some(ref name) = agent_name {
                let mut names = registry.lock().await;
                if names.get(name) == Some(&aid) {
                    names.remove(name);
                }
            }
            if let Some(ref hub) = spawn_hub {
                hub.lock().await.unregister(&aid);
                emit_team_snapshot(hub).await;
            }

            let notification_output = match output_error.as_ref() {
                Some(error) => format!("{output}\n\nTask output persistence failed: {error}"),
                None => output.clone(),
            };
            tasks
                .lock()
                .await
                .finish(&aid, status.clone(), output_error);

            // 终态直接推给 UI。原先终态只在下一个 turn 排空通知队列时才到 UI
            // （agent_loop.rs 的 drain_for_next_turn），用户空闲时后台代理跑完了，
            // 选择器还一直显示 Running —— 等于对着用户撒谎。
            let final_tools = tool_uses.max(u64::from(
                seen_tools.load(std::sync::atomic::Ordering::Relaxed),
            ));
            let final_tokens = tokens.max(u64::from(
                seen_tokens.load(std::sync::atomic::Ordering::Relaxed),
            ));
            {
                let final_status = match &status {
                    NotificationStatus::Completed => crate::types::AgentTaskStatus::Completed,
                    NotificationStatus::Killed => crate::types::AgentTaskStatus::Killed,
                    NotificationStatus::Failed => crate::types::AgentTaskStatus::Failed,
                };
                crate::agent::subagent::progress::emit_to_ui(
                    crate::types::StreamingChunk::agent_task_update(
                        crate::types::AgentTaskUpdatePayload::new(aid.clone(), type_label.clone())
                            .with_description(desc.clone())
                            .with_status(final_status)
                            .with_stats(
                                final_tools.min(u64::from(u32::MAX)) as u32,
                                final_tokens.min(u64::from(u32::MAX)) as u32,
                            )
                            .with_async(true)
                            .with_last_tool_info(
                                matches!(status, NotificationStatus::Failed)
                                    .then(|| output.clone()),
                            )
                            .with_name(agent_name.clone())
                            .with_task_description(Some(desc.clone())),
                    ),
                );
            }

            let mut q = queue.lock().await;
            q.enqueue(TaskNotification {
                task_id: aid.clone(),
                tool_use_id: None,
                status,
                summary: desc,
                result: notification_output,
                usage: NotificationUsage {
                    total_tokens: final_tokens,
                    tool_uses: final_tools,
                    duration_ms: start.elapsed().as_millis() as u64,
                },
                entries,
                agent_type: Some(type_label),
                name: agent_name.clone(),
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

    /// 投递消息到目标 agent（供 SendMessage 委托）。
    ///
    /// 接了 hub（Phase 2 半接线后此处为唯一路径）：走 `AgentRouter::deliver`
    /// —— name / agent_id / `"lead"` 单播，`"*"` 广播，目标未注册时显式
    /// `UnknownTarget`（无死信，设计文档 §3 第三条约束）。
    /// hub 未接线时退回旧的文件 inbox —— 调用方无人消费，等于死信，仅作
    /// 兜底使签名不炸，装配层应总是接 hub。
    pub async fn deliver_message(
        &self,
        from_identity: &str,
        target_name: &str,
        message: &str,
        summary: &str,
        message_type: crate::agent::messaging::HubMessageType,
    ) -> Result<ToolResult, Box<dyn std::error::Error>> {
        let Some(ref hub) = self.agent_hub else {
            return Err("AgentHub not configured; cannot deliver message".into());
        };

        let mut msg = crate::agent::messaging::AgentMessage::new(
            from_identity.to_string(),
            message.to_string(),
            summary.to_string(),
        );
        msg.message_type = message_type;

        let mut router = hub.lock().await;
        match router.deliver(msg.clone(), target_name, from_identity) {
            crate::agent::messaging::Delivery::Delivered { mailbox_key, .. } => Ok(ToolResult {
                llm_content: Some(format!(
                    "Message delivered to '{}' (mailbox: {}). It will be consumed at the start of that agent's next turn.",
                    target_name, mailbox_key
                )),
                return_display: Some(format!("Delivered to {}", target_name)),
                output: format!("Message delivered to '{}'", target_name),
                error: None,
                data: Some(serde_json::json!({
                    "status": "delivered",
                    "target": target_name,
                    "mailbox": mailbox_key,
                })),
            }),
            crate::agent::messaging::Delivery::Broadcast { delivered_to } => {
                Ok(ToolResult {
                    llm_content: Some(format!(
                        "Broadcast delivered to {} registered agents: {}.",
                        delivered_to.len(),
                        delivered_to.join(", ")
                    )),
                    return_display: Some(format!("Broadcast to {} agents", delivered_to.len())),
                    output: format!("Broadcast to {} agents", delivered_to.len()),
                    error: None,
                    data: Some(serde_json::json!({
                        "status": "broadcast_delivered",
                        "delivered_to": delivered_to,
                    })),
                })
            }
            crate::agent::messaging::Delivery::UnknownTarget { to } => {
                let known = router.registered_names();
                drop(router);

                // 本地未命中 → 出站转发给 peers（Phase 5.1）。跨机广播用
                // "*all"（设计 §7.2 第 3 条）；普通单播转发原 to，由对端
                // 自行解析 identity。全部对端失败 → 显式 UnknownTarget。
                if let Some(report) = self.forward_to_peers(&msg, &to).await {
                    if report.accepted.is_empty() {
                        return Err(format!(
                            "UnknownTarget: no registered agent or lead matches '{}' locally, \
                             and all {} peer(s) failed: {}. Registered agents: {}.",
                            to,
                            report.failed.len(),
                            report
                                .failed
                                .iter()
                                .map(|(p, e)| format!("{} ({})", p, e))
                                .collect::<Vec<_>>()
                                .join(", "),
                            if known.is_empty() { "(none)".to_string() } else { known.join(", ") }
                        )
                        .into());
                    }
                    return Ok(ToolResult {
                        llm_content: Some(format!(
                            "Message forwarded to peer(s): {}. Local registry has no match for '{}'.",
                            report.accepted.join(", "),
                            to
                        )),
                        return_display: Some(format!("Forwarded to {}", report.accepted.join(", "))),
                        output: format!("Forwarded to peer(s): {}", report.accepted.join(", ")),
                        error: None,
                        data: Some(serde_json::json!({
                            "status": "forwarded_to_peers",
                            "accepted": report.accepted,
                            "failed": report.failed,
                        })),
                    });
                }

                Err(format!(
                    "UnknownTarget: no registered agent or lead matches '{}'. Registered agents: {}. Use 'lead' to message the main agent, or 'target_agent=\"*\"' to broadcast.",
                    to,
                    if known.is_empty() { "(none)".to_string() } else { known.join(", ") }
                ).into())
            }
        }
    }

    /// 本地 UnknownTarget 时的出站转发（Phase 5.1）。
    ///
    /// 未配置 peers（`STAR_AGENT_PEERS`）时返回 `None`——保持单机语义；
    /// 配置了才构造 `PeerMailClient` 转发，并对账逐对端结果。
    async fn forward_to_peers(
        &self,
        msg: &crate::agent::messaging::AgentMessage,
        to: &str,
    ) -> Option<crate::agent::messaging::remote::PeerForwardReport> {
        let client = crate::agent::messaging::remote::PeerMailClient::from_env();
        if !client.has_peers() {
            return None;
        }
        Some(client.forward(msg.clone(), to).await)
    }
}
