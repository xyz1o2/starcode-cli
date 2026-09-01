use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct SubscribePRTool;

impl SubscribePRTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct SubscribePRParams {
    pub repo: String,
    pub pr_number: u32,
    pub events: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Clone)]
pub struct SubscribePROutput {
    pub subscribed: bool,
    pub subscription_id: String,
}

pub struct SubscribePRInvocation {
    params: SubscribePRParams,
}

impl ToolInvocation for SubscribePRInvocation {
    fn get_description(&self) -> String {
        format!("Subscribe to PR #{} in {}", self.params.pr_number, self.params.repo)
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
            let repo = params.repo.clone();
            let pr_number = params.pr_number;
            let events = params.events.unwrap_or_else(|| {
                vec![
                    "comment".to_string(),
                    "review".to_string(),
                    "ci".to_string(),
                    "merge".to_string(),
                    "close".to_string(),
                ]
            });

            // In a real implementation, this would:
            // 1. Create a subscription for the PR events
            // 2. Register the subscription with the notification system

            // For now, return a placeholder response
            let subscription_id = format!("sub_{}_{}", repo.replace("/", "_"), pr_number);

            Ok(ToolResult {
                llm_content: Some(format!(
                    "Subscribed to PR #{} in {} for events: {:?}",
                    pr_number, repo, events
                )),
                return_display: Some(format!("Subscribed to PR #{}", pr_number)),
                output: serde_json::to_string(&SubscribePROutput {
                    subscribed: true,
                    subscription_id: subscription_id.clone(),
                })?,
                error: None,
                data: Some(serde_json::json!({
                    "repo": repo,
                    "pr_number": pr_number,
                    "events": events,
                    "subscription_id": subscription_id
                })),
            })
        })
    }
}

impl BaseDeclarativeTool for SubscribePRTool {
    fn name(&self) -> &str {
        "subscribe_pr"
    }

    fn display_name(&self) -> &str {
        "SubscribePR"
    }

    fn description(&self) -> &str {
        "订阅GitHub Pull Request事件（评论、审查、CI、合并、关闭）。(Subscribe to GitHub Pull Request events - comments, reviews, CI, merges, closes.)"
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "repo": {
                    "type": "string",
                    "description": "仓库名称，格式为 owner/repo (Repository name in owner/repo format)"
                },
                "pr_number": {
                    "type": "integer",
                    "description": "PR编号 (PR number)"
                },
                "events": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "enum": ["comment", "review", "ci", "merge", "close"]
                    },
                    "description": "要订阅的事件类型，默认全部 (Event types to subscribe to, defaults to all)"
                }
            },
            "required": ["repo", "pr_number"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: SubscribePRParams = serde_json::from_value(params)?;
        Ok(Box::new(SubscribePRInvocation { params }))
    }

    fn is_read_only(&self) -> bool {
        false
    }
}