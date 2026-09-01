//! Verify Agent - build / test / lint verification
//!
//! Automatically detects project type and runs the appropriate
//! verification commands: build, test, lint.
//!
//! Used when the main agent needs to verify that changes compile
//! and pass tests before considering a task complete.

use super::{SubAgent, SubTask, SubTaskResult};
use async_trait::async_trait;
use std::time::Duration;

pub struct VerifyAgent {
    id: String,
}

impl VerifyAgent {
    pub fn new() -> Self {
        Self {
            id: "verify".to_string(),
        }
    }

    pub fn boxed() -> Box<dyn SubAgent> {
        Box::new(Self::new())
    }

    /// Run a command with timeout and capture output.
    fn run_bash(cmd: &str, args: &[&str], timeout_secs: u64) -> (bool, String, String) {
        use std::process::{Command as StdCommand, Stdio};
        use std::sync::mpsc;
        use std::thread;

        let cmd_owned = cmd.to_string();
        let args_owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();

        let (tx, rx) = mpsc::channel();

        let child = match StdCommand::new(&cmd_owned)
            .args(&args_owned)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return (false, String::new(), format!("Failed to start {}: {}", cmd_owned, e)),
        };

        let child_id = child.id();
        thread::spawn(move || {
            let result = (|| -> Result<(bool, String, String), String> {
                let output = child
                    .wait_with_output()
                    .map_err(|e| format!("Failed to wait on {}: {}", cmd_owned, e))?;

                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                Ok((output.status.success(), stdout, stderr))
            })();

            let _ = tx.send(result);
        });

        match rx.recv_timeout(Duration::from_secs(timeout_secs)) {
            Ok(Ok((success, stdout, stderr))) => (success, stdout, stderr),
            Ok(Err(e)) => (false, String::new(), e),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Kill the child process to prevent zombies
                let _ = StdCommand::new("kill")
                    .args(["-9", &child_id.to_string()])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn();
                (false, String::new(), format!("Command timed out after {}s", timeout_secs))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                (false, String::new(), format!("Command {} process ended unexpectedly", cmd))
            }
        }
    }

    /// Auto-detect project type and return verification commands.
    fn detect_commands(&self, task: &SubTask) -> Vec<(&'static str, &'static str, Vec<&'static str>, u64)> {
        let target = task.target.clone();
        let cwd = std::env::current_dir().unwrap_or_default();
        let search_dir = if target.is_empty() || target == "." {
            cwd
        } else {
            std::path::PathBuf::from(&target)
        };

        // Detect language by checking for build files
        let has_cargo = search_dir.join("Cargo.toml").exists()
            || std::env::current_dir()
                .unwrap_or_default()
                .join("Cargo.toml")
                .exists();
        let has_package_json = search_dir.join("package.json").exists();
        let has_pyproject = search_dir.join("pyproject.toml").exists();

        if has_cargo {
            vec![
                ("build", "cargo", vec!["check"], 120),
                ("test", "cargo", vec!["test"], 120),
                ("lint", "cargo", vec!["clippy", "--", "-D", "warnings"], 120),
            ]
        } else if has_package_json {
            vec![
                ("build", "npm", vec!["run", "build"], 120),
                ("test", "npm", vec!["test"], 120),
                ("lint", "npm", vec!["run", "lint"], 60),
            ]
        } else if has_pyproject {
            vec![
                ("build", "python", vec!["-m", "compileall", "."], 60),
                ("test", "pytest", vec![], 120),
                ("lint", "ruff", vec!["check", "."], 60),
            ]
        } else {
            vec![
                ("build", "cargo", vec!["check"], 120),
                ("test", "cargo", vec!["test"], 120),
            ]
        }
    }
}

#[async_trait]
impl SubAgent for VerifyAgent {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        "Verify Agent"
    }

    fn capabilities(&self) -> Vec<String> {
        vec![
            "verify".to_string(),
            "test".to_string(),
            "build".to_string(),
            "lint".to_string(),
        ]
    }

    async fn execute(
        &self,
        task: SubTask,
    ) -> Result<SubTaskResult, Box<dyn std::error::Error>> {
        let commands = self.detect_commands(&task);

        let mut results = Vec::new();
        let mut all_passed = true;

        for (_label, cmd, args, timeout) in &commands {
            let (success, stdout, stderr) =
                Self::run_bash(cmd, args, *timeout);

            let status = if success { "PASS" } else { "FAIL" };
            let output_summary = if stdout.len() > 500 {
                format!("{}... ({} chars total)", &stdout[..500], stdout.len())
            } else {
                stdout.clone()
            };

            results.push(format!(
                "[{}] {} {} {} — {}",
                status,
                cmd,
                args.join(" "),
                if !stderr.is_empty() {
                    format!("\n  stderr: {}",
                        if stderr.len() > 300 { format!("{}...", &stderr[..300]) } else { stderr.clone() }
                    )
                } else {
                    String::new()
                },
                if success {
                    "OK".to_string()
                } else {
                    format!("Output: {}", output_summary)
                }
            ));

            if !success {
                all_passed = false;
            }
        }

        let summary = if all_passed {
            "All verification steps passed.".to_string()
        } else {
            format!(
                "{} verification step(s) failed. See details above.",
                results.iter().filter(|r| r.starts_with("[FAIL]")).count()
            )
        };

        Ok(SubTaskResult {
            task_id: task.id.clone(),
            success: all_passed,
            summary,
            details: Some(results.join("\n")),
            data: Some(serde_json::json!({
                "all_passed": all_passed,
                "steps": commands.iter().map(|(label, cmd, args, _)| {
                    serde_json::json!({
                        "label": label,
                        "command": format!("{} {}", cmd, args.join(" ")),
                    })
                }).collect::<Vec<_>>()
            })),
            next_action: if all_passed {
                None
            } else {
                Some("Review the failed verification steps above and fix the issues.".to_string())
            },
            error: if all_passed {
                None
            } else {
                Some("Some verification steps failed".to_string())
            },
        })
    }
}

impl Default for VerifyAgent {
    fn default() -> Self {
        Self::new()
    }
}
