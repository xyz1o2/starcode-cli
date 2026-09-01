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
    static ROOT: OnceLock<std::sync::Mutex<Option<PathBuf>>> = OnceLock::new();
    let lock = ROOT.get_or_init(|| std::sync::Mutex::new(None));
    let cached = lock.lock().unwrap_or_else(|p| p.into_inner()).clone();
    cached.or_else(|| std::env::current_dir().ok())
}

/// 覆盖进程级项目根目录缓存（供 eval 夹具副本等场景使用）。
pub(crate) fn override_project_root(path: PathBuf) {
    static ROOT: OnceLock<std::sync::Mutex<Option<PathBuf>>> = OnceLock::new();
    let lock = ROOT.get_or_init(|| std::sync::Mutex::new(None));
    *lock.lock().unwrap_or_else(|p| p.into_inner()) = Some(path);
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
