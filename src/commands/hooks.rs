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

    match cmd {
        HooksCommand::List => {
            let hooks = crate::core::hooks::store::list_hooks(&cwd)
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
                crate::core::hooks::store::add_hook(&cwd, name, event, command, timeout, blocking)
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
            let removed = crate::core::hooks::store::remove_hook(&cwd, &name)
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
                &cwd,
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
