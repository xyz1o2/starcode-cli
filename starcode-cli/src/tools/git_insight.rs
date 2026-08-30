use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation,
    ToolResult as CoreToolResult,
};
use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Clone)]
pub struct GitInsightTool;

impl GitInsightTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GitInsightParams {
    pub command: String, // "log", "diff", "status", "blame", "churn", "related"
    pub max_count: Option<usize>, // limit for log
    pub file_path: Option<String>, // filter by file
    pub branch: Option<String>, // for diff/log
    pub author: Option<String>, // filter log by author
    pub since: Option<String>, // filter log by time
    pub message: Option<String>, // for commit message
    pub title: Option<String>, // for pr_create
    pub body: Option<String>, // for pr_create
}

pub struct GitInsightInvocation {
    params: GitInsightParams,
}

impl ToolInvocation for GitInsightInvocation {
    fn get_description(&self) -> String {
        format!(
            "Git Insight: {} {}",
            self.params.command,
            self.params.file_path.as_deref().unwrap_or("")
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
            dyn std::future::Future<Output = Result<CoreToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let params = self.params.command.clone();
        let max_count = self.params.max_count;
        let file_path = self.params.file_path.clone();
        let branch = self.params.branch.clone();
        let author = self.params.author.clone();
        let since = self.params.since.clone();
        let message = self.params.message.clone();
        let title = self.params.title.clone();
        let body = self.params.body.clone();

        Box::pin(async move {
            let result = execute_git_command(
                &params,
                max_count,
                file_path.as_deref(),
                branch.as_deref(),
                author.as_deref(),
                since.as_deref(),
                message.as_deref(),
                title.as_deref(),
                body.as_deref(),
            )
            .await;

            match result {
                Ok(output) => {
                    // Truncate large git output to prevent UI freeze
                    const MAX_GIT_OUTPUT: usize = 100 * 1024; // 100KB
                    let truncated_output = if output.len() > MAX_GIT_OUTPUT {
                        let safe: String = output.chars().take(MAX_GIT_OUTPUT).collect();
                        format!(
                            "{}...\n(output truncated to 100KB, total: {})",
                            safe,
                            output.len()
                        )
                    } else {
                        output
                    };

                    let mut data = None;
                    let final_output = if params == "diff" {
                        data = Some(serde_json::json!({
                            "diff": truncated_output
                        }));
                        "Git Diff Result:".to_string()
                    } else {
                        truncated_output
                    };

                    Ok(CoreToolResult {
                        llm_content: None,
                        return_display: None,
                        output: final_output,
                        error: None,
                        data,
                    })
                }
                Err(e) => Ok(CoreToolResult {
                    llm_content: None,
                    return_display: None,
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "execution_error".to_string(),
                        message: e.to_string(),
                    }),
                    data: None,
                }),
            }
        })
    }
}

impl BaseDeclarativeTool for GitInsightTool {
    fn name(&self) -> &str {
        "git_insight"
    }

    fn display_name(&self) -> &str {
        "Git Insight"
    }

    fn description(&self) -> &str {
        "Retrieve git history, diffs, and status to understand code evolution."
    }

    fn kind(&self) -> Kind {
        Kind::Execute // Using Execute kind as it runs commands
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "enum": ["log", "diff", "status", "blame", "churn", "related", "add", "commit", "push", "pull", "checkout", "branch", "merge", "reset", "pr_create"],
                    "description": "Git operation to perform. 'churn' analyzes file modification frequency. 'pr_create' creates a GitHub PR."
                },
                "max_count": {
                    "type": "integer",
                    "description": "Max number of commits for log (default 10)"
                },
                "file_path": {
                    "type": "string",
                    "description": "Optional file path to filter log, diff, or blame. Required for 'add'."
                },
                "branch": {
                    "type": "string",
                    "description": "Optional branch or commit reference"
                },
                "author": {
                    "type": "string",
                    "description": "Optional author filter for log"
                },
                "since": {
                    "type": "string",
                    "description": "Optional time filter for log (e.g. '2 weeks ago')"
                },
                "message": {
                    "type": "string",
                    "description": "Commit message (required for 'commit' command)"
                },
                "title": {
                    "type": "string",
                    "description": "PR title (required for 'pr_create')"
                },
                "body": {
                    "type": "string",
                    "description": "PR body (required for 'pr_create')"
                }
            },
            "required": ["command"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: GitInsightParams = serde_json::from_value(params)?;
        Ok(Box::new(GitInsightInvocation { params }))
    }
}

async fn execute_git_command(
    command: &str,
    max_count: Option<usize>,
    file_path: Option<&str>,
    branch: Option<&str>,
    author: Option<&str>,
    since: Option<&str>,
    message: Option<&str>,
    title: Option<&str>,
    body: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    if command == "pr_create" {
        let title = title.ok_or("Title is required for pr_create")?;
        let body = body.ok_or("Body is required for pr_create")?;

        let mut cmd = Command::new("gh");
        cmd.arg("pr");
        cmd.arg("create");
        cmd.arg("--title");
        cmd.arg(title);
        cmd.arg("--body");
        cmd.arg(body);

        if let Some(b) = branch {
            cmd.arg("--head");
            cmd.arg(b);
        }

        let output = cmd.output()?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(format!("gh pr create failed: {}", err).into());
        }
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }

    let mut cmd = Command::new("git");
    cmd.arg("--no-pager"); // Ensure no pager is used

    match command {
        "add" => {
            let path = file_path.ok_or("File path is required for add")?;
            cmd.arg("add");
            cmd.arg(path);
        }
        "commit" => {
            let msg = message.ok_or("Commit message is required")?;
            cmd.arg("commit");
            cmd.arg("-m");
            cmd.arg(msg);
        }
        "push" => {
            cmd.arg("push");
            if let Some(b) = branch {
                cmd.arg("origin");
                cmd.arg(b);
            }
        }
        "pull" => {
            cmd.arg("pull");
            if let Some(b) = branch {
                cmd.arg("origin");
                cmd.arg(b);
            }
        }
        "checkout" => {
            cmd.arg("checkout");
            if let Some(b) = branch {
                cmd.arg(b);
            } else if let Some(path) = file_path {
                // Checkout file (restore)
                cmd.arg(path);
            } else {
                return Err("Branch or file path is required for checkout".into());
            }
        }
        "branch" => {
            cmd.arg("branch");
            if let Some(b) = branch {
                // Create branch if message is provided as "create" or default, delete if "delete"
                if let Some(action) = message {
                    if action == "delete" {
                        cmd.arg("-d");
                    } else if action == "create" {
                        // Default behavior for `git branch <name>` is create
                    }
                }
                cmd.arg(b);
            } else {
                // List branches
                cmd.arg("-a");
            }
        }
        "merge" => {
            cmd.arg("merge");
            if let Some(b) = branch {
                cmd.arg(b);
            } else {
                return Err("Branch name is required for merge".into());
            }
        }
        "reset" => {
            cmd.arg("reset");
            if let Some(b) = branch {
                // Soft reset to a commit/branch
                cmd.arg(b);
            }
            // If file_path provided, reset that file
            if let Some(path) = file_path {
                cmd.arg(path);
            }
        }
        "log" => {
            cmd.arg("log");
            if let Some(n) = max_count {
                cmd.arg(format!("-n{}", n));
            } else {
                cmd.arg("-n10");
            }
            cmd.arg("--pretty=format:Commit: %h%nAuthor: %an%nDate: %ad%nSummary: %s%n");

            if let Some(b) = branch {
                cmd.arg(b);
            }

            if let Some(a) = author {
                cmd.arg(format!("--author={}", a));
            }

            if let Some(s) = since {
                cmd.arg(format!("--since={}", s));
            }

            if let Some(path) = file_path {
                cmd.arg("--");
                cmd.arg(path);
            }
        }
        "diff" => {
            cmd.arg("diff");
            if let Some(b) = branch {
                cmd.arg(b);
            }
            if let Some(path) = file_path {
                cmd.arg("--");
                cmd.arg(path);
            }
        }
        "status" => {
            cmd.arg("status");
            cmd.arg("-s"); // Short format
        }
        "blame" => {
            cmd.arg("blame");
            if let Some(path) = file_path {
                cmd.arg(path);
            } else {
                return Err("File path is required for blame".into());
            }
        }
        "churn" => {
            // Complex command: git log --name-only --format= | sort | uniq -c | sort -nr | head -n 20
            // We implement this by running git log and processing in Rust
            cmd.arg("log");
            cmd.arg("--name-only");
            cmd.arg("--format="); // Empty format to only get file names

            if let Some(s) = since {
                cmd.arg(format!("--since={}", s));
            } else {
                // Default to last 3 months for churn analysis if not specified
                cmd.arg("--since=3.months.ago");
            }

            let output = cmd.output()?;
            if !output.status.success() {
                let err = String::from_utf8_lossy(&output.stderr).to_string();
                return Err(format!("Git command failed: {}", err).into());
            }

            let output_str = String::from_utf8_lossy(&output.stdout);
            let mut counts: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();

            for line in output_str.lines() {
                let line = line.trim();
                if !line.is_empty() {
                    *counts.entry(line.to_string()).or_insert(0) += 1;
                }
            }

            let mut sorted: Vec<_> = counts.into_iter().collect();
            sorted.sort_by(|a, b| b.1.cmp(&a.1));

            let limit = max_count.unwrap_or(20);
            let mut result = String::from("File Modification Frequency (Churn):\n");
            for (file, count) in sorted.into_iter().take(limit) {
                result.push_str(&format!("{}: {}\n", count, file));
            }
            return Ok(result);
        }
        "related" => {
            // Find files frequently committed together with the target file
            let target_path =
                file_path.ok_or("File path is required for related files analysis")?;

            // 1. Get all commit hashes that touched this file
            let mut hash_cmd = Command::new("git");
            hash_cmd.arg("log").arg("--format=%H").arg(target_path);
            if let Some(s) = since {
                hash_cmd.arg(format!("--since={}", s));
            }

            let hash_output = hash_cmd.output()?;
            if !hash_output.status.success() {
                return Err("Failed to get commit history for file".into());
            }

            let hashes = String::from_utf8_lossy(&hash_output.stdout);

            let mut counts: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();

            // 2. For each commit, list all files
            // To be efficient, we can do: git show --name-only --format= <hash1> <hash2> ...
            // But command line length limit might be an issue.
            // Better: git log --name-only --format=COMMIT_BOUNDARY <hashes> is tricky because we need to group by commit.
            // Actually, we can just run: git log --name-only --format= <path>
            // Wait, that only lists the file itself (and others in the same commit? No, it filters commits, but lists all files in those commits? No, `git log <path>` filters diffs to only that path).
            // We need: Find commits touching <path>, then list ALL files in those commits.

            // Approach:
            // git log --format=%H <path> -> list of hashes
            // then: git show --name-only --format= %H

            // Optimization: Process in chunks or just iterate if not too many.
            // Let's take latest 50 commits to avoid performance issues.

            let hash_list: Vec<&str> = hashes.lines().take(50).collect();

            if hash_list.is_empty() {
                return Ok("No history found for this file.".to_string());
            }

            // We can pass multiple hashes to `git show`
            let mut show_cmd = Command::new("git");
            show_cmd.arg("show").arg("--name-only").arg("--format=");
            show_cmd.args(&hash_list);

            let show_output = show_cmd.output()?;
            let show_content = String::from_utf8_lossy(&show_output.stdout);

            for line in show_content.lines() {
                let line = line.trim();
                if !line.is_empty() && line != target_path {
                    *counts.entry(line.to_string()).or_insert(0) += 1;
                }
            }

            let mut sorted: Vec<_> = counts.into_iter().collect();
            sorted.sort_by(|a, b| b.1.cmp(&a.1));

            let limit = max_count.unwrap_or(10);
            let mut result = String::from(format!(
                "Files frequently co-changed with '{}':\n",
                target_path
            ));
            if sorted.is_empty() {
                result.push_str("None found.");
            } else {
                for (file, count) in sorted.into_iter().take(limit) {
                    result.push_str(&format!("{}: {}\n", count, file));
                }
            }
            return Ok(result);
        }
        _ => return Err("Invalid command".into()),
    }

    let output = cmd.output()?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let err = String::from_utf8_lossy(&output.stderr).to_string();
        Err(format!("Git command failed: {}", err).into())
    }
}
