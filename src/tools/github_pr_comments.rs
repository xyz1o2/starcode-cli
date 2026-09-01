use crate::core::tools::tools::{BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation};
use crate::core::tools::tools::{ToolError, ToolResult as CoreToolResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use tokio::process::Command;

#[derive(Clone)]
pub struct GhPrCommentsTool;

impl GhPrCommentsTool {
    pub fn new() -> Self {
        Self
    }

    pub async fn fetch(
        &self,
        pr_number: Option<u64>,
        repo: Option<String>,
    ) -> Result<CoreToolResult, Box<dyn std::error::Error>> {
        let (owner, repo_name, pr, head_ref) = match resolve_repo_pr(pr_number, repo).await {
            Ok(v) => v,
            Err(e) => {
                return Ok(CoreToolResult {
                    llm_content: None,
                    return_display: None,
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "execution_error".to_string(),
                        message: e,
                    }),
                    data: None,
                });
            }
        };

        let issue_comments_url = format!("repos/{}/{}/issues/{}/comments", owner, repo_name, pr);
        let review_comments_url = format!("repos/{}/{}/pulls/{}/comments", owner, repo_name, pr);

        let issue_comments_v = match gh_api_json(&["api", &issue_comments_url]).await {
            Ok(v) => v,
            Err(e) => {
                return Ok(CoreToolResult {
                    llm_content: None,
                    return_display: None,
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "gh_api_error".to_string(),
                        message: format!("gh api failed: {}", e),
                    }),
                    data: None,
                });
            }
        };

        let review_comments_v = match gh_api_json(&["api", &review_comments_url]).await {
            Ok(v) => v,
            Err(e) => {
                return Ok(CoreToolResult {
                    llm_content: None,
                    return_display: None,
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "gh_api_error".to_string(),
                        message: format!("gh api failed: {}", e),
                    }),
                    data: None,
                });
            }
        };

        let issue_comments: Vec<GhIssueComment> =
            serde_json::from_value(issue_comments_v).unwrap_or_default();
        let review_comments: Vec<GhReviewComment> =
            serde_json::from_value(review_comments_v).unwrap_or_default();

        let text = format_comments(&issue_comments, &review_comments, head_ref.as_deref());

        let has_any = !issue_comments.is_empty() || !review_comments.is_empty();
        Ok(CoreToolResult {
            llm_content: None,
            return_display: None,
            output: text,
            error: None,
            data: Some(serde_json::json!({
                "repo": format!("{}/{}", owner, repo_name),
                "pr_number": pr,
                "head_ref": head_ref,
                "issue_comment_count": issue_comments.len(),
                "review_comment_count": review_comments.len(),
                "has_any": has_any,
            })),
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GhPrCommentsParams {
    pub pr_number: Option<u64>,
    pub repo: Option<String>,
}

pub struct GhPrCommentsInvocation {
    tool: GhPrCommentsTool,
    params: GhPrCommentsParams,
}

impl ToolInvocation for GhPrCommentsInvocation {
    fn get_description(&self) -> String {
        "Get GitHub PR comments".to_string()
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
            dyn std::future::Future<Output = Result<CoreToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let pr_number = self.params.pr_number;
        let repo = self.params.repo.clone();
        let tool = self.tool.clone();

        Box::pin(async move { tool.fetch(pr_number, repo).await })
    }
}

impl BaseDeclarativeTool for GhPrCommentsTool {
    fn name(&self) -> &str {
        "gh_pr_comments"
    }

    fn display_name(&self) -> &str {
        "GitHub PR Comments"
    }

    fn description(&self) -> &str {
        "Fetch GitHub PR comments (PR-level + code review) in threaded format. Requires gh CLI."
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
                    "description": "PR number (optional; if omitted, tries to auto-detect)"
                },
                "repo": {
                    "type": "string",
                    "description": "owner/repo (optional; if omitted, tries to auto-detect)"
                }
            }
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: GhPrCommentsParams = serde_json::from_value(params)?;
        Ok(Box::new(GhPrCommentsInvocation {
            tool: self.clone(),
            params,
        }))
    }
}

#[derive(Debug, Clone, Deserialize)]
struct GhUser {
    login: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct GhIssueComment {
    body: Option<String>,
    user: Option<GhUser>,
    created_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct GhReviewComment {
    id: u64,
    body: Option<String>,
    user: Option<GhUser>,
    path: Option<String>,
    line: Option<u64>,
    original_line: Option<u64>,
    diff_hunk: Option<String>,
    in_reply_to_id: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct GhPrViewOwner {
    login: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GhPrViewRepo {
    name: String,
    owner: GhPrViewOwner,
}

#[derive(Debug, Clone, Deserialize)]
struct GhPrView {
    number: u64,
    #[serde(rename = "headRepository")]
    head_repository: Option<GhPrViewRepo>,
    #[serde(rename = "baseRepository")]
    base_repository: Option<GhPrViewRepo>,
    #[serde(rename = "headRefName")]
    head_ref_name: Option<String>,
}

async fn resolve_repo_pr(
    pr_number: Option<u64>,
    repo: Option<String>,
) -> Result<(String, String, u64, Option<String>), String> {
    if let (Some(repo), Some(pr)) = (repo.as_deref(), pr_number) {
        let (owner, repo_name) = parse_owner_repo(repo)?;
        return Ok((owner, repo_name, pr, None));
    }

    // 尝试从当前工作区上下文获取
    let v = gh_api_json(&[
        "pr",
        "view",
        "--json",
        "number,headRepository,baseRepository,headRefName",
    ])
    .await
    .map_err(|e| format!("gh pr view failed: {}", e))?;

    let pr_view: GhPrView =
        serde_json::from_value(v).map_err(|e| format!("parse gh pr view json failed: {}", e))?;

    let pr = pr_number.unwrap_or(pr_view.number);

    let repo_name = if let Some(repo) = repo.as_deref() {
        let (owner, repo_name) = parse_owner_repo(repo)?;
        return Ok((owner, repo_name, pr, pr_view.head_ref_name));
    } else if let Some(r) = pr_view.head_repository.or(pr_view.base_repository) {
        (r.owner.login, r.name)
    } else {
        return Err(
            "Cannot parse repo from gh pr view, please provide repo=owner/repo explicitly"
                .to_string(),
        );
    };

    Ok((repo_name.0, repo_name.1, pr, pr_view.head_ref_name))
}

fn parse_owner_repo(s: &str) -> Result<(String, String), String> {
    let s = s.trim();
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() != 2 {
        return Err(format!(
            "invalid repo format '{}', expected 'owner/repo'",
            s
        ));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

async fn gh_api_json(args: &[&str]) -> Result<Value, String> {
    let mut cmd = Command::new("gh");
    cmd.args(args);

    // 尽量非交互
    cmd.env("PAGER", "cat");
    cmd.env("GIT_PAGER", "cat");
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("CI", "1");

    let output = cmd.output().await.map_err(|e| e.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let msg = if !stderr.trim().is_empty() {
            stderr.trim().to_string()
        } else {
            stdout.trim().to_string()
        };
        return Err(msg);
    }

    let s = String::from_utf8_lossy(&output.stdout).to_string();
    serde_json::from_str::<Value>(&s).map_err(|e| format!("invalid json: {}", e))
}

fn format_comments(
    issues: &[GhIssueComment],
    reviews: &[GhReviewComment],
    head_ref: Option<&str>,
) -> String {
    let mut out = String::new();

    if let Some(r) = head_ref {
        out.push_str(&format!("## PR Comments (Head: {})\n\n", r));
    } else {
        out.push_str("## PR Comments\n\n");
    }

    if issues.is_empty() && reviews.is_empty() {
        out.push_str("No comments found.");
        return out;
    }

    // Combine and sort by date?
    // Usually user wants to see general comments and then code comments.

    if !issues.is_empty() {
        out.push_str("### General Comments\n");
        for c in issues {
            let user = c
                .user
                .as_ref()
                .map(|u| u.login.as_deref().unwrap_or("unknown"))
                .unwrap_or("unknown");
            let body = c.body.as_deref().unwrap_or("");
            let date = c.created_at.as_deref().unwrap_or("");
            out.push_str(&format!("- **{}** ({}):\n", user, date));
            for line in body.lines() {
                out.push_str(&format!("  > {}\n", line));
            }
            out.push('\n');
        }
    }

    if !reviews.is_empty() {
        out.push_str("### Code Review Comments\n");
        // Group by thread (in_reply_to_id)
        let mut threads: HashMap<u64, Vec<&GhReviewComment>> = HashMap::new();
        let mut root_comments: Vec<&GhReviewComment> = Vec::new();

        for c in reviews {
            if let Some(pid) = c.in_reply_to_id {
                threads.entry(pid).or_default().push(c);
            } else {
                root_comments.push(c);
            }
        }

        // Sort roots by date?
        // root_comments.sort_by_key(|c| c.created_at.clone());

        for root in root_comments {
            print_review_comment(&mut out, root, 0);
            if let Some(replies) = threads.get(&root.id) {
                for reply in replies {
                    print_review_comment(&mut out, reply, 1);
                }
            }
        }
    }

    out
}

fn print_review_comment(out: &mut String, c: &GhReviewComment, depth: usize) {
    let indent = "  ".repeat(depth);
    let user = c
        .user
        .as_ref()
        .map(|u| u.login.as_deref().unwrap_or("unknown"))
        .unwrap_or("unknown");
    let body = c.body.as_deref().unwrap_or("");
    let path = c.path.as_deref().unwrap_or("");
    let line = c.line.or(c.original_line).unwrap_or(0);

    if depth == 0 {
        out.push_str(&format!("- **{}** at `{}:{}`:\n", user, path, line));
        if let Some(hunk) = &c.diff_hunk {
            out.push_str(&format!("  ```diff\n  {}\n  ```\n", hunk));
        }
    } else {
        out.push_str(&format!("{} - **{}** replied:\n", indent, user));
    }

    for l in body.lines() {
        out.push_str(&format!("{}  > {}\n", indent, l));
    }
    out.push('\n');
}
