use crate::commands::execution::{CommandContext, CommandResult};
use crate::types::{ChatEntry, ChatEntryType};
use clap::Subcommand;
use std::path::PathBuf;
use tokio::fs;

#[derive(Subcommand)]
pub enum MemoryCommand {
    /// Show current memory contents
    Show,
    /// Add content to the memory
    Add {
        /// Text to remember
        #[arg(required = true)]
        content: Vec<String>,
    },
    /// Refresh the memory from the source
    Refresh,
}

pub async fn execute_memory_command(ctx: CommandContext<'_>, cmd: MemoryCommand) -> CommandResult {
    match cmd {
        MemoryCommand::Show => show_memory(ctx).await,
        MemoryCommand::Add { content } => add_memory(ctx, content.join(" ")).await,
        MemoryCommand::Refresh => refresh_memory(ctx).await,
    }
}

async fn get_memory_file_path() -> Result<PathBuf, String> {
    let mut path = std::env::current_dir().map_err(|e| e.to_string())?;
    path.push(".star");
    if !path.exists() {
        fs::create_dir_all(&path).await.map_err(|e| e.to_string())?;
    }
    path.push("memory.md");
    Ok(path)
}

async fn show_memory(ctx: CommandContext<'_>) -> CommandResult {
    let path = get_memory_file_path().await?;
    let content = if path.exists() {
        let content = fs::read_to_string(&path).await.map_err(|e| e.to_string())?;
        if content.trim().is_empty() {
            "Current memory is empty.".to_string()
        } else {
            format!("Current project memory:\n\n---\n{}\n---", content)
        }
    } else {
        "Current memory is empty.".to_string()
    };

    ctx.state
        .chat_history
        .push(ChatEntry::assistant(content).with_streaming(false));
    Ok(())
}

async fn add_memory(ctx: CommandContext<'_>, content: String) -> CommandResult {
    // Construct a fake tool call to trigger the confirmation UI
    let tool_call = crate::types::StarToolCall {
        id: uuid::Uuid::new_v4().to_string(),
        call_type: "function".to_string(),
        function: crate::types::StarToolCallFunction {
            name: "save_memory".to_string(),
            arguments: serde_json::json!({ "fact": content }).to_string(),
        },
    };

    // Push a ToolCall entry to chat history
    ctx.state
        .chat_history
        .push(ChatEntry::assistant_with_tool_calls(vec![tool_call.clone()]).with_streaming(false));

    // Set pending tool calls to trigger confirmation UI
    ctx.state.pending_tool_calls = Some(vec![tool_call.clone()]);
    ctx.state.pending_message_id = Some(ctx.state.active_message_id.unwrap_or(0));

    // Also push a status message
    ctx.state
        .chat_history
        .push(ChatEntry::assistant("ℹ️ 正在尝试保存到记忆...").with_streaming(false));

    // Build confirmation object
    let confirmation =
        crate::ui::components::confirmation_dialog::build_confirmation_from_tool_call(&tool_call)
            .await;

    // Push Confirmation Entry
    ctx.state.chat_history.push(
        ChatEntry::new(ChatEntryType::ToolConfirmation, "".to_string())
            .with_confirmation(confirmation)
            .with_streaming(false),
    );

    // Point to the confirmation entry
    ctx.state.pending_confirmation_entry_idx = Some(ctx.state.chat_history.len() - 1);

    Ok(())
}

async fn refresh_memory(mut ctx: CommandContext<'_>) -> CommandResult {
    // memory.md 在每次请求时自动重读（agent/context.rs 按内容哈希检测变化并重注入），
    // 无需手动刷新。这里只做存在性检查并如实告知。
    let path = get_memory_file_path().await?;
    let content = if path.exists() {
        "Memory is auto-reloaded on every request — no manual refresh needed.\n\
         Use /memory show to view the current content."
            .to_string()
    } else {
        "No memory file yet — it will be created when you add the first memory (/memory add <text>)."
            .to_string()
    };

    ctx.state
        .chat_history
        .push(ChatEntry::assistant(content).with_streaming(false));
    Ok(())
}
