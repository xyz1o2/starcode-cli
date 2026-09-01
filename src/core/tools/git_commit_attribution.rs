use crate::core::tools::git_utils::run_git;
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Clone)]
pub struct GitCommitAttributionTool {
    config: Arc<crate::core::config::Config>,
}

impl GitCommitAttributionTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self { config }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct GitCommitAttributionParams {
    pub message: String,
    #[serde(default = "default_true")]
    pub include_attribution: bool,
}

fn default_true() -> bool {
    true
}

pub struct GitCommitAttributionInvocation {
    config: Arc<crate::core::config::Config>,
    params: GitCommitAttributionParams,
}

impl ToolInvocation for GitCommitAttributionInvocation {
    fn get_description(&self) -> String {
        format!(
            "Git commit with{} attribution: {}",
            if self.params.include_attribution {
                ""
            } else {
                "out"
            },
            &self.params.message[..self.params.message.len().min(50)]
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

            let commit_message = if params.include_attribution {
                format!(
                    "{}\n\nCo-authored-by: StarCode CLI <starcode@ai-assistant.local>\nGenerated-with: StarCode CLI",
                    params.message
                )
            } else {
                params.message.clone()
            };

            run_git(root, &["add", "-A"])
                .await
                .map_err(|e| format!("Failed to stage changes: {}", e))?;

            let output = run_git(root, &["commit", "-m", &commit_message])
                .await
                .map_err(|e| format!("Failed to commit: {}", e))?;

            let hash = run_git(root, &["rev-parse", "--short", "HEAD"])
                .await
                .unwrap_or_default();
            let hash = hash.trim();

            Ok(ToolResult {
                llm_content: Some(format!("Committed as {}:\n{}", hash, commit_message)),
                return_display: Some(format!("Committed {}", hash)),
                output: format!(
                    "Commit {} created.\n\n{}\n\n{}",
                    hash,
                    commit_message,
                    output.trim()
                ),
                error: None,
                data: Some(serde_json::json!({
                    "hash": hash,
                    "message": params.message,
                    "has_attribution": params.include_attribution,
                })),
            })
        })
    }
}

impl BaseDeclarativeTool for GitCommitAttributionTool {
    fn name(&self) -> &str {
        "git_commit_attribution"
    }

    fn display_name(&self) -> &str {
        "Git Commit Attribution"
    }

    fn description(&self) -> &str {
        "创建带有 AI 署名的 Git 提交。(Create a git commit with optional AI attribution in the commit message.)"
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "提交信息 (Commit message)"
                },
                "include_attribution": {
                    "type": "boolean",
                    "description": "是否包含 AI 署名，默认 true (Include AI attribution, default: true)"
                }
            },
            "required": ["message"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: GitCommitAttributionParams = serde_json::from_value(params)?;
        Ok(Box::new(GitCommitAttributionInvocation {
            config: self.config.clone(),
            params,
        }))
    }
}
