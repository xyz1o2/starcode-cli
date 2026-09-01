use super::{PluginToolPermission, ResolvedPluginTool};
use crate::core::tools::{
    BaseDeclarativeTool, ConfirmationType, Kind, ToolCallConfirmationDetails, ToolError,
    ToolInvocation, ToolLocation, ToolResult,
};
use serde_json::{json, Value};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const DEFAULT_PLUGIN_TOOL_TIMEOUT_SECS: u64 = 180;

pub fn build_plugin_declarative_tools(
    tools: Vec<ResolvedPluginTool>,
) -> Vec<Arc<dyn BaseDeclarativeTool>> {
    tools
        .into_iter()
        .map(|tool| Arc::new(PluginDeclarativeTool::new(tool)) as Arc<dyn BaseDeclarativeTool>)
        .collect()
}

struct PluginDeclarativeTool {
    spec: ResolvedPluginTool,
    description: String,
    display_name: String,
}

impl PluginDeclarativeTool {
    fn new(spec: ResolvedPluginTool) -> Self {
        let description = if spec.description.trim().is_empty() {
            format!("插件 `{}` 提供的工具。", spec.plugin_name)
        } else {
            format!("{} (plugin: {})", spec.description.trim(), spec.plugin_name)
        };

        Self {
            display_name: format!("Plugin Tool: {}", spec.plugin_name),
            spec,
            description,
        }
    }
}

impl BaseDeclarativeTool for PluginDeclarativeTool {
    fn name(&self) -> &str {
        &self.spec.name
    }

    fn display_name(&self) -> &str {
        &self.display_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn kind(&self) -> Kind {
        match self.spec.required_permission {
            PluginToolPermission::ReadOnly => Kind::Read,
            PluginToolPermission::WorkspaceWrite | PluginToolPermission::DangerFullAccess => {
                Kind::Execute
            }
        }
    }

    fn parameter_schema(&self) -> Value {
        self.spec.input_schema.clone()
    }

    fn create_invocation(
        &self,
        params: Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Box::new(PluginToolInvocation {
            spec: self.spec.clone(),
            params,
        }))
    }

    fn is_read_only(&self) -> bool {
        self.spec.required_permission == PluginToolPermission::ReadOnly
    }

    fn permission_cache_identity(&self) -> Option<String> {
        Some(plugin_tool_permission_identity(&self.spec))
    }

    fn normalize_confirmation_outcome(
        &self,
        outcome: crate::types::ToolConfirmationOutcome,
    ) -> crate::types::ToolConfirmationOutcome {
        normalize_plugin_tool_outcome(self.spec.required_permission, outcome)
    }
}

struct PluginToolInvocation {
    spec: ResolvedPluginTool,
    params: Value,
}

impl ToolInvocation for PluginToolInvocation {
    fn get_description(&self) -> String {
        format!(
            "执行插件工具 `{}` (plugin: {})",
            self.spec.name, self.spec.plugin_name
        )
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        let mut locations = vec![ToolLocation {
            path: self.spec.working_dir.clone(),
            location_type: crate::core::tools::tools::LocationType::Execute,
        }];

        let location_type = match self.spec.required_permission {
            PluginToolPermission::ReadOnly => crate::core::tools::tools::LocationType::Read,
            PluginToolPermission::WorkspaceWrite | PluginToolPermission::DangerFullAccess => {
                crate::core::tools::tools::LocationType::Write
            }
        };

        locations.push(ToolLocation {
            path: self.spec.project_root.clone(),
            location_type,
        });
        locations
    }

    fn should_confirm_execute(
        &self,
        _abort_signal: Option<&tokio_util::sync::CancellationToken>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        Option<ToolCallConfirmationDetails>,
                        Box<dyn std::error::Error + Send + Sync>,
                    >,
                > + Send
                + '_,
        >,
    > {
        let spec = self.spec.clone();
        let params = self.params.clone();
        Box::pin(async move {
            let confirmation_type = match spec.required_permission {
                PluginToolPermission::DangerFullAccess => ConfirmationType::Danger,
                PluginToolPermission::ReadOnly | PluginToolPermission::WorkspaceWrite => {
                    ConfirmationType::Warning
                }
            };

            Ok(Some(ToolCallConfirmationDetails {
                confirmation_type,
                title: plugin_tool_confirmation_title(&spec),
                prompt: plugin_tool_confirmation_prompt(&spec, &params),
                on_confirm: Arc::new(|_outcome| {}),
            }))
        })
    }

    fn execute(
        &self,
        signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let spec = self.spec.clone();
        let params = self.params.clone();
        let signal = signal.cloned();

        Box::pin(async move {
            if let Some(cancel) = signal.as_ref() {
                if cancel.is_cancelled() {
                    return Ok(plugin_tool_error_result(
                        &spec,
                        "plugin_tool_cancelled",
                        "Plugin tool execution was cancelled before start".to_string(),
                        None,
                        None,
                    ));
                }
            }

            let input_json = match serde_json::to_vec(&params) {
                Ok(input) => input,
                Err(err) => {
                    return Ok(plugin_tool_error_result(
                        &spec,
                        "plugin_tool_invalid_input",
                        format!("Plugin tool input serialization failed: {}", err),
                        None,
                        None,
                    ));
                }
            };

            let input_json_str = String::from_utf8_lossy(&input_json).to_string();
            let mut command = tokio::process::Command::new(&spec.command);
            command
                .args(&spec.args)
                .current_dir(&spec.working_dir)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .env("STAR_PLUGIN_NAME", &spec.plugin_name)
                .env("STAR_PLUGIN_SOURCE", &spec.source)
                .env("STAR_PLUGIN_TOOL_NAME", &spec.name)
                .env("STAR_PLUGIN_TOOL_INPUT", &input_json_str)
                .env("STAR_PLUGIN_ROOT", &spec.working_dir)
                .env("STAR_PLUGIN_PROJECT_ROOT", &spec.project_root)
                .env("STAR_PLUGIN_PERMISSION", spec.required_permission.as_str())
                .env("PAGER", "cat")
                .env("GIT_PAGER", "cat")
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("CI", "1");

            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(err) => {
                    return Ok(plugin_tool_error_result(
                        &spec,
                        "plugin_tool_spawn_failed",
                        format!("Plugin tool spawn failed: {}", err),
                        None,
                        None,
                    ));
                }
            };

            if let Some(mut stdin) = child.stdin.take() {
                if let Err(err) = stdin.write_all(&input_json).await {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    return Ok(plugin_tool_error_result(
                        &spec,
                        "plugin_tool_stdin_failed",
                        format!("Plugin tool stdin write failed: {}", err),
                        None,
                        None,
                    ));
                }
            }

            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            let stdout_task = tokio::spawn(async move {
                let mut buf = Vec::new();
                if let Some(mut stdout) = stdout {
                    let _ = stdout.read_to_end(&mut buf).await;
                }
                buf
            });
            let stderr_task = tokio::spawn(async move {
                let mut buf = Vec::new();
                if let Some(mut stderr) = stderr {
                    let _ = stderr.read_to_end(&mut buf).await;
                }
                buf
            });

            let timeout = std::time::Duration::from_secs(plugin_tool_timeout_secs());
            let cancelled = async {
                if let Some(signal) = signal.as_ref() {
                    signal.cancelled().await;
                } else {
                    std::future::pending::<()>().await;
                }
            };
            tokio::pin!(cancelled);

            let mut timed_out = false;
            let mut was_cancelled = false;
            let status = tokio::select! {
                result = child.wait() => match result {
                    Ok(status) => status,
                    Err(err) => {
                        let stdout = task_output_text(stdout_task).await;
                        let stderr = task_output_text(stderr_task).await;
                        return Ok(plugin_tool_error_result(
                            &spec,
                            "plugin_tool_wait_failed",
                            format!("Plugin tool wait failed: {}", err),
                            Some(stdout),
                            Some(stderr),
                        ));
                    }
                },
                _ = tokio::time::sleep(timeout) => {
                    timed_out = true;
                    let _ = child.kill().await;
                    match child.wait().await {
                        Ok(status) => status,
                        Err(err) => {
                            let stdout = task_output_text(stdout_task).await;
                            let stderr = task_output_text(stderr_task).await;
                            return Ok(plugin_tool_error_result(
                                &spec,
                                "plugin_tool_timeout_wait_failed",
                                format!("Plugin tool cleanup after timeout failed: {}", err),
                                Some(stdout),
                                Some(stderr),
                            ));
                        }
                    }
                },
                _ = &mut cancelled => {
                    was_cancelled = true;
                    let _ = child.kill().await;
                    match child.wait().await {
                        Ok(status) => status,
                        Err(err) => {
                            let stdout = task_output_text(stdout_task).await;
                            let stderr = task_output_text(stderr_task).await;
                            return Ok(plugin_tool_error_result(
                                &spec,
                                "plugin_tool_cancel_wait_failed",
                                format!("Plugin tool cleanup after cancel failed: {}", err),
                                Some(stdout),
                                Some(stderr),
                            ));
                        }
                    }
                }
            };

            let stdout_text = task_output_text(stdout_task).await;
            let stderr_text = task_output_text(stderr_task).await;

            if timed_out {
                return Ok(plugin_tool_error_result(
                    &spec,
                    "plugin_tool_timed_out",
                    format!(
                        "插件工具 `{}` 超时（{}s）",
                        spec.name,
                        plugin_tool_timeout_secs()
                    ),
                    Some(stdout_text),
                    Some(stderr_text),
                ));
            }

            if was_cancelled {
                return Ok(plugin_tool_error_result(
                    &spec,
                    "plugin_tool_cancelled",
                    format!("插件工具 `{}` 已取消", spec.name),
                    Some(stdout_text),
                    Some(stderr_text),
                ));
            }

            if !status.success() {
                let message = if !stderr_text.is_empty() {
                    stderr_text.clone()
                } else if !stdout_text.is_empty() {
                    stdout_text.clone()
                } else {
                    format!(
                        "Plugin tool `{}` failed with exit code {:?}",
                        spec.name,
                        status.code()
                    )
                };
                return Ok(plugin_tool_error_result(
                    &spec,
                    "plugin_tool_failed",
                    message,
                    Some(stdout_text),
                    Some(stderr_text),
                ));
            }

            let output = if !stdout_text.is_empty() {
                stdout_text.clone()
            } else if !stderr_text.is_empty() {
                stderr_text.clone()
            } else {
                format!("插件工具 `{}` 执行完成。", spec.name)
            };

            Ok(ToolResult {
                llm_content: Some(output.clone()),
                return_display: Some(output.clone()),
                output,
                error: None,
                data: Some(json!({
                    "plugin_name": spec.plugin_name,
                    "source": spec.source,
                    "tool_name": spec.name,
                    "required_permission": spec.required_permission.as_str(),
                    "stdout": stdout_text,
                    "stderr": stderr_text,
                    "exit_code": status.code(),
                })),
            })
        })
    }
}

fn plugin_tool_confirmation_title(spec: &ResolvedPluginTool) -> String {
    match spec.required_permission {
        PluginToolPermission::ReadOnly => {
            format!("允许插件只读工具 `{}` 执行", spec.name)
        }
        PluginToolPermission::WorkspaceWrite => {
            format!("允许插件工具 `{}` 写入工作区", spec.name)
        }
        PluginToolPermission::DangerFullAccess => {
            format!("允许插件工具 `{}` 完全访问本地环境", spec.name)
        }
    }
}

fn plugin_tool_confirmation_prompt(spec: &ResolvedPluginTool, params: &Value) -> String {
    let input_preview = truncate_chars(
        &serde_json::to_string_pretty(params).unwrap_or_else(|_| params.to_string()),
        3000,
    );

    format!(
        "插件 `{plugin}` 请求执行工具 `{tool}`。\n\n- source: {source}\n- required_permission: {permission}\n- working_dir: {working_dir}\n- command: {command}\n- grant_policy: {grant_policy}\n\n输入参数:\n{input}",
        plugin = spec.plugin_name,
        tool = spec.name,
        source = spec.source,
        permission = spec.required_permission.as_str(),
        working_dir = spec.working_dir.display(),
        command = render_command(&spec.command, &spec.args),
        grant_policy = plugin_tool_grant_policy(spec.required_permission),
        input = input_preview,
    )
}

fn plugin_tool_timeout_secs() -> u64 {
    std::env::var("STAR_PLUGIN_TOOL_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_PLUGIN_TOOL_TIMEOUT_SECS)
}

async fn task_output_text(task: tokio::task::JoinHandle<Vec<u8>>) -> String {
    let bytes = task.await.unwrap_or_default();
    String::from_utf8_lossy(&bytes).trim().to_string()
}

fn render_command(command: &str, args: &[String]) -> String {
    let mut parts = vec![quote_arg(command)];
    parts.extend(args.iter().map(|arg| quote_arg(arg)));
    parts.join(" ")
}

fn plugin_tool_permission_identity(spec: &ResolvedPluginTool) -> String {
    format!(
        "plugin_tool:{}:{}:{}:{}",
        spec.source,
        spec.name,
        spec.required_permission.as_str(),
        render_command(&spec.command, &spec.args)
    )
}

fn normalize_plugin_tool_outcome(
    permission: PluginToolPermission,
    outcome: crate::types::ToolConfirmationOutcome,
) -> crate::types::ToolConfirmationOutcome {
    match permission {
        PluginToolPermission::ReadOnly => outcome,
        PluginToolPermission::WorkspaceWrite => match outcome {
            crate::types::ToolConfirmationOutcome::ProceedAlwaysAndSave => {
                crate::types::ToolConfirmationOutcome::AllowSession
            }
            other => other,
        },
        PluginToolPermission::DangerFullAccess => match outcome {
            crate::types::ToolConfirmationOutcome::AllowSession
            | crate::types::ToolConfirmationOutcome::ProceedAlwaysAndSave => {
                crate::types::ToolConfirmationOutcome::ProceedAlways
            }
            other => other,
        },
    }
}

fn plugin_tool_grant_policy(permission: PluginToolPermission) -> &'static str {
    match permission {
        PluginToolPermission::ReadOnly => "支持一次、本会话、永久授权",
        PluginToolPermission::WorkspaceWrite => {
            "支持一次、本会话授权；“永久允许”会自动降级为“本会话允许”"
        }
        PluginToolPermission::DangerFullAccess => {
            "仅支持一次或“相同请求本会话允许”；“本会话允许/永久允许”都会自动降级"
        }
    }
}

fn quote_arg(value: &str) -> String {
    if value.is_empty() {
        "\"\"".to_string()
    } else if value
        .chars()
        .any(|ch| ch.is_whitespace() || ch == '"' || ch == '\'')
    {
        format!("{:?}", value)
    } else {
        value.to_string()
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        format!("{}...", value.chars().take(max_chars).collect::<String>())
    }
}

fn plugin_tool_error_result(
    spec: &ResolvedPluginTool,
    error_type: &str,
    message: String,
    stdout: Option<String>,
    stderr: Option<String>,
) -> ToolResult {
    let mut output = message.clone();
    if let Some(stdout) = stdout.as_deref().filter(|text| !text.is_empty()) {
        output.push_str("\n\nstdout:\n");
        output.push_str(stdout);
    }
    if let Some(stderr) = stderr.as_deref().filter(|text| !text.is_empty()) {
        output.push_str("\n\nstderr:\n");
        output.push_str(stderr);
    }

    ToolResult {
        llm_content: Some(output.clone()),
        return_display: Some(output.clone()),
        output,
        error: Some(ToolError {
            error_type: error_type.to_string(),
            message,
        }),
        data: Some(json!({
            "plugin_name": spec.plugin_name,
            "source": spec.source,
            "tool_name": spec.name,
            "required_permission": spec.required_permission.as_str(),
            "stdout": stdout,
            "stderr": stderr,
        })),
    }
}
