use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[path = "workflow_dynamic.rs"]
mod dynamic;

const DEFAULT_WORKFLOW_TIMEOUT_SECS: u64 = 300;
const MAX_WORKFLOW_TIMEOUT_SECS: u64 = 1800;

/// 即使外层 watchdog 丢弃执行 future，也要终止工作流留下的 Unix 进程组。
struct WorkflowProcessGroup {
    #[cfg(unix)]
    id: u32,
    armed: bool,
}

impl Drop for WorkflowProcessGroup {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            // spawn 已设置 process_group(0)，负 PID 只指向该命令自己的进程组。
            if self.armed && self.id > 0 && self.id <= i32::MAX as u32 {
                unsafe { libc::kill(-(self.id as i32), libc::SIGKILL) };
            }
        }
    }
}

async fn run_workflow_command(
    command: &mut tokio::process::Command,
    signal: Option<&tokio_util::sync::CancellationToken>,
    deadline: tokio::time::Instant,
) -> Result<std::process::Output, String> {
    if signal.is_some_and(|signal| signal.is_cancelled()) {
        return Err("Workflow cancelled".into());
    }
    if tokio::time::Instant::now() >= deadline {
        return Err("Workflow timed out".into());
    }
    command
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let mut group = WorkflowProcessGroup {
        #[cfg(unix)]
        id: child.id().ok_or("Workflow process has no PID")?,
        armed: true,
    };
    let mut stdout = child.stdout.take().ok_or("Workflow stdout is not piped")?;
    let mut stderr = child.stderr.take().ok_or("Workflow stderr is not piped")?;
    let result = {
        let wait = async {
            use tokio::io::AsyncReadExt;
            let read_stdout = async {
                let mut bytes = Vec::new();
                stdout.read_to_end(&mut bytes).await?;
                Ok::<_, std::io::Error>(bytes)
            };
            let read_stderr = async {
                let mut bytes = Vec::new();
                stderr.read_to_end(&mut bytes).await?;
                Ok::<_, std::io::Error>(bytes)
            };
            // 先并行排空管道，再收尸；在此之前组长 PID 保持被占用，取消清理不会误伤复用 PID。
            let (stdout, stderr) = tokio::try_join!(read_stdout, read_stderr)?;
            let status = child.wait().await?;
            group.armed = false;
            Ok::<_, std::io::Error>(std::process::Output {
                status,
                stdout,
                stderr,
            })
        };
        tokio::pin!(wait);
        let result = tokio::select! {
            biased;
            _ = async {
                match signal {
                    Some(signal) => signal.cancelled().await,
                    None => std::future::pending::<()>().await,
                }
            } => Err("Workflow cancelled".into()),
            _ = tokio::time::sleep_until(deadline) => Err("Workflow timed out".into()),
            output = &mut wait => output.map_err(|error| error.to_string()),
        };
        result
    };
    if result.is_err() {
        // 借用 child 的等待 future 已退出，先杀组再回收直接子进程。
        drop(group);
        let _ = child.kill().await;
    }
    result
}

/// 工作流名称是目录中的文件 stem，不能携带目录或盘符。
pub(crate) fn validate_workflow_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() || matches!(name, "." | "..") || name.contains(['/', '\\', ':', '\0'])
    {
        return Err("Workflow name must be a single file stem without a directory".into());
    }
    Ok(())
}

#[derive(Deserialize)]
struct WorkflowDefinition {
    steps: Vec<WorkflowStep>,
}

#[derive(Deserialize)]
struct WorkflowStep {
    name: Option<String>,
    command: String,
}

#[derive(Clone)]
pub struct WorkflowTool {
    config: Arc<crate::core::config::Config>,
}

impl WorkflowTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self { config }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct WorkflowParams {
    #[serde(default)]
    pub workflow: Option<String>,
    #[serde(default)]
    pub script: Option<String>,
    #[serde(default)]
    pub args: Option<serde_json::Value>,
    #[serde(default)]
    pub resume_from_run_id: Option<String>,
    #[serde(default)]
    pub max_concurrency: Option<usize>,
    #[serde(default)]
    pub params: Option<HashMap<String, serde_json::Value>>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

pub struct WorkflowInvocation {
    config: Arc<crate::core::config::Config>,
    params: WorkflowParams,
    parent_client: Option<crate::llm::client::StarClient>,
    parent_context: Option<Arc<Vec<crate::types::StarMessage>>>,
    parent_effort: Option<crate::types::ThinkingEffort>,
    parent_identity: Option<String>,
}

impl ToolInvocation for WorkflowInvocation {
    fn set_agent_parent_context(&mut self, messages: Arc<Vec<crate::types::StarMessage>>) {
        self.parent_context = Some(messages);
    }

    fn set_agent_parent_client(&mut self, client: &crate::llm::client::StarClient) {
        self.parent_client = Some(client.clone());
    }

    fn set_agent_parent_thinking_effort(&mut self, effort: crate::types::ThinkingEffort) {
        self.parent_effort = Some(effort);
    }

    fn set_agent_parent_identity(&mut self, identity: Option<String>) {
        self.parent_identity = identity;
    }

    fn get_description(&self) -> String {
        format!(
            "Execute workflow: {}",
            self.params
                .workflow
                .as_deref()
                .unwrap_or("inline JavaScript")
        )
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
    }

    fn execute(
        &self,
        signal: Option<&tokio_util::sync::CancellationToken>,
        update_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let config = self.config.clone();
        let params = self.params.clone();
        let signal = signal.cloned();
        Box::pin(async move {
            if params.script.is_some()
                || params.workflow.as_ref().is_some_and(|name| {
                    validate_workflow_name(name).is_ok()
                        && config
                            .storage()
                            .project_workflows_dir()
                            .join(format!("{name}.js"))
                            .is_file()
                })
            {
                return dynamic::execute(self, signal, update_output)
                    .await
                    .map_err(Into::into);
            }
            let workflow = params
                .workflow
                .as_deref()
                .ok_or("Provide workflow or script")?;
            if params.resume_from_run_id.is_some()
                || params.args.is_some()
                || params.max_concurrency.is_some()
            {
                return Err("Dynamic workflow options require a JavaScript workflow".into());
            }
            validate_workflow_name(workflow)?;
            let timeout_secs = params.timeout_secs.unwrap_or(DEFAULT_WORKFLOW_TIMEOUT_SECS);
            if !(1..=MAX_WORKFLOW_TIMEOUT_SECS).contains(&timeout_secs) {
                return Err("Workflow timeout_secs must be between 1 and 1800".into());
            }
            let deadline =
                tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
            let workflows_dir = config.storage().project_workflows_dir();
            let workflow_file = workflows_dir.join(format!("{workflow}.json"));

            if !workflow_file.exists() {
                let script_file = workflows_dir.join(format!("{}.sh", workflow));
                if script_file.exists() {
                    let mut cmd = tokio::process::Command::new("bash");
                    cmd.arg(&script_file)
                        .current_dir(config.storage().project_root())
                        .kill_on_drop(true);

                    if let Some(ref workflow_params) = params.params {
                        for (key, value) in workflow_params {
                            let val_str = match value {
                                serde_json::Value::String(s) => s.clone(),
                                other => other.to_string(),
                            };
                            cmd.env(format!("WF_{}", key.to_uppercase()), val_str);
                        }
                    }

                    let output = run_workflow_command(&mut cmd, signal.as_ref(), deadline)
                        .await
                        .map_err(|e| format!("Failed to execute workflow script: {}", e))?;

                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);

                    if output.status.success() {
                        return Ok(ToolResult {
                            llm_content: Some(stdout.to_string()),
                            return_display: Some(format!("Workflow '{}' completed", workflow)),
                            output: stdout.to_string(),
                            error: None,
                            data: None,
                        });
                    } else {
                        return Ok(ToolResult {
                            llm_content: None,
                            return_display: None,
                            output: String::new(),
                            error: Some(ToolError {
                                error_type: "workflow_error".to_string(),
                                message: format!(
                                    "Workflow failed (exit {}):\n{}",
                                    output.status.code().unwrap_or(-1),
                                    if stderr.is_empty() { &stdout } else { &stderr }
                                ),
                            }),
                            data: None,
                        });
                    }
                }

                return Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "not_found".to_string(),
                        message: format!("Workflow '{}' not found", workflow),
                    }),
                    data: None,
                });
            }

            let content = tokio::fs::read_to_string(&workflow_file)
                .await
                .map_err(|e| format!("Failed to read workflow: {}", e))?;

            let workflow_def: WorkflowDefinition = serde_json::from_str(&content)
                .map_err(|e| format!("Failed to parse workflow JSON: {}", e))?;
            if workflow_def.steps.is_empty()
                || workflow_def
                    .steps
                    .iter()
                    .any(|step| step.command.trim().is_empty())
            {
                return Err("Workflow must contain non-empty steps and commands".into());
            }

            let mut results = Vec::new();
            for (i, step) in workflow_def.steps.iter().enumerate() {
                let default_name = format!("step_{}", i);
                let step_name = step.name.as_deref().unwrap_or(&default_name);
                let mut cmd = tokio::process::Command::new("bash");
                cmd.args(["-c", &step.command])
                    .current_dir(config.storage().project_root())
                    .kill_on_drop(true);

                if let Some(ref workflow_params) = params.params {
                    for (key, value) in workflow_params {
                        let val_str = match value {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        cmd.env(format!("WF_{}", key.to_uppercase()), val_str);
                    }
                }

                let output = run_workflow_command(&mut cmd, signal.as_ref(), deadline)
                    .await
                    .map_err(|e| format!("Step '{}' failed: {}", step_name, e))?;

                let stdout = String::from_utf8_lossy(&output.stdout);
                results.push(format!(
                    "[{}] {}: {}",
                    step_name,
                    if output.status.success() {
                        "OK"
                    } else {
                        "FAIL"
                    },
                    stdout.trim()
                ));
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Ok(ToolResult {
                        llm_content: Some(results.join("\n")),
                        output: results.join("\n"),
                        return_display: None,
                        error: Some(ToolError {
                            error_type: "workflow_error".into(),
                            message: format!(
                                "Step '{}' failed (exit {}):\n{}",
                                step_name,
                                output.status.code().unwrap_or(-1),
                                stderr.trim()
                            ),
                        }),
                        data: None,
                    });
                }
            }

            Ok(ToolResult {
                llm_content: Some(results.join("\n")),
                return_display: Some(format!("Workflow '{}' executed", workflow)),
                output: results.join("\n"),
                error: None,
                data: None,
            })
        })
    }
}

impl BaseDeclarativeTool for WorkflowTool {
    fn name(&self) -> &str {
        "WorkflowTool"
    }

    fn display_name(&self) -> &str {
        "Workflow"
    }

    fn description(&self) -> &str {
        "执行预定义的工作流脚本。工作流定义在 .star/workflows/ 目录中。(Execute a predefined workflow script. Workflows are defined in .star/workflows/ directory.)"
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "workflow": {
                    "type": "string",
                    "description": "工作流名称 (Workflow name)"
                },
                "script": { "type": "string", "description": "Inline JavaScript with agent, parallel, pipeline, phase, log and args; requires Node.js" },
                "args": { "description": "JSON input for a JavaScript workflow" },
                "resume_from_run_id": { "type": "string", "description": "Reuse successful matching calls from a previous run; starts a new run" },
                "max_concurrency": { "type": "integer", "minimum": 1, "maximum": 16, "default": 4 },
                "params": {
                    "type": "object",
                    "description": "Shell workflow parameters as key-value pairs"
                },
                "timeout_secs": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 1800,
                    "default": 300,
                    "description": "Total execution limit for the entire workflow, excluding approval time"
                }
            },
            "oneOf": [{ "required": ["workflow"], "not": { "required": ["script"] } }, { "required": ["script"], "not": { "required": ["workflow"] } }]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: WorkflowParams = serde_json::from_value(params)?;
        if params.workflow.is_some() == params.script.is_some() {
            return Err("Provide exactly one of workflow or script".into());
        }
        Ok(Box::new(WorkflowInvocation {
            config: self.config.clone(),
            params,
            parent_client: None,
            parent_context: None,
            parent_effort: None,
            parent_identity: None,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn execute_in(root: &std::path::Path, name: &str) -> Result<ToolResult, String> {
        let config = Arc::new(crate::core::config::Config::new(
            crate::core::config::ConfigParameters {
                target_dir: root.to_path_buf(),
                ..Default::default()
            },
        ));
        let tool = WorkflowTool::new(config);
        let invocation = tool
            .create_invocation(serde_json::json!({
                "workflow": name, "params": {"value": "from-param"}
            }))
            .unwrap();
        invocation
            .execute(None, None)
            .await
            .map_err(|e| e.to_string())
    }

    #[tokio::test]
    async fn cancelled_workflow_does_not_spawn_a_command() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("should-not-exist");
        let signal = tokio_util::sync::CancellationToken::new();
        signal.cancel();
        let mut command = tokio::process::Command::new("bash");
        command
            .args(["-c", "touch \"$1\"", "workflow-test"])
            .arg(&marker);
        let error = run_workflow_command(
            &mut command,
            Some(&signal),
            tokio::time::Instant::now() + std::time::Duration::from_secs(5),
        )
        .await
        .unwrap_err();
        assert!(error.contains("cancelled"));
        assert!(!marker.exists());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn cancellation_and_dropped_execution_terminate_the_running_process_group() {
        for drop_execution in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let marker = dir.path().join("child-pid");
            let signal = tokio_util::sync::CancellationToken::new();
            let task_signal = signal.clone();
            let task_marker = marker.clone();
            let task = tokio::spawn(async move {
                let mut command = tokio::process::Command::new("bash");
                command
                    .args([
                        "-c",
                        "sleep 30 & printf '%s' \"$!\" > \"$1\"; wait",
                        "workflow-test",
                    ])
                    .arg(task_marker);
                run_workflow_command(
                    &mut command,
                    Some(&task_signal),
                    tokio::time::Instant::now() + std::time::Duration::from_secs(5),
                )
                .await
            });
            let pid = tokio::time::timeout(std::time::Duration::from_secs(3), async {
                loop {
                    if let Ok(value) = tokio::fs::read_to_string(&marker).await {
                        if let Ok(pid) = value.parse::<u32>() {
                            break pid;
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            if drop_execution {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            } else {
                signal.cancel();
                assert!(task.await.unwrap().unwrap_err().contains("cancelled"));
            }
            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                loop {
                    // 孤儿由系统 init 收尸；僵尸已停止执行，不等同仍存活的脚本。
                    match tokio::fs::read_to_string(format!("/proc/{pid}/stat")).await {
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                        Ok(stat)
                            if stat
                                .rsplit_once(") ")
                                .is_some_and(|(_, fields)| fields.starts_with('Z')) =>
                        {
                            break
                        }
                        _ => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
                    }
                }
            })
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn workflow_command_observes_cancellation_and_deadline() {
        let signal = tokio_util::sync::CancellationToken::new();
        let mut command = tokio::process::Command::new("bash");
        command.args(["-c", "sleep 30"]);
        let cancel = signal.clone();
        let cancellation = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            cancel.cancel();
        });
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            run_workflow_command(
                &mut command,
                Some(&signal),
                tokio::time::Instant::now() + std::time::Duration::from_secs(10),
            ),
        )
        .await
        .unwrap()
        .unwrap_err();
        cancellation.await.unwrap();
        assert!(error.contains("cancelled"));

        let mut command = tokio::process::Command::new("bash");
        command.args(["-c", "sleep 30"]);
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            run_workflow_command(
                &mut command,
                None,
                tokio::time::Instant::now() + std::time::Duration::from_millis(50),
            ),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert!(error.contains("timed out"));
    }

    #[tokio::test]
    async fn json_steps_share_one_deadline_and_stop_before_later_steps() {
        let dir = tempfile::tempdir().unwrap();
        let config = Arc::new(crate::core::config::Config::new(
            crate::core::config::ConfigParameters {
                target_dir: dir.path().to_path_buf(),
                ..Default::default()
            },
        ));
        let workflows = config.storage().project_workflows_dir();
        std::fs::create_dir_all(&workflows).unwrap();
        std::fs::write(
            workflows.join("deadline.json"),
            serde_json::json!({"steps": [
                {"command": "sleep 0.7"},
                {"command": "sleep 0.7"},
                {"command": "touch should-not-exist"}
            ]})
            .to_string(),
        )
        .unwrap();
        let invocation = WorkflowTool::new(config)
            .create_invocation(serde_json::json!({
                "workflow": "deadline", "timeout_secs": 1
            }))
            .unwrap();
        let error = invocation.execute(None, None).await.unwrap_err();
        assert!(error.to_string().contains("timed out"));
        assert!(!dir.path().join("should-not-exist").exists());
    }

    #[tokio::test]
    async fn json_failure_stops_later_steps_and_preserves_diagnostics() {
        let dir = tempfile::tempdir().unwrap();
        let workflows = crate::core::config::storage::Storage::new(dir.path().to_path_buf())
            .project_workflows_dir();
        std::fs::create_dir_all(&workflows).unwrap();
        std::fs::write(
            workflows.join("fail.json"),
            serde_json::json!({"steps": [
                {"name": "first", "command": "printf first"},
                {"name": "broken", "command": "printf problem >&2; exit 7"},
                {"name": "later", "command": "touch should-not-exist"}
            ]})
            .to_string(),
        )
        .unwrap();
        let result = execute_in(dir.path(), "fail").await.unwrap();
        let error = result.error.unwrap();
        assert!(
            error.message.contains("broken")
                && error.message.contains("exit 7")
                && error.message.contains("problem")
        );
        assert!(result.output.contains("[first] OK") && result.output.contains("[broken] FAIL"));
        assert!(!dir.path().join("should-not-exist").exists());
    }

    #[tokio::test]
    async fn both_formats_inherit_project_root_and_parameters() {
        let dir = tempfile::tempdir().unwrap();
        let workflows = crate::core::config::storage::Storage::new(dir.path().to_path_buf())
            .project_workflows_dir();
        std::fs::create_dir_all(&workflows).unwrap();
        std::fs::write(
            workflows.join("json.json"),
            serde_json::json!({"steps": [
                {"command": "printf '%s' \"$WF_VALUE\" > json-marker"}
            ]})
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            workflows.join("shell.sh"),
            "printf '%s' \"$WF_VALUE\" > shell-marker",
        )
        .unwrap();
        let nested = dir.path().join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        for name in ["json", "shell"] {
            assert!(execute_in(&nested, name).await.unwrap().error.is_none());
            assert_eq!(
                std::fs::read_to_string(dir.path().join(format!("{name}-marker"))).unwrap(),
                "from-param"
            );
        }
    }

    #[tokio::test]
    async fn invalid_definitions_are_rejected_before_any_step_runs() {
        let dir = tempfile::tempdir().unwrap();
        let workflows = crate::core::config::storage::Storage::new(dir.path().to_path_buf())
            .project_workflows_dir();
        std::fs::create_dir_all(&workflows).unwrap();
        for content in [
            r#"{}"#,
            r#"{"steps":[]}"#,
            r#"{"steps":[{}]}"#,
            r#"{"steps":[{"command":"touch should-not-exist"},{"command":" "}]}"#,
        ] {
            std::fs::write(workflows.join("invalid.json"), content).unwrap();
            assert!(execute_in(dir.path(), "invalid").await.is_err());
            assert!(!dir.path().join("should-not-exist").exists());
        }
        for name in ["", "..", "../outside", "/tmp/outside", "a\\b", "C:outside"] {
            assert!(execute_in(dir.path(), name).await.is_err());
        }
    }
}
