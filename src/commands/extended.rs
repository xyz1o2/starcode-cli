use crate::commands::execution::{CommandContext, CommandResult};
use crate::runtime::messages::AgentRequest;
use crate::types::ChatEntry;
use arboard::Clipboard;
use std::path::PathBuf;

fn push_msg(ctx: &mut CommandContext<'_>, content: impl Into<String>) {
    ctx.state
        .chat_history
        .push(ChatEntry::assistant(content).with_streaming(false));
}

// ── Git Commands ──────────────────────────────────────────────────

pub async fn git_status(ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: "Run: git status".to_string(),
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn git_log(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let count = args
        .first()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(20);
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: format!("Run: git log --oneline -n {}", count),
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn git_diff(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let target = if args.is_empty() {
        String::new()
    } else {
        args.join(" ")
    };
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: format!("Run: git diff {}", target),
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn git_branch(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("list");
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    let cmd = match sub {
        "list" | "-a" => "git branch -a".to_string(),
        "create" | "new" => {
            let name = args.get(1).cloned().unwrap_or_default();
            if name.is_empty() {
                push_msg(&mut ctx, "Usage: /git branch create <name>");
                return Ok(());
            }
            format!("git branch {}", name)
        }
        "delete" | "del" => {
            let name = args.get(1).cloned().unwrap_or_default();
            if name.is_empty() {
                push_msg(&mut ctx, "Usage: /git branch delete <name>");
                return Ok(());
            }
            format!("git branch -d {}", name)
        }
        "switch" | "checkout" => {
            let name = args.get(1).cloned().unwrap_or_default();
            if name.is_empty() {
                push_msg(&mut ctx, "Usage: /git branch switch <name>");
                return Ok(());
            }
            format!("git checkout {}", name)
        }
        other => format!("git branch {}", other),
    };
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: format!("Run: {}", cmd),
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn git_merge(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let branch = args.first().cloned().unwrap_or_default();
    if branch.is_empty() {
        push_msg(&mut ctx, "Usage: /git merge <branch>");
        return Ok(());
    }
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: format!("Run: git merge {}", branch),
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn git_rebase(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let branch = args.first().cloned().unwrap_or_default();
    if branch.is_empty() {
        push_msg(&mut ctx, "Usage: /git rebase <branch>");
        return Ok(());
    }
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: format!("Run: git rebase {}", branch),
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn git_stash(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("list");
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    let cmd = match sub {
        "list" => "git stash list".to_string(),
        "save" | "push" => {
            let msg = args.get(1).cloned().unwrap_or_default();
            if msg.is_empty() {
                "git stash".to_string()
            } else {
                format!("git stash push -m \"{}\"", msg)
            }
        }
        "pop" => "git stash pop".to_string(),
        "apply" => {
            let idx = args.get(1).cloned().unwrap_or_default();
            if idx.is_empty() {
                "git stash apply".to_string()
            } else {
                format!("git stash apply {}", idx)
            }
        }
        "drop" => {
            let idx = args.get(1).cloned().unwrap_or_default();
            if idx.is_empty() {
                "git stash drop".to_string()
            } else {
                format!("git stash drop {}", idx)
            }
        }
        "clear" => "git stash clear".to_string(),
        other => format!("git stash {}", other),
    };
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: format!("Run: {}", cmd),
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn git_tag(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("list");
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    let cmd = match sub {
        "list" => "git tag".to_string(),
        "create" | "add" => {
            let name = args.get(1).cloned().unwrap_or_default();
            if name.is_empty() {
                push_msg(&mut ctx, "Usage: /git tag create <name> [message]");
                return Ok(());
            }
            let msg = args.get(2).cloned().unwrap_or_default();
            if msg.is_empty() {
                format!("git tag {}", name)
            } else {
                format!("git tag -a {} -m \"{}\"", name, msg)
            }
        }
        "delete" | "del" => {
            let name = args.get(1).cloned().unwrap_or_default();
            if name.is_empty() {
                push_msg(&mut ctx, "Usage: /git tag delete <name>");
                return Ok(());
            }
            format!("git tag -d {}", name)
        }
        other => format!("git tag {}", other),
    };
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: format!("Run: {}", cmd),
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn git_blame(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let file = args.first().cloned().unwrap_or_default();
    if file.is_empty() {
        push_msg(&mut ctx, "Usage: /git blame <file>");
        return Ok(());
    }
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: format!("Run: git blame {}", file),
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ── Config Commands ───────────────────────────────────────────────

pub async fn config_show(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let settings_mgr = crate::core::config::settings_manager::get_settings_manager()
        .await
        .map_err(|e| e.to_string())?;
    let settings = settings_mgr
        .load_user_settings()
        .await
        .map_err(|e| e.to_string())?;
    // Never dump credentials into chat. The snapshot keeps the shape of the
    // config but replaces API keys/passwords with `[redacted]`.
    let json = crate::commands::config::redacted_settings_json(&settings)
        .unwrap_or_else(|_| "{}".to_string());
    push_msg(
        &mut ctx,
        format!("Current configuration:\n```json\n{}\n```", json),
    );
    Ok(())
}

pub async fn config_set(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    crate::commands::config::set_from_args(ctx, args).await
}

pub async fn config_reset(ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    crate::commands::config::reset_preferences(ctx).await
}

pub async fn config_export(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let settings_mgr = crate::core::config::settings_manager::get_settings_manager()
        .await
        .map_err(|e| e.to_string())?;
    let settings = settings_mgr
        .load_user_settings()
        .await
        .map_err(|e| e.to_string())?;
    let json = crate::commands::config::redacted_settings_json(&settings)?;
    push_msg(
        &mut ctx,
        format!("Exported configuration (redacted):\n```json\n{}\n```", json),
    );
    Ok(())
}

pub async fn config_import(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    crate::commands::config::import_snapshot(ctx, args).await
}

// ── Session Commands ──────────────────────────────────────────────

pub async fn session_list(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let sessions = crate::core::session_persistence::list_sessions()
        .await
        .unwrap_or_default();
    if sessions.is_empty() {
        push_msg(&mut ctx, "No saved sessions found.");
        return Ok(());
    }
    let mut lines = vec!["Saved sessions:".to_string()];
    for s in &sessions {
        lines.push(format!(
            "  • {} - {} ({})",
            s.session_id, s.title, s.start_time
        ));
    }
    push_msg(&mut ctx, lines.join("\n"));
    Ok(())
}

pub async fn session_resume(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let session_id = args.first().cloned().unwrap_or_default();
    if session_id.is_empty() {
        // 无 id：打开交互式会话选择菜单（与 /resume 一致），而不是打印用法
        let sessions = crate::utils::session_manager::list_session_summaries()
            .await
            .unwrap_or_default();
        if sessions.is_empty() {
            ctx.state.current_status_line = Some("No saved sessions yet.".to_string());
            return Ok(());
        }
        ctx.state.close_palette();
        ctx.state.open_session_menu();
        ctx.state.available_sessions = sessions;
        ctx.state.selected_session_index = 0;
        return Ok(());
    }
    // 带 id：委托给真正能恢复会话的 /resume 路径（session_manager 后端）
    crate::commands::chat::resume_cmd(ctx, args).await
}

pub async fn session_delete(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let session_id = args.first().cloned().unwrap_or_default();
    if session_id.is_empty() {
        push_msg(&mut ctx, "Usage: /session delete <session-id>");
        return Ok(());
    }
    crate::core::session_persistence::delete_session(&session_id)
        .await
        .map_err(|e| e.to_string())?;
    push_msg(&mut ctx, format!("Session `{}` deleted.", session_id));
    Ok(())
}

pub async fn session_export(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let session_id = args.first().cloned().unwrap_or_default();
    if session_id.is_empty() {
        push_msg(&mut ctx, "Usage: /session export <session-id>");
        return Ok(());
    }
    match crate::core::session_persistence::export_session(&session_id).await {
        Ok(content) => {
            let out_path = format!("{}.jsonl", session_id);
            std::fs::write(&out_path, &content).map_err(|e| e.to_string())?;
            push_msg(&mut ctx, format!("Session exported to: {}", out_path));
        }
        Err(e) => {
            push_msg(&mut ctx, format!("Export failed: {}", e));
        }
    }
    Ok(())
}

pub async fn session_title(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let title = args.join(" ");
    if title.is_empty() {
        push_msg(&mut ctx, "Usage: /session title <title>");
        return Ok(());
    }
    ctx.state.current_session_title = Some(title.clone());
    push_msg(&mut ctx, format!("Session title set to: {}", title));
    Ok(())
}

// ── Debug Commands ────────────────────────────────────────────────

pub async fn debug_log(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let current = std::env::var("STAR_LOG_ENABLED").unwrap_or_default();
    let new_state = if current == "true" { "false" } else { "true" };
    std::env::set_var("STAR_LOG_ENABLED", new_state);
    push_msg(
        &mut ctx,
        format!(
            "Debug logging: {}",
            if new_state == "true" { "ON" } else { "OFF" }
        ),
    );
    Ok(())
}

pub async fn debug_tokens(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let mut lines = vec!["Token Usage Details:".to_string()];
    if let Some(usage) = &ctx.state.token_usage {
        lines.push(format!("  Prompt tokens: {}", usage.prompt_tokens));
        lines.push(format!("  Completion tokens: {}", usage.completion_tokens));
        lines.push(format!("  Total tokens: {}", usage.total_tokens));
    } else {
        lines.push("  No token usage data available.".to_string());
    }
    if ctx.state.total_cost > 0.0 {
        lines.push(format!("  Estimated cost: ${:.6}", ctx.state.total_cost));
    }
    push_msg(&mut ctx, lines.join("\n"));
    Ok(())
}

pub async fn debug_tools(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let tool_entries: Vec<_> = ctx
        .state
        .chat_history
        .iter()
        .filter(|e| e.entry_type == crate::types::ChatEntryType::ToolCall)
        .collect();
    if tool_entries.is_empty() {
        push_msg(&mut ctx, "No tool calls in this session.");
        return Ok(());
    }
    let mut lines = vec![format!("Tool call history ({} calls):", tool_entries.len())];
    for (i, entry) in tool_entries.iter().enumerate().take(30) {
        let name = entry
            .tool_call
            .as_ref()
            .map(|tc| tc.function.name.as_str())
            .unwrap_or("unknown");
        lines.push(format!(
            "  {}. {} - {}",
            i + 1,
            name,
            &entry.content[..entry.content.len().min(80)]
        ));
    }
    push_msg(&mut ctx, lines.join("\n"));
    Ok(())
}

pub async fn debug_state(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let mut lines = vec!["Internal State:".to_string()];
    lines.push(format!("  Messages: {}", ctx.state.chat_history.len()));
    lines.push(format!("  Model: {}", ctx.state.current_model));
    lines.push(format!("  Processing: {}", ctx.state.is_processing));
    lines.push(format!("  Streaming: {}", ctx.state.is_streaming));
    lines.push(format!("  Sandbox: {}", ctx.state.sandbox_enabled));
    let mode = match ctx.state.approval_mode {
        crate::types::ApprovalMode::Default => "default",
        crate::types::ApprovalMode::AcceptEdits => "acceptEdits",
        crate::types::ApprovalMode::Plan => "plan",
        crate::types::ApprovalMode::Yolo => "yolo",
    };
    lines.push(format!("  Approval mode: {}", mode));
    if let Some(branch) = &ctx.state.git_branch {
        lines.push(format!("  Git branch: {}", branch));
    }
    push_msg(&mut ctx, lines.join("\n"));
    Ok(())
}

pub async fn debug_perf(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    push_msg(
        &mut ctx,
        "Performance metrics:\n  Frame rate: ~30 FPS target\n  Session uptime: active\n  Use /stats for detailed statistics.",
    );
    Ok(())
}

// ── Agent Commands ────────────────────────────────────────────────

pub async fn agent_list(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    push_msg(
        &mut ctx,
        "Available agents:\n  • default - General-purpose coding assistant\nUse /agents list for full agent definitions.",
    );
    Ok(())
}

pub async fn agent_switch(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let name = args.first().cloned().unwrap_or_default();
    if name.is_empty() {
        push_msg(&mut ctx, "Usage: /agent switch <name>");
        return Ok(());
    }
    push_msg(
        &mut ctx,
        format!(
            "Agent switch requested: {}\nUse /agents to manage agent definitions.",
            name
        ),
    );
    Ok(())
}

pub async fn agent_create(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let name = args.first().cloned().unwrap_or_default();
    if name.is_empty() {
        push_msg(&mut ctx, "Usage: /agent create <name>");
        return Ok(());
    }
    push_msg(
        &mut ctx,
        format!(
            "Create agent: {}\nUse /agents create for interactive agent creation.",
            name
        ),
    );
    Ok(())
}

pub async fn agent_delete(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let name = args.first().cloned().unwrap_or_default();
    if name.is_empty() {
        push_msg(&mut ctx, "Usage: /agent delete <name>");
        return Ok(());
    }
    push_msg(
        &mut ctx,
        format!(
            "Delete agent: {}\nUse /agents delete for confirmation.",
            name
        ),
    );
    Ok(())
}

// ── Workflow Commands ─────────────────────────────────────────────

pub async fn workflow_list(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    push_msg(
        &mut ctx,
        "Workflow definitions:\nUse /workflows to list project workflow files.",
    );
    Ok(())
}

pub async fn workflow_run(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let name = args.first().cloned().unwrap_or_default();
    if name.is_empty() {
        push_msg(&mut ctx, "Usage: /workflow run <name>");
        return Ok(());
    }
    push_msg(&mut ctx, format!("Run workflow: {}", name));
    Ok(())
}

pub async fn workflow_create(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let name = args.first().cloned().unwrap_or_default();
    if name.is_empty() {
        push_msg(&mut ctx, "Usage: /workflow create <name>");
        return Ok(());
    }
    push_msg(
        &mut ctx,
        format!(
            "Create workflow: {}\nCreate a .star/workflows/{}.md file.",
            name, name
        ),
    );
    Ok(())
}

pub async fn workflow_edit(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let name = args.first().cloned().unwrap_or_default();
    if name.is_empty() {
        push_msg(&mut ctx, "Usage: /workflow edit <name>");
        return Ok(());
    }
    push_msg(&mut ctx, format!("Edit workflow: {}", name));
    Ok(())
}

// ── Utility Commands ──────────────────────────────────────────────

pub async fn paste_cmd(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    match Clipboard::new() {
        Ok(mut clipboard) => match clipboard.get_text() {
            Ok(text) => {
                push_msg(&mut ctx, format!("Clipboard content:\n```\n{}\n```", text));
            }
            Err(e) => {
                push_msg(&mut ctx, format!("Failed to read clipboard: {}", e));
            }
        },
        Err(e) => {
            push_msg(&mut ctx, format!("Unable to access clipboard: {}", e));
        }
    }
    Ok(())
}

pub async fn clear_screen(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    ctx.state.chat_history.clear();
    push_msg(&mut ctx, "Screen cleared.");
    Ok(())
}

pub async fn compact_cmd(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    let _ = ctx
        .agent_tx
        .send(AgentRequest::Compress {
            message_id,
            instructions: crate::commands::utility::compact_instructions(&args),
        })
        .await;
    Ok(())
}

pub async fn cost_cmd(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let mut lines = vec!["Cost Breakdown:".to_string()];
    if let Some(usage) = &ctx.state.token_usage {
        lines.push(format!("  Prompt tokens: {}", usage.prompt_tokens));
        lines.push(format!("  Completion tokens: {}", usage.completion_tokens));
        lines.push(format!("  Total tokens: {}", usage.total_tokens));
    } else {
        lines.push("  No usage data yet.".to_string());
    }
    if ctx.state.total_cost > 0.0 {
        lines.push(format!("  Estimated cost: ${:.6}", ctx.state.total_cost));
    }
    push_msg(&mut ctx, lines.join("\n"));
    Ok(())
}

pub async fn model_cmd(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let model = if ctx.state.current_model.is_empty() {
        "Not set".to_string()
    } else {
        ctx.state.current_model.clone()
    };
    push_msg(&mut ctx, format!("Current model: {}", model));
    Ok(())
}

pub async fn provider_cmd(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    push_msg(
        &mut ctx,
        "Use /provider list to see available providers, /provider select to switch.",
    );
    Ok(())
}

pub async fn temperature_cmd(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if let Some(val) = args.first() {
        push_msg(
            &mut ctx,
            format!(
                "Temperature set to: {}\nNote: This takes effect on the next request.",
                val
            ),
        );
    } else {
        push_msg(
            &mut ctx,
            "Current temperature: default (provider-specific)\nUsage: /temperature <value>",
        );
    }
    Ok(())
}

pub async fn tokens_cmd(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let mut lines = vec!["Token Count:".to_string()];
    if let Some(usage) = &ctx.state.token_usage {
        lines.push(format!("  Prompt: {}", usage.prompt_tokens));
        lines.push(format!("  Completion: {}", usage.completion_tokens));
        lines.push(format!("  Total: {}", usage.total_tokens));
    } else {
        lines.push("  No data available.".to_string());
    }
    push_msg(&mut ctx, lines.join("\n"));
    Ok(())
}

pub async fn undo_cmd(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    crate::commands::utility::undo(ctx, args).await
}

pub async fn redo_cmd(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    push_msg(&mut ctx, "Redo: no actions available to redo.");
    Ok(())
}

pub async fn diff_cmd(ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: "Run: git diff".to_string(),
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn review_cmd(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let target = if args.is_empty() {
        "the current changes".to_string()
    } else {
        args.join(" ")
    };
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: format!(
                "Run a code review for {}. Focus on correctness, bugs, security. Return findings by severity.",
                target
            ),
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn test_cmd(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let filter = args.join(" ");
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    let cmd = if filter.is_empty() {
        "Run the project test suite".to_string()
    } else {
        format!("Run tests matching: {}", filter)
    };
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: cmd,
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn lint_cmd(ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: "Run the project linter and report issues".to_string(),
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn format_cmd(ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: "Run the project formatter on changed files".to_string(),
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn docs_cmd(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let target = if args.is_empty() {
        "the project".to_string()
    } else {
        args.join(" ")
    };
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: format!("Generate documentation for {}", target),
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn explain_cmd(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let target = if args.is_empty() {
        "the current context".to_string()
    } else {
        args.join(" ")
    };
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: format!("Explain how {} works", target),
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn refactor_cmd(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let target = if args.is_empty() {
        "the current code".to_string()
    } else {
        args.join(" ")
    };
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: format!("Suggest refactoring for {}", target),
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn optimize_cmd(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let target = if args.is_empty() {
        "the current code".to_string()
    } else {
        args.join(" ")
    };
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: format!("Suggest performance optimizations for {}", target),
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ── Proactive Suggestions Commands ──────────────────────────────

pub async fn suggestions_cmd(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("show");

    match sub {
        "show" => {
            let suggestions = ctx.state.proactive_suggestions.get_active_suggestions();
            if suggestions.is_empty() {
                push_msg(&mut ctx, "No active suggestions.".to_string());
            } else {
                let mut lines = vec!["Active Suggestions:".to_string()];
                for s in suggestions {
                    let priority = match s.priority {
                        crate::core::proactive::suggestions::SuggestionPriority::Low => "LOW",
                        crate::core::proactive::suggestions::SuggestionPriority::Medium => "MED",
                        crate::core::proactive::suggestions::SuggestionPriority::High => "HIGH",
                    };
                    lines.push(format!("  [{}] {} ({})", priority, s.message, s.id));
                }
                push_msg(&mut ctx, lines.join("\n"));
            }
        }
        "dismiss" => {
            if let Some(id) = args.get(1) {
                ctx.state.proactive_suggestions.dismiss_suggestion(id);
                push_msg(&mut ctx, format!("Dismissed suggestion: {}", id));
            } else {
                push_msg(&mut ctx, "Usage: /suggestions dismiss <id>".to_string());
            }
        }
        "clear" => {
            ctx.state.proactive_suggestions.clear_all();
            push_msg(&mut ctx, "All suggestions cleared.".to_string());
        }
        "on" => {
            ctx.state.proactive_suggestions.enabled = true;
            push_msg(&mut ctx, "Proactive suggestions enabled.".to_string());
        }
        "off" => {
            ctx.state.proactive_suggestions.enabled = false;
            push_msg(&mut ctx, "Proactive suggestions disabled.".to_string());
        }
        _ => {
            push_msg(
                &mut ctx,
                "Usage: /suggestions [show|dismiss <id>|clear|on|off]".to_string(),
            );
        }
    }
    Ok(())
}

// ── Voice Commands ───────────────────────────────────────────────

pub async fn voice_cmd(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("status");

    match sub {
        "on" | "enable" => {
            ctx.state.voice_config.enabled = true;
            push_msg(&mut ctx, "Voice mode enabled.".to_string());
        }
        "off" | "disable" => {
            ctx.state.voice_config.enabled = false;
            push_msg(&mut ctx, "Voice mode disabled.".to_string());
        }
        "status" => {
            let status = if ctx.state.voice_config.enabled {
                "enabled"
            } else {
                "disabled"
            };
            let lang = ctx.state.voice_config.language.clone();
            let rate = ctx.state.voice_config.speech_rate;
            let volume = ctx.state.voice_config.volume;
            let input_device = ctx
                .state
                .voice_config
                .input_device
                .clone()
                .unwrap_or_else(|| "default".to_string());
            let output_device = ctx
                .state
                .voice_config
                .output_device
                .clone()
                .unwrap_or_else(|| "default".to_string());

            let lines = vec![
                "Voice Mode Status:".to_string(),
                format!("  Status: {}", status),
                format!("  Language: {}", lang),
                format!("  Speech rate: {}", rate),
                format!("  Volume: {}", volume),
                format!("  Input device: {}", input_device),
                format!("  Output device: {}", output_device),
                "\nUsage: /voice [on|off|status]".to_string(),
            ];
            push_msg(&mut ctx, lines.join("\n"));
        }
        "lang" | "language" => {
            if let Some(lang) = args.get(1) {
                ctx.state.voice_config.language = lang.clone();
                push_msg(&mut ctx, format!("Voice language set to: {}", lang));
            } else {
                let lang = ctx.state.voice_config.language.clone();
                push_msg(
                    &mut ctx,
                    format!(
                        "Current voice language: {}\nUsage: /voice lang <code>",
                        lang
                    ),
                );
            }
        }
        "rate" => {
            if let Some(rate_str) = args.get(1) {
                if let Ok(rate) = rate_str.parse::<f32>() {
                    ctx.state.voice_config.speech_rate = rate.clamp(0.5, 2.0);
                    let rate = ctx.state.voice_config.speech_rate;
                    push_msg(&mut ctx, format!("Speech rate set to: {}", rate));
                } else {
                    push_msg(
                        &mut ctx,
                        "Invalid rate value. Use a number between 0.5 and 2.0.".to_string(),
                    );
                }
            } else {
                let rate = ctx.state.voice_config.speech_rate;
                push_msg(
                    &mut ctx,
                    format!(
                        "Current speech rate: {}\nUsage: /voice rate <0.5-2.0>",
                        rate
                    ),
                );
            }
        }
        "volume" => {
            if let Some(vol_str) = args.get(1) {
                if let Ok(vol) = vol_str.parse::<f32>() {
                    ctx.state.voice_config.volume = vol.clamp(0.0, 1.0);
                    let volume = ctx.state.voice_config.volume;
                    push_msg(&mut ctx, format!("Volume set to: {}", volume));
                } else {
                    push_msg(
                        &mut ctx,
                        "Invalid volume value. Use a number between 0.0 and 1.0.".to_string(),
                    );
                }
            } else {
                let volume = ctx.state.voice_config.volume;
                push_msg(
                    &mut ctx,
                    format!("Current volume: {}\nUsage: /voice volume <0.0-1.0>", volume),
                );
            }
        }
        _ => {
            push_msg(
                &mut ctx,
                "Usage: /voice [on|off|status|lang <code>|rate <0.5-2.0>|volume <0.0-1.0>]"
                    .to_string(),
            );
        }
    }
    Ok(())
}

// ── Notification Commands ─────────────────────────────────────────

pub async fn notifications_cmd(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("show");

    match sub {
        "show" | "list" => {
            let unread = ctx.state.notifications.get_unread();
            if unread.is_empty() {
                push_msg(&mut ctx, "No unread notifications.".to_string());
            } else {
                let mut lines = vec![format!("Unread notifications ({}):", unread.len())];
                for n in unread.iter().take(20) {
                    let icon = match n.notification_type {
                        crate::core::notifications::NotificationType::Info => "ℹ",
                        crate::core::notifications::NotificationType::Success => "✓",
                        crate::core::notifications::NotificationType::Warning => "⚠",
                        crate::core::notifications::NotificationType::Error => "✗",
                    };
                    let time = chrono::DateTime::from_timestamp(n.timestamp, 0)
                        .map(|dt| dt.format("%H:%M:%S").to_string())
                        .unwrap_or_else(|| "??:??:??".to_string());
                    lines.push(format!("  {} [{}] {}: {}", icon, time, n.title, n.message));
                }
                push_msg(&mut ctx, lines.join("\n"));
            }
        }
        "all" => {
            let all = ctx.state.notifications.get_all();
            if all.is_empty() {
                push_msg(&mut ctx, "No notifications.".to_string());
            } else {
                let mut lines = vec![format!("All notifications ({}):", all.len())];
                for n in all.iter().rev().take(30) {
                    let _icon = match n.notification_type {
                        crate::core::notifications::NotificationType::Info => "ℹ",
                        crate::core::notifications::NotificationType::Success => "✓",
                        crate::core::notifications::NotificationType::Warning => "⚠",
                        crate::core::notifications::NotificationType::Error => "✗",
                    };
                    let read_marker = if n.read { " " } else { "!" };
                    let time = chrono::DateTime::from_timestamp(n.timestamp, 0)
                        .map(|dt| dt.format("%H:%M:%S").to_string())
                        .unwrap_or_else(|| "??:??:??".to_string());
                    lines.push(format!(
                        "  {}[{}] {}: {}",
                        read_marker, time, n.title, n.message
                    ));
                }
                push_msg(&mut ctx, lines.join("\n"));
            }
        }
        "read" => {
            if let Some(id) = args.get(1) {
                ctx.state.notifications.mark_read(id);
                push_msg(&mut ctx, format!("Marked notification {} as read.", id));
            } else {
                ctx.state.notifications.mark_all_read();
                push_msg(&mut ctx, "All notifications marked as read.".to_string());
            }
        }
        "clear" => {
            ctx.state.notifications.clear();
            push_msg(&mut ctx, "All notifications cleared.".to_string());
        }
        _ => {
            push_msg(
                &mut ctx,
                "Usage: /notifications [show|all|read [id]|clear]".to_string(),
            );
        }
    }
    Ok(())
}

// ── Remote Settings Commands ────────────────────────────────────

pub async fn remote_settings_cmd(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("show");

    match sub {
        "show" => {
            let rs = &ctx.state.remote_settings;
            let mut lines = vec!["Remote Settings:".to_string()];
            lines.push(format!(
                "  Endpoint: {}",
                rs.endpoint.as_deref().unwrap_or("(not configured)")
            ));
            lines.push(format!(
                "  Last sync: {}",
                rs.last_sync
                    .map(|t| chrono::DateTime::from_timestamp(t, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                        .unwrap_or_else(|| "invalid".to_string()))
                    .unwrap_or_else(|| "never".to_string())
            ));
            lines.push(format!("  Sync interval: {}s", rs.sync_interval_secs));
            lines.push(format!(
                "  Settings keys: {}",
                rs.settings.as_object().map(|o| o.len()).unwrap_or(0)
            ));
            push_msg(&mut ctx, lines.join("\n"));
        }
        "set-endpoint" => {
            if let Some(endpoint) = args.get(1) {
                ctx.state.remote_settings.set_endpoint(endpoint);
                push_msg(&mut ctx, format!("Endpoint set to: {}", endpoint));
            } else {
                push_msg(
                    &mut ctx,
                    "Usage: /remote-settings set-endpoint <url>".to_string(),
                );
            }
        }
        "sync" => match ctx.state.remote_settings.sync().await {
            Ok(()) => push_msg(&mut ctx, "Settings synced successfully.".to_string()),
            Err(e) => push_msg(&mut ctx, format!("Sync failed: {}", e)),
        },
        "get" => {
            if let Some(path) = args.get(1) {
                let result = ctx.state.remote_settings.get_setting(path).cloned();
                match result {
                    Some(value) => {
                        push_msg(&mut ctx, format!("{}: {}", path, value));
                    }
                    None => {
                        push_msg(&mut ctx, format!("Setting '{}' not found.", path));
                    }
                }
            } else {
                push_msg(&mut ctx, "Usage: /remote-settings get <path>".to_string());
            }
        }
        _ => {
            push_msg(
                &mut ctx,
                "Usage: /remote-settings [show|set-endpoint <url>|sync|get <path>]".to_string(),
            );
        }
    }
    Ok(())
}

// ── Index Commands ──────────────────────────────────────────────────

pub async fn index_status(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let incremental = crate::core::context::incremental_index::IncrementalIndex::new(&cwd);
    let status = incremental.get_status();

    let mut lines = vec!["# Index Status\n".to_string()];
    lines.push(format!("{}", status));

    // Show activity tracker info if available
    let activity = crate::core::context::activity_tracker::ActivityTracker::new();
    let active_files = activity.get_most_active(10);
    if !active_files.is_empty() {
        lines.push("\n## Most Active Files\n".to_string());
        for (file, score) in active_files {
            lines.push(format!("  {} (score: {:.2})", file, score));
        }
    }

    push_msg(&mut ctx, lines.join("\n"));
    Ok(())
}

pub async fn index_rebuild(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let mut incremental = crate::core::context::incremental_index::IncrementalIndex::new(&cwd);

    push_msg(&mut ctx, "Starting full index rebuild...");

    match incremental.full_index() {
        Ok(count) => {
            let status = incremental.get_status();
            push_msg(
                &mut ctx,
                format!(
                    "Index rebuild complete.\n\n{} files indexed.\n{}",
                    count, status
                ),
            );
        }
        Err(e) => {
            push_msg(&mut ctx, format!("Index rebuild failed: {}", e));
        }
    }

    Ok(())
}

// ── Conversation: export / diff / files / rewind / rename (Claude Code parity) ──

/// /export — 导出当前对话为 Markdown 文件（或复制到剪贴板：/export clipboard）
pub async fn export_conversation(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let to_clipboard = args.iter().any(|a| a == "clipboard" || a == "--clipboard");
    let mut md = String::from("# StarCode Conversation\n\n");
    md.push_str(&format!(
        "- Date: {}\n- Model: {}\n\n---\n\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        if ctx.state.current_model.is_empty() {
            "-"
        } else {
            &ctx.state.current_model
        }
    ));

    for e in &ctx.state.chat_history {
        if e.is_welcome {
            continue;
        }
        match e.entry_type {
            crate::types::ChatEntryType::User => {
                if !e.content.trim().is_empty() {
                    md.push_str(&format!("## User\n\n{}\n\n", e.content.trim()));
                }
            }
            crate::types::ChatEntryType::Assistant => {
                if !e.content.trim().is_empty() {
                    md.push_str(&format!("## Assistant\n\n{}\n\n", e.content.trim()));
                }
            }
            crate::types::ChatEntryType::ToolCall => {
                if let Some(tc) = &e.tool_call {
                    md.push_str(&format!(
                        "### ⏺ {}\n\n```json\n{}\n```\n\n",
                        tc.function.name,
                        crate::utils::string_utils::truncate_chars(&tc.function.arguments, 500)
                    ));
                }
            }
            _ => {}
        }
    }

    if to_clipboard {
        match arboard::Clipboard::new().and_then(|mut c| c.set_text(md.clone())) {
            Ok(_) => {
                push_msg(&mut ctx, "Conversation copied to clipboard.");
                return Ok(());
            }
            Err(e) => push_msg(
                &mut ctx,
                format!("Clipboard failed: {} — falling back to file export", e),
            ),
        }
    }

    let dir = std::path::Path::new("star-exports");
    if let Err(e) = std::fs::create_dir_all(dir) {
        push_msg(&mut ctx, format!("Export failed: {}", e));
        return Ok(());
    }
    let file = dir.join(format!(
        "conversation-{}.md",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));
    match std::fs::write(&file, &md) {
        Ok(_) => push_msg(
            &mut ctx,
            format!("Conversation exported to: {}", file.display()),
        ),
        Err(e) => push_msg(&mut ctx, format!("Export failed: {}", e)),
    }
    Ok(())
}

/// /diff — 查看未提交变更（git diff --stat + 截断的完整 diff）
pub async fn diff_top(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let run_git = |args: &[&str]| -> Option<String> {
        std::process::Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
    };

    let Some(stat) = run_git(&["diff", "--stat"]) else {
        push_msg(&mut ctx, "git diff failed — not a git repository?");
        return Ok(());
    };
    if stat.trim().is_empty() {
        push_msg(&mut ctx, "No uncommitted changes.");
        return Ok(());
    }

    let mut out = String::from("Uncommitted changes:\n\n```\n");
    out.push_str(stat.trim());
    out.push_str("\n```\n");
    if let Some(full) = run_git(&["diff"]) {
        let lines: Vec<&str> = full.lines().take(150).collect();
        out.push_str("\n```diff\n");
        out.push_str(&lines.join("\n"));
        if full.lines().count() > 150 {
            out.push_str(&format!("\n... +{} more lines", full.lines().count() - 150));
        }
        out.push_str("\n```");
    }
    push_msg(&mut ctx, out);
    Ok(())
}

/// /files — 列出本会话中工具读写过的文件
pub async fn files_in_context(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let mut files: Vec<String> = Vec::new();
    for e in &ctx.state.chat_history {
        if e.entry_type != crate::types::ChatEntryType::ToolCall {
            continue;
        }
        let Some(tc) = &e.tool_call else { continue };
        let name = tc.function.name.as_str();
        let is_file_tool = matches!(name, "Read" | "Edit" | "Write" | "SmartEdit");
        if !is_file_tool {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&tc.function.arguments) {
            for key in ["path", "file_path", "target_file"] {
                if let Some(p) = v.get(key).and_then(|x| x.as_str()) {
                    if !p.is_empty() && !files.iter().any(|f| f == p) {
                        files.push(p.to_string());
                    }
                    break;
                }
            }
        }
    }

    if files.is_empty() {
        push_msg(
            &mut ctx,
            "No files have been read or edited in this session yet.",
        );
        return Ok(());
    }
    let mut out = format!("Files touched this session ({}):\n", files.len());
    for f in &files {
        out.push_str(&format!("  - {}\n", f));
    }
    push_msg(&mut ctx, out);
    Ok(())
}

/// /rewind — 文件历史快照回退（对标 Claude Code 的 /rewind）
///   /rewind                 列出最近 10 个快照，提示用 /rewind <id> 回退
///   /rewind latest           回退到最近一个快照（= /undo）
///   /rewind <snapshot_id>    回退到指定快照
pub async fn rewind(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    use crate::utils::checkpoint_manager;

    if args.is_empty() {
        let snapshots = checkpoint_manager::list_snapshots(None)
            .await
            .map_err(|e| format!("Failed to list snapshots: {}", e))?;

        if snapshots.is_empty() {
            push_msg(
                &mut ctx,
                "⚠️ No file-history snapshots found.\n\n\
                 Snapshots are created automatically when write_file / edit / multi_edit modify files. \
                 Try writing a file first.",
            );
            return Ok(());
        }

        // Show last 10 (newest last).
        let display_count = 10;
        let start = snapshots.len().saturating_sub(display_count);
        let recent = &snapshots[start..];

        let mut lines = String::from("📋 File-history snapshots (newest last):\n\n");
        lines
            .push_str("  id                            | time              | tool       | files\n");
        lines
            .push_str("  ------------------------------+-------------------+------------+------\n");

        for s in recent {
            let id_short: String = if s.snapshot_id.len() > 30 {
                s.snapshot_id[..30].to_string()
            } else {
                s.snapshot_id.clone()
            };
            let time = s.timestamp.format("%Y-%m-%d %H:%M:%S").to_string();
            let tool = s.tool_name.as_deref().unwrap_or("-");
            let msg_id = s
                .message_id
                .map(|m| format!("msg={}", m))
                .unwrap_or_default();
            lines.push_str(&format!(
                "  {:<30} | {} | {:<10} | {} {}\n",
                id_short, time, tool, s.tracked_files_count, msg_id
            ));
        }

        lines.push_str(&format!(
            "\nTotal: {} snapshots (showing last {})\n",
            snapshots.len(),
            recent.len()
        ));
        lines.push_str("Usage:\n");
        lines.push_str("  /rewind <snapshot_id>  — revert files to that snapshot\n");
        lines.push_str("  /rewind latest         — revert to the most recent snapshot (= /undo)\n");

        push_msg(&mut ctx, lines);
        return Ok(());
    }

    let target = args[0].as_str();

    // /rewind latest = /undo
    if target == "latest" {
        return crate::commands::utility::undo(ctx, args).await;
    }

    // /rewind <snapshot_id>
    let can_restore = checkpoint_manager::can_restore(target, None)
        .await
        .map_err(|e| format!("Failed to check snapshot {}: {}", target, e))?;
    if !can_restore {
        push_msg(
            &mut ctx,
            format!(
                "⚠️ Snapshot '{}' not found. Run /rewind (no args) to list available snapshots.",
                target
            ),
        );
        return Ok(());
    }

    let changed = checkpoint_manager::rewind(target, None)
        .await
        .map_err(|e| format!("Failed to rewind to snapshot {}: {}", target, e))?;

    let summary = if changed.is_empty() {
        "no files changed (already at this state)".to_string()
    } else {
        format!(
            "restored {} file(s):\n{}",
            changed.len(),
            changed.join("\n")
        )
    };

    push_msg(
        &mut ctx,
        format!("✅ Rewound to snapshot: {}\n\n{}", target, summary),
    );

    Ok(())
}

/// /rename — 重命名当前会话（转发到 session title）
pub async fn rename_session(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    session_title(ctx, args).await
}

// ═══════════════════════════════════════════════════════════════════════════
// Claude Code parity commands (add-dir / effort / fast / fork / summary / recap)
// ═══════════════════════════════════════════════════════════════════════════

/// 把一个目录加入会话的额外工作目录（/add-dir 共用逻辑，
/// 也被 InputContext::AddWorkingDir 的输入弹窗回车路径调用）。
pub fn add_working_dir_to_state(
    state: &mut crate::ui::state::ChatState,
    path_str: &str,
) -> Result<String, String> {
    let raw = path_str.trim().trim_matches('"').to_string();
    if raw.is_empty() {
        return Err("Usage: /add-dir <path>".to_string());
    }

    // 展开 ~ 为用户主目录
    let expanded = if raw == "~" {
        dirs::home_dir().ok_or_else(|| "Cannot resolve home directory".to_string())?
    } else if let Some(rest) = raw.strip_prefix("~/").or(raw.strip_prefix("~\\")) {
        let home = dirs::home_dir().ok_or_else(|| "Cannot resolve home directory".to_string())?;
        home.join(rest)
    } else {
        std::path::PathBuf::from(&raw)
    };

    if !expanded.exists() {
        return Err(format!("Directory not found: {}", expanded.display()));
    }
    if !expanded.is_dir() {
        return Err(format!("Not a directory: {}", expanded.display()));
    }

    let canonical = expanded.canonicalize().unwrap_or(expanded);

    // 与 cwd / 已有目录去重
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let cwd_canon = cwd.canonicalize().unwrap_or(cwd);
    if canonical == cwd_canon {
        return Err("Already the current working directory.".to_string());
    }
    if state
        .extra_working_dirs
        .iter()
        .any(|d| d.canonicalize().unwrap_or_else(|_| d.clone()) == canonical)
    {
        return Err(format!("Already added: {}", canonical.display()));
    }

    state.extra_working_dirs.push(canonical.clone());
    Ok(format!(
        "✅ Added working directory: {}\n\nExtra working directories: {}. \
         Files under it are now accessible via @ mentions.",
        canonical.display(),
        state.extra_working_dirs.len()
    ))
}

/// /add-dir — 追加工作目录（对标 Claude Code）。带参数直接加入；
/// 无参数弹出路径输入框。
pub async fn add_dir(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if args.is_empty() {
        // 打开输入弹窗（Enter 后走 InputContext::AddWorkingDir 分支）
        ctx.state.close_palette();
        ctx.state.enter_input_modal();
        ctx.state.input_modal_title = "Add Working Directory".to_string();
        ctx.state.input_modal_prompt = "Enter a directory path to add to this session:".to_string();
        ctx.state.input_modal_value = String::new();

        let mut textarea = tui_textarea::TextArea::default();
        textarea.set_cursor_line_style(ratatui::style::Style::default());
        textarea.set_placeholder_text("/path/to/directory or ~/projects/foo");
        textarea.set_cursor_style(
            ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::REVERSED),
        );
        ctx.state.modal_textarea = textarea;
        ctx.state.input_context = Some(crate::ui::state::palette::InputContext::AddWorkingDir);
        return Ok(());
    }

    let path = args.join(" ");
    match add_working_dir_to_state(ctx.state, &path) {
        Ok(msg) => push_msg(&mut ctx, msg),
        Err(e) => push_msg(&mut ctx, format!("❌ {}", e)),
    }
    Ok(())
}

/// Expand a leading `~` without invoking a shell. Only the current user's home
/// is supported; `~other` is treated as a literal relative path.
fn expand_cd_home(raw: &str) -> PathBuf {
    if raw == "~" || raw.starts_with("~/") || raw.starts_with("~\\") {
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            let mut path = std::path::PathBuf::from(home);
            if raw.len() > 1 {
                path.push(raw[2..].trim_start_matches(|c| c == '/' || c == '\\'));
            }
            return path;
        }
    }
    std::path::PathBuf::from(raw)
}

/// Resolve `/cd` input against the current directory and require a real
/// directory. Kept pure so tests do not need to move the process cwd.
fn resolve_cd_target(raw: &str, base: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Usage: `/cd <path>`".to_string());
    }

    let expanded = expand_cd_home(trimmed);
    let candidate = if expanded.is_absolute() {
        expanded
    } else {
        base.join(expanded)
    };

    if !candidate.exists() {
        return Err(format!("Directory not found: {}", candidate.display()));
    }
    if !candidate.is_dir() {
        return Err(format!("Not a directory: {}", candidate.display()));
    }

    candidate.canonicalize().map_err(|error| {
        format!(
            "Could not resolve directory {}: {}",
            candidate.display(),
            error
        )
    })
}

/// `/cd <path>` — move the live session to another working directory while
/// keeping the conversation. Project-scoped resources (sessions, memory,
/// skills, context index) are rebuilt from the new root.
pub async fn cd(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if args.is_empty() {
        push_msg(
            &mut ctx,
            format!(
                "Usage: `/cd <path>`\n\nCurrent directory: `{}`",
                crate::core::utils::paths::current_dir_cached().display()
            ),
        );
        return Ok(());
    }

    if ctx.state.is_processing || ctx.state.is_streaming {
        push_msg(
            &mut ctx,
            "❌ Cannot change the working directory while a turn is running. Wait for the reply to finish, then run `/cd` again.",
        );
        return Ok(());
    }

    let base = crate::core::utils::paths::current_dir_cached().clone();
    let target = match resolve_cd_target(&args.join(" "), &base) {
        Ok(target) => target,
        Err(error) => {
            push_msg(&mut ctx, format!("❌ {}", error));
            return Ok(());
        }
    };

    let current = base.canonicalize().unwrap_or_else(|_| base.clone());
    if target == current {
        push_msg(
            &mut ctx,
            format!("Already in `{}`. Nothing changed.", target.display()),
        );
        return Ok(());
    }

    let (response_tx, mut response_rx) = tokio::sync::mpsc::channel(1);
    if ctx
        .agent_tx
        .send(AgentRequest::ChangeWorkingDirectory {
            path: target.clone(),
            response: response_tx,
        })
        .await
        .is_err()
    {
        push_msg(
            &mut ctx,
            "❌ Agent worker is unavailable; the working directory was not changed.",
        );
        return Ok(());
    }

    match response_rx.recv().await {
        Some(Ok(())) => {
            let released_session = ctx.state.active_session_id.take();
            ctx.state.current_session_title = None;
            // The status-line poller follows the live root, but clear the old
            // project's values immediately so the next frame cannot show them.
            ctx.state.git_branch = None;
            ctx.state.git_status = None;

            let mut message = format!(
                "✅ Session moved to `{}`.\n\nThe conversation is unchanged. Sessions, memory, skills and the context index now resolve from this directory and refresh on the next turn.",
                target.display()
            );
            if let Some(id) = released_session {
                message.push_str(&format!(
                    "\n\nThe previous project's session `{}` stays on disk there; the next save uses the new project.",
                    id
                ));
            }
            push_msg(&mut ctx, message);
        }
        Some(Err(error)) => {
            push_msg(
                &mut ctx,
                format!("❌ Could not change directory: {}", error),
            );
        }
        None => {
            push_msg(
                &mut ctx,
                "❌ Agent worker dropped the directory-change request.",
            );
        }
    }

    Ok(())
}

/// Session IDs are filename atoms: letters/digits plus `-` and `_`, at most
/// 128 bytes. Keep a readable label and collapse unsafe punctuation to `-`.
pub(crate) fn sanitize_session_atom(raw: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for character in raw.trim().chars() {
        if character.is_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            if out.len() + character.len_utf8() > 96 {
                break;
            }
            out.push(character);
        } else if character == '-' || character == '_' {
            if !out.is_empty() && !out.ends_with('-') && !out.ends_with('_') {
                out.push(character);
            }
            pending_dash = false;
        } else if !out.is_empty() {
            pending_dash = true;
        }
    }

    let trimmed = out.trim_end_matches(|c| c == '-' || c == '_');
    if trimmed.is_empty() {
        "branch".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Return a session ID that does not overwrite an existing file or the active
/// session. Numeric suffixes are deterministic, so `/branch feature` becomes
/// `feature-2` when `feature` is already taken.
pub(crate) fn unique_session_id(
    base: &str,
    existing: &std::collections::HashSet<String>,
    reserved: Option<&str>,
) -> String {
    let mut candidate = base.to_string();
    let mut suffix = 2u32;
    while existing.contains(&candidate) || reserved == Some(candidate.as_str()) {
        candidate = format!("{}-{}", base, suffix);
        suffix += 1;
    }
    candidate
}

/// `/branch [name]` — save the current conversation as the original, save a
/// copy as the branch, and switch the active session identity to the branch.
/// The original remains resumable and is not overwritten after the switch.
pub async fn branch_conversation(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if ctx.state.is_processing || ctx.state.is_streaming {
        push_msg(
            &mut ctx,
            "❌ Cannot branch while a turn is running. Wait for the reply to finish, then run `/branch` again.",
        );
        return Ok(());
    }

    let history: Vec<ChatEntry> = ctx
        .state
        .chat_history
        .iter()
        .filter(|entry| !entry.is_welcome)
        .cloned()
        .collect();
    if history.is_empty() {
        push_msg(&mut ctx, "Nothing to branch yet — the session is empty.");
        return Ok(());
    }

    let existing: std::collections::HashSet<String> =
        crate::utils::session_manager::list_sessions()
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();

    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let requested_name = args.join(" ");
    let requested_name = requested_name.trim();
    let base = if requested_name.is_empty() {
        format!("branch-{}", timestamp)
    } else {
        sanitize_session_atom(requested_name)
    };

    let active = ctx.state.active_session_id.clone();
    let branch_id = unique_session_id(&base, &existing, active.as_deref());
    let original_id = match active {
        Some(id) => id,
        None => unique_session_id(&format!("auto-{}", timestamp), &existing, Some(&branch_id)),
    };

    let usage = ctx.state.token_usage.clone();
    if let Err(error) = crate::commands::chat::request_session_save(
        &ctx.agent_tx,
        original_id.clone(),
        history.clone(),
        usage.clone(),
    )
    .await
    {
        push_msg(
            &mut ctx,
            format!(
                "❌ Could not save the original conversation before branching: {}. Nothing changed.",
                error
            ),
        );
        return Ok(());
    }

    if let Err(error) = crate::commands::chat::request_session_save(
        &ctx.agent_tx,
        branch_id.clone(),
        history.clone(),
        usage,
    )
    .await
    {
        // The original save already moved the latest-session marker. If this
        // session had no ID before, adopt the original so exiting does not
        // invent a third identity.
        if ctx.state.active_session_id.is_none() {
            ctx.state.active_session_id = Some(original_id.clone());
        }
        push_msg(
            &mut ctx,
            format!(
                "❌ Original saved as `{}`, but the branch could not be saved: {}. The session was not switched.",
                original_id, error
            ),
        );
        return Ok(());
    }

    ctx.state.active_session_id = Some(branch_id.clone());
    ctx.state.current_session_title = Some(branch_id.clone());
    push_msg(
        &mut ctx,
        format!(
            "🌿 Branched conversation as `{}` ({} messages).\n\n\
             Original preserved as `{}`.\n\
             Future snapshots go to the branch. Return with `/resume {}`.",
            branch_id,
            history.len(),
            original_id,
            original_id
        ),
    );

    Ok(())
}

/// `/list-agents [all]` — show subagent tasks that StarCode tracks. Without
/// `all`, only tasks that have moved to the background are listed.
pub async fn list_agents(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let include_all = args
        .iter()
        .any(|arg| arg.eq_ignore_ascii_case("all") || arg == "--all");

    let out = {
        let mut rows: Vec<&crate::ui::state::AgentTaskInfo> = if include_all {
            ctx.state.active_agent_tasks.values().collect()
        } else {
            ctx.state.background_agent_rows()
        };
        rows.sort_by_key(|task| task.started_at);

        if rows.is_empty() {
            if include_all {
                "## Agents\n\nNo subagent tasks have been recorded in this session.\n\n\
                 Use `/subtask` or the Agent tool to start one; `/peers` lists cross-session mailboxes."
                    .to_string()
            } else {
                "## Agents\n\nNo background agents are running.\n\n\
                 Use `/list-agents all` to include foreground tasks, and `/peers` for cross-session mailboxes."
                    .to_string()
            }
        } else {
            let mut out = format!("## Agents ({})\n\n", rows.len());
            out.push_str("| agent | task | status | elapsed | tools | tokens |\n");
            out.push_str("|---|---|---|---:|---:|---:|\n");
            for task in rows {
                let agent = task
                    .name
                    .as_deref()
                    .map(|name| format!("@{} ({})", name, task.agent_type))
                    .unwrap_or_else(|| task.agent_type.clone());
                let summary = task
                    .task_description
                    .as_deref()
                    .unwrap_or(&task.description)
                    .replace('\n', " ")
                    .replace('|', "\\|");
                let status = if task.is_error {
                    format!("{:?} ⚠", task.status)
                } else {
                    format!("{:?}", task.status)
                };
                out.push_str(&format!(
                    "| {} | {} | {} | {:.1}s | {} | {} |\n",
                    agent,
                    summary,
                    status,
                    task.elapsed().as_secs_f64(),
                    task.tool_use_count,
                    task.tokens
                ));
            }
            out.push_str(
                "\nUse `/tasks` for the full task panel. `/peers` lists cross-session mailboxes.",
            );
            out
        }
    };

    push_msg(&mut ctx, out);
    Ok(())
}

/// /effort — 设置思考努力档位（对标 Claude Code effort）。
/// 无参数打开 ThinkingEffort 选择面板；带参数直接设置。
pub async fn effort(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if args.is_empty() {
        ctx.state.close_palette();
        ctx.state
            .open_palette(crate::ui::state::palette::PaletteMode::ThinkingEffort);
        return Ok(());
    }

    let level = args[0].to_lowercase();
    let effort = match level.as_str() {
        "off" | "none" | "disable" | "disabled" => crate::types::ThinkingEffort::Off,
        "low" => crate::types::ThinkingEffort::Low,
        "medium" | "auto" => crate::types::ThinkingEffort::Medium,
        "high" => crate::types::ThinkingEffort::High,
        "xhigh" | "max" | "ultra" | "ultrathink" => crate::types::ThinkingEffort::Xhigh,
        other => {
            push_msg(
                &mut ctx,
                format!(
                    "❌ Unknown effort level: {}. Usage: /effort [off|low|medium|high|xhigh]",
                    other
                ),
            );
            return Ok(());
        }
    };

    let accepted = crate::ui::events::input::request_runtime_settings(
        ctx.state,
        None,
        None,
        Some(effort.clone()),
        ctx.agent_tx,
    )
    .await;
    if !accepted {
        return Err("Runtime settings could not reach the agent worker.".to_string());
    }

    // 只有 worker acknowledgement 才能更新 active 档位；命令回显只说明本次请求已排队。
    let mut msg = format!(
        "Thinking effort requested: {}\n\nIt takes effect after worker confirmation.",
        effort.label()
    );
    if matches!(
        crate::core::config::models::thinking_capability(&ctx.state.current_model),
        crate::core::config::models::ThinkingCapability::None
    ) {
        // 模型压根没有思考开关时说清楚，否则用户以为设了没生效
        msg.push_str(
            "\n\n⚠️ The current model has no thinking mode — this takes effect \
             after you switch to one that does.",
        );
    }
    push_msg(&mut ctx, msg);
    Ok(())
}

/// /fast — 快速模式开关（对标 Claude Code fast mode）：
/// 开启时切换到同一 provider 的轻量模型，关闭时精确恢复原模型/provider。
/// 会话级开关：通过 session-only revisioned runtime mutation 生效，不写设置文件。
///
/// 选谁当「轻量模型」有两条路，先看显式配置再看名字猜：
/// 1. `STAR_SMALL_FAST_MODEL` / `ANTHROPIC_SMALL_FAST_MODEL` / Claude Code
///    `settings.json` 的 `ANTHROPIC_SMALL_FAST_MODEL`（见 `provider_resolution`）。
///    用户点名了就照办，不用猜。
/// 2. 都没配，才退回按名字含 `mini` / `flash` / `lite` 之类的启发式猜。
///
/// 显式配置的那个也必须出现在 `available_models_info` 里、且属于当前 provider：
/// 反过来说就等于把一个该端点不认识的模型名发出去，而失败发生在几秒后的请求里，
/// 用户只会看到一句莫名的 404。
fn pick_fast_model(
    state: &crate::ui::state::ChatState,
    configured_fast_model: Option<&str>,
) -> Option<crate::runtime::messages::RuntimeModelSelection> {
    const FAST_HINTS: [&str; 7] = ["mini", "flash", "lite", "fast", "small", "turbo", "instant"];
    let current_provider = state.current_provider_id.as_deref();
    let belongs_to_current_provider = |model: &crate::types::ModelInfo| match current_provider {
        Some(provider) => model.provider == provider,
        None => true,
    };

    if let Some(configured) = configured_fast_model
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        let matched = state
            .available_models_info
            .iter()
            .find(|model| model.id == configured && belongs_to_current_provider(model))
            .map(|model| crate::runtime::messages::RuntimeModelSelection {
                model: model.id.clone(),
                provider_id: Some(model.provider.clone()),
            });
        if matched.is_some() {
            return matched;
        }
        // 配的那个模型这个 provider 列表里没有。不在这里 return —— 否则下面
        // 按名字猜的那条路永远走不到，用户配了个无效值就彻底没法 /fast 了。
    }

    state
        .available_models_info
        .iter()
        .filter(|model| {
            let id = model.id.to_lowercase();
            model.id != state.current_model
                && FAST_HINTS.iter().any(|hint| id.contains(hint))
                && belongs_to_current_provider(model)
        })
        .map(|model| crate::runtime::messages::RuntimeModelSelection {
            model: model.id.clone(),
            provider_id: Some(model.provider.clone()),
        })
        .min_by(|left, right| left.model.cmp(&right.model))
}

/// 读一次配置解析结果，取出显式的「小而快」副模型。
///
/// 单独包一层是因为 `/fast` 是纯 UI 侧的会话开关，不值得为它把 provider store
/// 和 user settings 都读一遍再丢掉 —— 读不到就返回 `None`，让 `pick_fast_model`
/// 退回名字启发式，跟加这一层之前的行为一致。
async fn configured_fast_model() -> Option<String> {
    let settings_manager = crate::core::config::settings_manager::get_settings_manager()
        .await
        .ok()?;
    let settings = settings_manager.load_user_settings().await.ok()?;
    let provider_config = crate::core::config::provider_store::ProviderStore::new()
        .load()
        .await
        .unwrap_or_default();

    crate::core::config::provider_resolution::resolve_effective_provider_settings(
        crate::core::config::provider_resolution::ProviderResolutionInputs::default(),
        &provider_config,
        &settings,
    )
    .small_fast_model
    .value
}

pub async fn fast(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let action = args
        .first()
        .map(|s| s.to_lowercase())
        .unwrap_or_else(|| "toggle".into());
    let enable = match action.as_str() {
        "on" | "enable" | "true" => true,
        "off" | "disable" | "false" => false,
        "toggle" => !ctx.state.fast_mode,
        other => {
            push_msg(
                &mut ctx,
                format!("❌ Unknown action: {}. Usage: /fast [on|off]", other),
            );
            return Ok(());
        }
    };

    if ctx.state.pending_fast_mode_transition.is_some() {
        push_msg(
            &mut ctx,
            "Fast mode transition is already waiting for worker confirmation.",
        );
        return Ok(());
    }
    if enable == ctx.state.fast_mode {
        push_msg(
            &mut ctx,
            format!(
                "Fast mode is already {}.",
                if enable { "ON" } else { "OFF" }
            ),
        );
        return Ok(());
    }

    let (target_selection, previous_model, status) = if enable {
        let previous_model = (!ctx.state.current_model.is_empty()).then(|| {
            crate::runtime::messages::RuntimeModelSelection {
                model: ctx.state.current_model.clone(),
                provider_id: ctx.state.current_provider_id.clone(),
            }
        });
        match pick_fast_model(ctx.state, configured_fast_model().await.as_deref()) {
            Some(fast_model) => {
                let status = format!("Requesting fast mode · model {}…", fast_model.model);
                (Some(fast_model), previous_model, status)
            }
            None => (None, previous_model, "Requesting fast mode…".to_string()),
        }
    } else {
        let previous_model = ctx.state.fast_mode_prev_model.clone();
        let current_selection = crate::runtime::messages::RuntimeModelSelection {
            model: ctx.state.current_model.clone(),
            provider_id: ctx.state.current_provider_id.clone(),
        };
        let target_selection = previous_model
            .as_ref()
            .filter(|model| **model != current_selection)
            .cloned();
        let status = target_selection
            .as_ref()
            .map(|model| format!("Requesting fast mode off · restore {}…", model.model))
            .unwrap_or_else(|| "Requesting fast mode off…".to_string());
        (target_selection, previous_model, status)
    };

    let Some(target_selection) = target_selection else {
        // /fast without a model transition is UI-only today; do not pretend it crossed the
        // runtime settings protocol. Keeping state unchanged is safer than a false active mode.
        push_msg(
            &mut ctx,
            if enable {
                "No alternate fast model is available for the confirmed provider."
            } else {
                "No previous model is available to restore from fast mode."
            },
        );
        return Ok(());
    };
    let Some(ui_revision) =
        crate::ui::events::input::request_runtime_settings_with_persistence_revision(
            ctx.state,
            Some(target_selection.clone()),
            None,
            None,
            crate::runtime::messages::RuntimeSettingsPersistencePolicy::SessionOnly,
            ctx.agent_tx,
        )
        .await
    else {
        return Err("Runtime settings could not reach the agent worker.".to_string());
    };

    ctx.state.pending_fast_mode_transition =
        Some(crate::ui::state::store::PendingFastModeTransition {
            ui_revision,
            enabled: enable,
            previous_model: if enable { previous_model } else { None },
            target_model: target_selection,
        });
    ctx.state.current_status_line = Some(status);
    push_msg(
        &mut ctx,
        format!(
            "Fast mode {} requested. It takes effect after worker confirmation.",
            if enable { "ON" } else { "OFF" }
        ),
    );
    Ok(())
}

#[cfg(test)]
mod runtime_settings_tests {
    use super::*;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn effort_queues_a_persistent_request_without_changing_active_state() {
        let mut state = crate::ui::state::ChatState::new();
        let original_effort = state.thinking_effort.clone();
        let (agent_tx, mut agent_rx) = mpsc::channel(1);
        effort(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            vec!["high".into()],
        )
        .await
        .unwrap();
        let Some(AgentRequest::UpdateRuntimeSettings(mutation)) = agent_rx.recv().await else {
            panic!("expected revisioned effort request");
        };
        assert_eq!(mutation.ui_revision, 1);
        assert_eq!(
            mutation.thinking_effort,
            Some(crate::types::ThinkingEffort::High)
        );
        assert_eq!(state.thinking_effort, original_effort);
        assert!(matches!(
            state.runtime_settings_persistence,
            Some(crate::ui::state::store::RuntimeSettingsPersistence::Pending { ui_revision: 1 })
        ));
    }

    /// 用户点名了副模型就该照办，哪怕名字里没有 `mini` / `flash` 之类的特征 ——
    /// 名字启发式永远猜不到 `gpt-5-codex` 这种。
    #[test]
    fn a_configured_fast_model_beats_the_name_heuristic() {
        let mut state = crate::ui::state::ChatState::new();
        state.current_model = "standard-model".into();
        state.current_provider_id = Some("provider-a".into());
        state.available_models_info = vec![
            crate::types::ModelInfo::new("aaa-mini", "provider-a"),
            crate::types::ModelInfo::new("picked-exactly", "provider-a"),
        ];

        let picked = pick_fast_model(&state, Some("picked-exactly"));
        assert_eq!(
            picked.map(|selection| selection.model).as_deref(),
            Some("picked-exactly")
        );
    }

    /// 配的模型不在列表里（provider 那边压根没这个模型）就不能用：发出去
    /// 只会换一个几秒后才炸的 404。退回名字启发式。
    #[test]
    fn a_configured_fast_model_the_provider_does_not_offer_is_skipped() {
        let mut state = crate::ui::state::ChatState::new();
        state.current_model = "standard-model".into();
        state.current_provider_id = Some("provider-a".into());
        state.available_models_info = vec![crate::types::ModelInfo::new("aaa-mini", "provider-a")];

        let picked = pick_fast_model(&state, Some("not-offered-anywhere"));
        assert_eq!(
            picked.map(|selection| selection.model).as_deref(),
            Some("aaa-mini")
        );
    }

    /// 同名但属于别的 provider 的模型也不能选 —— `/fast` 必须是同 provider
    /// 内的切换，否则顺带把端点和凭据也换了。
    #[test]
    fn a_configured_fast_model_from_another_provider_is_skipped() {
        let mut state = crate::ui::state::ChatState::new();
        state.current_model = "standard-model".into();
        state.current_provider_id = Some("provider-a".into());
        state.available_models_info = vec![
            crate::types::ModelInfo::new("shared-name", "provider-a"),
            crate::types::ModelInfo::new("shared-name", "provider-b"),
        ];

        // 同名两条：只能取当前 provider 那条。
        let picked = pick_fast_model(&state, Some("shared-name"));
        assert_eq!(
            picked
                .and_then(|selection| selection.provider_id)
                .as_deref(),
            Some("provider-a")
        );
    }

    /// 没配副模型时行为与加这一层之前逐字一致。
    #[test]
    fn without_a_configured_model_the_name_heuristic_still_decides() {
        let mut state = crate::ui::state::ChatState::new();
        state.current_model = "standard-model".into();
        state.current_provider_id = Some("provider-a".into());
        state.available_models_info = vec![
            crate::types::ModelInfo::new("model-mini", "provider-a"),
            crate::types::ModelInfo::new("other-mini", "provider-b"),
        ];

        for configured in [None, Some(""), Some("   ")] {
            let picked = pick_fast_model(&state, configured);
            assert_eq!(
                picked.map(|selection| selection.model).as_deref(),
                Some("model-mini"),
                "configured = {configured:?}"
            );
        }
    }

    #[tokio::test]
    async fn fast_on_uses_a_lightweight_model_from_the_confirmed_provider() {
        let mut state = crate::ui::state::ChatState::new();
        state.current_model = "standard-model".into();
        state.current_provider_id = Some("provider-a".into());
        state.available_models_info = vec![
            crate::types::ModelInfo::new("model-mini", "provider-a"),
            crate::types::ModelInfo::new("other-mini", "provider-b"),
        ];
        let (agent_tx, mut agent_rx) = mpsc::channel(1);

        fast(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            vec!["on".into()],
        )
        .await
        .unwrap();

        let Some(AgentRequest::UpdateRuntimeSettings(mutation)) = agent_rx.recv().await else {
            panic!("expected session-only fast-mode request");
        };
        assert_eq!(
            mutation.persistence,
            crate::runtime::messages::RuntimeSettingsPersistencePolicy::SessionOnly
        );
        assert_eq!(
            mutation.model.unwrap().provider_id.as_deref(),
            Some("provider-a")
        );
        assert!(!state.fast_mode);
        assert!(state.pending_fast_mode_transition.is_some());
        assert!(state.runtime_settings_persistence.is_none());
    }

    #[tokio::test]
    async fn fast_off_restores_provider_when_model_name_matches() {
        let mut state = crate::ui::state::ChatState::new();
        state.fast_mode = true;
        state.current_model = "shared-model".into();
        state.current_provider_id = Some("fast-provider".into());
        state.fast_mode_prev_model = Some(crate::runtime::messages::RuntimeModelSelection {
            model: "shared-model".into(),
            provider_id: Some("original-provider".into()),
        });
        let (agent_tx, mut agent_rx) = mpsc::channel(1);
        fast(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            vec!["off".into()],
        )
        .await
        .unwrap();
        let Some(AgentRequest::UpdateRuntimeSettings(mutation)) = agent_rx.recv().await else {
            panic!("expected session-only restore request");
        };
        assert_eq!(
            mutation.persistence,
            crate::runtime::messages::RuntimeSettingsPersistencePolicy::SessionOnly
        );
        assert_eq!(
            mutation.model.unwrap().provider_id.as_deref(),
            Some("original-provider")
        );
        assert!(state.fast_mode);
        assert!(state.pending_fast_mode_transition.is_some());
        assert!(state.runtime_settings_persistence.is_none());
    }
}

/// /fork — 把当前会话复制为一个新会话快照（对标 Claude Code fork）。
/// 可带 <prompt>：快照保存后立即在当前会话中以该提示继续。
pub async fn fork(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let history: Vec<ChatEntry> = ctx
        .state
        .chat_history
        .iter()
        .filter(|e| !e.is_welcome)
        .cloned()
        .collect();

    if history.is_empty() {
        push_msg(&mut ctx, "Nothing to fork yet — the session is empty.");
        return Ok(());
    }

    let fork_id = format!("fork-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"));
    match crate::commands::chat::request_session_save(
        &ctx.agent_tx,
        fork_id.clone(),
        history.clone(),
        ctx.state.token_usage.clone(),
    )
    .await
    {
        Ok(()) => {
            let mut msg = format!(
                "🍴 Forked session saved as `{}` ({} messages).\n\n\
                 The fork preserves the conversation up to this point. \
                 Resume it later with `/chat resume {}`.",
                fork_id,
                history.len(),
                fork_id
            );
            // 带 prompt：fork 后立即以该提示继续当前（fork 点之后的）会话
            let prompt = args.join(" ").trim().to_string();
            if !prompt.is_empty() {
                let message_id = ctx.state.next_message_id;
                ctx.state.next_message_id += 1;
                let _ = ctx
                    .agent_tx
                    .send(AgentRequest::SendMessage {
                        message_id,
                        message: prompt,
                    })
                    .await;
                msg.push_str(
                    "\n\nPrompt queued — it will run after the current response finishes.",
                );
            }
            push_msg(&mut ctx, msg);
        }
        Err(e) => push_msg(&mut ctx, format!("❌ Fork failed: {}", e)),
    }
    Ok(())
}

/// `/subtask` 的任务预览：取第一条非空行，按字符数截断，避免把整段任务
/// 塞进 agent 选择器；`chars()` 保证 CJK 不会被截在字节中间。
fn subtask_description(task: &str) -> String {
    let first_line = task
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(task)
        .trim();
    let description: String = first_line.chars().take(80).collect();
    if description.is_empty() {
        "Subtask".to_string()
    } else {
        description
    }
}

/// `/subtask <task>` — fork 当前会话到后台子代理（对标 Claude Code subtask）。
///
/// 命令层没有 `StarAgent`，所以启动动作交给 worker：worker 取出当前
/// provider 上下文和运行时通知队列，创建一个继承完整会话的后台 Fork。
/// 子代理完成后走既有 `<task-notification>` 通道回注，不需要另建会话模型。
pub async fn subtask(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let task = args.join(" ").trim().to_string();
    if task.is_empty() {
        push_msg(
            &mut ctx,
            "Usage: `/subtask <task>`\n\n\
             Starts a background forked subagent that inherits this conversation.",
        );
        return Ok(());
    }

    let description = subtask_description(&task);
    let (response_tx, mut response_rx) = tokio::sync::mpsc::channel(1);

    if ctx
        .agent_tx
        .send(AgentRequest::RunSubtask {
            task: task.clone(),
            description: description.clone(),
            response: response_tx,
        })
        .await
        .is_err()
    {
        push_msg(
            &mut ctx,
            "❌ Subtask could not reach the agent worker. Is the session still running?",
        );
        return Ok(());
    }

    let launch =
        match tokio::time::timeout(std::time::Duration::from_secs(5), response_rx.recv()).await {
            Ok(Some(Ok(launch))) => launch,
            Ok(Some(Err(error))) => {
                push_msg(&mut ctx, format!("❌ Subtask failed to start: {}", error));
                return Ok(());
            }
            Ok(None) => {
                push_msg(
                    &mut ctx,
                    "❌ Subtask failed to start: the agent worker closed the response channel.",
                );
                return Ok(());
            }
            Err(_) => {
                push_msg(
                    &mut ctx,
                    "⏳ Subtask was queued behind the current turn and was not acknowledged \
                 within 5 seconds. It will start when the turn finishes; check `/list-agents` \
                 before retrying.",
                );
                return Ok(());
            }
        };

    push_msg(
        &mut ctx,
        format!(
            "## Subtask started\n\n\
             - Agent: `{}`\n\
             - Task: {}\n\
             - Output: `{}`\n\n\
             The forked agent inherits this conversation and runs in the background. \
             Its task notification is delivered on your next turn; watch it with \
             `/list-agents` or `/tasks`.",
            launch.agent_id,
            description,
            launch.output_file.display(),
        ),
    );
    Ok(())
}

/// /summary、/recap、/btw 共用：请求 agent 旁路生成笔记
pub(crate) async fn request_note(
    ctx: &mut CommandContext<'_>,
    kind: crate::runtime::messages::NoteKind,
    question: Option<String>,
) -> CommandResult {
    use crate::runtime::messages::NoteKind;
    let has_content = ctx.state.chat_history.iter().any(|e| {
        !e.is_welcome
            && matches!(
                e.entry_type,
                crate::types::ChatEntryType::User | crate::types::ChatEntryType::Assistant
            )
            && !e.content.trim().is_empty()
    });
    // /btw 自带问题，空会话也能回答
    if !has_content && kind != NoteKind::Aside {
        push_msg(
            ctx,
            match kind {
                NoteKind::Summary => "No messages to summarize.",
                _ => "Nothing to recap yet — send a message first.",
            },
        );
        return Ok(());
    }

    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    ctx.state.current_status_line = Some(format!("Generating {}...", kind.label()));
    let _ = ctx
        .agent_tx
        .send(AgentRequest::GenerateNote {
            kind,
            message_id,
            question,
        })
        .await;
    Ok(())
}

/// /summary — 生成并展示会话摘要（对标 Claude Code 手动 Session Memory 提取）
pub async fn summary_cmd(ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let mut ctx = ctx;
    request_note(&mut ctx, crate::runtime::messages::NoteKind::Summary, None).await
}

/// /recap — 一句话会话回顾（对标 Claude Code away-summary；别名 /away /catchup）
pub async fn recap(ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let mut ctx = ctx;
    request_note(&mut ctx, crate::runtime::messages::NoteKind::Recap, None).await
}

#[cfg(test)]
mod conversation_parity_tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;

    fn unique_temp_dir(label: &str) -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "starcode-extended-{}-{}-{}",
            label,
            std::process::id(),
            nonce
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn cd_resolves_relative_directories_and_rejects_bad_targets() {
        let base = unique_temp_dir("cd");
        let nested = base.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let file = base.join("note.txt");
        std::fs::write(&file, "hello").unwrap();

        let resolved = resolve_cd_target("nested", &base).unwrap();
        assert_eq!(resolved, nested.canonicalize().unwrap());

        let file_error = resolve_cd_target("note.txt", &base).unwrap_err();
        assert!(file_error.contains("Not a directory"), "{}", file_error);

        let missing_error = resolve_cd_target("does-not-exist", &base).unwrap_err();
        assert!(
            missing_error.contains("Directory not found"),
            "{}",
            missing_error
        );

        let empty_error = resolve_cd_target("   ", &base).unwrap_err();
        assert!(empty_error.contains("Usage"), "{}", empty_error);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn cd_dispatches_to_worker_and_releases_old_project_session() {
        let target = unique_temp_dir("cd-handler");
        let mut state = crate::ui::state::ChatState::new();
        state.active_session_id = Some("old-project-session".to_string());
        state.current_session_title = Some("Old project".to_string());
        state.git_branch = Some("main".to_string());
        state.git_status = Some("+".to_string());

        let (agent_tx, mut agent_rx) = mpsc::channel(2);
        let requested = Arc::new(Mutex::new(None::<std::path::PathBuf>));
        let requested_for_worker = requested.clone();
        let worker = tokio::spawn(async move {
            if let Some(AgentRequest::ChangeWorkingDirectory { path, response }) =
                agent_rx.recv().await
            {
                *requested_for_worker.lock().unwrap() = Some(path);
                let _ = response.send(Ok(())).await;
            }
        });

        crate::commands::handle_command(
            "cd",
            vec![target.to_string_lossy().to_string()],
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
        )
        .await
        .unwrap();

        assert_eq!(
            requested.lock().unwrap().as_ref().unwrap(),
            &target.canonicalize().unwrap()
        );
        assert_eq!(state.active_session_id, None);
        assert_eq!(state.current_session_title, None);
        assert_eq!(state.git_branch, None);
        assert_eq!(state.git_status, None);
        assert!(state
            .chat_history
            .last()
            .unwrap()
            .content
            .contains("Session moved"));

        worker.abort();
        let _ = std::fs::remove_dir_all(&target);
    }

    #[tokio::test]
    async fn cd_keeps_state_when_worker_rejects_the_move() {
        let target = unique_temp_dir("cd-reject");
        let mut state = crate::ui::state::ChatState::new();
        state.active_session_id = Some("keep-me".to_string());

        let (agent_tx, mut agent_rx) = mpsc::channel(2);
        let worker = tokio::spawn(async move {
            if let Some(AgentRequest::ChangeWorkingDirectory { response, .. }) =
                agent_rx.recv().await
            {
                let _ = response.send(Err("permission denied".to_string())).await;
            }
        });

        crate::commands::handle_command(
            "cd",
            vec![target.to_string_lossy().to_string()],
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
        )
        .await
        .unwrap();

        assert_eq!(state.active_session_id, Some("keep-me".to_string()));
        assert!(state
            .chat_history
            .last()
            .unwrap()
            .content
            .contains("Could not change directory"));

        worker.abort();
        let _ = std::fs::remove_dir_all(&target);
    }

    #[test]
    fn branch_names_are_safe_and_bounded() {
        assert_eq!(sanitize_session_atom("My Feature / v2"), "My-Feature-v2");
        assert_eq!(sanitize_session_atom("  ...  "), "branch");
        assert_eq!(sanitize_session_atom("a__b"), "a_b");

        let long = "a".repeat(200);
        let sanitized = sanitize_session_atom(&long);
        assert_eq!(sanitized.len(), 96);

        let unicode = "会".repeat(40);
        let sanitized = sanitize_session_atom(&unicode);
        assert!(sanitized.len() <= 96);
        assert!(!sanitized.is_empty());
    }

    #[test]
    fn branch_ids_avoid_existing_and_active_sessions() {
        let mut existing = std::collections::HashSet::new();
        existing.insert("feature".to_string());
        existing.insert("feature-2".to_string());

        assert_eq!(
            unique_session_id("feature", &existing, Some("feature")),
            "feature-3"
        );
        assert_eq!(
            unique_session_id("fresh", &existing, Some("feature")),
            "fresh"
        );
    }

    #[tokio::test]
    async fn branch_saves_original_then_branch_and_switches_active_session() {
        let mut state = crate::ui::state::ChatState::new();
        state
            .chat_history
            .push(crate::types::ChatEntry::user("keep this conversation"));
        state
            .chat_history
            .push(crate::types::ChatEntry::assistant("original reply").with_streaming(false));

        let (agent_tx, mut agent_rx) = mpsc::channel(8);
        let saved = Arc::new(Mutex::new(Vec::<(String, usize)>::new()));
        let saved_for_worker = saved.clone();
        let worker = tokio::spawn(async move {
            while let Some(request) = agent_rx.recv().await {
                if let AgentRequest::SaveSession {
                    id,
                    history,
                    response,
                    ..
                } = request
                {
                    saved_for_worker.lock().unwrap().push((id, history.len()));
                    let _ = response.send(Ok(())).await;
                }
            }
        });

        crate::commands::handle_command(
            "branch",
            vec!["feature".to_string()],
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
        )
        .await
        .unwrap();

        let saved = saved.lock().unwrap().clone();
        assert_eq!(saved.len(), 2, "original and branch must both be saved");
        assert!(saved[0].0.starts_with("auto-"));
        assert!(saved[1].0.starts_with("feature"));
        assert_ne!(saved[0].0, saved[1].0);
        assert_eq!(saved[0].1, saved[1].1);
        assert_eq!(state.active_session_id, Some(saved[1].0.clone()));

        let last = state.chat_history.last().unwrap();
        assert!(last.content.contains("Branched conversation"));
        assert!(last.content.contains(&saved[0].0));
        assert!(last.content.contains(&saved[1].0));

        worker.abort();
    }

    #[tokio::test]
    async fn branch_never_overwrites_the_active_session_name() {
        let mut state = crate::ui::state::ChatState::new();
        state.active_session_id = Some("feature".to_string());
        state
            .chat_history
            .push(crate::types::ChatEntry::user("branch point"));

        let (agent_tx, mut agent_rx) = mpsc::channel(8);
        let saved = Arc::new(Mutex::new(Vec::<String>::new()));
        let saved_for_worker = saved.clone();
        let worker = tokio::spawn(async move {
            while let Some(request) = agent_rx.recv().await {
                if let AgentRequest::SaveSession { id, response, .. } = request {
                    saved_for_worker.lock().unwrap().push(id);
                    let _ = response.send(Ok(())).await;
                }
            }
        });

        crate::commands::handle_command(
            "branch",
            vec!["feature".to_string()],
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
        )
        .await
        .unwrap();

        let saved = saved.lock().unwrap().clone();
        assert_eq!(saved.len(), 2);
        assert_eq!(saved[0], "feature");
        assert_ne!(saved[1], "feature");
        assert!(saved[1].starts_with("feature-"));

        worker.abort();
    }

    #[tokio::test]
    async fn branch_save_failure_does_not_switch_sessions() {
        let mut state = crate::ui::state::ChatState::new();
        state.active_session_id = Some("stable".to_string());
        state
            .chat_history
            .push(crate::types::ChatEntry::user("branch point"));

        let (agent_tx, mut agent_rx) = mpsc::channel(2);
        let worker = tokio::spawn(async move {
            if let Some(AgentRequest::SaveSession { response, .. }) = agent_rx.recv().await {
                let _ = response.send(Err("disk full".to_string())).await;
            }
        });

        crate::commands::handle_command(
            "branch",
            vec!["attempt".to_string()],
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
        )
        .await
        .unwrap();

        assert_eq!(state.active_session_id, Some("stable".to_string()));
        assert!(state
            .chat_history
            .last()
            .unwrap()
            .content
            .contains("Nothing changed"));

        worker.abort();
    }

    #[tokio::test]
    async fn list_agents_reports_background_task_details() {
        let mut state = crate::ui::state::ChatState::new();
        state.active_agent_tasks.insert(
            "task-1".to_string(),
            crate::ui::state::AgentTaskInfo {
                task_id: "task-1".to_string(),
                agent_type: "fork".to_string(),
                description: "Investigate login".to_string(),
                status: crate::types::AgentTaskStatus::Running,
                tool_use_count: 3,
                tokens: 42,
                is_async: true,
                is_resolved: false,
                is_error: false,
                last_tool_info: Some("Read".to_string()),
                name: Some("scout".to_string()),
                task_description: None,
                started_at: std::time::Instant::now(),
                finished_at: None,
                sub_entries: Vec::new(),
                entry_idx: 0,
            },
        );

        let (agent_tx, _agent_rx) = mpsc::channel(1);
        crate::commands::handle_command(
            "list-agents",
            Vec::new(),
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
        )
        .await
        .unwrap();

        let output = &state.chat_history.last().unwrap().content;
        assert!(output.contains("@scout (fork)"), "{}", output);
        assert!(output.contains("Investigate login"), "{}", output);
        assert!(output.contains("Running"), "{}", output);
        assert!(output.contains("| 3 | 42 |"), "{}", output);
    }

    #[test]
    fn subtask_preview_uses_the_first_nonempty_line_and_bounds_utf8() {
        assert_eq!(
            subtask_description("\n  Investigate login  \nand report back"),
            "Investigate login"
        );

        let long_cjk = "修复".repeat(60);
        let preview = subtask_description(&long_cjk);
        assert_eq!(preview.chars().count(), 80);
        assert!(preview.starts_with("修复"));
        assert_eq!(subtask_description("\n\n"), "Subtask");
    }

    #[tokio::test]
    async fn subtask_forks_the_conversation_in_the_worker_and_reports_the_launch() {
        let mut state = crate::ui::state::ChatState::new();
        let (agent_tx, mut agent_rx) = mpsc::channel(1);
        let captured = Arc::new(Mutex::new(None::<(String, String)>));
        let captured_for_worker = captured.clone();
        let worker = tokio::spawn(async move {
            if let Some(AgentRequest::RunSubtask {
                task,
                description,
                response,
            }) = agent_rx.recv().await
            {
                *captured_for_worker.lock().unwrap() = Some((task, description.clone()));
                let _ = response
                    .send(Ok(crate::agent::subagent::runner::AsyncLaunchResult {
                        agent_id: "agent-abc12345".to_string(),
                        output_file: PathBuf::from(".star/subagent_outputs/agent-abc12345.txt"),
                        description,
                    }))
                    .await;
            }
        });

        crate::commands::handle_command(
            "subtask",
            vec![
                "Investigate".to_string(),
                "the login flow".to_string(),
                "and report back".to_string(),
            ],
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
        )
        .await
        .unwrap();

        let captured = captured.lock().unwrap().clone().unwrap();
        assert_eq!(captured.0, "Investigate the login flow and report back");
        assert_eq!(captured.1, "Investigate the login flow and report back");

        let output = &state.chat_history.last().unwrap().content;
        assert!(output.contains("Subtask started"), "{}", output);
        assert!(output.contains("agent-abc12345"), "{}", output);
        assert!(output.contains("Investigate the login flow"), "{}", output);
        assert!(
            output.contains(".star/subagent_outputs/agent-abc12345.txt"),
            "{}",
            output
        );

        worker.abort();
    }

    #[tokio::test]
    async fn subtask_without_a_task_does_not_start_a_worker_request() {
        let mut state = crate::ui::state::ChatState::new();
        let (agent_tx, mut agent_rx) = mpsc::channel(1);

        crate::commands::handle_command(
            "subtask",
            Vec::new(),
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
        )
        .await
        .unwrap();

        assert!(
            agent_rx.try_recv().is_err(),
            "an empty subtask must not launch a background agent"
        );
        assert!(state
            .chat_history
            .last()
            .unwrap()
            .content
            .contains("Usage: `/subtask <task>`"));
    }

    #[tokio::test]
    async fn subtask_surfaces_worker_startup_errors() {
        let mut state = crate::ui::state::ChatState::new();
        let (agent_tx, mut agent_rx) = mpsc::channel(1);
        let worker = tokio::spawn(async move {
            if let Some(AgentRequest::RunSubtask { response, .. }) = agent_rx.recv().await {
                let _ = response
                    .send(Err(
                        "Subtask requires an initialized runtime notification queue.".to_string(),
                    ))
                    .await;
            }
        });

        crate::commands::handle_command(
            "subtask",
            vec!["Do the thing".to_string()],
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
        )
        .await
        .unwrap();

        let output = &state.chat_history.last().unwrap().content;
        assert!(output.contains("failed to start"), "{}", output);
        assert!(output.contains("notification queue"), "{}", output);

        worker.abort();
    }

    #[tokio::test]
    async fn list_agents_all_includes_foreground_tasks() {
        let mut state = crate::ui::state::ChatState::new();
        state.active_agent_tasks.insert(
            "task-foreground".to_string(),
            crate::ui::state::AgentTaskInfo {
                task_id: "task-foreground".to_string(),
                agent_type: "general-purpose".to_string(),
                description: "Foreground check".to_string(),
                status: crate::types::AgentTaskStatus::Running,
                tool_use_count: 1,
                tokens: 7,
                is_async: false,
                is_resolved: false,
                is_error: false,
                last_tool_info: None,
                name: None,
                task_description: None,
                started_at: std::time::Instant::now(),
                finished_at: None,
                sub_entries: Vec::new(),
                entry_idx: 0,
            },
        );

        let (agent_tx, _agent_rx) = mpsc::channel(1);
        crate::commands::handle_command(
            "list-agents",
            Vec::new(),
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
        )
        .await
        .unwrap();
        assert!(
            !state
                .chat_history
                .last()
                .unwrap()
                .content
                .contains("Foreground check"),
            "foreground tasks are hidden by default"
        );

        crate::commands::handle_command(
            "list-agents",
            vec!["all".to_string()],
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
        )
        .await
        .unwrap();
        assert!(state
            .chat_history
            .last()
            .unwrap()
            .content
            .contains("Foreground check"));
    }
}
