use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct SuggestBackgroundPRTool;

impl SuggestBackgroundPRTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct SuggestBackgroundPRParams {
    pub title: String,
    pub description: String,
    pub branch: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct SuggestBackgroundPROutput {
    pub suggested: bool,
    pub suggestion_id: String,
}

pub struct SuggestBackgroundPRInvocation {
    params: SuggestBackgroundPRParams,
}

impl ToolInvocation for SuggestBackgroundPRInvocation {
    fn get_description(&self) -> String {
        format!("Suggest background PR: {}", self.params.title)
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
            let title = params.title.clone();
            let description = params.description.clone();
            let branch = params.branch.unwrap_or_else(|| "background-pr".to_string());

            // In a real implementation, this would:
            // 1. Create a suggestion for a background PR
            // 2. Store the suggestion for later processing

            // For now, return a placeholder response
            let suggestion_id = format!("sug_{}", uuid::Uuid::new_v4());

            Ok(ToolResult {
                llm_content: Some(format!(
                    "Suggested background PR '{}' on branch '{}'",
                    title, branch
                )),
                return_display: Some(format!("PR suggestion created: {}", title)),
                output: serde_json::to_string(&SuggestBackgroundPROutput {
                    suggested: true,
                    suggestion_id: suggestion_id.clone(),
                })?,
                error: None,
                data: Some(serde_json::json!({
                    "title": title,
                    "description": description,
                    "branch": branch,
                    "suggestion_id": suggestion_id
                })),
            })
        })
    }
}

impl BaseDeclarativeTool for SuggestBackgroundPRTool {
    fn name(&self) -> &str {
        "suggest_background_pr"
    }

    fn display_name(&self) -> &str {
        "SuggestBackgroundPR"
    }

    fn description(&self) -> &str {
        "建议在后台创建一个PR来处理后续工作。(Suggest creating a background PR to handle follow-up work.)"
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "PR标题 (PR title)"
                },
                "description": {
                    "type": "string",
                    "description": "PR描述 (PR description)"
                },
                "branch": {
                    "type": "string",
                    "description": "分支名称，默认为 'background-pr' (Branch name, defaults to 'background-pr')"
                }
            },
            "required": ["title", "description"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: SuggestBackgroundPRParams = serde_json::from_value(params)?;
        Ok(Box::new(SuggestBackgroundPRInvocation { params }))
    }

    fn is_read_only(&self) -> bool {
        false
    }
}