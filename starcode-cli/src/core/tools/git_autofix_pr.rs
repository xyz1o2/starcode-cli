use crate::core::tools::git_utils::run_git;
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Clone)]
pub struct GitAutofixPrTool {
    config: Arc<crate::core::config::Config>,
}

impl GitAutofixPrTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self { config }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct GitAutofixPrParams {
    pub pr_number: u64,
    pub fix_prompt: String,
}

pub struct GitAutofixPrInvocation {
    config: Arc<crate::core::config::Config>,
    params: GitAutofixPrParams,
}

impl ToolInvocation for GitAutofixPrInvocation {
    fn get_description(&self) -> String {
        format!(
            "Auto-fix PR #{}: {}",
            self.params.pr_number,
            &self.params.fix_prompt[..self.params.fix_prompt.len().min(50)]
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

            let pr_info = tokio::process::Command::new("gh")
                .args([
                    "pr",
                    "view",
                    &params.pr_number.to_string(),
                    "--json",
                    "headRefName,baseRefName,state,body",
                ])
                .current_dir(root)
                .output()
                .await
                .map_err(|e| format!("Failed to get PR info: {}", e))?;

            if !pr_info.status.success() {
                let stderr = String::from_utf8_lossy(&pr_info.stderr);
                return Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "gh_error".to_string(),
                        message: format!("Failed to get PR info: {}", stderr),
                    }),
                    data: None,
                });
            }

            let pr_data: serde_json::Value =
                serde_json::from_str(&String::from_utf8_lossy(&pr_info.stdout))
                    .unwrap_or_default();

            let head_branch = pr_data
                .get("headRefName")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let base_branch = pr_data
                .get("baseRefName")
                .and_then(|v| v.as_str())
                .unwrap_or("main");

            let checkout = run_git(root, &["checkout", head_branch]).await;
            if let Err(e) = checkout {
                return Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "git_error".to_string(),
                        message: format!("Failed to checkout {}: {}", head_branch, e),
                    }),
                    data: None,
                });
            }

            let diff = run_git(
                root,
                &["diff", &format!("{}..HEAD", base_branch)],
            )
            .await
            .unwrap_or_default();

            let review_comments = tokio::process::Command::new("gh")
                .args([
                    "pr",
                    "view",
                    &params.pr_number.to_string(),
                    "--json",
                    "comments,reviews",
                ])
                .current_dir(root)
                .output()
                .await
                .map_err(|e| format!("Failed to get PR comments: {}", e));

            let comments_text = match review_comments {
                Ok(out) if out.status.success() => {
                    String::from_utf8_lossy(&out.stdout).to_string()
                }
                _ => "(unable to fetch comments)".to_string(),
            };

            let fix_context = format!(
                "PR #{}: {}\nBase: {} ← Head: {}\n\nFix instructions: {}\n\nDiff (first 200 lines):\n{}\n\nReview comments:\n{}",
                params.pr_number,
                pr_data.get("body").and_then(|v| v.as_str()).unwrap_or(""),
                base_branch,
                head_branch,
                params.fix_prompt,
                diff.lines().take(200).collect::<Vec<_>>().join("\n"),
                comments_text.chars().take(2000).collect::<String>()
            );

            let fix_file = root.join(".star").join("autofix_context.md");
            if let Some(parent) = fix_file.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            let _ = tokio::fs::write(&fix_file, &fix_context).await;

            Ok(ToolResult {
                llm_content: Some(format!(
                    "Auto-fix context prepared for PR #{}.\nFix prompt: {}\nContext saved to: {}",
                    params.pr_number,
                    params.fix_prompt,
                    fix_file.display()
                )),
                return_display: Some(format!("Auto-fix PR #{}", params.pr_number)),
                output: format!(
                    "Branch {} checked out.\nFix context prepared.\nApply fixes based on: {}\nThen commit and push to update the PR.",
                    head_branch,
                    params.fix_prompt
                ),
                error: None,
                data: Some(serde_json::json!({
                    "pr_number": params.pr_number,
                    "head_branch": head_branch,
                    "base_branch": base_branch,
                    "fix_prompt": params.fix_prompt,
                    "context_file": fix_file.display().to_string(),
                })),
            })
        })
    }
}

impl BaseDeclarativeTool for GitAutofixPrTool {
    fn name(&self) -> &str {
        "git_autofix_pr"
    }

    fn display_name(&self) -> &str {
        "Auto-fix PR"
    }

    fn description(&self) -> &str {
        "自动修复 PR 中发现的问题。检出 PR 分支并准备修复上下文。(Auto-fix issues found in PR review. Checks out the PR branch and prepares fix context.)"
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pr_number": {
                    "type": "integer",
                    "description": "PR 编号 (PR number)"
                },
                "fix_prompt": {
                    "type": "string",
                    "description": "修复指令 (Fix instructions/prompt)"
                }
            },
            "required": ["pr_number", "fix_prompt"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: GitAutofixPrParams = serde_json::from_value(params)?;
        Ok(Box::new(GitAutofixPrInvocation {
            config: self.config.clone(),
            params,
        }))
    }
}
