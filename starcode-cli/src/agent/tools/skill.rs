use crate::agent::skills::{
    register_custom_subagents, AnalyzerAgent, AutoFixAgent, EditorAgent, NavigatorAgent,
    SearchAgent, SubAgentManager, SubTask,
};
use crate::core::tools::{
    BaseDeclarativeTool, Kind, ToolCallConfirmationDetails, ToolError, ToolInvocation,
    ToolLocation, ToolResult as CoreToolResult,
};

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::core::config::Config;
use crate::llm::client::StarClient;

pub struct SkillTool {
    manager: Arc<SubAgentManager>,
    description: String,
    available_skill_names: Vec<String>,
}

impl SkillTool {
    pub fn new(client: StarClient, config: Arc<Config>) -> Self {
        let mut manager = SubAgentManager::new();
        let mut sub_config = (*config).clone();
        sub_config.recursion_depth = sub_config.recursion_depth.saturating_add(1);
        let sub_config = Arc::new(sub_config);
        manager.register(Box::new(AnalyzerAgent::new(
            client.clone(),
            sub_config.clone(),
        )));
        manager.register(Box::new(EditorAgent::new(
            client.clone(),
            sub_config.clone(),
        )));
        manager.register(Box::new(SearchAgent::new(
            client.clone(),
            sub_config.clone(),
        )));
        manager.register(Box::new(NavigatorAgent::new(
            client.clone(),
            sub_config.clone(),
        )));

        // AutoFixAgent needs client and config to spawn sub-agents
        manager.register(Box::new(AutoFixAgent::new(
            client.clone(),
            sub_config.clone(),
        )));
        let custom_defs = register_custom_subagents(&mut manager, client, sub_config.clone());
        let mut available_skill_names = manager.agent_ids();
        available_skill_names.sort();
        available_skill_names.dedup();
        let mut description = format!(
            "Execute a specialized skill. Available skills: {}. Use this tool to delegate complex tasks to specialized sub-agents.",
            available_skill_names.join(", ")
        );
        if !custom_defs.is_empty() {
            description.push_str(" Includes custom skills loaded from project/user .star/agents.");
        }

        Self {
            manager: Arc::new(manager),
            description,
            available_skill_names,
        }
    }
}

impl BaseDeclarativeTool for SkillTool {
    fn name(&self) -> &str {
        "skill"
    }

    fn display_name(&self) -> &str {
        "Skill Tool"
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "skill": {
                    "type": "string",
                    "description": format!(
                        "The name of the skill to execute (available: {})",
                        self.available_skill_names.join(", ")
                    )
                },
                "args": {
                    "type": "object",
                    "description": "Arguments for the skill. Common fields: 'objective', 'target', 'task_type'.",
                    "properties": {
                        "objective": { "type": "string", "description": "The goal of the task" },
                        "task_type": { "type": "string", "description": "Type of task (analyze, edit, search, check, fix)" },
                        "target": { "type": "string", "description": "Target file or context" },
                        "params": { "type": "object", "description": "Additional parameters specific to the skill" }
                    }
                }
            },
            "required": ["skill"]
        })
    }

    fn create_invocation(
        &self,
        params: Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let skill_name = params
            .get("skill")
            .and_then(|v| v.as_str())
            .ok_or("Missing skill name")?
            .to_string();
        let args = params.get("args").cloned().unwrap_or(json!({}));

        // Dereference the Arc to get the manager (SubAgentManager is Clone)
        Ok(Box::new(SkillInvocation {
            manager: (*self.manager).clone(),
            skill_name,
            args,
        }))
    }
}

pub struct SkillInvocation {
    manager: SubAgentManager,
    skill_name: String,
    args: Value,
}

#[async_trait]
impl ToolInvocation for SkillInvocation {
    fn get_description(&self) -> String {
        format!("Executing skill: {}", self.skill_name)
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        update_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<CoreToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let manager = self.manager.clone();
        let skill_name = self.skill_name.clone();
        let args = self.args.clone();
        let update_output = update_output.clone();

        Box::pin(async move {
            if let Some(cb) = update_output.as_ref() {
                cb(format!("Running skill `{}`\n", skill_name));
            }

            // Construct SubTask
            let objective = args
                .get("objective")
                .and_then(|v| v.as_str())
                .unwrap_or(&skill_name)
                .to_string();
            let task_type = args
                .get("task_type")
                .and_then(|v| v.as_str())
                .unwrap_or(&skill_name)
                .to_string();
            let target = args
                .get("target")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let params = args
                .get("params")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();

            let mut task = SubTask::new(
                uuid::Uuid::new_v4().to_string(),
                objective,
                task_type,
                target,
            );

            for (k, v) in params {
                task = task.with_param(k, v);
            }

            // Try to find agent by ID first, then by task matching
            let agent = if let Some(agent) = manager.get_agent(&skill_name) {
                Some(agent)
            } else {
                manager.select_agent(&task)
            };

            if let Some(agent) = agent {
                match agent.execute(task).await {
                    Ok(result) => {
                        let output = if result.success {
                            result.summary
                        } else {
                            format!(
                                "Skill execution failed: {}",
                                result.error.clone().unwrap_or_default()
                            )
                        };

                        let mut details = String::new();
                        if let Some(d) = result.details {
                            details.push_str(&format!("\nDetails:\n{}", d));
                        }
                        if let Some(data) = result.data {
                            details.push_str(&format!(
                                "\nData:\n{}",
                                serde_json::to_string_pretty(&data).unwrap_or_default()
                            ));
                        }

                        let final_output = format!("{}{}", output, details);
                        if let Some(cb) = update_output.as_ref() {
                            cb(format!("Skill `{}` completed\n", skill_name));
                        }

                        Ok(CoreToolResult {
                            llm_content: Some(final_output.clone()),
                            return_display: Some(final_output.clone()),
                            output: final_output,
                            error: if result.success {
                                None
                            } else {
                                Some(ToolError {
                                    error_type: "ExecutionFailed".to_string(),
                                    message: result.error.unwrap_or_default(),
                                })
                            },
                            data: None,
                        })
                    }
                    Err(e) => Ok(CoreToolResult {
                        llm_content: Some(format!("Skill execution error: {}", e)),
                        return_display: Some(format!("Skill execution error: {}", e)),
                        output: format!("Skill execution error: {}", e),
                        error: Some(ToolError {
                            error_type: "RuntimeError".to_string(),
                            message: e.to_string(),
                        }),
                        data: None,
                    }),
                }
            } else {
                // ============ 外部 Skill 加载 (GitHub / Local) ============
                // 如果没有内置 Agent，尝试作为外部 Skill 加载

                let skill_def = if skill_name.starts_with("http") || skill_name.starts_with("git@")
                {
                    // GitHub / Remote Git
                    crate::agent::skills::loader::SkillLoader::load_skill_from_github(
                        &skill_name,
                        None,
                    )
                    .await
                } else {
                    // Local Path
                    let path = std::path::Path::new(&skill_name);
                    if path.exists() {
                        crate::agent::skills::loader::SkillLoader::load_skill_from_file(path).await
                    } else {
                        None
                    }
                };

                if let Some(skill) = skill_def {
                    // 成功加载外部 Skill
                    // 将其内容作为上下文注入返回
                    // 沙箱隔离：明确标记来源和边界
                    // 执行内联 shell 命令 (!(...) 语法)
                    let rendered_body = crate::agent::skills::loader::SkillLoader::render_skill_prompt(&skill);
                    let output = format!(
                        "✅ External Skill Loaded Successfully\n\n<skill_sandbox name=\"{}\" source=\"{}\">\n{}\n</skill_sandbox>\n\n⚠️ This skill is running in a restricted context. Please follow the instructions above.",
                        skill.name, skill.location, rendered_body
                    );

                    Ok(CoreToolResult {
                        llm_content: Some(output.clone()),
                        return_display: Some(output.clone()),
                        output,
                        error: None,
                        data: None,
                    })
                } else {
                    let output = format!(
                        "No suitable agent or external skill found for: {}",
                        skill_name
                    );
                    Ok(CoreToolResult {
                        llm_content: Some(output.clone()),
                        return_display: Some(output.clone()),
                        output: output.clone(),
                        error: Some(ToolError {
                            error_type: "NoAgent".to_string(),
                            message: output,
                        }),
                        data: None,
                    })
                }
            }
        })
    }
}
