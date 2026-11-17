use crate::agent::subagent::progress::{emit_to_ui, SubAgentProgressSink};
use crate::agent::subagent::router::{route_agent_call, AgentRoute};
use crate::agent::subagent::runner::AsyncSubagentRunner;
use crate::core::agents::{
    agent_type_label, AgentToolFullInput, SharedSubAgentRunner, SubAgentErrorKind, SubAgentRequest,
    SubagentType,
};
use crate::core::tools::tools::{BaseDeclarativeTool, ToolInvocation, ToolLocation, ToolResult};
use crate::types::{AgentTaskStatus, AgentTaskUpdatePayload, StreamingChunk};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

/// 同步 Agent 未显式指定 max_rounds 时的默认轮次上限
const DEFAULT_SYNC_MAX_ROUNDS: u32 = 50;

fn deserialize_effort<'de, D>(
    deserializer: D,
) -> Result<Option<crate::types::ThinkingEffort>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match Option::<String>::deserialize(deserializer)? {
        Some(value) => crate::types::ThinkingEffort::parse_skill_effort(&value)
            .map(Some)
            .ok_or_else(|| serde::de::Error::custom("invalid thinking effort")),
        None => Ok(None),
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct AgentToolParams {
    pub description: String,
    pub prompt: String,
    #[serde(default)]
    pub subagent_type: Option<SubagentType>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub background: Option<bool>,
    #[serde(default)]
    pub max_rounds: Option<u32>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub resume: Option<String>,
    /// 子代理本次调用的思考力度；省略时继承调用 Agent 的冻结档位。
    #[serde(default, deserialize_with = "deserialize_effort")]
    pub effort: Option<crate::types::ThinkingEffort>,
    /// **调用方**所处的递归深度 —— 本子代理落在 `depth + 1`。
    ///
    /// 由执行器（`ToolExecutor::normalize_agent_args`）覆盖写入，不在参数
    /// schema 里，模型看不到也伪造不了。它必须由**真正发起这次调用的
    /// 那个 agent** 提供：`AgentTool` 装在共享的 `ToolRegistry` 里，它自己
    /// 持有的 runner 是父级的，拿它算深度会永远停在父级那一层。
    #[serde(default, rename = "_parent_depth")]
    pub parent_depth: usize,
}

impl From<AgentToolParams> for AgentToolFullInput {
    fn from(p: AgentToolParams) -> Self {
        Self {
            description: p.description,
            prompt: p.prompt,
            subagent_type: p.subagent_type,
            name: p.name,
            isolation: None,
            model: p.model,
            resume: p.resume,
            effort: p.effort,
            background: p.background,
            max_rounds: p.max_rounds,
            parent_depth: p.parent_depth,
        }
    }
}

pub struct AgentTool {
    runner: SharedSubAgentRunner,
    async_runner: Option<Arc<AsyncSubagentRunner>>,
}

impl AgentTool {
    pub fn new(runner: SharedSubAgentRunner) -> Self {
        Self {
            runner,
            async_runner: None,
        }
    }

    /// 注入异步 runner（启用后台执行能力）
    pub fn with_async_runner(mut self, async_runner: Arc<AsyncSubagentRunner>) -> Self {
        self.async_runner = Some(async_runner);
        self
    }
}

impl BaseDeclarativeTool for AgentTool {
    fn name(&self) -> &str {
        "Agent"
    }

    fn display_name(&self) -> &str {
        "Agent"
    }

    fn description(&self) -> &str {
        "Launch a new agent (SubAgent) to perform a complex task. \
         Use subagent_type to select a specialized agent (explorer/analyzer/editor/code_reviewer). \
         Set background=true for long-running async tasks. \
         Set name to enable send_message communication."
    }

    fn kind(&self) -> crate::core::tools::tools::Kind {
        crate::core::tools::tools::Kind::Read
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn parameter_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "A short 3-5 word description of the task (e.g. 'fix auth bug', 'review parser'). REQUIRED."
                },
                "prompt": {
                    "type": "string",
                    "description": "The detailed task description for the agent. Be specific about expected output."
                },
                "subagent_type": {
                    "type": "string",
                    "enum": ["general_purpose", "explorer", "analyzer", "editor", "code_reviewer"],
                    "description": "Type of specialized agent to use. Defaults to the source type when resuming, otherwise general_purpose."
                },
                "name": {
                    "type": "string",
                    "description": "Optional name for this agent instance (enables send_message targeting)."
                },
                "background": {
                    "type": "boolean",
                    "description": "Run in background; completion arrives via <task-notification>. Explorer defaults to true, other agent types to false. Set false to wait for an Explorer result."
                },
                "model": {
                    "type": "string",
                    "description": "Optional model ID on the current provider. Omit or use inherit to keep the parent model; this does not switch providers."
                },
                "resume": {
                    "type": "string",
                    "description": "Continue from an ended agent ID in this project using its last saved native context. A new task ID is created; current permissions, provider and working directory apply. Missing snapshots are errors. Cannot combine with a different subagent type or fork."
                },
                "effort": {
                    "type": "string",
                    "enum": ["off", "low", "medium", "high", "xhigh", "max"],
                    "description": "Thinking effort for this agent call. Omit to inherit the caller's current turn setting."
                },
                "max_rounds": {
                    "type": "integer",
                    "description": "Maximum conversation turns for this agent. Default: 50."
                }
            },
            "required": ["description", "prompt"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: AgentToolParams = serde_json::from_value(params)?;
        if params
            .resume
            .as_deref()
            .is_some_and(|id| !crate::agent::subagent::session::valid_task_id(id))
        {
            return Err("Invalid agent session ID".into());
        }
        Ok(Box::new(AgentToolInvocation {
            full_input: params.into(),
            runner: self.runner.clone(),
            async_runner: self.async_runner.clone(),
            parent_context: Arc::new(Vec::new()),
            parent_thinking_effort: None,
        }))
    }
}

pub struct AgentToolInvocation {
    full_input: AgentToolFullInput,
    runner: SharedSubAgentRunner,
    async_runner: Option<Arc<AsyncSubagentRunner>>,
    parent_context: Arc<Vec<crate::types::StarMessage>>,
    parent_thinking_effort: Option<crate::types::ThinkingEffort>,
}

impl AgentToolInvocation {
    fn inherit_parent_effort(&self, input: &mut AgentToolFullInput) {
        if input.effort.is_none() {
            input.effort = self.parent_thinking_effort.clone();
        }
    }

    fn fork_request(&self, request: SubAgentRequest, name: &str) -> SubAgentRequest {
        request
            .with_parent_context(self.parent_context.as_ref().clone(), Vec::new())
            .with_identity(name)
    }
}

impl ToolInvocation for AgentToolInvocation {
    fn set_agent_parent_client(&mut self, client: &crate::llm::client::StarClient) {
        self.runner = Arc::new(self.runner.with_parent_client(client));
        self.async_runner = self
            .async_runner
            .as_ref()
            .map(|runner| Arc::new(runner.with_parent_client(client)));
    }

    fn set_agent_parent_context(&mut self, messages: Arc<Vec<crate::types::StarMessage>>) {
        self.parent_context = messages;
    }

    fn set_agent_parent_thinking_effort(&mut self, effort: crate::types::ThinkingEffort) {
        if self.full_input.effort.is_none() {
            self.parent_thinking_effort = Some(effort);
        }
    }

    fn get_description(&self) -> String {
        format!(
            "Agent [{}]: {}",
            self.full_input.description, self.full_input.prompt
        )
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let mut input = self.full_input.clone();
        self.inherit_parent_effort(&mut input);
        let runner = self.runner.clone();
        let async_runner = self.async_runner.clone();

        Box::pin(async move {
            runner.resolve_resume_type(&mut input).await?;
            let route = route_agent_call(&input, false, false);

            match route {
                // 同步路径：阻塞等待，但实时把进度回流到父级 UI
                AgentRoute::SyncNamedAgent {
                    agent_id,
                    subagent_type,
                    request,
                } => {
                    let type_label = subagent_type.user_facing_name().to_string();
                    let description = input.description.clone();
                    let name = input.name.clone();

                    // 建条目：让 UI 立即出现 "Initializing…" 的 AgentTask
                    emit_to_ui(StreamingChunk::agent_task_update(
                        AgentTaskUpdatePayload::new(&agent_id, &type_label)
                            .with_description(&description)
                            .with_status(AgentTaskStatus::Running)
                            .with_name(name.clone()),
                    ));

                    // 进度回调：每条子消息都刷新统计 + 最近工具摘要
                    let sink: SubAgentProgressSink = {
                        let agent_id = agent_id.clone();
                        let type_label = type_label.clone();
                        let description = description.clone();
                        let name = name.clone();
                        Arc::new(move |p| {
                            emit_to_ui(StreamingChunk::agent_task_update(
                                AgentTaskUpdatePayload::new(&agent_id, &type_label)
                                    .with_description(&description)
                                    .with_status(AgentTaskStatus::Running)
                                    .with_stats(p.tool_use_count, p.tokens)
                                    .with_last_tool_info(p.last_tool_info.clone())
                                    .with_name(name.clone())
                                    .with_sub_entries(p.new_entries.clone()),
                            ));
                        })
                    };

                    // max_rounds 现在真正生效：调用方给了就用，否则回落默认值
                    let rounds = request.max_rounds.unwrap_or(DEFAULT_SYNC_MAX_ROUNDS);
                    let request = request
                        .with_max_rounds(rounds)
                        .with_identity(agent_id.clone());

                    match runner.run_with_progress(request, Some(sink)).await {
                        Ok(result) => {
                            emit_to_ui(StreamingChunk::agent_task_update(
                                AgentTaskUpdatePayload::new(&agent_id, &type_label)
                                    .with_description(&description)
                                    .with_status(AgentTaskStatus::Completed)
                                    .with_stats(result.tool_use_count, result.total_tokens)
                                    .with_last_tool_info(result.last_tool_info.clone())
                                    .with_name(name.clone()),
                            ));
                            Ok(ToolResult {
                                llm_content: None,
                                return_display: None,
                                output: result.output,
                                error: None,
                                data: Some(serde_json::json!({
                                    "status": "sync_completed",
                                    "agent_id": agent_id,
                                    "agent_type": type_label,
                                    "name": name,
                                    "tool_use_count": result.tool_use_count,
                                    "total_tokens": result.total_tokens,
                                })),
                            })
                        }
                        Err(err) => {
                            // 递归超限属于「被拒绝」语义（对标
                            // renderToolUseRejectedMessage），其余算执行失败
                            let status = if err.kind == SubAgentErrorKind::RecursionLimitExceeded {
                                AgentTaskStatus::Rejected
                            } else {
                                AgentTaskStatus::Failed
                            };
                            emit_to_ui(StreamingChunk::agent_task_update(
                                AgentTaskUpdatePayload::new(&agent_id, &type_label)
                                    .with_description(&description)
                                    .with_status(status)
                                    .with_last_tool_info(Some(err.message.clone()))
                                    .with_name(name.clone()),
                            ));

                            let error_type = match err.kind {
                                SubAgentErrorKind::RecursionLimitExceeded => {
                                    "RecursionLimitExceeded".to_string()
                                }
                                SubAgentErrorKind::InitializationFailed
                                | SubAgentErrorKind::ExecutionFailed => {
                                    "SubAgentExecutionError".to_string()
                                }
                            };
                            Ok(ToolResult {
                                llm_content: Some(err.message.clone()),
                                return_display: Some(err.message.clone()),
                                output: err.message.clone(),
                                error: Some(crate::core::tools::tools::ToolError {
                                    error_type,
                                    message: err.message,
                                }),
                                data: Some(serde_json::json!({
                                    "status": "failed",
                                    "agent_id": agent_id,
                                    "agent_type": type_label,
                                })),
                            })
                        }
                    }
                }

                // 异步路径
                AgentRoute::AsyncAgent {
                    agent_id: _,
                    request,
                    name,
                } => {
                    let async_runner = async_runner
                        .ok_or_else(|| "AsyncSubagentRunner not configured".to_string())?;

                    let type_label = agent_type_label(input.subagent_type.as_ref()).to_string();

                    // 命名后台 agent 拿 name 作 mailbox 身份（对标 CC：Agent 的
                    // `name` 参数即注册进隐式 team 的可寻址名，`name@team` 里的
                    // name 部分）。匿名后台 agent 不带 identity，spawn 时回落
                    // agent_id —— SendMessage 仍可按 agent_id 单播它。
                    let request = match name.as_ref() {
                        Some(n) => request.with_identity(n.clone()),
                        None => request,
                    };
                    let launch = async_runner
                        .spawn_background(
                            request,
                            name.clone(),
                            input.description.clone(),
                            type_label.clone(),
                        )
                        .await;

                    Ok(ToolResult {
                        llm_content: Some(format!(
                            "Agent launched in background.\nagent_id: {}\n{}\nOutput file: {}\nWait for <task-notification> for results.",
                            launch.agent_id,
                            name.as_ref().map(|n| format!("Addressable as '{}' via SendMessage (it can reply with SendMessage target_agent='lead').", n))
                                .unwrap_or_default(),
                            launch.output_file.display(),
                        )),
                        return_display: Some(format!("Agent {} started", launch.agent_id)),
                        output: String::new(),
                        error: None,
                        data: Some(serde_json::json!({
                            "status": "async_launched",
                            "agent_id": launch.agent_id,
                            "agent_type": type_label,
                            "name": name,
                            "description": input.description,
                            "output_file": launch.output_file.to_string_lossy(),
                        })),
                    })
                }

                // Coordinator Worker 路径
                AgentRoute::CoordinatorWorker { agent_id, request } => {
                    let async_runner = async_runner.ok_or_else(|| {
                        "AsyncSubagentRunner not configured for coordinator".to_string()
                    })?;

                    let type_label = "Worker".to_string();
                    // Coordinator worker 以 agent_id 为 mailbox 身份注册进 hub：
                    // SendMessage 可按 agent_id 单播，worker 可回话给 "lead"。
                    // （lead → worker 的任务指派正文已在 prompt 里，无需再发。）
                    let request = request.with_identity(agent_id.clone());
                    let launch = async_runner
                        .spawn_background(
                            request,
                            None,
                            input.description.clone(),
                            type_label.clone(),
                        )
                        .await;

                    Ok(ToolResult {
                        llm_content: Some(format!(
                            "Coordinator worker launched.\nagent_id: {}\nResults will arrive via <task-notification>.",
                            launch.agent_id,
                        )),
                        return_display: None,
                        output: String::new(),
                        error: None,
                        data: Some(serde_json::json!({
                            "status": "async_launched",
                            "agent_id": launch.agent_id,
                            "agent_type": type_label,
                            "description": input.description,
                            "coordinator_worker": true,
                        })),
                    })
                }

                AgentRoute::ForkAgent { agent_id, request } => {
                    let async_runner = async_runner
                        .ok_or_else(|| "AsyncSubagentRunner not configured for fork".to_string())?;

                    let inherited_messages = self.parent_context.len();
                    let fork_name = input
                        .name
                        .clone()
                        .unwrap_or_else(|| format!("fork-{}", agent_id));
                    // 与 /subtask 共用安装路径，保留消息角色、工具结果和完整文本。
                    let request_with_id = self.fork_request(request.base, &fork_name);
                    let launch = async_runner
                        .spawn_background(
                            request_with_id,
                            Some(fork_name.clone()),
                            request.description.clone(),
                            "Fork".to_string(),
                        )
                        .await;

                    Ok(ToolResult {
                        llm_content: Some(format!(
                            "Fork agent launched.\nagent_id: {}\nAddressable as '{}' via SendMessage.\nInherited {} parent messages.\nResults will arrive via <task-notification>.",
                            launch.agent_id,
                            fork_name,
                            inherited_messages,
                        )),
                        return_display: Some(format!("Fork {} started", agent_id)),
                        output: String::new(),
                        error: None,
                        data: Some(serde_json::json!({
                            "status": "fork_launched",
                            "agent_id": launch.agent_id,
                            "agent_type": "Fork",
                            "name": fork_name,
                            "description": request.description,
                            "inherited_messages": inherited_messages,
                            "output_file": launch.output_file.to_string_lossy(),
                        })),
                    })
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invocation_model_reaches_every_request_route() {
        let params: AgentToolParams = serde_json::from_value(json!({
            "description": "inspect code", "prompt": "find caller", "model": "child-model", "resume": "agent-old"
        }))
        .unwrap();
        let mut input: AgentToolFullInput = params.into();
        let mut invocation = AgentToolInvocation {
            full_input: input.clone(),
            runner: crate::agent::subagent::runner::StarAgentRunner::shared(
                crate::llm::client::StarClient::new("", None, None, None, None),
                Arc::new(crate::core::config::Config::new(
                    crate::core::config::ConfigParameters::default(),
                )),
            ),
            async_runner: None,
            parent_context: Arc::new(Vec::new()),
            parent_thinking_effort: None,
        };
        invocation.set_agent_parent_thinking_effort(crate::types::ThinkingEffort::High);
        invocation.inherit_parent_effort(&mut input);
        assert_eq!(input.effort, Some(crate::types::ThinkingEffort::High));
        for background in [false, true] {
            input.background = Some(background);
            let request = match route_agent_call(&input, false, false) {
                AgentRoute::SyncNamedAgent { request, .. }
                | AgentRoute::AsyncAgent { request, .. } => request,
                other => panic!("unexpected route: {other:?}"),
            };
            assert_eq!(request.model.as_deref(), Some("child-model"));
            assert_eq!(request.effort, Some(crate::types::ThinkingEffort::High));
            assert_eq!(request.resume.as_deref(), Some("agent-old"));
        }
        let explicit = AgentToolFullInput {
            effort: Some(crate::types::ThinkingEffort::Off),
            background: Some(false),
            ..input
        };
        let request = match route_agent_call(&explicit, false, false) {
            AgentRoute::SyncNamedAgent { request, .. } => request,
            other => panic!("expected synchronous route, got {other:?}"),
        };
        assert_eq!(request.effort, Some(crate::types::ThinkingEffort::Off));
    }

    #[test]
    fn fork_invocation_preserves_structured_history_without_truncation() {
        let config = Arc::new(crate::core::config::Config::new(
            crate::core::config::ConfigParameters::default(),
        ));
        let runner = crate::agent::subagent::runner::StarAgentRunner::shared(
            crate::llm::client::StarClient::new("", None, None, None, None),
            config,
        );
        let params: AgentToolParams = serde_json::from_value(json!({
            "description": "fork auth investigation", "prompt": "check callers"
        }))
        .unwrap();
        let mut invocation = AgentToolInvocation {
            full_input: params.into(),
            runner,
            async_runner: None,
            parent_context: Arc::new(Vec::new()),
            parent_thinking_effort: None,
        };
        let mut assistant = crate::types::StarMessage::assistant("x".repeat(1200));
        assistant.tool_calls = Some(vec![crate::types::StarToolCall {
            id: "read-auth".into(),
            call_type: "function".into(),
            function: crate::types::StarToolCallFunction {
                name: "Read".into(),
                arguments: r#"{"file_path":"auth.rs"}"#.into(),
            },
        }]);
        let history = vec![
            crate::types::StarMessage::user("investigate auth"),
            assistant,
            crate::types::StarMessage::tool("read-auth", "auth source"),
        ];
        invocation.set_agent_parent_context(Arc::new(history.clone()));
        invocation.set_agent_parent_thinking_effort(crate::types::ThinkingEffort::High);
        let mut inherited = invocation.full_input.clone();
        invocation.inherit_parent_effort(&mut inherited);
        assert_eq!(inherited.effort, Some(crate::types::ThinkingEffort::High));
        let inherited_request = match route_agent_call(&inherited, false, false) {
            AgentRoute::SyncNamedAgent { request, .. } => request,
            other => panic!("expected synchronous route, got {other:?}"),
        };
        assert_eq!(
            inherited_request.effort,
            Some(crate::types::ThinkingEffort::High)
        );

        let mut explicit_off = invocation.full_input.clone();
        explicit_off.effort = Some(crate::types::ThinkingEffort::Off);
        invocation.inherit_parent_effort(&mut explicit_off);
        assert_eq!(explicit_off.effort, Some(crate::types::ThinkingEffort::Off));

        let request = invocation.fork_request(
            SubAgentRequest::new("check callers")
                .with_model("fork-model")
                .with_effort(crate::types::ThinkingEffort::High),
            "fork-auth",
        );
        assert_eq!(request.model.as_deref(), Some("fork-model"));
        assert_eq!(request.prompt, "check callers");
        assert_eq!(request.effort, Some(crate::types::ThinkingEffort::High));
        assert_eq!(request.identity.as_deref(), Some("fork-auth"));
        assert_eq!(
            serde_json::to_value(&request.parent_messages).unwrap(),
            serde_json::to_value(&history).unwrap()
        );
    }
}
