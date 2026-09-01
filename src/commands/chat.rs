use crate::commands::execution::{CommandContext, CommandResult};
use crate::types::ChatEntry;
use crate::utils::session_manager;
use chrono::Local;

pub async fn run(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if args.is_empty() {
        return list(ctx, &[]).await;
    }

    match args[0].as_str() {
        "save" => save(ctx, &args[1..]).await,
        "resume" | "load" => resume(ctx, &args[1..]).await,
        "list" | "ListDir" => list(ctx, &args[1..]).await,
        "delete" | "rm" => delete(ctx, &args[1..]).await,
        "share" => share(ctx, &args[1..]).await,
        _ => Err(format!("Unknown subcommand: {}", args[0])),
    }
}

/// 导出当前会话为可分享的文本文件（markdown 格式）
async fn share(ctx: CommandContext<'_>, args: &[String]) -> CommandResult {
    let label = if args.is_empty() {
        Local::now().format("%Y%m%d_%H%M%S").to_string()
    } else {
        args[0].clone()
    };

    let mut md = String::from("# Session Export\n\n");
    for entry in &ctx.state.chat_history {
        let role = match entry.entry_type {
            crate::types::ChatEntryType::User => "user",
            crate::types::ChatEntryType::Assistant => "assistant",
            crate::types::ChatEntryType::SystemMessage => "system",
            crate::types::ChatEntryType::ErrorMessage => "error",
            crate::types::ChatEntryType::CompactSummary => "summary",
            _ => continue,
        };
        md.push_str(&format!("**{}**:\n\n{}\n\n---\n\n", role, entry.content));
    }

    let dir = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(".star")
        .join("exports");
    if !dir.exists() {
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create export dir: {}", e))?;
    }
    let file_name = format!("session_{}.md", label);
    let path = dir.join(&file_name);
    std::fs::write(&path, md).map_err(|e| e.to_string())?;

    ctx.state.current_status_line = Some(format!(
        "Session exported to {} ({} messages)",
        path.display(),
        ctx.state.chat_history.len()
    ));
    Ok(())
}

async fn save(ctx: CommandContext<'_>, args: &[String]) -> CommandResult {
    let tag = if args.is_empty() {
        Local::now().format("%Y%m%d_%H%M%S").to_string()
    } else {
        args[0].clone()
    };

    session_manager::save_session(&tag, &ctx.state.chat_history)
        .await
        .map_err(|e| e.to_string())?;

    ctx.state.current_status_line = Some(format!("Session saved as '{}'", tag));
    Ok(())
}

async fn resume(ctx: CommandContext<'_>, args: &[String]) -> CommandResult {
    let (session, resumed_label) = if args.is_empty() {
        // 无参数：打开交互式会话选择菜单，而不是静默用最近会话替换当前对话
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
    } else {
        let tag = &args[0];
        let session = session_manager::load_session(tag)
            .await
            .map_err(|e| e.to_string())?;
        let label = format!("'{}'", tag);
        (session, label)
    };

    let restored_history = session.chat_history;
    let restored_len = restored_history.len();

    // Replace history and reset view/runtime transient states so restored chat
    // is rendered from fresh cache and visible immediately.
    ctx.state.chat_history = restored_history;
    ctx.state.clear_cache();
    ctx.state.last_item_heights.clear();
    ctx.state.total_rendered_lines = 0;
    ctx.state.scroll = 0;
    ctx.state.auto_follow = true;
    ctx.state.is_streaming = false;
    ctx.state.is_processing = false;
    ctx.state.processing_started_at = None;
    ctx.state.active_message_id = None;
    ctx.state.stream_targets.clear();
    ctx.state.message_start_indices.clear();
    ctx.state.pending_tool_calls = None;
    ctx.state.pending_message_id = None;
    ctx.state.pending_confirmation_entry_idx = None;
    ctx.state.is_awaiting_confirmation = false;
    ctx.state.pending_tool_call_id = None;

    if restored_len == 0 {
        ctx.state.chat_list_state.select(None);
    } else {
        ctx.state.chat_list_state.select(Some(restored_len - 1));
    }

    ctx.state.current_status_line = Some(format!(
        "Resumed session {} ({} messages)",
        resumed_label, restored_len
    ));
    Ok(())
}

async fn list(ctx: CommandContext<'_>, _args: &[String]) -> CommandResult {
    let summaries = session_manager::list_session_summaries()
        .await
        .map_err(|e| e.to_string())?;

    let content = if summaries.is_empty() {
        "No saved sessions found.".to_string()
    } else {
        let mut lines = format!("Saved sessions ({}):\n", summaries.len());
        for s in &summaries {
            lines.push_str(&format!("• {}\n  {} · {}\n", s.id, s.title, s.subtitle));
        }
        lines
    };

    ctx.state
        .chat_history
        .push(ChatEntry::assistant(content).with_streaming(false));
    Ok(())
}

async fn delete(ctx: CommandContext<'_>, args: &[String]) -> CommandResult {
    if args.is_empty() {
        return Err("Usage: /chat delete <tag>".to_string());
    }
    let tag = &args[0];

    session_manager::delete_session(tag)
        .await
        .map_err(|e| e.to_string())?;

    ctx.state.current_status_line = Some(format!("Deleted session '{}'", tag));
    Ok(())
}

pub async fn resume_cmd(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    resume(ctx, &args).await
}
