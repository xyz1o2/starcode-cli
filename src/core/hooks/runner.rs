use super::store::{list_hooks, matcher_matches, ManagedHookEvent};
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
    /// 确认请求的 id —— 外部 harness 用它应答 remote inbox 的 `approve_tool`。
    ///
    /// 只有 `PermissionRequest` / `PermissionDenied` 这类事件会带；其它事件是 `None`。
    pub tool_call_id: Option<String>,
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
        // matcher 是 Claude Code 配置带来的过滤条件。`/hooks add` 建的 hook
        // matcher 为 None，即不过滤 —— 保持既有行为不变。
        .filter(|h| matcher_matches(h.matcher.as_deref(), context.tool_name.as_deref()))
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
    // 确认请求的 id —— 外部 harness 拿到它才能用 remote inbox 的
    // `approve_tool` 应答（`message` 字段填的就是这个 id）。
    //
    // 它只在 `PermissionRequest` / `PermissionDenied` 这类事件上有值；
    // 其它事件的 `tool_call_id` 是 None，变量不会被设置。
    if let Some(tool_call_id) = context.tool_call_id.as_ref() {
        cmd.env("STAR_HOOK_TOOL_CALL_ID", tool_call_id);
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
    cmd.env(
        "STAR_HOOK_STOP_HOOK_ACTIVE",
        if context.stop_hook_active {
            "true"
        } else {
            "false"
        },
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::hooks::store::{ManagedHook, ManagedHookEvent};

    /// 外部 harness 要用 remote inbox 的 `approve_tool` 应答工具确认，
    /// 靠的就是这个环境变量 —— 它必须真的传出去。
    #[tokio::test]
    async fn the_tool_call_id_reaches_the_hook_script() {
        let dir = std::env::temp_dir().join(format!("star-hook-cid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let out = dir.join("captured.txt");

        // 脚本把关键环境变量写进文件。
        let script = format!(
            "printf '%s\\n%s\\n' \"$STAR_HOOK_EVENT\" \"$STAR_HOOK_TOOL_CALL_ID\" > {}",
            out.display()
        );
        let now = chrono::Utc::now().timestamp();
        let hook = ManagedHook {
            name: "capture".to_string(),
            event: ManagedHookEvent::PermissionRequest,
            command: script,
            timeout_secs: 10,
            enabled: true,
            blocking: false,
            created_at: now,
            updated_at: now,
            source: None,
            working_dir: None,
            matcher: None,
        };

        let store = crate::core::hooks::store::HookStore { hooks: vec![hook] };
        crate::core::hooks::store::save_store(&dir, &store)
            .await
            .expect("store saved");

        let results = run_hooks(
            &dir,
            ManagedHookEvent::PermissionRequest,
            &HookRunContext {
                user_message: String::new(),
                status: "permission_request".to_string(),
                tool_name: Some("Write".to_string()),
                tool_arguments: Some("{}".to_string()),
                tool_success: None,
                stop_reason: None,
                stop_hook_active: false,
                tool_call_id: Some("call-abc123".to_string()),
            },
        )
        .await
        .expect("hooks ran");

        assert!(results.iter().any(|r| r.success), "{results:?}");
        let captured = std::fs::read_to_string(&out).expect("captured file");
        let mut lines = captured.lines();
        assert_eq!(lines.next(), Some("PermissionRequest"));
        assert_eq!(
            lines.next(),
            Some("call-abc123"),
            "tool_call_id 必须传到脚本：{captured}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
