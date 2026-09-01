use crate::agent::subagent::router::{route_agent_call, AgentRoute};
use crate::agent::subagent::runner::AsyncSubagentRunner;
use crate::core::agents::{
    AgentToolFullInput, SharedSubAgentRunner, SubAgentErrorKind, SubAgentRequest, SubagentType,
};
use crate::core::tools::tools::{BaseDeclarativeTool, ToolInvocation, ToolLocation, ToolResult};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

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
}

impl From<AgentToolParams> for AgentToolFullInput {
    fn from(p: AgentToolParams) -> Self {
        Self {
            description: p.description,
            prompt: p.prompt,
            subagent_type: p.subagent_type,
            name: p.name,
            isolation: None,
            model: None,
            background: p.background,
            max_rounds: p.max_rounds,
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
                    "description": "Type of specialized agent to use. Default: general_purpose."
                },
                "name": {
                    "type": "string",
                    "description": "Optional name for this agent instance (enables send_message targeting)."
                },
                "background": {
                    "type": "boolean",
                    "description": "Run in background. Results arrive via <task-notification>. Default: false."
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
        Ok(Box::new(AgentToolInvocation {
            full_input: params.into(),
            runner: self.runner.clone(),
            async_runner: self.async_runner.clone(),
        }))
    }
}

pub struct AgentToolInvocation {
    full_input: AgentToolFullInput,
    runner: SharedSubAgentRunner,
    async_runner: Option<Arc<AsyncSubagentRunner>>,
}

impl ToolInvocation for AgentToolInvocation {
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
        let input = self.full_input.clone();
        let runner = self.runner.clone();
        let async_runner = self.async_runner.clone();

        Box::pin(async move {
            let route = route_agent_call(&input, false, false);

            match route {
                // 同步路径（现有逻辑不变）
                AgentRoute::SyncNamedAgent { request, .. } => {
                    match runner.run(request.with_max_rounds(50)).await {
                        Ok(result) => Ok(ToolResult {
                            llm_content: None,
                            return_display: None,
                            output: result.output,
                            error: None,
                            data: None,
                        }),
                        Err(err) => {
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
                                data: None,
                            })
                        }
                    }
                }

                // 异步路径
                AgentRoute::AsyncAgent {
                    agent_id,
                    request,
                    name,
                } => {
                    let async_runner = async_runner
                        .ok_or_else(|| "AsyncSubagentRunner not configured".to_string())?;

                    let launch = async_runner.spawn_background(
                        SubAgentRequest {
                            prompt: request.prompt,
                            max_rounds: request.max_rounds,
                        },
                        name,
                        input.description.clone(),
                    );

                    Ok(ToolResult {
                        llm_content: Some(format!(
                            "Agent launched in background.\nagent_id: {}\nOutput file: {}\nWait for <task-notification> for results.",
                            launch.agent_id,
                            launch.output_file.display(),
                        )),
                        return_display: Some(format!("Agent {} started", launch.agent_id)),
                        output: String::new(),
                        error: None,
                        data: Some(serde_json::json!({
                            "status": "async_launched",
                            "agent_id": launch.agent_id,
                            "output_file": launch.output_file.to_string_lossy(),
                        })),
                    })
                }

                // Coordinator Worker 路径
                AgentRoute::CoordinatorWorker {
                    agent_id: _,
                    request,
                } => {
                    let async_runner = async_runner.ok_or_else(|| {
                        "AsyncSubagentRunner not configured for coordinator".to_string()
                    })?;

                    let launch = async_runner.spawn_background(
                        SubAgentRequest {
                            prompt: request.prompt,
                            max_rounds: request.max_rounds,
                        },
                        None,
                        input.description.clone(),
                    );

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
                            "coordinator_worker": true,
                        })),
                    })
                }

                AgentRoute::ForkAgent { agent_id, request } => {
                    let async_runner = async_runner
                        .ok_or_else(|| "AsyncSubagentRunner not configured for fork".to_string())?;

                    // Fork Agent 继承父上下文：将父消息历史注入到 prompt 前面
                    let mut enriched_prompt = String::new();
                    if !request.parent_messages.is_empty() {
                        enriched_prompt.push_str("## Inherited Context from Parent Agent\n\n");
                        enriched_prompt.push_str("The following conversation history was inherited from the parent agent. ");
                        enriched_prompt
                            .push_str("Use this context to understand the ongoing work:\n\n");
                        for msg in &request.parent_messages {
                            let role = if msg.role == "user" {
                                "User"
                            } else {
                                "Assistant"
                            };
                            let content = msg.content.as_deref().unwrap_or("(empty)");
                            let content_preview: String = content.chars().take(500).collect();
                            enriched_prompt
                                .push_str(&format!("**{}**: {}\n\n", role, content_preview));
                        }
                        enriched_prompt.push_str("---\n\n## Fork Task\n\n");
                    }
                    enriched_prompt.push_str(&request.base.prompt);

                    let launch = async_runner.spawn_background(
                        SubAgentRequest {
                            prompt: enriched_prompt,
                            max_rounds: request.base.max_rounds,
                        },
                        Some(format!("fork-{}", agent_id)),
                        request.description.clone(),
                    );

                    Ok(ToolResult {
                        llm_content: Some(format!(
                            "Fork agent launched.\nagent_id: {}\nInherited {} parent messages.\nResults will arrive via <task-notification>.",
                            launch.agent_id,
                            request.parent_messages.len(),
                        )),
                        return_display: Some(format!("Fork {} started", agent_id)),
                        output: String::new(),
                        error: None,
                        data: Some(serde_json::json!({
                            "status": "fork_launched",
                            "agent_id": launch.agent_id,
                            "inherited_messages": request.parent_messages.len(),
                            "output_file": launch.output_file.to_string_lossy(),
                        })),
                    })
                }
            }
        })
    }
}
