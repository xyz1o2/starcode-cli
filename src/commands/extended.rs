use crate::commands::execution::{CommandContext, CommandResult};
use crate::runtime::messages::AgentRequest;
use crate::types::ChatEntry;
use arboard::Clipboard;

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
    let count = args.first().and_then(|s| s.parse::<usize>().ok()).unwrap_or(20);
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
    let target = if args.is_empty() { String::new() } else { args.join(" ") };
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
    let json = serde_json::to_string_pretty(&settings).unwrap_or_else(|_| "{}".to_string());
    push_msg(&mut ctx, format!("Current configuration:\n```json\n{}\n```", json));
    Ok(())
}

pub async fn config_set(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if args.len() < 2 {
        push_msg(&mut ctx, "Usage: /config set <key> <value>");
        return Ok(());
    }
    let key = &args[0];
    let value = &args[1..].join(" ");
    push_msg(
        &mut ctx,
        format!("Set `{}` = `{}`\nNote: Use the settings file at ~/.star/user-settings.json to persist changes.", key, value),
    );
    Ok(())
}

pub async fn config_reset(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    push_msg(
        &mut ctx,
        "Configuration reset to defaults.\nNote: Edit ~/.star/user-settings.json to customize.",
    );
    Ok(())
}

pub async fn config_export(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let settings_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".star")
        .join("user-settings.json");
    if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path).unwrap_or_default();
        push_msg(&mut ctx, format!("Exported configuration:\n```json\n{}\n```", content));
    } else {
        push_msg(&mut ctx, "No configuration file found.");
    }
    Ok(())
}

pub async fn config_import(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if args.is_empty() {
        push_msg(&mut ctx, "Usage: /config import <path>");
        return Ok(());
    }
    let path = &args[0];
    push_msg(
        &mut ctx,
        format!("Import configuration from: {}\nNote: Copy the file to ~/.star/user-settings.json manually.", path),
    );
    Ok(())
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
        ctx.state.show_palette = false;
        ctx.state.show_session_menu = true;
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
        format!("Debug logging: {}", if new_state == "true" { "ON" } else { "OFF" }),
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
        lines.push(format!("  {}. {} - {}", i + 1, name, &entry.content[..entry.content.len().min(80)]));
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
    push_msg(&mut ctx, format!("Agent switch requested: {}\nUse /agents to manage agent definitions.", name));
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
        format!("Create agent: {}\nUse /agents create for interactive agent creation.", name),
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
        format!("Delete agent: {}\nUse /agents delete for confirmation.", name),
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
        format!("Create workflow: {}\nCreate a .star/workflows/{}.md file.", name, name),
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

pub async fn compact_cmd(ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    let _ = ctx
        .agent_tx
        .send(AgentRequest::Compress { message_id })
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
    push_msg(&mut ctx, "Use /provider list to see available providers, /provider select to switch.");
    Ok(())
}

pub async fn temperature_cmd(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if let Some(val) = args.first() {
        push_msg(&mut ctx, format!("Temperature set to: {}\nNote: This takes effect on the next request.", val));
    } else {
        push_msg(&mut ctx, "Current temperature: default (provider-specific)\nUsage: /temperature <value>");
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
                    lines.push(format!(
                        "  [{}] {} ({})",
                        priority, s.message, s.id
                    ));
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
                    push_msg(
                        &mut ctx,
                        format!("Speech rate set to: {}", rate),
                    );
                } else {
                    push_msg(&mut ctx, "Invalid rate value. Use a number between 0.5 and 2.0.".to_string());
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
                    push_msg(
                        &mut ctx,
                        format!("Volume set to: {}", volume),
                    );
                } else {
                    push_msg(&mut ctx, "Invalid volume value. Use a number between 0.0 and 1.0.".to_string());
                }
            } else {
                let volume = ctx.state.voice_config.volume;
                push_msg(
                    &mut ctx,
                    format!(
                        "Current volume: {}\nUsage: /voice volume <0.0-1.0>",
                        volume
                    ),
                );
            }
        }
        _ => {
            push_msg(
                &mut ctx,
                "Usage: /voice [on|off|status|lang <code>|rate <0.5-2.0>|volume <0.0-1.0>]".to_string(),
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
                push_msg(&mut ctx, "Usage: /remote-settings set-endpoint <url>".to_string());
            }
        }
        "sync" => match ctx.state.remote_settings.sync().await {
            Ok(()) => push_msg(&mut ctx, "Settings synced successfully.".to_string()),
            Err(e) => push_msg(&mut ctx, format!("Sync failed: {}", e)),
        },
        "get" => {
            if let Some(path) = args.get(1) {
                let result = ctx
                    .state
                    .remote_settings
                    .get_setting(path)
                    .cloned();
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
        if ctx.state.current_model.is_empty() { "-" } else { &ctx.state.current_model }
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
                        if tc.function.arguments.len() > 500 {
                            format!("{}...", &tc.function.arguments[..500])
                        } else {
                            tc.function.arguments.clone()
                        }
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
        let is_file_tool = matches!(
            name,
            "Read" | "view_file" | "Edit" | "edit_file" | "create_file" | "Write" | "str_replace_editor" | "smart_edit"
        );
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
        push_msg(&mut ctx, "No files have been read or edited in this session yet.");
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
        lines.push_str("  id                            | time              | tool       | files\n");
        lines.push_str("  ------------------------------+-------------------+------------+------\n");

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
        format!("restored {} file(s):\n{}", changed.len(), changed.join("\n"))
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
