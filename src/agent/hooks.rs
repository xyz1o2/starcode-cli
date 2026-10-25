use crate::types::{StarToolCall, ToolResult};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::OnceLock;

fn hook_result_reason(result: &crate::core::hooks::runner::HookRunResult) -> String {
    if let Some(err) = &result.error {
        if !err.trim().is_empty() {
            return err.clone();
        }
    }
    if !result.stderr.trim().is_empty() {
        return result.stderr.clone();
    }
    format!("exit {:?}", result.exit_code)
}

/// 项目根目录（进程级缓存，可被 eval/worktree 场景覆盖）。
///
/// 注意：这是一个进程级全局，默认取启动时的 cwd。多项目/eval 场景下
/// 应先调用 [`override_project_root`] 切换，避免缓存指向错误目录。
pub(crate) fn cached_project_root() -> Option<PathBuf> {
    let cached = project_root_override()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    cached.or_else(|| std::env::current_dir().ok())
}

fn project_root_override() -> &'static std::sync::Mutex<Option<PathBuf>> {
    static ROOT: OnceLock<std::sync::Mutex<Option<PathBuf>>> = OnceLock::new();
    ROOT.get_or_init(|| std::sync::Mutex::new(None))
}

/// 覆盖进程级项目根目录缓存（供 eval 夹具副本等场景使用）。
pub(crate) fn override_project_root(path: PathBuf) {
    *project_root_override()
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = Some(path);
}

/// Drop an explicit project-root override after the session moves to a new
/// working directory, so hooks resolve against the new process cwd again.
pub(crate) fn clear_project_root_override() {
    *project_root_override()
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = None;
}

async fn hooks_maybe_enabled_for_event(
    cwd: &std::path::Path,
    event: crate::core::hooks::store::ManagedHookEvent,
) -> bool {
    match crate::core::hooks::store::has_enabled_hooks_for_events(cwd, &[event]).await {
        Ok(enabled) => enabled,
        Err(_) => true,
    }
}

pub(crate) async fn run_stage_hooks(
    user_input: &str,
    event: crate::core::hooks::store::ManagedHookEvent,
    status: &str,
    details: Option<Value>,
) -> Result<(), String> {
    run_stage_hooks_with_call_id(user_input, event, status, details, None).await
}

/// 同上，但能带上确认请求的 id —— 外部 harness 靠它应答 `approve_tool`。
pub(crate) async fn run_stage_hooks_with_call_id(
    user_input: &str,
    event: crate::core::hooks::store::ManagedHookEvent,
    status: &str,
    details: Option<Value>,
    tool_call_id: Option<String>,
) -> Result<(), String> {
    let Some(cwd_owned) = cached_project_root() else {
        return Ok(());
    };
    let cwd = &cwd_owned;

    if !hooks_maybe_enabled_for_event(cwd, event.clone()).await {
        return Ok(());
    }

    let event_name = event.as_str().to_string();
    let tool_arguments = details.map(|v| v.to_string());

    match crate::core::hooks::runner::run_hooks(
        cwd,
        event,
        &crate::core::hooks::runner::HookRunContext {
            user_message: user_input.to_string(),
            status: status.to_string(),
            tool_name: None,
            tool_arguments,
            tool_success: None,
            stop_reason: None,
            stop_hook_active: false,
            tool_call_id,
        },
    )
    .await
    {
        Ok(results) => {
            let mut blocking_errors = Vec::new();
            for result in results {
                if result.success {
                    continue;
                }
                let reason = hook_result_reason(&result);
                if result.blocking {
                    blocking_errors.push(format!("{} ({})", result.name, reason));
                } else {
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[WARN] {} hook [{}] failed: {}",
                        event_name, result.name, reason
                    ));
                }
            }
            if blocking_errors.is_empty() {
                Ok(())
            } else {
                Err(format!(
                    "{} blocked by {} hook: {}",
                    status,
                    event_name,
                    blocking_errors.join(", ")
                ))
            }
        }
        Err(err) => {
            crate::utils::logging::append_debug_log_line(&format!(
                "[WARN] {} hook engine error: {}",
                event_name, err
            ));
            Ok(())
        }
    }
}

/// 工具确认请求**发出**时触发 `PermissionRequest`。
///
/// 外部 harness 靠这个事件知道「有一个工具在等人批准」—— 拿到 tool_call_id 之后
/// 才能用 remote inbox 的 `approve_tool` 动作应答。
pub(crate) async fn run_permission_request_hook(tool_call: &StarToolCall, source: &str) {
    let details = Some(serde_json::json!({
        "tool_name": tool_call.function.name,
        "tool_arguments": tool_call.function.arguments,
        "source": source,
    }));
    // 通知型事件：不阻塞 —— 它只是告诉外部"来人了"。
    let _ = run_stage_hooks_with_call_id(
        "",
        crate::core::hooks::store::ManagedHookEvent::PermissionRequest,
        "permission_request",
        details,
        Some(tool_call.id.clone()),
    )
    .await;
}

/// 工具确认被**拒绝**时触发 `PermissionDenied`。
pub(crate) async fn run_permission_denied_hook(tool_call: &StarToolCall, reason: &str) {
    let details = Some(serde_json::json!({
        "tool_name": tool_call.function.name,
        "tool_arguments": tool_call.function.arguments,
        "reason": reason,
    }));
    let _ = run_stage_hooks(
        "",
        crate::core::hooks::store::ManagedHookEvent::PermissionDenied,
        "permission_denied",
        details,
    )
    .await;
}

/// 工具执行**失败**时触发 `PostToolUseFailure`。
///
/// 之前有个 `run_post_tool_use_failure_hooks` 是空壳（循环体只填 `success: true`，
/// 从不执行命令），而且没有调用方。这里给它一个真的实现和真的调用点。
pub(crate) async fn run_post_tool_failure_hooks(tool_call: &StarToolCall, error: &str) {
    let details = Some(serde_json::json!({
        "tool_name": tool_call.function.name,
        "tool_arguments": tool_call.function.arguments,
        "error": error,
    }));
    let _ = run_stage_hooks(
        "",
        crate::core::hooks::store::ManagedHookEvent::PostToolUseFailure,
        "post_tool_use_failure",
        details,
    )
    .await;
}

/// 子代理**开始**时触发 `SubagentStart`。
pub(crate) async fn run_subagent_start_hook(
    agent_id: &str,
    description: &str,
    subagent_type: &str,
) {
    let details = Some(serde_json::json!({
        "agent_id": agent_id,
        "description": description,
        "subagent_type": subagent_type,
    }));
    let _ = run_stage_hooks(
        "",
        crate::core::hooks::store::ManagedHookEvent::SubagentStart,
        "subagent_start",
        details,
    )
    .await;
}

/// 上下文压缩**完成**后触发 `PostCompact`。
pub(crate) async fn run_post_compact_hook(before_tokens: usize, after_tokens: usize) {
    let details = Some(serde_json::json!({
        "before_tokens": before_tokens,
        "after_tokens": after_tokens,
    }));
    let _ = run_stage_hooks(
        "",
        crate::core::hooks::store::ManagedHookEvent::PostCompact,
        "post_compact",
        details,
    )
    .await;
}

pub(crate) async fn run_pre_tool_hooks(
    user_input: &str,
    tool_call: &StarToolCall,
) -> Result<(), String> {
    let Some(cwd_owned) = cached_project_root() else {
        return Ok(());
    };
    let cwd = &cwd_owned;

    if !hooks_maybe_enabled_for_event(cwd, crate::core::hooks::store::ManagedHookEvent::PreToolUse)
        .await
    {
        return Ok(());
    }

    match crate::core::hooks::runner::run_hooks(
        cwd,
        crate::core::hooks::store::ManagedHookEvent::PreToolUse,
        &crate::core::hooks::runner::HookRunContext {
            user_message: user_input.to_string(),
            status: "before_tool".to_string(),
            tool_name: Some(tool_call.function.name.clone()),
            tool_arguments: Some(tool_call.function.arguments.clone()),
            tool_success: None,
            stop_reason: None,
            stop_hook_active: false,
            tool_call_id: Some(tool_call.id.clone()),
        },
    )
    .await
    {
        Ok(results) => {
            let mut blocking_errors = Vec::new();
            for result in results {
                if result.success {
                    continue;
                }
                let reason = hook_result_reason(&result);
                if result.blocking {
                    blocking_errors.push(format!("{} ({})", result.name, reason));
                } else {
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[WARN] PreToolUse hook [{}] failed: {}",
                        result.name, reason
                    ));
                }
            }
            if blocking_errors.is_empty() {
                Ok(())
            } else {
                Err(format!(
                    "tool `{}` blocked by PreToolUse hook: {}",
                    tool_call.function.name,
                    blocking_errors.join(", ")
                ))
            }
        }
        Err(err) => {
            crate::utils::logging::append_debug_log_line(&format!(
                "[WARN] PreToolUse hook engine error for `{}`: {}",
                tool_call.function.name, err
            ));
            Ok(())
        }
    }
}

pub(crate) async fn run_post_tool_hooks(
    user_input: &str,
    tool_call: &StarToolCall,
    result: &ToolResult,
) {
    let Some(cwd_owned) = cached_project_root() else {
        return;
    };
    let cwd = &cwd_owned;

    if !hooks_maybe_enabled_for_event(
        cwd,
        crate::core::hooks::store::ManagedHookEvent::PostToolUse,
    )
    .await
    {
        return;
    }

    match crate::core::hooks::runner::run_hooks(
        cwd,
        crate::core::hooks::store::ManagedHookEvent::PostToolUse,
        &crate::core::hooks::runner::HookRunContext {
            user_message: user_input.to_string(),
            status: "after_tool".to_string(),
            tool_name: Some(tool_call.function.name.clone()),
            tool_arguments: Some(tool_call.function.arguments.clone()),
            tool_success: Some(result.success),
            stop_reason: result.error.clone(),
            stop_hook_active: false,
            tool_call_id: None,
        },
    )
    .await
    {
        Ok(results) => {
            for hook in results {
                if !hook.success {
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[WARN] PostToolUse hook [{}] failed: {}",
                        hook.name,
                        hook_result_reason(&hook)
                    ));
                }
            }
        }
        Err(err) => {
            crate::utils::logging::append_debug_log_line(&format!(
                "[WARN] PostToolUse hook engine error for `{}`: {}",
                tool_call.function.name, err
            ));
        }
    }
}
