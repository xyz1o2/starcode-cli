use crate::core::tools::git_utils::run_git;
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Clone)]
pub struct SuggestBackgroundPRTool {
    config: Arc<crate::core::config::Config>,
}

impl SuggestBackgroundPRTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self { config }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct SuggestPRParams {
    #[serde(default)]
    pub base_branch: Option<String>,
}

pub struct SuggestPRInvocation {
    config: Arc<crate::core::config::Config>,
    params: SuggestPRParams,
}

impl ToolInvocation for SuggestPRInvocation {
    fn get_description(&self) -> String {
        "Analyze branch changes and suggest PR".to_string()
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

            // 1. Get current branch name
            let current_branch = match run_git(&root, &["branch", "--show-current"]).await {
                Ok(b) => b.trim().to_string(),
                Err(e) => {
                    return Ok(ToolResult {
                        error: Some(ToolError {
                            error_type: "git_error".to_string(),
                            message: format!("Failed to get current branch: {}", e),
                        }),
                        ..Default::default()
                    });
                }
            };

            if current_branch.is_empty() {
                return Ok(ToolResult {
                    error: Some(ToolError {
                        error_type: "git_error".to_string(),
                        message: "Not on a branch (detached HEAD?)".to_string(),
                    }),
                    ..Default::default()
                });
            }

            // 2. Determine base branch
            let base_candidates = if let Some(ref base) = params.base_branch {
                vec![base.as_str()]
            } else {
                vec!["main", "master"]
            };

            let mut base_branch = String::new();
            for candidate in &base_candidates {
                if let Ok(output) = run_git(&root, &["rev-parse", "--verify", candidate]).await {
                    if !output.trim().is_empty() {
                        base_branch = candidate.to_string();
                        break;
                    }
                }
            }

            if base_branch.is_empty() {
                return Ok(ToolResult {
                    error: Some(ToolError {
                        error_type: "git_error".to_string(),
                        message: "Could not find base branch (tried main, master). Specify base_branch parameter."
                            .to_string(),
                    }),
                    ..Default::default()
                });
            }

            // 3. Get commit log
            let log_output =
                run_git(&root, &["log", &format!("{}..HEAD", base_branch), "--oneline"])
                    .await
                    .unwrap_or_default();

            // 4. Get diff stat
            let diff_stat =
                run_git(&root, &["diff", &format!("{}..HEAD", base_branch), "--stat"])
                    .await
                    .unwrap_or_default();

            // 5. Get diff (first 200 lines)
            let diff_output =
                run_git(&root, &["diff", &format!("{}..HEAD", base_branch)])
                    .await
                    .unwrap_or_default();
            let diff_truncated: String = diff_output
                .lines()
                .take(200)
                .collect::<Vec<_>>()
                .join("\n");

            // 6. Infer PR title from branch name
            let title = current_branch
                .replace('-', " ")
                .replace('_', " ")
                .replace('/', " / ");

            let suggestion = format!(
                "## PR Suggestion\n\n\
                 **Base:** `{}` ← **Head:** `{}`\n\n\
                 ### Proposed Title\n\
                 {}\n\n\
                 ### Commits\n\
                 ```\n{}\n```\n\n\
                 ### Changed Files\n\
                 ```\n{}\n```\n\n\
                 ### Diff (first 200 lines)\n\
                 ```diff\n{}\n```\n",
                base_branch,
                current_branch,
                title,
                if log_output.is_empty() {
                    "(no commits?)"
                } else {
                    &log_output
                },
                if diff_stat.is_empty() {
                    "(no changes?)"
                } else {
                    &diff_stat
                },
                if diff_truncated.is_empty() {
                    "(no diff?)"
                } else {
                    &diff_truncated
                },
            );

            Ok(ToolResult {
                llm_content: Some(suggestion.clone()),
                return_display: Some(format!(
                    "PR suggestion for {} → {}",
                    current_branch, base_branch
                )),
                output: suggestion,
                error: None,
                data: Some(serde_json::json!({
                    "base": base_branch,
                    "head": current_branch,
                    "title": title,
                })),
            })
        })
    }
}

impl BaseDeclarativeTool for SuggestBackgroundPRTool {
    fn name(&self) -> &str {
        "suggest_pr"
    }

    fn display_name(&self) -> &str {
        "Suggest PR"
    }

    fn description(&self) -> &str {
        "分析当前分支变更并生成 PR 建议。不提交 PR，仅生成建议。(Analyze current branch changes and format a PR suggestion. Does NOT create the PR, only generates a formatted suggestion.)"
    }

    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "base_branch": {
                    "type": "string",
                    "description": "目标分支 (Target base branch, defaults to main or master)"
                }
            },
            "required": []
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: SuggestPRParams = serde_json::from_value(params)?;
        Ok(Box::new(SuggestPRInvocation {
            config: self.config.clone(),
            params,
        }))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}
