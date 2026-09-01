use crate::core::hooks::runner::{run_hooks, HookRunContext, HookRunResult};
use crate::core::hooks::store::{has_enabled_hooks_for_events, ManagedHookEvent};
use crate::core::plugins::{run_plugin_lifecycle, PluginLifecycleExecution, PluginLifecycleStage};
use crate::utils::logging::append_debug_log_line;
use std::path::Path;

#[derive(Debug, Default)]
pub struct HookExecutionSummary {
    pub assistant_notes: Vec<String>,
    pub blocking_failures: Vec<String>,
}

pub async fn run_session_start(project_root: Option<&Path>) {
    let Some(project_root) = project_root else {
        return;
    };

    log_plugin_lifecycle_results(
        run_plugin_lifecycle(project_root, PluginLifecycleStage::Init).await,
    );
    let _ = run_hooks(
        project_root,
        ManagedHookEvent::SessionStart,
        &HookRunContext {
            user_message: String::new(),
            status: "session_start".to_string(),
            tool_name: None,
            tool_arguments: None,
            tool_success: None,
            stop_reason: None,
            stop_hook_active: false,
        },
    )
    .await;
}

pub async fn run_session_end(project_root: Option<&Path>, stop_reason: &str) {
    let Some(project_root) = project_root else {
        return;
    };

    let _ = run_hooks(
        project_root,
        ManagedHookEvent::SessionEnd,
        &HookRunContext {
            user_message: String::new(),
            status: "session_end".to_string(),
            tool_name: None,
            tool_arguments: None,
            tool_success: None,
            stop_reason: Some(stop_reason.to_string()),
            stop_hook_active: false,
        },
    )
    .await;
    log_plugin_lifecycle_results(
        run_plugin_lifecycle(project_root, PluginLifecycleStage::Shutdown).await,
    );
}

pub async fn has_preflight_hooks(project_root: Option<&Path>) -> bool {
    let Some(project_root) = project_root else {
        return false;
    };

    has_enabled_hooks_for_events(
        project_root,
        &[
            ManagedHookEvent::UserPromptSubmit,
            ManagedHookEvent::BeforeAgent,
        ],
    )
    .await
    .unwrap_or(false)
}

pub async fn run_preflight_hooks(
    project_root: Option<&Path>,
    user_message: &str,
) -> HookExecutionSummary {
    let Some(project_root) = project_root else {
        return HookExecutionSummary::default();
    };

    let mut summary = HookExecutionSummary::default();
    for (event, status) in [
        (ManagedHookEvent::UserPromptSubmit, "submitted"),
        (ManagedHookEvent::BeforeAgent, "before"),
    ] {
        match run_hooks(
            project_root,
            event.clone(),
            &HookRunContext {
                user_message: user_message.to_string(),
                status: status.to_string(),
                tool_name: None,
                tool_arguments: None,
                tool_success: None,
                stop_reason: None,
                stop_hook_active: false,
            },
        )
        .await
        {
            Ok(results) => {
                for result in results {
                    if result.success {
                        continue;
                    }

                    let reason = hook_failure_reason(&result);
                    if result.blocking {
                        summary.blocking_failures.push(format!(
                            "{}:{} ({})",
                            event.as_str(),
                            result.name,
                            reason
                        ));
                        continue;
                    }

                    let note = format!(
                        "[WARN] Hook [{}:{}] failed: {}",
                        event.as_str(),
                        result.name,
                        reason
                    );
                    summary.assistant_notes.push(note.clone());
                    emit_notification_hook(&note, "hook_failure").await;
                }
            }
            Err(err) => {
                let note = format!("[WARN] Hook engine failed for {}: {}", event.as_str(), err);
                summary.assistant_notes.push(note.clone());
                emit_notification_hook(&note, "hook_engine_failure").await;
            }
        }
    }

    summary
}

pub async fn run_stop_hooks(project_root: Option<&Path>, user_message: &str, stop_reason: &str) {
    let Some(project_root) = project_root else {
        return;
    };

    let _ = run_hooks(
        project_root,
        ManagedHookEvent::Stop,
        &HookRunContext {
            user_message: user_message.to_string(),
            status: "stopped".to_string(),
            tool_name: None,
            tool_arguments: None,
            tool_success: None,
            stop_reason: Some(stop_reason.to_string()),
            stop_hook_active: false,
        },
    )
    .await;
}

pub async fn run_after_agent_hooks(project_root: Option<&Path>, user_message: &str) -> Vec<String> {
    let Some(project_root) = project_root else {
        return Vec::new();
    };

    let mut assistant_notes = Vec::new();
    if let Ok(results) = run_hooks(
        project_root,
        ManagedHookEvent::AfterAgent,
        &HookRunContext {
            user_message: user_message.to_string(),
            status: "after".to_string(),
            tool_name: None,
            tool_arguments: None,
            tool_success: None,
            stop_reason: None,
            stop_hook_active: false,
        },
    )
    .await
    {
        for result in results {
            if result.success {
                continue;
            }

            let note = format!(
                "[WARN] Hook [{}] failed after agent{}: {}",
                result.name,
                if result.blocking {
                    " (blocking=true)"
                } else {
                    ""
                },
                hook_failure_reason(&result)
            );
            assistant_notes.push(note.clone());
            emit_notification_hook(&note, "after_agent_hook_failure").await;
        }
    }

    assistant_notes
}

pub async fn run_pre_compact_hooks(project_root: Option<&Path>) -> HookExecutionSummary {
    let Some(project_root) = project_root else {
        return HookExecutionSummary::default();
    };

    let mut summary = HookExecutionSummary::default();
    match run_hooks(
        project_root,
        ManagedHookEvent::PreCompact,
        &HookRunContext {
            user_message: "manual_compress".to_string(),
            status: "before_compress".to_string(),
            tool_name: None,
            tool_arguments: None,
            tool_success: None,
            stop_reason: None,
            stop_hook_active: false,
        },
    )
    .await
    {
        Ok(results) => {
            for result in results {
                if result.success {
                    continue;
                }

                let reason = hook_failure_reason(&result);
                if result.blocking {
                    summary
                        .blocking_failures
                        .push(format!("{} ({})", result.name, reason));
                    continue;
                }

                let note = format!(
                    "[WARN] Hook [PreCompact:{}] failed: {}",
                    result.name, reason
                );
                summary.assistant_notes.push(note.clone());
                emit_notification_hook(&note, "pre_compact_hook_failure").await;
            }
        }
        Err(err) => {
            let note = format!("[WARN] Hook engine failed for PreCompact: {}", err);
            summary.assistant_notes.push(note.clone());
            emit_notification_hook(&note, "pre_compact_hook_engine_failure").await;
        }
    }

    summary
}

pub async fn emit_notification_hook(message: &str, status: &str) {
    let cwd = crate::core::utils::paths::current_dir_cached();
    let _ = run_hooks(
        cwd,
        ManagedHookEvent::Notification,
        &HookRunContext {
            user_message: message.to_string(),
            status: status.to_string(),
            tool_name: None,
            tool_arguments: None,
            tool_success: None,
            stop_reason: None,
            stop_hook_active: false,
        },
    )
    .await;
}

fn hook_failure_reason(result: &HookRunResult) -> String {
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

fn log_plugin_lifecycle_results(result: Result<Vec<PluginLifecycleExecution>, String>) {
    match result {
        Ok(results) => {
            for entry in results.into_iter().filter(|entry| !entry.success) {
                append_debug_log_line(&format!(
                    "[PluginLifecycle] {}:{} failed (plugin={}, source={}): {}",
                    entry.stage.as_str(),
                    entry.name,
                    entry.plugin_name,
                    entry.source,
                    entry
                        .error
                        .as_deref()
                        .or(if entry.stderr.is_empty() {
                            None
                        } else {
                            Some(entry.stderr.as_str())
                        })
                        .unwrap_or("unknown error")
                ));
            }
        }
        Err(error) => {
            append_debug_log_line(&format!(
                "[PluginLifecycle] failed to enumerate lifecycle commands: {}",
                error
            ));
        }
    }
}
