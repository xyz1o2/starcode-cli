use crate::commands::execution::{CommandContext, CommandResult};
use crate::types::ChatEntry;

pub async fn stats(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("session");

    if sub != "session" {
        ctx.state
            .chat_history
            .push(ChatEntry::assistant(format!(
                "Unknown stats category '{}'. Available: session (default). Try /cost for token/cost details.",
                sub
            )).with_streaming(false));
        return Ok(());
    }

    // 与 /cost 同口径：优先用最终 usage（prompt/completion/total），
    // 流式期间才回退到实时计数的 token_count
    let (prompt, completion, total, source) = match &ctx.state.token_usage {
        Some(u) if u.total_tokens > 0 || u.prompt_tokens > 0 => (
            u.prompt_tokens,
            u.completion_tokens,
            u.total_tokens.max(u.prompt_tokens + u.completion_tokens),
            "final usage",
        ),
        _ => (0, 0, ctx.state.token_count, "streaming estimate"),
    };

    let content = format!(
        "Session Stats:\n- Messages: {}\n- Tokens: {} ({})\n  - Prompt: {}\n  - Completion: {}\n- Model: {}\n- Processing Time: {}s",
        ctx.state.chat_history.len(),
        total,
        source,
        prompt,
        completion,
        if ctx.state.current_model.is_empty() { "-" } else { &ctx.state.current_model },
        ctx.state.processing_time_secs,
    );

    ctx.state
        .chat_history
        .push(ChatEntry::assistant(content).with_streaming(false));
    Ok(())
}

pub async fn tools(ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let content = "Available Tools (canonical names):\n- Bash\n- view_file\n- Read\n- Edit\n- Grep\n- Glob\n- ListDir\n- create_file\n- Write\n- SemanticSearch\n- ProjectMap\n- complete_task\n\nCompatibility aliases:\n- edit_file -> Edit\n- str_replace_editor -> Edit\n- search_file_content -> Grep\n- run_shell_command -> Bash";

    ctx.state
        .chat_history
        .push(ChatEntry::assistant(content).with_streaming(false));
    Ok(())
}
