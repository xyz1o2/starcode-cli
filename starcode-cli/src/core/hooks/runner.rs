use super::store::{list_hooks, ManagedHookEvent};
use std::process::Stdio;

#[derive(Debug, Clone)]
pub struct HookRunContext {
    pub user_message: String,
    pub status: String,
    pub tool_name: Option<String>,
    pub tool_arguments: Option<String>,
    pub tool_success: Option<bool>,
    pub stop_reason: Option<String>,
    pub stop_hook_active: bool,
}

#[derive(Debug, Clone)]
pub struct HookRunResult {
    pub name: String,
    pub event: ManagedHookEvent,
    pub blocking: bool,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub error: Option<String>,
}

pub async fn run_hooks(
    project_root: &std::path::Path,
    event: ManagedHookEvent,
    context: &HookRunContext,
) -> Result<Vec<HookRunResult>, String> {
    let hooks = list_hooks(project_root).await?;
    let selected: Vec<_> = hooks
        .into_iter()
        .filter(|h| h.enabled && h.event == event)
        .collect();

    let mut out = Vec::new();
    for hook in selected {
        let result = run_one_hook(
            &hook.name,
            &hook.command,
            hook.timeout_secs,
            hook.blocking,
            hook.working_dir.as_deref(),
            hook.source.as_deref(),
            &event,
            context,
        )
        .await;
        out.push(result);
    }

    Ok(out)
}

async fn run_one_hook(
    name: &str,
    command: &str,
    timeout_secs: u64,
    blocking: bool,
    working_dir: Option<&std::path::Path>,
    source: Option<&str>,
    event: &ManagedHookEvent,
    context: &HookRunContext,
) -> HookRunResult {
    let mut cmd = if cfg!(target_os = "windows") {
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/C").arg(command);
        c
    } else {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-lc").arg(command);
        c
    };

    cmd.env("STAR_HOOK_EVENT", event.as_str());
    cmd.env("STAR_HOOK_STATUS", &context.status);
    cmd.env("STAR_HOOK_MESSAGE", &context.user_message);
    if let Some(tool_name) = context.tool_name.as_ref() {
        cmd.env("STAR_HOOK_TOOL_NAME", tool_name);
    }
    if let Some(tool_arguments) = context.tool_arguments.as_ref() {
        cmd.env("STAR_HOOK_TOOL_ARGUMENTS", tool_arguments);
    }
    if let Some(tool_success) = context.tool_success {
        cmd.env(
            "STAR_HOOK_TOOL_SUCCESS",
            if tool_success { "true" } else { "false" },
        );
    }
    if let Some(stop_reason) = context.stop_reason.as_ref() {
        cmd.env("STAR_HOOK_STOP_REASON", stop_reason);
    }
    cmd.env("STAR_HOOK_STOP_HOOK_ACTIVE", if context.stop_hook_active { "true" } else { "false" });
    if let Some(working_dir) = working_dir {
        cmd.current_dir(working_dir);
        cmd.env("STAR_HOOK_WORKING_DIR", working_dir);
    }
    if let Some(source) = source {
        cmd.env("STAR_HOOK_SOURCE", source);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs.max(1)),
        cmd.output(),
    )
    .await;

    match output {
        Ok(Ok(out)) => {
            let code = out.status.code();
            HookRunResult {
                name: name.to_string(),
                event: event.clone(),
                blocking,
                success: out.status.success(),
                exit_code: code,
                stdout: String::from_utf8_lossy(&out.stdout).trim().to_string(),
                stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
                error: None,
            }
        }
        Ok(Err(err)) => HookRunResult {
            name: name.to_string(),
            event: event.clone(),
            blocking,
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            error: Some(format!("failed to execute hook command: {}", err)),
        },
        Err(_) => HookRunResult {
            name: name.to_string(),
            event: event.clone(),
            blocking,
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            error: Some(format!("hook timed out after {}s", timeout_secs.max(1))),
        },
    }
}
