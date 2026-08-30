use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Clone)]
pub struct GitPrSubscribeTool {
    config: Arc<crate::core::config::Config>,
}

impl GitPrSubscribeTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self { config }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct GitPrSubscribeParams {
    pub repo: String,
    pub pr_number: u64,
}

pub struct GitPrSubscribeInvocation {
    config: Arc<crate::core::config::Config>,
    params: GitPrSubscribeParams,
}

impl ToolInvocation for GitPrSubscribeInvocation {
    fn get_description(&self) -> String {
        format!(
            "Subscribe to PR #{} in {}",
            self.params.pr_number, self.params.repo
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
        let config = self.config.clone();
        let params = self.params.clone();
        Box::pin(async move {
            let root = config.project_root();
            let subs_dir = root.join(".star").join("pr_subscriptions");
            if !subs_dir.exists() {
                tokio::fs::create_dir_all(&subs_dir)
                    .await
                    .map_err(|e| format!("Failed to create subscriptions dir: {}", e))?;
            }

            let sub_file = subs_dir.join(format!(
                "{}_{}.json",
                params.repo.replace('/', "_"),
                params.pr_number
            ));

            let current_state = get_pr_state(&params.repo, params.pr_number).await;

            let subscription = serde_json::json!({
                "repo": params.repo,
                "pr_number": params.pr_number,
                "subscribed_at": chrono::Utc::now().to_rfc3339(),
                "last_state": current_state,
            });

            let content = serde_json::to_string_pretty(&subscription)
                .map_err(|e| format!("Failed to serialize: {}", e))?;
            tokio::fs::write(&sub_file, content)
                .await
                .map_err(|e| format!("Failed to write subscription: {}", e))?;

            Ok(ToolResult {
                llm_content: Some(format!(
                    "Subscribed to PR #{} in {}. Current state: {}",
                    params.pr_number, params.repo, current_state
                )),
                return_display: Some(format!(
                    "Subscribed to PR #{}",
                    params.pr_number
                )),
                output: format!(
                    "Subscribed to PR #{} in {}.\nCurrent state: {}\nSubscription saved to: {}",
                    params.pr_number,
                    params.repo,
                    current_state,
                    sub_file.display()
                ),
                error: None,
                data: Some(subscription),
            })
        })
    }
}

async fn get_pr_state(repo: &str, pr_number: u64) -> String {
    let output = tokio::process::Command::new("gh")
        .args([
            "pr",
            "view",
            &pr_number.to_string(),
            "--repo",
            repo,
            "--json",
            "state,statusCheckRollup",
        ])
        .output()
        .await;

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let data: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_default();
            let state = data
                .get("state")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let checks = data
                .get("statusCheckRollup")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            format!("{} ({} checks)", state, checks)
        }
        _ => "unknown (gh CLI unavailable)".to_string(),
    }
}

impl BaseDeclarativeTool for GitPrSubscribeTool {
    fn name(&self) -> &str {
        "git_pr_subscribe"
    }

    fn display_name(&self) -> &str {
        "PR Subscribe"
    }

    fn description(&self) -> &str {
        "订阅 PR 变更通知。跟踪 PR 状态和检查结果。(Subscribe to PR changes and get notifications about status updates and check results.)"
    }

    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "repo": {
                    "type": "string",
                    "description": "仓库路径 owner/repo (Repository path, e.g. owner/repo)"
                },
                "pr_number": {
                    "type": "integer",
                    "description": "PR 编号 (PR number)"
                }
            },
            "required": ["repo", "pr_number"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: GitPrSubscribeParams = serde_json::from_value(params)?;
        Ok(Box::new(GitPrSubscribeInvocation {
            config: self.config.clone(),
            params,
        }))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}
