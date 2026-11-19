use crate::core::tools::tools::ToolResult as CoreToolResult;
use crate::core::tools::tools::{BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

#[derive(Clone)]
pub struct RunTestsTool {
    working_dir: PathBuf,
}

impl RunTestsTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self {
            working_dir: config.agent_working_dir().to_path_buf(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RunTestsParams {
    pub command: Option<String>,
    pub filter: Option<String>,
}

pub struct RunTestsInvocation {
    params: RunTestsParams,
    working_dir: PathBuf,
}

impl ToolInvocation for RunTestsInvocation {
    fn get_description(&self) -> String {
        format!(
            "Run Tests: {}",
            self.params.command.as_deref().unwrap_or("auto-detect")
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
        let command = self.params.command.clone();
        let filter = self.params.filter.clone();
        let working_dir = self.working_dir.clone();

        Box::pin(async move {
            let res = tokio::task::spawn_blocking(move || -> Result<CoreToolResult, String> {
                let (cmd, args) = match command {
                    Some(command) => parse_test_command(&command).map_err(|e| e.to_string())?,
                    None => detect_test_runner(&working_dir).map_err(|e| e.to_string())?,
                };

                let mut command_builder = test_command(&cmd, &args, &working_dir);

                if let Some(f) = filter {
                    command_builder.arg(f);
                }

                let output = command_builder.output().map_err(|e| e.to_string())?;
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = format!("STDOUT:\n{}\n\nSTDERR:\n{}", stdout, stderr);

                // Analyze output for failures
                let (status, summary) = analyze_test_output(&cmd, &stdout, &stderr);

                // Safe char-based truncation + ANSI strip to avoid UI artifacts
                let truncated: String = if combined.len() > 5000 {
                    let cleaned = crate::ui::utils::render::strip_ansi_codes(&combined);
                    format!(
                        "{}... (truncated)",
                        cleaned.chars().take(5000).collect::<String>()
                    )
                } else {
                    crate::ui::utils::render::strip_ansi_codes(&combined)
                };

                let result_json = serde_json::json!({
                    "status": status,
                    "summary": summary,
                    "full_output_truncated": truncated,
                });

                Ok(CoreToolResult {
                    llm_content: None,
                    return_display: None,
                    output: serde_json::to_string_pretty(&result_json)
                        .map_err(|e| e.to_string())?,
                    error: None,
                    data: Some(result_json),
                })
            })
            .await;

            match res {
                Ok(inner) => inner.map_err(|e| -> Box<dyn std::error::Error> { e.into() }),
                Err(e) => Err(Box::new(e) as Box<dyn std::error::Error>),
            }
        })
    }
}

fn parse_test_command(
    command: &str,
) -> Result<(String, Vec<String>), Box<dyn std::error::Error + Send + Sync>> {
    let parts: Vec<&str> = command.split_whitespace().collect();
    if parts.is_empty() {
        return Err("Empty command".into());
    }
    Ok((
        parts[0].to_string(),
        parts[1..].iter().map(|part| (*part).to_string()).collect(),
    ))
}

fn detect_test_runner(
    working_dir: &Path,
) -> Result<(String, Vec<String>), Box<dyn std::error::Error + Send + Sync>> {
    if working_dir.join("Cargo.toml").exists() {
        Ok(("cargo".to_string(), vec!["test".to_string()]))
    } else if working_dir.join("package.json").exists() {
        Ok(("npm".to_string(), vec!["test".to_string()]))
    } else if working_dir.join("pyproject.toml").exists()
        || working_dir.join("requirements.txt").exists()
    {
        Ok(("pytest".to_string(), Vec::new()))
    } else {
        Err("Could not auto-detect test runner. Please specify command.".into())
    }
}

fn test_command(program: &str, args: &[String], working_dir: &Path) -> Command {
    let mut command = tokio::process::Command::new(program);
    crate::agent::worktree::clear_git_repository_env(&mut command);
    command.args(args).current_dir(working_dir);
    command.into_std()
}

impl BaseDeclarativeTool for RunTestsTool {
    fn name(&self) -> &str {
        "RunTests"
    }

    fn display_name(&self) -> &str {
        "Run Tests"
    }

    fn description(&self) -> &str {
        "Run project tests and analyze results. Auto-detects Rust, JS, and Python projects."
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Test command to run (e.g., 'cargo test', 'npm test'). If omitted, auto-detects."
                },
                "filter": {
                    "type": "string",
                    "description": "Filter to run specific tests (passed as argument to the test runner)"
                }
            }
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: RunTestsParams = serde_json::from_value(params)?;
        Ok(Box::new(RunTestsInvocation {
            params,
            working_dir: self.working_dir.clone(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn config_for(root: &Path) -> crate::core::config::Config {
        let mut params = crate::core::config::ConfigParameters::default();
        params.model = "test-model".to_string();
        params.core_tools = Some(Vec::new());
        params.mcp_enabled = Some(false);
        params.enable_agents = Some(false);
        params.target_dir = root.to_path_buf();
        let mut config = crate::core::config::Config::new(params);
        config.initialize().await.expect("initialize test config");
        config
    }

    #[tokio::test]
    async fn invocation_uses_agent_worktree_for_command_and_auto_detection() {
        let process_cwd = std::env::current_dir().expect("read process cwd");
        let parent = tempfile::tempdir().expect("create parent directory");
        let worktree = tempfile::tempdir().expect("create agent worktree");
        std::fs::write(
            parent.path().join("Cargo.toml"),
            "[package]\nname = \"parent\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .expect("write parent manifest");
        std::fs::write(
            worktree.path().join("Cargo.toml"),
            "[package]\nname = \"child\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .expect("write worktree manifest");
        let mut config = config_for(parent.path()).await;
        config.agent_working_dir = Some(worktree.path().to_path_buf());
        assert_eq!(
            detect_test_runner(worktree.path())
                .expect("detect child manifest")
                .0,
            "cargo"
        );
        let tool = RunTestsTool::new(Arc::new(config));

        let marker = tool
            .create_invocation(serde_json::json!({ "command": "printf child-marker" }))
            .expect("create marker invocation");
        let result = marker
            .execute(None, None)
            .await
            .expect("execute local printf command");
        assert!(result.output.contains("child-marker"), "{}", result.output);

        let pwd = tool
            .create_invocation(serde_json::json!({ "command": "pwd" }))
            .expect("create pwd invocation");
        let result = pwd
            .execute(None, None)
            .await
            .expect("execute local pwd command");
        assert!(
            result
                .output
                .contains(worktree.path().to_string_lossy().as_ref()),
            "{}",
            result.output
        );
        assert!(
            !result
                .output
                .contains(parent.path().to_string_lossy().as_ref()),
            "{}",
            result.output
        );
        assert_eq!(
            std::env::current_dir().expect("read process cwd"),
            process_cwd
        );
    }

    #[test]
    fn test_command_sets_directory_and_clears_git_repository_environment() {
        let worktree = tempfile::tempdir().expect("create worktree directory");
        let command = test_command("printf", &["marker".to_string()], worktree.path());
        assert_eq!(command.get_current_dir(), Some(worktree.path()));
        assert!(command
            .get_envs()
            .any(|(name, value)| name == "GIT_DIR" && value.is_none()));
    }
}

fn analyze_test_output(runner: &str, stdout: &str, stderr: &str) -> (String, String) {
    if runner == "cargo" {
        if stdout.contains("FAILED") || stderr.contains("FAILED") {
            // Extract failed tests
            let re = Regex::new(r"test (.*?) \.\.\. FAILED").unwrap();
            let failed: Vec<String> = re
                .captures_iter(stdout)
                .map(|cap| cap[1].to_string())
                .collect();

            if !failed.is_empty() {
                return (
                    "failed".to_string(),
                    format!("Failed tests: {}", failed.join(", ")),
                );
            }
            return (
                "failed".to_string(),
                "Tests failed (see output)".to_string(),
            );
        }
    } else if runner == "npm" {
        if stdout.contains("failing") || stderr.contains("failing") {
            return ("failed".to_string(), "JS Tests failed".to_string());
        }
    }

    // Default success check
    if !stdout.contains("error") && !stderr.contains("error") && !stdout.contains("FAIL") {
        ("passed".to_string(), "All tests passed".to_string())
    } else {
        (
            "failed".to_string(),
            "Tests failed or errors detected".to_string(),
        )
    }
}
