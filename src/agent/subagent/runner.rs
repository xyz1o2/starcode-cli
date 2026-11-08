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

/// 把 `/subtask` 的父会话装进刚创建的子代理。
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

/// Explore 子代理的只读工具白名单（对标 CC 官方 docs 对 Explore agent 的
/// 定义：Read / Grep / Glob / 只读 Bash，禁一切可变更工具）。
///
/// 挂在子 config 的 `core_tools` 上 —— 那是注册层（`is_core_tool_enabled`）
/// 既有的一条门，覆盖它等于子代理的**注册面**收口，而不仅是模型侧提示。
/// `Bash` 保留在名单里：只读命令由 `is_readonly_fs_command` 在确认门把关，
/// 这里收掉 Write/Edit 一类的可变更工具就足够表达"只读"语义。
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
}

impl StarAgentRunner {
    pub fn new(client: StarClient, config: Arc<Config>) -> Self {
        let agent_hub = config.runtime_agent_hub();
        Self {
            client,
            config,
            agent_hub,
        }
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
    /// 按 `request.subagent_type` 派发：`Explorer` 走确定性检索
    /// （`ExploreAgent` 的 ACE 广搜 + 可选 deep filter），其余走下面的通用
    /// LLM 循环。之前 `subagent_type` 只当 UI 标签用，`explorer` 实际跑的是
    /// 通用 agent，真正检索专家要靠 `skill` 工具才能触发，而且那条路径不上
    /// UI——现在两条路并成一条，AgentTool 的进度回流对 explorer 同样生效。
    pub async fn run_with_progress(
        &self,
        request: SubAgentRequest,
        sink: Option<SubAgentProgressSink>,
    ) -> Result<SubAgentResult, SubAgentError> {
        // 子代理开始 —— 外部 harness 靠这个事件知道"多了一个干活的"。
        // 之前 `SubagentStop` 只在**失败**时触发，成功路径静悄悄，外部看不到
        // 子代理的完整生命周期。
        crate::agent::hooks::run_subagent_start_hook(
            "",
            &request.prompt,
            request.subagent_type.as_str(),
        )
        .await;

        if self.config.recursion_depth >= 3 {
            return Err(SubAgentError::new(
                SubAgentErrorKind::RecursionLimitExceeded,
                "Maximum SubAgent recursion depth reached (3). Cannot start another SubAgent.",
            ));
        }

        // Explorer 是确定性检索，不是 LLM 循环，单独走
        if request.subagent_type == crate::core::agents::SubagentType::Explorer {
            return self.run_explore(request, sink).await;
        }

        let mut sub_config = (*self.config).clone();
        sub_config.recursion_depth = sub_config.recursion_depth.saturating_add(1);
        // Explorer 走通用 LLM 循环（专用检索代理 2026-11-05 下线后），这里把
        // 子代理的**工具注册面**收口成只读白名单 —— 对标 CC 官方 docs 的
        // Explore agent（Read/Grep/Glob/只读 Bash）。core_tools 是注册层
        // `is_core_tool_enabled` 既有的门，不需要新增机制。
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
            Some(format!("agent-{}", &id[..8]))
        });
        sub_config.agent_identity = child_identity.clone();

        let mut agent = crate::agent::StarAgent::new_with_provider(
            &self.client.api_key,
            Some(self.client.model.clone()),
            Some(self.client.base_url.clone()),
            request.max_rounds,
            Some(self.client.is_openai_compatible),
            Some(Arc::new(sub_config)),
            // 继承父 client 的 provider id：否则子 agent 的协议推断和 key 环境变量
            // 解析都会退回按模型名猜，聚合网关上的模型会被发去厂商官方端点。
            self.client.provider_id.clone(),
        )
        .await
        .map_err(|e| {
            SubAgentError::new(
                SubAgentErrorKind::InitializationFailed,
                format!("Failed to create SubAgent: {}", e),
            )
        })?;

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
                format!("agent-{}", &id[..8])
            });
            agent.set_agent_hub(hub, identity);
        }

        // `/subtask` forks the current conversation rather than starting from a
        // blank child. Install the parent's native provider context before the
        // new task prompt is appended, so tool calls/results survive intact.
        install_parent_context(&mut agent, &request);

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

    /// Explorer 派发：**已下线**（2026-11-05）。
    ///
    /// `ExploreAgent` 是写死在二进制里的**内置**检索专家，它的 Rust 实现
    /// 已随其余内置子代理（analyzer / auto_fix / editor / navigator /
    /// verify）一起从 `src/agent/skills/` 删除。
    ///
    /// 现在 `explorer` 请求落到通用 LLM 循环：模型自己决定用哪些工具检索。
    /// 这条路径本来就能工作（`analyzer` / `editor` / `code_reviewer` 一直都
    /// 是这么跑的），只是少了"确定性广搜 + deep filter"这套固定编排。
    ///
    /// 用户的**自定义** agent（在 `.star/agents/` 定义）不受影响 ——
    /// 它们走 `SubAgentManager`，与这里无关。
    ///
    /// 保留这个函数（而不是直接删掉调用点）是为了让"explorer 已无专用实现"
    /// 这件事在代码里留有明确痕迹，而不是变成一条悄悄改了行为的路径。
    async fn run_explore(
        &self,
        request: SubAgentRequest,
        sink: Option<SubAgentProgressSink>,
    ) -> Result<SubAgentResult, SubAgentError> {
        if let Some(sink) = sink.as_ref() {
            sink(SubAgentProgress {
                last_tool_info: Some("Searching codebase…".to_string()),
                ..Default::default()
            });
        }

        // 内置检索专家已下线：改用通用 LLM 循环跑这个子代理。
        //
        // 这里刻意**不**走 `SubAgent::execute` —— 那是给 `SubAgentManager`
        // 里注册的 agent 用的接口（用户自定义 agent 走那条），而通用循环在
        // 上面的 `run_with_progress` 里已经跑完了，重复调用会跑两遍。
        let mut agent = crate::agent::StarAgent::new_with_provider(
            &self.client.api_key,
            Some(self.client.model.clone()),
            Some(self.client.base_url.clone()),
            request.max_rounds,
            Some(self.client.is_openai_compatible),
            Some(Arc::new(self.child_config())),
            // 继承父 client 的 provider id（见上方通用路径的同一处理）。
            self.client.provider_id.clone(),
        )
        .await
        .map_err(|e| {
            SubAgentError::new(
                SubAgentErrorKind::InitializationFailed,
                format!("Failed to create SubAgent: {}", e),
            )
        })?;

        // explorer 路径同样接 hub（前端进度模式无 mailbox 消费者，给一次性
        // 身份 —— 只是不让它回落成 "lead" 抢主 agent 的邮箱）。
        if let Some(hub) = self
            .agent_hub
            .clone()
            .or_else(|| self.config.runtime_agent_hub())
        {
            let id = uuid::Uuid::new_v4().to_string();
            agent.set_agent_hub(hub, format!("agent-{}", &id[..8]));
        }

        let entries = agent
            .process_user_message(&request.prompt)
            .await
            .map_err(|e| SubAgentError::new(SubAgentErrorKind::ExecutionFailed, e.to_string()))?;

        // `process_user_message` 返回 `Vec<ChatEntry>`（一条 assistant 条目）。
        // 子代理结果的 output 是纯文本，所以取那条条目的正文。
        let output = entries
            .iter()
            .map(|entry| entry.content.as_str())
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");

        Ok(SubAgentResult {
            output,
            entries: Vec::new(),
            tool_use_count: 0,
            total_tokens: 0,
            last_tool_info: Some("Search complete".to_string()),
        })
    }

    /// 子代理的递归深度配置（父配置 +1 层）。
    ///
    /// explorer 专用（通用循环的 explorer 已在上面 `run_with_progress` 的
    /// 分支里收口）：同样应用只读白名单，subtype 已知必为 Explorer。
    fn child_config(&self) -> crate::core::config::Config {
        let mut sub_config = (*self.config).clone();
        sub_config.recursion_depth = sub_config.recursion_depth.saturating_add(1);
        sub_config.core_tools = Some(
            EXPLORER_READONLY_TOOLS
                .iter()
                .map(|s| s.to_string())
                .collect(),
        );
        sub_config
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
    /// 且它的专用实现已经不在了（`run_explore` 里没有 `ExploreAgent`）。
    /// 派发路由本身的覆盖见 `router.rs`。
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
    /// Agent 间消息路由 hub（进程级单例）。`None` = 装配方没接 hub，
    /// `deliver_message` 退回写文件死信（旧行为），注册/注销跳过。
    agent_hub: Option<crate::agent::messaging::SharedAgentRouter>,
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
            agent_hub: None,
        }
    }

    /// 注入消息路由 hub（启用进程内互通：注册 / 投递 / 注销全走它）。
    pub fn with_agent_hub(mut self, hub: crate::agent::messaging::SharedAgentRouter) -> Self {
        self.agent_hub = Some(hub);
        self
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
        request: SubAgentRequest,
        name: Option<String>,
        description: String,
        agent_type_label: String,
    ) -> Result<SubAgentResult, SubAgentError> {
        if let Some(ref agent_name) = name {
            let aid = format!("agent-fork-{}", &uuid::Uuid::new_v4().to_string()[..8]);
            self.name_registry
                .lock()
                .await
                .insert(agent_name.clone(), aid.clone());
            self.register_in_hub(&aid, Some(agent_name));
        }
        let _ = description;
        let _ = agent_type_label;
        let result = self.sync_runner.run(request).await;
        if let Some(ref agent_name) = name {
            self.unregister_from_hub(agent_name);
        }
        result
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
        let out_path = output_file.clone();
        let runner = self.sync_runner.clone();
        let queue = self.notification_queue.clone();
        let registry = self.name_registry.clone();
        let desc = description.clone();
        let aid = agent_id.clone();
        let agent_name = name.clone();
        let type_label = agent_type_label.clone();
        let spawn_hub = self.agent_hub.clone();
        // 调用方显式指派的 mailbox 身份（`SubAgentRequest::with_identity`）；
        // 未指派时回落 name / agent_id（见下方注册处）。
        let hub_identity = request.identity.clone();

        tokio::spawn(async move {
            // 注册 name → agent_id 映射。放进 spawn 内用 async lock，
            // 避免在 async 上下文里调用 blocking_lock 触发 panic。
            if let Some(ref n) = agent_name {
                registry.lock().await.insert(n.clone(), aid.clone());
            }

            // 消息 hub 注册：此后 SendMessage 可按 name / agent_id 投递到本
            // worker；worker 自己每轮 turn 开头从 mailbox drain（Phase 3）。
            // 身份键与 worker 内部构造时用的 sub_config.agent_identity 同源
            // （request.identity → name → agent_id），保证 SendMessage 发件人
            // 与 mailbox 键一致。
            if let Some(ref hub) = spawn_hub {
                let identity = hub_identity
                    .clone()
                    .or_else(|| agent_name.clone())
                    .unwrap_or_else(|| aid.clone());
                let mut router = hub.lock().await;
                router.register_entry(&identity, &aid, agent_name.clone());
                crate::utils::logging::append_debug_log_line(&format!(
                    "[AgentHub] registered worker '{}' ({})",
                    identity, aid
                ));
                drop(router);
                emit_team_snapshot(hub).await;
            }

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

            // 结果落盘：`AsyncLaunchResult.output_file` 这条路径已经作为
            // "Output file: …" 告诉了模型，必须真的有内容，否则后续读取只会拿到
            // "file not found"。写失败不影响任务本身，只记日志。
            if let Some(parent) = out_path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            if let Err(e) = tokio::fs::write(&out_path, &output).await {
                crate::utils::logging::append_agent_log_line(&format!(
                    "[AsyncSubagent] failed to write {}: {}",
                    out_path.display(),
                    e
                ));
            }

            // 终态直接推给 UI。原先终态只在下一个 turn 排空通知队列时才到 UI
            // （agent_loop.rs 的 drain_for_next_turn），用户空闲时后台代理跑完了，
            // 选择器还一直显示 Running —— 等于对着用户撒谎。
            {
                use std::sync::atomic::Ordering;
                let final_status = match &status {
                    NotificationStatus::Completed => crate::types::AgentTaskStatus::Completed,
                    _ => crate::types::AgentTaskStatus::Failed,
                };
                let final_tools = (tool_uses as u32).max(seen_tools.load(Ordering::Relaxed));
                let final_tokens = (tokens as u32).max(seen_tokens.load(Ordering::Relaxed));
                crate::agent::subagent::progress::emit_to_ui(
                    crate::types::StreamingChunk::agent_task_update(
                        crate::types::AgentTaskUpdatePayload::new(aid.clone(), type_label.clone())
                            .with_description(desc.clone())
                            .with_status(final_status)
                            .with_stats(final_tools, final_tokens)
                            .with_async(true)
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
                result: output,
                usage: NotificationUsage {
                    total_tokens: tokens,
                    tool_uses,
                    duration_ms: start.elapsed().as_millis() as u64,
                },
                entries,
                agent_type: Some(type_label),
                name: agent_name.clone(),
            });

            // 终态注销：旧 agent_id 不再解析（对迟到的消息显式 UnknownTarget），
            // mailbox 保留 —— 迟到消息落进可见的空箱而不是复活一个幽灵。
            if let Some(ref hub) = spawn_hub {
                hub.lock().await.unregister(&aid);
                crate::utils::logging::append_debug_log_line(&format!(
                    "[AgentHub] unregistered worker '{}' ({})",
                    agent_name.as_deref().unwrap_or(&aid),
                    aid
                ));
                emit_team_snapshot(hub).await;
            }
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

    fn register_in_hub(&self, agent_id: &str, name: Option<&String>) {
        let Some(ref hub) = self.agent_hub else {
            return;
        };
        let hub = hub.clone();
        let identity = name
            .map(|n| n.to_string())
            .unwrap_or_else(|| agent_id.to_string());
        let aid = agent_id.to_string();
        let n = name.cloned();
        tokio::spawn(async move {
            hub.lock().await.register_entry(identity, aid, n);
        });
    }

    fn unregister_from_hub(&self, name: &str) {
        let Some(ref hub) = self.agent_hub else {
            return;
        };
        let hub = hub.clone();
        let n = name.to_string();
        tokio::spawn(async move {
            hub.lock().await.unregister(&n);
        });
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
