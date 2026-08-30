use crate::core::tools::tools::ToolResult as CoreToolResult;
use crate::core::tools::tools::{BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::process::Command;
use std::sync::Arc;

#[derive(Clone)]
pub struct RunTestsTool;

impl RunTestsTool {
    pub fn new(_config: Arc<crate::core::config::Config>) -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RunTestsParams {
    pub command: Option<String>,
    pub filter: Option<String>,
}

pub struct RunTestsInvocation {
    params: RunTestsParams,
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

        Box::pin(async move {
            let res = tokio::task::spawn_blocking(move || {
                let (cmd, args) = if let Some(c) = command {
                    let parts: Vec<&str> = c.split_whitespace().collect();
                    if parts.is_empty() {
                        return Err(Box::<dyn std::error::Error + Send + Sync>::from(
                            "Empty command",
                        ));
                    }
                    (
                        parts[0].to_string(),
                        parts[1..].iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                    )
                } else {
                    // Auto-detect
                    if std::path::Path::new("Cargo.toml").exists() {
                        ("cargo".to_string(), vec!["test".to_string()])
                    } else if std::path::Path::new("package.json").exists() {
                        ("npm".to_string(), vec!["test".to_string()])
                    } else if std::path::Path::new("pyproject.toml").exists()
                        || std::path::Path::new("requirements.txt").exists()
                    {
                        ("pytest".to_string(), vec![])
                    } else {
                        return Err(Box::<dyn std::error::Error + Send + Sync>::from(
                            "Could not auto-detect test runner. Please specify command.",
                        ));
                    }
                };

                let mut command_builder = Command::new(&cmd);
                command_builder.args(&args);

                if let Some(f) = filter {
                    command_builder.arg(f);
                }

                let output = command_builder
                    .output()
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
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
                        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?,
                    error: None,
                    data: Some(result_json),
                })
            })
            .await;

            match res {
                Ok(inner) => inner.map_err(|e| e as Box<dyn std::error::Error>),
                Err(e) => Err(Box::new(e) as Box<dyn std::error::Error>),
            }
        })
    }
}

impl BaseDeclarativeTool for RunTestsTool {
    fn name(&self) -> &str {
        "run_tests"
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
        Ok(Box::new(RunTestsInvocation { params }))
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
