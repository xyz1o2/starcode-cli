mod agent_team_presets;
mod agent_team_render;
mod agent_team_support;
pub mod agents;
pub mod chat;
pub mod compat;
pub mod config;
pub mod connect;
pub mod core;
pub mod doctor;
pub mod eval;
pub mod execution;
pub mod extended;
pub mod extension;
pub mod features;
pub mod git;
pub mod hooks;
pub mod init;

pub mod loop_cmd;
pub mod mcp;
pub mod memory;
pub mod model;
pub mod permissions;
pub mod plan;
pub mod plugin;
pub mod provider;
pub mod skills;
pub mod remote;
pub mod system;
pub mod test;
pub mod tools;
pub mod utility;

use clap::{FromArgMatches, Subcommand};
use execution::{CommandContext, CommandResult};

macro_rules! simple_command_wrapper {
    ($fn_name:ident, $cmd_name:literal, $cmd_ty:path, $executor:path) => {
        async fn $fn_name(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
            let cmd = clap::Command::new($cmd_name);
            let cmd = <$cmd_ty>::augment_subcommands(cmd);

            let cli_args = std::iter::once($cmd_name.to_string()).chain(args.into_iter());

            match cmd.try_get_matches_from(cli_args) {
                Ok(matches) => {
                    let parsed =
                        <$cmd_ty>::from_arg_matches(&matches).map_err(|e| e.to_string())?;
                    $executor(ctx, parsed).await.map_err(|e| e.to_string())?;
                    Ok(())
                }
                Err(e) => {
                    ctx.state.chat_history.push(crate::types::ChatEntry {
                        is_streaming: Some(false),
                        ..crate::types::ChatEntry::assistant(e.render().to_string())
                    });
                    Ok(())
                }
            }
        }
    };
}

pub async fn handle_command(
    name: &str,
    args: Vec<String>,
    ctx: CommandContext<'_>,
) -> CommandResult {
    match name {
        "help" => core::help(ctx, args).await,
        "clear" => core::clear(ctx, args).await,
        "exit" => core::exit(ctx, args).await,
        "about" => core::about(ctx, args).await,
        "status" => core::status(ctx, args).await,
        "chat" => chat::run(ctx, args).await,
        "share" => {
            let mut a = vec!["share".to_string()];
            a.extend(args);
            chat::run(ctx, a).await
        }
        "resume" => chat::resume_cmd(ctx, args).await,
        "restore" => utility::undo(ctx, args).await,
        // /mcp 无参数默认 status，避免 clap usage 错误倾倒到聊天
        "mcp" => {
            let effective_args = if args.is_empty() {
                vec!["status".to_string()]
            } else {
                args
            };
            mcp_wrapper(ctx, effective_args).await
        }
        "memory" => memory_wrapper(ctx, args).await,
        // /provider 无参数或 select 缺 id：打开提供商选择菜单（交互优先于报错）
        "provider" if args.is_empty() || (args[0] == "select" && args.len() < 2) => {
            ctx.state.show_palette = false;
            ctx.state.show_provider_menu = true;
            ctx.state.selected_provider_index = 0;
            Ok(())
        }
        "provider" => provider_wrapper(ctx, args).await,
        "stats" => tools::stats(ctx, args).await,
        "cost" => compat::cost(ctx, args).await,
        "review" => compat::review(ctx, args).await,
        "pr-comments" | "pr_comments" => compat::pr_comments(ctx, args).await,
        "prs" => compat::prs(ctx, args).await,
        "bug" => compat::bug(ctx, args).await,
        "terminal-setup" => compat::terminal_setup(ctx, args).await,
        "mdm" => mdm_wrapper(ctx, args).await,
        "vim" => compat::vim(ctx, args).await,
        "login" => compat::login(ctx, args).await,
        "logout" => compat::logout(ctx, args).await,
        // ── New compat commands ──────────────────────────────
        "code-review" => compat::code_review(ctx, args).await,
        "security-review" => compat::security_review(ctx, args).await,
        "simplify" => compat::simplify(ctx, args).await,
        "run" => compat::run(ctx, args).await,
        "feedback" => compat::feedback(ctx, args).await,
        "tasks" => compat::tasks(ctx, args).await,
        "workflows" => compat::workflows(ctx, args).await,
        "context" => compat::context(ctx, args).await,
        "bashes" => compat::bashes(ctx, args).await,
        "lint" => compat::lint(ctx, args).await,
        "upgrade" => compat::upgrade(ctx, args).await,
        "ide" => compat::ide(ctx, args).await,
        // ── Aliases ─────────────────────────────────────────
        "version" => core::about(ctx, args).await,
        "config" => config::settings(ctx, args).await,
        "compact" => utility::compress(ctx, args).await,
        "tokens" => compat::cost(ctx, args).await,
        "usage" => compat::cost(ctx, args).await,
        "remember" => {
            let mut a = vec!["add".to_string()];
            a.extend(args);
            memory_wrapper(ctx, a).await
        }
        "forget" => compat::forget(ctx, args).await,
        "todos" => compat::tasks(ctx, args).await,
        "tools" => tools::tools(ctx, args).await,
        "model" | "models" => model_wrapper(ctx, args).await,
        "settings" => config::settings(ctx, args).await,
        "lang" => config::lang(ctx, args).await,
        "theme" => config::theme(ctx, args).await,
        "copy" => utility::copy(ctx, args).await,
        "undo" => utility::undo(ctx, args).await,
        "compress" => utility::compress(ctx, args).await,
        "init" => init::run(ctx, args).await,
        "plan" => plan::run(ctx, args).await,
        "loop" => loop_wrapper(ctx, args).await,
        "agents" => agents_wrapper(ctx, args).await,
        "hooks" => hooks_wrapper(ctx, args).await,
        "plugin" => plugin_wrapper(ctx, args).await,
        "skills" => skills_wrapper(ctx, args).await,
        "extension" | "ext" => extension_wrapper(ctx, args).await,
        "remote" => remote_wrapper(ctx, args).await,
        "permissions" => permissions::run(ctx, args).await,
        "test" => test::run(ctx, args).await,
        "index" => index_ext_wrapper(ctx, args).await,
        "doctor" => doctor::run(ctx, args).await,
        "eval" => eval::run(ctx, args).await,
        "connect" => connect::run(ctx, args).await,
        "deep-link" => features::deep_link(ctx, args).await,
        "teleport" => features::teleport(ctx, args).await,
        "wiki" => features::wiki(ctx, args).await,
        "buddy" => features::buddy(ctx, args).await,
        "commit-and-push" => git_wrapper(args).await,
        "sandbox" => sandbox_wrapper(ctx, args).await,
        "feature-flags" => feature_flags_cmd(ctx, args).await,
        // ── Git subcommands ────────────────────────────────────
        "git" => git_ext_wrapper(ctx, args).await,
        "git-status" => extended::git_status(ctx, args).await,
        "git-log" => extended::git_log(ctx, args).await,
        "git-diff" => extended::git_diff(ctx, args).await,
        "git-branch" => extended::git_branch(ctx, args).await,
        "git-merge" => extended::git_merge(ctx, args).await,
        "git-rebase" => extended::git_rebase(ctx, args).await,
        "git-stash" => extended::git_stash(ctx, args).await,
        "git-tag" => extended::git_tag(ctx, args).await,
        "git-blame" => extended::git_blame(ctx, args).await,
        // ── Config subcommands ─────────────────────────────────
        "config-ext" => config_ext_wrapper(ctx, args).await,
        "config-show" => extended::config_show(ctx, args).await,
        "config-set" => extended::config_set(ctx, args).await,
        "config-reset" => extended::config_reset(ctx, args).await,
        "config-export" => extended::config_export(ctx, args).await,
        "config-import" => extended::config_import(ctx, args).await,
        // ── Session subcommands ────────────────────────────────
        "session" => session_ext_wrapper(ctx, args).await,
        "session-list" => extended::session_list(ctx, args).await,
        "session-resume" => extended::session_resume(ctx, args).await,
        "session-delete" => extended::session_delete(ctx, args).await,
        "session-export" => extended::session_export(ctx, args).await,
        "session-title" => extended::session_title(ctx, args).await,
        // ── Debug subcommands ──────────────────────────────────
        "debug" => debug_ext_wrapper(ctx, args).await,
        "debug-log" => extended::debug_log(ctx, args).await,
        "debug-tokens" => extended::debug_tokens(ctx, args).await,
        "debug-tools" => extended::debug_tools(ctx, args).await,
        "debug-state" => extended::debug_state(ctx, args).await,
        "debug-perf" => extended::debug_perf(ctx, args).await,
        // ── Agent subcommands ──────────────────────────────────
        "agent" => agent_ext_wrapper(ctx, args).await,
        "agent-list" => extended::agent_list(ctx, args).await,
        "agent-switch" => extended::agent_switch(ctx, args).await,
        "agent-create" => extended::agent_create(ctx, args).await,
        "agent-delete" => extended::agent_delete(ctx, args).await,
        // ── Workflow subcommands ───────────────────────────────
        "workflow" => workflow_ext_wrapper(ctx, args).await,
        "workflow-list" => extended::workflow_list(ctx, args).await,
        "workflow-run" => extended::workflow_run(ctx, args).await,
        "workflow-create" => extended::workflow_create(ctx, args).await,
        "workflow-edit" => extended::workflow_edit(ctx, args).await,
        // ── Utility commands ───────────────────────────────────
        "paste" => extended::paste_cmd(ctx, args).await,
        "clear-screen" => extended::clear_screen(ctx, args).await,
        "compact-ext" => extended::compact_cmd(ctx, args).await,
        "cost-breakdown" => extended::cost_cmd(ctx, args).await,
        "model-info" => extended::model_cmd(ctx, args).await,
        "provider-info" => extended::provider_cmd(ctx, args).await,
        "temperature" => extended::temperature_cmd(ctx, args).await,
        "token-count" => extended::tokens_cmd(ctx, args).await,
        "undo-edit" => extended::undo_cmd(ctx, args).await,
        "redo-edit" => extended::redo_cmd(ctx, args).await,
        "pending-diff" => extended::diff_cmd(ctx, args).await,
        "code-review-ext" => extended::review_cmd(ctx, args).await,
        "run-tests" => extended::test_cmd(ctx, args).await,
        "run-lint" => extended::lint_cmd(ctx, args).await,
        "run-format" => extended::format_cmd(ctx, args).await,
        "gen-docs" => extended::docs_cmd(ctx, args).await,
        "explain-code" => extended::explain_cmd(ctx, args).await,
        "suggest-refactor" => extended::refactor_cmd(ctx, args).await,
        "suggest-optimize" => extended::optimize_cmd(ctx, args).await,
        // ── Proactive & Remote Settings ─────────────────────────
        "suggestions" => extended::suggestions_cmd(ctx, args).await,
        "remote-settings" => extended::remote_settings_cmd(ctx, args).await,
        "voice" => extended::voice_cmd(ctx, args).await,
        "notifications" => extended::notifications_cmd(ctx, args).await,
        // ── Conversation (Claude Code parity) ───────────────────
        "export" => extended::export_conversation(ctx, args).await,
        "diff" => extended::diff_top(ctx, args).await,
        "files" => extended::files_in_context(ctx, args).await,
        "rewind" | "checkpoint" => extended::rewind(ctx, args).await,
        "rename" => extended::rename_session(ctx, args).await,
        _ => plugin_command_fallback(name, args, ctx).await,
    }
}

simple_command_wrapper!(
    model_wrapper,
    "model",
    crate::commands::model::ModelCommand,
    crate::commands::model::execute_model_command
);

simple_command_wrapper!(
    provider_wrapper,
    "provider",
    crate::commands::provider::ProviderCommand,
    crate::commands::provider::execute_provider_command
);

simple_command_wrapper!(
    memory_wrapper,
    "memory",
    crate::commands::memory::MemoryCommand,
    crate::commands::memory::execute_memory_command
);

async fn mcp_wrapper(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let cmd = clap::Command::new("mcp");
    let cmd = crate::commands::mcp::McpCommand::augment_subcommands(cmd);

    let cli_args = std::iter::once("mcp".to_string()).chain(args.into_iter());

    match cmd.try_get_matches_from(cli_args) {
        Ok(matches) => {
            let mcp_cmd = crate::commands::mcp::McpCommand::from_arg_matches(&matches)
                .map_err(|e| e.to_string())?;

            let needs_refresh = match &mcp_cmd {
                crate::commands::mcp::McpCommand::Add { .. }
                | crate::commands::mcp::McpCommand::Remove { .. }
                | crate::commands::mcp::McpCommand::Import { .. }
                | crate::commands::mcp::McpCommand::Refresh
                | crate::commands::mcp::McpCommand::Install { .. } => true,
                _ => false,
            };

            let msg = crate::commands::mcp::execute_mcp_command(mcp_cmd)
                .await
                .map_err(|e| e.to_string())?;

            if needs_refresh {
                let _ = ctx
                    .agent_tx
                    .send(crate::runtime::messages::AgentRequest::McpRefresh)
                    .await;
            }

            ctx.state.chat_history.push(crate::types::ChatEntry {
                is_streaming: Some(false),
                ..crate::types::ChatEntry::assistant(msg)
            });
            Ok(())
        }
        Err(e) => {
            ctx.state.chat_history.push(crate::types::ChatEntry {
                is_streaming: Some(false),
                ..crate::types::ChatEntry::assistant(e.render().to_string())
            });
            Ok(())
        }
    }
}

simple_command_wrapper!(
    skills_wrapper,
    "skills",
    crate::commands::skills::SkillsCommand,
    crate::commands::skills::execute_skills_command
);

simple_command_wrapper!(
    loop_wrapper,
    "loop",
    crate::commands::loop_cmd::LoopCommand,
    crate::commands::loop_cmd::execute_loop_command
);

simple_command_wrapper!(
    agents_wrapper,
    "agents",
    crate::commands::agents::AgentsCommand,
    crate::commands::agents::execute_agents_command
);

simple_command_wrapper!(
    hooks_wrapper,
    "hooks",
    crate::commands::hooks::HooksCommand,
    crate::commands::hooks::execute_hooks_command
);

simple_command_wrapper!(
    plugin_wrapper,
    "plugin",
    crate::commands::plugin::PluginCommand,
    crate::commands::plugin::execute_plugin_command
);

async fn extension_wrapper(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let args_str: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    match crate::commands::extension::execute_extension_command(&args_str).await {
        Ok(msg) => {
            ctx.state.chat_history.push(crate::types::ChatEntry {
                is_streaming: Some(false),
                ..crate::types::ChatEntry::assistant(msg)
            });
            Ok(())
        }
        Err(e) => {
            ctx.state.chat_history.push(crate::types::ChatEntry {
                is_streaming: Some(false),
                ..crate::types::ChatEntry::assistant(format!("❌ {}", e))
            });
            Ok(())
        }
    }
}

simple_command_wrapper!(
    remote_wrapper,
    "remote",
    crate::commands::remote::RemoteCommand,
    crate::commands::remote::execute_remote_command
);

async fn git_wrapper(args: Vec<String>) -> CommandResult {
    let cmd = clap::Command::new("commit-and-push");
    let cmd = crate::commands::git::GitCommand::augment_subcommands(cmd);

    let cli_args = std::iter::once("commit-and-push".to_string())
        .chain(std::iter::once("commit-and-push".to_string()))
        .chain(args.into_iter());

    match cmd.try_get_matches_from(cli_args) {
        Ok(matches) => {
            let git_cmd = crate::commands::git::GitCommand::from_arg_matches(&matches)
                .map_err(|e| e.to_string())?;
            crate::commands::git::execute_git_command(git_cmd)
                .await
                .map_err(|e| e.to_string())
        }
        Err(e) => Err(e.render().to_string()),
    }
}

async fn git_ext_wrapper(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let sub = args.first().cloned().unwrap_or_else(|| "status".to_string());
    let rest: Vec<String> = args.into_iter().skip(1).collect();
    match sub.as_str() {
        "status" => extended::git_status(ctx, rest).await,
        "log" => extended::git_log(ctx, rest).await,
        "diff" => extended::git_diff(ctx, rest).await,
        "branch" => extended::git_branch(ctx, rest).await,
        "merge" => extended::git_merge(ctx, rest).await,
        "rebase" => extended::git_rebase(ctx, rest).await,
        "stash" => extended::git_stash(ctx, rest).await,
        "tag" => extended::git_tag(ctx, rest).await,
        "blame" => extended::git_blame(ctx, rest).await,
        _ => {
            push_msg_to_ctx(ctx, format!("Unknown git subcommand: {}. Use: status, log, diff, branch, merge, rebase, stash, tag, blame", sub));
            Ok(())
        }
    }
}

async fn config_ext_wrapper(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let sub = args.first().cloned().unwrap_or_else(|| "show".to_string());
    let rest: Vec<String> = args.into_iter().skip(1).collect();
    match sub.as_str() {
        "show" => extended::config_show(ctx, rest).await,
        "set" => extended::config_set(ctx, rest).await,
        "reset" => extended::config_reset(ctx, rest).await,
        "export" => extended::config_export(ctx, rest).await,
        "import" => extended::config_import(ctx, rest).await,
        _ => {
            push_msg_to_ctx(ctx, format!("Unknown config subcommand: {}. Use: show, set, reset, export, import", sub));
            Ok(())
        }
    }
}

async fn session_ext_wrapper(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let sub = args.first().cloned().unwrap_or_else(|| "list".to_string());
    let rest: Vec<String> = args.into_iter().skip(1).collect();
    match sub.as_str() {
        "list" => extended::session_list(ctx, rest).await,
        "resume" => extended::session_resume(ctx, rest).await,
        "delete" => extended::session_delete(ctx, rest).await,
        "export" => extended::session_export(ctx, rest).await,
        "title" => extended::session_title(ctx, rest).await,
        _ => {
            push_msg_to_ctx(ctx, format!("Unknown session subcommand: {}. Use: list, resume, delete, export, title", sub));
            Ok(())
        }
    }
}

async fn debug_ext_wrapper(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let sub = args.first().cloned().unwrap_or_else(|| "state".to_string());
    let rest: Vec<String> = args.into_iter().skip(1).collect();
    match sub.as_str() {
        "log" => extended::debug_log(ctx, rest).await,
        "tokens" => extended::debug_tokens(ctx, rest).await,
        "tools" => extended::debug_tools(ctx, rest).await,
        "state" => extended::debug_state(ctx, rest).await,
        "perf" => extended::debug_perf(ctx, rest).await,
        _ => {
            push_msg_to_ctx(ctx, format!("Unknown debug subcommand: {}. Use: log, tokens, tools, state, perf", sub));
            Ok(())
        }
    }
}

async fn agent_ext_wrapper(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let sub = args.first().cloned().unwrap_or_else(|| "list".to_string());
    let rest: Vec<String> = args.into_iter().skip(1).collect();
    match sub.as_str() {
        "list" => extended::agent_list(ctx, rest).await,
        "switch" => extended::agent_switch(ctx, rest).await,
        "create" => extended::agent_create(ctx, rest).await,
        "delete" => extended::agent_delete(ctx, rest).await,
        _ => {
            push_msg_to_ctx(ctx, format!("Unknown agent subcommand: {}. Use: list, switch, create, delete", sub));
            Ok(())
        }
    }
}

async fn workflow_ext_wrapper(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let sub = args.first().cloned().unwrap_or_else(|| "list".to_string());
    let rest: Vec<String> = args.into_iter().skip(1).collect();
    match sub.as_str() {
        "list" => extended::workflow_list(ctx, rest).await,
        "run" => extended::workflow_run(ctx, rest).await,
        "create" => extended::workflow_create(ctx, rest).await,
        "edit" => extended::workflow_edit(ctx, rest).await,
        _ => {
            push_msg_to_ctx(ctx, format!("Unknown workflow subcommand: {}. Use: list, run, create, edit", sub));
            Ok(())
        }
    }
}

async fn index_ext_wrapper(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let sub = args.first().cloned().unwrap_or_else(|| "status".to_string());
    let rest: Vec<String> = args.into_iter().skip(1).collect();
    match sub.as_str() {
        "status" => extended::index_status(ctx, rest).await,
        "rebuild" => extended::index_rebuild(ctx, rest).await,
        "structure" => index_cmd(ctx, rest).await,
        _ => {
            push_msg_to_ctx(ctx, format!("Unknown index subcommand: {}. Use: status, rebuild, structure", sub));
            Ok(())
        }
    }
}

fn push_msg_to_ctx(ctx: CommandContext<'_>, content: impl Into<String>) {
    ctx.state
        .chat_history
        .push(crate::types::ChatEntry::assistant(content).with_streaming(false));
}

/// Sandbox command wrapper
async fn sandbox_wrapper(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let sub_cmd = args.first().map(|s| s.as_str()).unwrap_or("status");

    let message = match sub_cmd {
        "on" | "enable" => {
            ctx.state.sandbox_enabled = true;
            if crate::core::sandbox::SandboxManager::is_available() {
                "✅ Sandbox enabled\n\nSandbox will isolate file system and network access for command execution.".to_string()
            } else {
                let help = crate::core::sandbox::SandboxManager::get_installation_help();
                format!(
                    "⚠️ Sandbox not available\n\n{}\n\nPlease follow the steps above to install and try again.",
                    help.join("\n")
                )
            }
        }
        "off" | "disable" => {
            ctx.state.sandbox_enabled = false;
            "✅ Sandbox disabled\n\nCommands will execute directly without isolation protection.".to_string()
        }
        "help" | "install" => {
            let help = crate::core::sandbox::SandboxManager::get_installation_help();
            help.join("\n")
        }
        "status" | _ => {
            let enabled = ctx.state.sandbox_enabled;
            let available = crate::core::sandbox::SandboxManager::is_available();

            let mode = if available {
                #[cfg(target_os = "linux")]
                {
                    "bubblewrap (Linux 命名空间隔离)"
                }
                #[cfg(target_os = "macos")]
                {
                    "Seatbelt (macOS 进程隔离)"
                }
                #[cfg(target_os = "windows")]
                {
                    "WSL2 + bubblewrap (Linux 子系统)"
                }
                #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
                {
                    "不支持"
                }
            } else {
                "不可用"
            };

            let help_hint = if !available {
                "\n\n使用 /sandbox help 查看安装指南"
            } else {
                ""
            };

            format!(
                "📦 沙箱状态\n\n状态: {}\n模式: {}{}",
                if enabled {
                    "已启用 ✅"
                } else {
                    "已禁用 ⭕"
                },
                mode,
                help_hint
            )
        }
    };

    ctx.state.chat_history.push(crate::types::ChatEntry {
        is_streaming: Some(false),
        ..crate::types::ChatEntry::assistant(message)
    });

    Ok(())
}

async fn plugin_command_fallback(
    name: &str,
    args: Vec<String>,
    ctx: CommandContext<'_>,
) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let result = crate::core::plugins::execute_plugin_command(&cwd, name, &args).await?;

    let Some(result) = result else {
        // 未知命令：用已有的 fuzzy 评分给出 "did you mean" 建议
        let mut suggestion = String::new();
        let query = name.to_lowercase();
        let mut best: Option<(i32, &str)> = None;
        for cmd in system::ALL_COMMANDS {
            if let Some(score) = system::fuzzy_score(cmd, &query) {
                let better = best
                    .as_ref()
                    .map(|(s, _)| score > *s)
                    .unwrap_or(true);
                if better && score >= 300 {
                    best = Some((score, cmd.name));
                }
            }
        }
        if let Some((_, cmd_name)) = best {
            suggestion = format!(" — did you mean /{}?", cmd_name);
        }
        return Err(format!("Unknown command: /{}{}", name, suggestion));
    };

    let mut lines = vec![format!(
        "Plugin command `/{}` from `{}` ({}) {}",
        result.command_name,
        result.source,
        result.plugin_name,
        if result.success {
            "completed."
        } else {
            "failed."
        }
    )];

    if !args.is_empty() {
        lines.push(format!("args: {}", args.join(" ")));
    }
    if !result.stdout.is_empty() {
        lines.push(format!("stdout:\n{}", result.stdout));
    }
    if !result.stderr.is_empty() {
        lines.push(format!("stderr:\n{}", result.stderr));
    }
    if !result.success {
        if result.timed_out {
            lines.push("status: timed out".to_string());
        } else {
            lines.push(format!("exit_code: {:?}", result.exit_code));
        }
    }

    ctx.state.chat_history.push(crate::types::ChatEntry {
        is_streaming: Some(false),
        ..crate::types::ChatEntry::assistant(lines.join("\n"))
    });

    Ok(())
}

/// /index command - manually trigger code structure indexing
async fn index_cmd(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    use crate::core::context::structure_index::StructureIndex;
    use std::path::Path;
    use walkdir::WalkDir;

    let start = std::time::Instant::now();
    
    // Get the directory to index (default: current directory)
    let dir = if args.is_empty() {
        std::env::current_dir().map_err(|e| e.to_string())?
    } else {
        Path::new(&args[0]).to_path_buf()
    };

    if !dir.exists() {
        ctx.state.chat_history.push(crate::types::ChatEntry {
            is_streaming: Some(false),
            ..crate::types::ChatEntry::assistant(format!("❌ Directory not found: {}", dir.display()))
        });
        return Ok(());
    }

    let mut index = StructureIndex::new();
    let mut file_count = 0;
    let mut error_count = 0;

    // Walk through the directory and index all supported files
    for entry in WalkDir::new(&dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        let path_str = path.to_string_lossy().to_string();
        
        // Skip hidden directories and common non-source directories
        if path_str.contains("/.") || path_str.contains("\\.") 
            || path_str.contains("node_modules") 
            || path_str.contains("target/") 
            || path_str.contains("__pycache__")
            || path_str.contains(".git/")
        {
            continue;
        }

        // Check if it's a supported file type
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !["rs", "py", "js", "jsx", "ts", "tsx"].contains(&ext) {
            continue;
        }

        // Read and index the file
        match std::fs::read_to_string(path) {
            Ok(content) => {
                index.index_file(&path_str, &content);
                file_count += 1;
            }
            Err(e) => {
                error_count += 1;
                log::warn!("Failed to read {}: {}", path_str, e);
            }
        }
    }

    let elapsed = start.elapsed();
    
    // Build summary
    let mut summary = Vec::new();
    summary.push(format!("📊 Code Structure Index Complete"));
    summary.push(format!(""));
    summary.push(format!("📁 Directory: {}", dir.display()));
    summary.push(format!("📄 Files indexed: {}", file_count));
    summary.push(format!("⏱️  Time: {:.2}s", elapsed.as_secs_f64()));
    
    if error_count > 0 {
        summary.push(format!("⚠️  Errors: {}", error_count));
    }
    
    summary.push(format!(""));
    summary.push(format!("📈 Index Statistics:"));
    summary.push(format!("   Functions: {}", index.functions.len()));
    summary.push(format!("   Types: {}", index.types.len()));
    summary.push(format!("   Imports: {}", index.imports.len()));
    summary.push(format!("   Call graph entries: {}", index.call_graph.len()));

    // Store the index in the state for later use
    ctx.state.structure_index = Some(index);

    ctx.state.chat_history.push(crate::types::ChatEntry {
        is_streaming: Some(false),
        ..crate::types::ChatEntry::assistant(summary.join("\n"))
    });

    Ok(())
}

/// Feature flags command - list or toggle feature flags
async fn feature_flags_cmd(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if args.is_empty() {
        // List all feature flags
        let flags = ctx.state.feature_flags.list_flags();
        let mut output = String::from("🚩 Feature Flags:\n\n");

        for (name, enabled) in flags {
            let status = if enabled { "✅" } else { "❌" };
            output.push_str(&format!("  {} {}: {}\n", status, name, if enabled { "enabled" } else { "disabled" }));
        }

        output.push_str("\nUsage: /feature-flags <flag_name> [on|off]\n");
        output.push_str("Example: /feature-flags vim_mode off\n");

        ctx.state.chat_history.push(crate::types::ChatEntry {
            is_streaming: Some(false),
            ..crate::types::ChatEntry::assistant(output)
        });
    } else if args.len() == 1 {
        // Show specific flag details
        let flag_name = &args[0];
        if let Some(flag) = ctx.state.feature_flags.get_flag(flag_name) {
            let output = format!(
                "🚩 Feature Flag: {}\n\nDescription: {}\nStatus: {}\nRollout: {}%\nAllowed users: {}\nDenied users: {}",
                flag.name,
                flag.description,
                if flag.enabled { "✅ enabled" } else { "❌ disabled" },
                flag.rollout_percentage,
                if flag.allowed_users.is_empty() { "none".to_string() } else { flag.allowed_users.join(", ") },
                if flag.denied_users.is_empty() { "none".to_string() } else { flag.denied_users.join(", ") }
            );

            ctx.state.chat_history.push(crate::types::ChatEntry {
                is_streaming: Some(false),
                ..crate::types::ChatEntry::assistant(output)
            });
        } else {
            ctx.state.chat_history.push(crate::types::ChatEntry {
                is_streaming: Some(false),
                ..crate::types::ChatEntry::assistant(format!("❌ Unknown feature flag: {}", flag_name))
            });
        }
    } else if args.len() == 2 {
        // Toggle flag
        let flag_name = &args[0];
        let state = &args[1];

        if ctx.state.feature_flags.get_flag(flag_name).is_none() {
            ctx.state.chat_history.push(crate::types::ChatEntry {
                is_streaming: Some(false),
                ..crate::types::ChatEntry::assistant(format!("❌ Unknown feature flag: {}", flag_name))
            });
            return Ok(());
        }

        match state.to_lowercase().as_str() {
            "on" | "enable" | "true" | "1" => {
                ctx.state.feature_flags.set_enabled(flag_name, true);
                ctx.state.chat_history.push(crate::types::ChatEntry {
                    is_streaming: Some(false),
                    ..crate::types::ChatEntry::assistant(format!("✅ Feature flag '{}' enabled", flag_name))
                });
            }
            "off" | "disable" | "false" | "0" => {
                ctx.state.feature_flags.set_enabled(flag_name, false);
                ctx.state.chat_history.push(crate::types::ChatEntry {
                    is_streaming: Some(false),
                    ..crate::types::ChatEntry::assistant(format!("✅ Feature flag '{}' disabled", flag_name))
                });
            }
            _ => {
                ctx.state.chat_history.push(crate::types::ChatEntry {
                    is_streaming: Some(false),
                    ..crate::types::ChatEntry::assistant(format!("❌ Invalid state '{}'. Use: on/off, enable/disable, true/false", state))
                });
            }
        }
    } else {
        ctx.state.chat_history.push(crate::types::ChatEntry {
            is_streaming: Some(false),
            ..crate::types::ChatEntry::assistant("Usage: /feature-flags [flag_name] [on|off]".to_string())
        });
    }

    Ok(())
}

/// MDM command wrapper
async fn mdm_wrapper(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let sub_cmd = args.first().map(|s| s.as_str()).unwrap_or("status");

    let message = match sub_cmd {
        "enroll" => {
            if let Some(server_url) = args.get(1) {
                match ctx.state.mdm.enroll(server_url) {
                    Ok(()) => format!("✅ Enrolled in MDM server: {}", server_url),
                    Err(e) => format!("❌ Failed to enroll: {}", e),
                }
            } else {
                "❌ Usage: /mdm enroll <server_url>".to_string()
            }
        }
        "unenroll" => {
            ctx.state.mdm.unenroll();
            "✅ Unenrolled from MDM server".to_string()
        }
        "sync" => {
            match ctx.state.mdm.sync_policies().await {
                Ok(()) => {
                    let status = ctx.state.mdm.get_status();
                    format!("✅ Policies synced\n\n{}", status)
                }
                Err(e) => format!("❌ Failed to sync policies: {}", e),
            }
        }
        "status" | _ => {
            let status = ctx.state.mdm.get_status();
            format!("📋 MDM Status\n\n{}", status)
        }
    };

    ctx.state.chat_history.push(crate::types::ChatEntry {
        is_streaming: Some(false),
        ..crate::types::ChatEntry::assistant(message)
    });

    Ok(())
}
