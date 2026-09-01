use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone)]
pub struct SkillTool;

impl SkillTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct SkillParams {
    pub name: String,
    pub args: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Serialize, Clone)]
pub struct SkillOutput {
    pub result: serde_json::Value,
    pub skill_name: String,
}

pub struct SkillInvocation {
    params: SkillParams,
}

impl ToolInvocation for SkillInvocation {
    fn get_description(&self) -> String {
        format!("Execute skill: {}", self.params.name)
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let params = self.params.clone();
        Box::pin(async move {
            let name = params.name.clone();
            let args = params.args.unwrap_or_default();

            // In a real implementation, this would:
            // 1. Look up the skill in the skill registry
            // 2. Execute the skill with the provided args
            // 3. Return the result

            // For now, return a placeholder response
            Ok(ToolResult {
                llm_content: Some(format!("Executed skill '{}' with args", name)),
                return_display: Some(format!("Skill '{}' executed", name)),
                output: serde_json::to_string(&SkillOutput {
                    result: serde_json::json!({
                        "status": "success",
                        "message": format!("Skill '{}' executed successfully", name)
                    }),
                    skill_name: name.clone(),
                })?,
                error: None,
                data: Some(serde_json::json!({
                    "skill_name": name,
                    "args": args
                })),
            })
        })
    }
}

impl BaseDeclarativeTool for SkillTool {
    fn name(&self) -> &str {
        "skill"
    }

    fn display_name(&self) -> &str {
        "Skill"
    }

    fn description(&self) -> &str {
        "执行已注册的技能/命令（斜杠命令、插件命令）。(Execute a registered skill/command - slash commands, plugin commands.)"
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "技能名称 (Skill name)"
                },
                "args": {
                    "type": "object",
                    "description": "技能参数 (Skill arguments)",
                    "additionalProperties": true
                }
            },
            "required": ["name"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: SkillParams = serde_json::from_value(params)?;
        Ok(Box::new(SkillInvocation { params }))
    }

    fn is_read_only(&self) -> bool {
        false
    }
}