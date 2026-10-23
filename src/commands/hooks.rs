use crate::commands::execution::{CommandContext, CommandResult};
use clap::Subcommand;

#[derive(Subcommand)]
pub enum HooksCommand {
    /// List managed hooks
    List,
    /// Add a managed hook
    #[command(arg_required_else_help = true)]
    Add {
        /// Unique hook name
        name: String,
        /// Event: SessionStart | SessionEnd | BeforeAgent | AfterAgent | Notification | UserPromptSubmit | Stop | PreCompact | BeforeToolSelection | BeforeModel | AfterModel | PreToolUse | PostToolUse | SubagentStop
        event: String,
        /// Command to run
        #[arg(required = true)]
        command: Vec<String>,
        /// Timeout in seconds
        #[arg(long, default_value_t = 20)]
        timeout: u64,
        /// Block agent execution when this hook fails (recommended for BeforeAgent checks)
        #[arg(long, default_value_t = false)]
        blocking: bool,
    },
    /// Remove hook by name
    #[command(arg_required_else_help = true)]
    Remove {
        /// Hook name
        name: String,
    },
    /// Run hooks for a specific event (manual test)
    #[command(arg_required_else_help = true)]
    Run {
        /// Event: SessionStart | SessionEnd | BeforeAgent | AfterAgent | Notification | UserPromptSubmit | Stop | PreCompact | BeforeToolSelection | BeforeModel | AfterModel | PreToolUse | PostToolUse | SubagentStop
        event: String,
        /// Optional message context
        #[arg(long, default_value = "")]
        message: String,
    },
}

pub async fn execute_hooks_command(ctx: CommandContext<'_>, cmd: HooksCommand) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    execute_hooks_command_in(ctx, cmd, &cwd).await
}

/// 真正的实现。`cwd` 走参数而不是 `current_dir()`，单测才能在临时目录里
/// 增删 hook —— 否则测一次就污染了开发者自己的 `.star/hooks.json`。
pub(crate) async fn execute_hooks_command_in(
    ctx: CommandContext<'_>,
    cmd: HooksCommand,
    cwd: &std::path::Path,
) -> CommandResult {
    match cmd {
        HooksCommand::List => {
            let hooks = crate::core::hooks::store::list_hooks(cwd)
                .await
                .map_err(|e| format!("failed to list hooks: {}", e))?;

            if hooks.is_empty() {
                ctx.state.chat_history.push(
                    crate::types::ChatEntry::assistant("No hooks configured.".to_string())
                        .with_streaming(false),
                );
                return Ok(());
            }

            let mut lines = vec!["# Hooks\n".to_string()];
            for h in hooks {
                lines.push(format!(
                    "- `{}` [{}]{}\n  - timeout: {}s\n  - blocking: {}\n  - source: {}\n  - command: {}",
                    h.name,
                    h.event.as_str(),
                    if h.enabled { "" } else { " (disabled)" },
                    h.timeout_secs,
                    if h.blocking { "yes" } else { "no" },
                    h.source.as_deref().unwrap_or("managed"),
                    h.command
                ));
                if let Some(working_dir) = h.working_dir.as_ref() {
                    lines.push(format!("  - working_dir: {}", working_dir.display()));
                }
            }
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(lines.join("\n")).with_streaming(false));
            Ok(())
        }
        HooksCommand::Add {
            name,
            event,
            command,
            timeout,
            blocking,
        } => {
            let event = crate::core::hooks::store::ManagedHookEvent::parse(&event)
                .ok_or("invalid event, expected SessionStart|SessionEnd|BeforeAgent|AfterAgent|Notification|UserPromptSubmit|Stop|PreCompact|BeforeToolSelection|BeforeModel|AfterModel|PreToolUse|PostToolUse|SubagentStop")?;
            let command = command.join(" ").trim().to_string();
            let hook =
                crate::core::hooks::store::add_hook(cwd, name, event, command, timeout, blocking)
                    .await
                    .map_err(|e| format!("failed to add hook: {}", e))?;

            let msg = format!(
                "Hook added\n\n- name: `{}`\n- event: {}\n- timeout: {}s\n- blocking: {}\n- command: {}",
                hook.name,
                hook.event.as_str(),
                hook.timeout_secs,
                if hook.blocking { "yes" } else { "no" },
                hook.command
            );
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
            Ok(())
        }
        HooksCommand::Remove { name } => {
            let removed = crate::core::hooks::store::remove_hook(cwd, &name)
                .await
                .map_err(|e| format!("failed to remove hook: {}", e))?;

            let msg = if removed {
                format!("Hook `{}` removed", name)
            } else {
                format!("Hook `{}` not found", name)
            };
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
            Ok(())
        }
        HooksCommand::Run { event, message } => {
            let event = crate::core::hooks::store::ManagedHookEvent::parse(&event)
                .ok_or("invalid event, expected SessionStart|SessionEnd|BeforeAgent|AfterAgent|Notification|UserPromptSubmit|Stop|PreCompact|BeforeToolSelection|BeforeModel|AfterModel|PreToolUse|PostToolUse|SubagentStop")?;

            let results = crate::core::hooks::runner::run_hooks(
                cwd,
                event.clone(),
                &crate::core::hooks::runner::HookRunContext {
                    user_message: message,
                    status: "manual".to_string(),
                    tool_name: None,
                    tool_arguments: None,
                    tool_success: None,
                    stop_reason: None,
                    stop_hook_active: false,
                },
            )
            .await
            .map_err(|e| format!("failed to run hooks: {}", e))?;

            if results.is_empty() {
                ctx.state.chat_history.push(
                    crate::types::ChatEntry::assistant(format!(
                        "No enabled hooks found for event {}.",
                        event.as_str()
                    ))
                    .with_streaming(false),
                );
                return Ok(());
            }

            let mut lines = vec![format!("# Hook Run Results ({})\n", event.as_str())];
            for r in results {
                let status = if r.success { "ok" } else { "failed" };
                lines.push(format!(
                    "- `{}`: {} (exit={:?}, blocking={})",
                    r.name,
                    status,
                    r.exit_code,
                    if r.blocking { "yes" } else { "no" }
                ));
                if !r.stdout.is_empty() {
                    lines.push(format!("  - stdout: {}", trim_line(&r.stdout)));
                }
                if !r.stderr.is_empty() {
                    lines.push(format!("  - stderr: {}", trim_line(&r.stderr)));
                }
                if let Some(err) = r.error {
                    lines.push(format!("  - error: {}", trim_line(&err)));
                }
            }

            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(lines.join("\n")).with_streaming(false));
            Ok(())
        }
    }
}

fn trim_line(s: &str) -> String {
    let one_line = s.replace('\n', " ").replace('\r', " ");
    if one_line.chars().count() <= 180 {
        one_line
    } else {
        format!("{}...", one_line.chars().take(180).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::execution::CommandContext;
    use crate::ui::state::ChatState;

    /// 每次调用都新建 state + channel：命令只往 `chat_history` 末尾追加一条，
    /// 断言最后一条就是这条命令的输出。
    async fn run_in(cwd: &std::path::Path, cmd: HooksCommand) -> Result<String, String> {
        let mut state = ChatState::test_fixture();
        let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(1);
        execute_hooks_command_in(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            cmd,
            cwd,
        )
        .await?;
        Ok(state
            .chat_history
            .last()
            .map(|entry| entry.content.clone())
            .unwrap_or_default())
    }

    fn add(name: &str, event: &str, command: &str) -> HooksCommand {
        HooksCommand::Add {
            name: name.to_string(),
            event: event.to_string(),
            command: command.split(' ').map(str::to_string).collect(),
            timeout: 20,
            blocking: false,
        }
    }

    #[tokio::test]
    async fn list_without_configuration_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let out = run_in(dir.path(), HooksCommand::List).await.unwrap();
        assert_eq!(out, "No hooks configured.");
    }

    #[tokio::test]
    async fn add_persists_and_list_reports_every_field() {
        let dir = tempfile::tempdir().unwrap();

        let added = run_in(dir.path(), add("guard", "PreToolUse", "echo guard"))
            .await
            .unwrap();
        assert!(added.contains("- name: `guard`"), "{added}");
        assert!(added.contains("- event: PreToolUse"), "{added}");
        assert!(added.contains("- timeout: 20s"), "{added}");
        assert!(added.contains("- blocking: no"), "{added}");
        assert!(added.contains("- command: echo guard"), "{added}");

        let listed = run_in(dir.path(), HooksCommand::List).await.unwrap();
        assert!(listed.starts_with("# Hooks"), "{listed}");
        assert!(listed.contains("- `guard` [PreToolUse]"), "{listed}");
        assert!(listed.contains("source: managed"), "{listed}");
        assert!(listed.contains("command: echo guard"), "{listed}");
    }

    #[tokio::test]
    async fn remove_reports_whether_the_hook_existed() {
        let dir = tempfile::tempdir().unwrap();
        run_in(dir.path(), add("guard", "PreToolUse", "echo guard"))
            .await
            .unwrap();

        let removed = run_in(
            dir.path(),
            HooksCommand::Remove {
                name: "guard".to_string(),
            },
        )
        .await
        .unwrap();
        assert_eq!(removed, "Hook `guard` removed");

        let again = run_in(
            dir.path(),
            HooksCommand::Remove {
                name: "guard".to_string(),
            },
        )
        .await
        .unwrap();
        assert_eq!(again, "Hook `guard` not found");
    }

    /// 事件名的大小写/下划线漂移由 `ManagedHookEvent::parse` 吸收；
    /// 真正非法的名字必须在落盘前被拒绝。
    #[tokio::test]
    async fn event_names_are_normalized_and_invalid_ones_are_rejected() {
        let dir = tempfile::tempdir().unwrap();

        let added = run_in(dir.path(), add("s", "session_start", "true"))
            .await
            .unwrap();
        assert!(added.contains("- event: SessionStart"), "{added}");

        let err = run_in(dir.path(), add("bad", "on-everything", "true"))
            .await
            .unwrap_err();
        assert!(err.contains("invalid event"), "{err}");

        let listed = run_in(dir.path(), HooksCommand::List).await.unwrap();
        assert!(
            !listed.contains("bad"),
            "rejected hook must not persist:\n{listed}"
        );
    }

    #[tokio::test]
    async fn adding_a_hook_without_a_command_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = run_in(
            dir.path(),
            HooksCommand::Add {
                name: "empty".to_string(),
                event: "Stop".to_string(),
                command: vec!["   ".to_string()],
                timeout: 5,
                blocking: false,
            },
        )
        .await
        .unwrap_err();
        assert!(err.contains("failed to add hook"), "{err}");
    }

    #[tokio::test]
    async fn run_without_matching_hooks_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let out = run_in(
            dir.path(),
            HooksCommand::Run {
                event: "Stop".to_string(),
                message: String::new(),
            },
        )
        .await
        .unwrap();
        assert_eq!(out, "No enabled hooks found for event Stop.");
    }

    #[tokio::test]
    async fn run_executes_matching_hooks_and_reports_their_output() {
        let dir = tempfile::tempdir().unwrap();
        run_in(dir.path(), add("say-hi", "SessionStart", "echo hi"))
            .await
            .unwrap();

        let out = run_in(
            dir.path(),
            HooksCommand::Run {
                event: "SessionStart".to_string(),
                message: String::new(),
            },
        )
        .await
        .unwrap();
        assert!(out.contains("# Hook Run Results (SessionStart)"), "{out}");
        assert!(out.contains("`say-hi`: ok"), "{out}");
        assert!(out.contains("stdout: hi"), "{out}");
    }

    #[tokio::test]
    async fn run_reports_failing_hooks_instead_of_hiding_them() {
        let dir = tempfile::tempdir().unwrap();
        run_in(
            dir.path(),
            add("boom", "Stop", "sh -c 'echo bad >&2; exit 3'"),
        )
        .await
        .unwrap();

        let out = run_in(
            dir.path(),
            HooksCommand::Run {
                event: "Stop".to_string(),
                message: String::new(),
            },
        )
        .await
        .unwrap();
        assert!(out.contains("`boom`: failed"), "{out}");
        assert!(out.contains("exit=Some(3)"), "{out}");
        assert!(out.contains("stderr: bad"), "{out}");
    }

    #[test]
    fn trim_line_flattens_newlines_and_caps_length() {
        assert_eq!(trim_line("a\nb\r\nc"), "a b  c");
        let long = "x".repeat(300);
        let trimmed = trim_line(&long);
        assert_eq!(trimmed.chars().count(), 183, "180 chars + ellipsis");
        assert!(trimmed.ends_with("..."));
    }
}
