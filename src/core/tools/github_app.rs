use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Clone)]
pub struct GithubAppTool {
    config: Arc<crate::core::config::Config>,
}

impl GithubAppTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self { config }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct GithubAppParams {
    pub action: String,
    pub repo: Option<String>,
    pub app_id: Option<String>,
}

pub struct GithubAppInvocation {
    config: Arc<crate::core::config::Config>,
    params: GithubAppParams,
}

impl GithubAppInvocation {
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

impl ToolInvocation for GithubAppInvocation {
    fn get_description(&self) -> String {
        format!(
            "GitHub App {}: {}",
            self.params.action,
            self.params.repo.as_deref().unwrap_or("N/A")
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
        let invocation = GithubAppInvocation {
            config: self.config.clone(),
            params: params.clone(),
        };
        Box::pin(async move {
            match params.action.as_str() {
                "install" => {
                    let repo = match params.repo {
                        Some(r) => r,
                        None => {
                            return Ok(Self::error_result("Missing repo parameter".to_string()))
                        }
                    };
                    Ok(ToolResult {
                        llm_content: Some(format!(
                            "To install a GitHub App for {}, visit: https://github.com/apps and search for the app, or use: gh api repos/{}/installation",
                            repo, repo
                        )),
                        return_display: Some(format!("GitHub App install info for {}", repo)),
                        output: format!(
                            "GitHub App installation for {}:\n\
                             1. Visit https://github.com/apps to find the app\n\
                             2. Or run: gh api repos/{}/installation\n\
                             3. Follow the installation prompts",
                            repo, repo
                        ),
                        error: None,
                        data: Some(serde_json::json!({
                            "action": "install",
                            "repo": repo,
                            "url": format!("https://github.com/{}/settings/installations", repo)
                        })),
                    })
                }
                "list" => {
                    match invocation
                        .run_gh(&[
                            "api",
                            "user/installations",
                            "--jq",
                            ".installations[] | {id, app_slug, account: .account.login}",
                        ])
                        .await
                    {
                        Ok(output) => Ok(ToolResult {
                            llm_content: Some(output.clone()),
                            return_display: Some("Listed GitHub App installations".to_string()),
                            output,
                            error: None,
                            data: None,
                        }),
                        Err(e) => Ok(Self::error_result(e)),
                    }
                }
                "status" => match invocation.run_gh(&["auth", "status"]).await {
                    Ok(output) => Ok(ToolResult {
                        llm_content: Some(output.clone()),
                        return_display: Some("GitHub CLI auth status".to_string()),
                        output,
                        error: None,
                        data: None,
                    }),
                    Err(e) => Ok(Self::error_result(e)),
                },
                _ => Ok(Self::error_result(format!(
                    "Unknown action: {}. Valid: install, list, status",
                    params.action
                ))),
            }
        })
    }
}

impl BaseDeclarativeTool for GithubAppTool {
    fn name(&self) -> &str {
        "github_app"
    }

    fn display_name(&self) -> &str {
        "GitHub App"
    }

    fn description(&self) -> &str {
        "Manage GitHub App installations and check auth status. Uses gh CLI. Requires GitHub CLI to be installed and authenticated."
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
                    "enum": ["install", "list", "status"],
                    "description": "Action type"
                },
                "repo": {
                    "type": "string",
                    "description": "Repository in format: owner/repo"
                },
                "app_id": {
                    "type": "string",
                    "description": "GitHub App ID"
                }
            },
            "required": ["action"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: GithubAppParams = serde_json::from_value(params)?;
        Ok(Box::new(GithubAppInvocation {
            config: self.config.clone(),
            params,
        }))
    }
}
