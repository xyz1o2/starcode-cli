use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Clone)]
pub struct GithubIssueTool {
    config: Arc<crate::core::config::Config>,
}

impl GithubIssueTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self { config }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct GithubIssueParams {
    pub action: String,
    pub repo: String,
    pub number: Option<u64>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub labels: Option<Vec<String>>,
    pub limit: Option<u32>,
}

pub struct GithubIssueInvocation {
    config: Arc<crate::core::config::Config>,
    params: GithubIssueParams,
}

impl GithubIssueInvocation {
    async fn run_gh(&self, args: &[&str]) -> Result<String, String> {
        let output = tokio::process::Command::new("gh")
            .args(args)
            .output()
            .await
            .map_err(|e| format!("Failed to run gh: {}. Is GitHub CLI installed?", e))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Err(format!("gh command failed: {}", stderr.trim()))
        }
    }

    fn error_result(message: String) -> ToolResult {
        ToolResult {
            llm_content: None,
            return_display: None,
            output: String::new(),
            error: Some(ToolError {
                error_type: "github_error".to_string(),
                message,
            }),
            data: None,
        }
    }
}

impl ToolInvocation for GithubIssueInvocation {
    fn get_description(&self) -> String {
        format!(
            "GitHub Issue {}: {}#{}",
            self.params.action,
            self.params.repo,
            self.params.number.unwrap_or(0)
        )
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
        let invocation = GithubIssueInvocation {
            config: self.config.clone(),
            params: params.clone(),
        };
        Box::pin(async move {
            match params.action.as_str() {
                "list" => {
                    let limit = params.limit.unwrap_or(30).to_string();
                    let args = vec![
                        "issue", "list",
                        "--repo", &params.repo,
                        "--limit", &limit,
                        "--json", "number,title,state,author,createdAt,labels",
                    ];
                    match invocation.run_gh(&args).await {
                        Ok(output) => Ok(ToolResult {
                            llm_content: Some(output.clone()),
                            return_display: Some(format!("Listed issues for {}", params.repo)),
                            output,
                            error: None,
                            data: None,
                        }),
                        Err(e) => Ok(Self::error_result(e)),
                    }
                }
                "get" => {
                    let number = match params.number {
                        Some(n) => n,
                        None => return Ok(Self::error_result("Missing issue number".to_string())),
                    };
                    let num_str = number.to_string();
                    let args = vec![
                        "issue", "view", &num_str,
                        "--repo", &params.repo,
                        "--json", "number,title,body,state,author,createdAt,labels,comments",
                    ];
                    match invocation.run_gh(&args).await {
                        Ok(output) => Ok(ToolResult {
                            llm_content: Some(output.clone()),
                            return_display: Some(format!("Issue #{} from {}", number, params.repo)),
                            output,
                            error: None,
                            data: None,
                        }),
                        Err(e) => Ok(Self::error_result(e)),
                    }
                }
                "create" => {
                    let title = match params.title {
                        Some(t) => t,
                        None => return Ok(Self::error_result("Missing issue title".to_string())),
                    };
                    let body = params.body.unwrap_or_default();
                    let mut args = vec![
                        "issue", "create",
                        "--repo", &params.repo,
                        "--title", &title,
                        "--body", &body,
                    ];
                    let labels_str;
                    if let Some(ref labels) = params.labels {
                        labels_str = labels.join(",");
                        args.push("--label");
                        args.push(&labels_str);
                    }
                    match invocation.run_gh(&args).await {
                        Ok(output) => Ok(ToolResult {
                            llm_content: Some(output.clone()),
                            return_display: Some(format!("Created issue: {}", title)),
                            output,
                            error: None,
                            data: None,
                        }),
                        Err(e) => Ok(Self::error_result(e)),
                    }
                }
                "comment" => {
                    let number = match params.number {
                        Some(n) => n,
                        None => return Ok(Self::error_result("Missing issue number".to_string())),
                    };
                    let body = match params.body {
                        Some(b) => b,
                        None => return Ok(Self::error_result("Missing comment body".to_string())),
                    };
                    let num_str = number.to_string();
                    let args = vec![
                        "issue", "comment", &num_str,
                        "--repo", &params.repo,
                        "--body", &body,
                    ];
                    match invocation.run_gh(&args).await {
                        Ok(output) => Ok(ToolResult {
                            llm_content: Some(output.clone()),
                            return_display: Some(format!("Commented on issue #{}", number)),
                            output,
                            error: None,
                            data: None,
                        }),
                        Err(e) => Ok(Self::error_result(e)),
                    }
                }
                "close" => {
                    let number = match params.number {
                        Some(n) => n,
                        None => return Ok(Self::error_result("Missing issue number".to_string())),
                    };
                    let num_str = number.to_string();
                    let args = vec![
                        "issue", "close", &num_str,
                        "--repo", &params.repo,
                    ];
                    match invocation.run_gh(&args).await {
                        Ok(output) => Ok(ToolResult {
                            llm_content: Some(output.clone()),
                            return_display: Some(format!("Closed issue #{}", number)),
                            output,
                            error: None,
                            data: None,
                        }),
                        Err(e) => Ok(Self::error_result(e)),
                    }
                }
                _ => Ok(Self::error_result(format!(
                    "Unknown action: {}. Valid: list, get, create, comment, close",
                    params.action
                ))),
            }
        })
    }
}

impl BaseDeclarativeTool for GithubIssueTool {
    fn name(&self) -> &str {
        "github_issue"
    }

    fn display_name(&self) -> &str {
        "GitHub Issue"
    }

    fn description(&self) -> &str {
        "Manage GitHub Issues using gh CLI. Supports list, get, create, comment, and close operations. Requires GitHub CLI (gh) to be installed and authenticated."
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "get", "create", "comment", "close"],
                    "description": "Action type"
                },
                "repo": {
                    "type": "string",
                    "description": "Repository in format: owner/repo"
                },
                "number": {
                    "type": "integer",
                    "description": "Issue number (for get, comment, close)"
                },
                "title": {
                    "type": "string",
                    "description": "Issue title (for create)"
                },
                "body": {
                    "type": "string",
                    "description": "Issue body or comment body"
                },
                "labels": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Labels to add (for create)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max issues to list (default: 30)"
                }
            },
            "required": ["action", "repo"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: GithubIssueParams = serde_json::from_value(params)?;
        Ok(Box::new(GithubIssueInvocation {
            config: self.config.clone(),
            params,
        }))
    }
}
