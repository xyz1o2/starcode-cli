use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation,
    ToolResult as CoreToolResult,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone)]
pub struct GitInsightTool {
    working_dir: PathBuf,
}

impl GitInsightTool {
    pub fn new() -> Self {
        Self::with_working_dir(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }

    /// 所有 Git 与 gh 子命令都使用所属代理的工作目录。
    pub fn with_working_dir(working_dir: PathBuf) -> Self {
        Self { working_dir }
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
    working_dir: PathBuf,
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
                &self.working_dir,
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
        "GitInsight"
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
        Ok(Box::new(GitInsightInvocation {
            params,
            working_dir: self.working_dir.clone(),
        }))
    }
}

fn repository_command(program: &str, working_dir: &Path) -> Command {
    // 复用工作树管理的清理规则，防止父进程环境把 Git 重定向到主仓库。
    let mut command = tokio::process::Command::new(program);
    crate::agent::worktree::clear_git_repository_env(&mut command);
    command.current_dir(working_dir);
    command.into_std()
}

async fn execute_git_command(
    working_dir: &Path,
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

        let mut cmd = repository_command("gh", working_dir);
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

    let mut cmd = repository_command("git", working_dir);
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
            let mut hash_cmd = repository_command("git", working_dir);
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
            let mut show_cmd = repository_command("git", working_dir);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn git(root: &Path, args: &[&str]) -> String {
        let output = repository_command("git", root)
            .args(args)
            .output()
            .expect("run fixture git command");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    async fn invoke(tool: &GitInsightTool, params: serde_json::Value) -> CoreToolResult {
        let invocation = tool
            .create_invocation(params)
            .expect("create GitInsight invocation");
        let result = invocation
            .execute(None, None)
            .await
            .expect("execute GitInsight invocation");
        assert!(
            result.error.is_none(),
            "GitInsight failed: {:?}",
            result.error
        );
        result
    }

    #[tokio::test]
    async fn relative_git_reads_and_add_use_agent_worktree_and_keep_parent_unchanged() {
        let process_cwd = std::env::current_dir().expect("read process cwd");
        let parent = tempfile::tempdir().expect("create parent repository");
        let worktree = tempfile::tempdir().expect("create worktree directory");
        git(parent.path(), &["init"]);
        std::fs::write(parent.path().join("shared.rs"), "fn original() {}\n")
            .expect("write shared fixture");
        std::fs::write(parent.path().join("co_changed.rs"), "fn companion() {}\n")
            .expect("write companion fixture");
        git(parent.path(), &["add", "shared.rs", "co_changed.rs"]);
        let hooks = parent.path().join("empty-hooks");
        std::fs::create_dir(&hooks).expect("create empty hook directory");
        let hooks_config = format!("core.hooksPath={}", hooks.display());
        git(
            parent.path(),
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
                "-c",
                &hooks_config,
                "commit",
                "-m",
                "fixture history",
            ],
        );
        git(
            parent.path(),
            &[
                "-c",
                &hooks_config,
                "worktree",
                "add",
                "-b",
                "isolated",
                worktree.path().to_str().expect("worktree path is UTF-8"),
            ],
        );
        std::fs::write(parent.path().join("shared.rs"), "fn parent_only() {}\n")
            .expect("modify parent fixture");
        std::fs::write(worktree.path().join("shared.rs"), "fn child_only() {}\n")
            .expect("modify worktree fixture");
        let parent_status = git(parent.path(), &["status", "--porcelain"]);
        let parent_index =
            std::fs::read(parent.path().join(".git/index")).expect("read parent index");
        let tool = GitInsightTool::with_working_dir(worktree.path().to_path_buf());

        let diff = invoke(
            &tool,
            serde_json::json!({"command": "diff", "file_path": "shared.rs"}),
        )
        .await;
        let diff_data = diff.data.expect("diff data");
        let diff_text = diff_data["diff"].as_str().expect("diff text");
        assert!(diff_text.contains("child_only"), "{diff_text}");
        assert!(!diff_text.contains("parent_only"), "{diff_text}");
        let related = invoke(
            &tool,
            serde_json::json!({"command": "related", "file_path": "shared.rs"}),
        )
        .await;
        assert!(
            related.output.contains("co_changed.rs"),
            "{}",
            related.output
        );
        let churn = invoke(&tool, serde_json::json!({"command": "churn"})).await;
        assert!(churn.output.contains("shared.rs"), "{}", churn.output);
        let log = invoke(
            &tool,
            serde_json::json!({"command": "log", "file_path": "shared.rs"}),
        )
        .await;
        assert!(log.output.contains("fixture history"), "{}", log.output);
        let blame = invoke(
            &tool,
            serde_json::json!({"command": "blame", "file_path": "shared.rs"}),
        )
        .await;
        assert!(blame.output.contains("child_only"), "{}", blame.output);
        invoke(
            &tool,
            serde_json::json!({"command": "add", "file_path": "shared.rs"}),
        )
        .await;
        let status = invoke(&tool, serde_json::json!({"command": "status"})).await;
        assert_eq!(status.output, "M  shared.rs\n");
        assert!(
            git(worktree.path(), &["diff", "--cached", "--", "shared.rs"]).contains("child_only")
        );
        assert_eq!(
            git(parent.path(), &["status", "--porcelain"]),
            parent_status
        );
        assert_eq!(
            std::fs::read(parent.path().join(".git/index")).expect("read parent index"),
            parent_index
        );
        assert_eq!(
            std::fs::read_to_string(parent.path().join("shared.rs")).expect("read parent file"),
            "fn parent_only() {}\n"
        );
        assert_eq!(
            std::env::current_dir().expect("read process cwd"),
            process_cwd
        );
    }

    #[test]
    fn git_and_gh_commands_clear_repository_environment_and_bind_working_dir() {
        let worktree = tempfile::tempdir().expect("create worktree directory");
        for program in ["git", "gh"] {
            let command = repository_command(program, worktree.path());
            assert_eq!(command.get_current_dir(), Some(worktree.path()));
            for key in [
                "GIT_DIR",
                "GIT_WORK_TREE",
                "GIT_INDEX_FILE",
                "GIT_COMMON_DIR",
                "GIT_OBJECT_DIRECTORY",
                "GIT_ALTERNATE_OBJECT_DIRECTORIES",
                "GIT_PREFIX",
                "GIT_SHALLOW_FILE",
                "GIT_QUARANTINE_PATH",
            ] {
                assert!(
                    command
                        .get_envs()
                        .any(|(name, value)| name == key && value.is_none()),
                    "{program} must remove inherited {key}"
                );
            }
        }
    }
}
