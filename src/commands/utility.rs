use crate::commands::execution::{CommandContext, CommandResult};
use crate::core::i18n;
use crate::types::{ChatEntry, ChatEntryType};
use crate::utils::checkpoint_manager;
use arboard::Clipboard;

pub async fn undo(ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    // File-history rewind: revert to the most recent snapshot.
    // Uses the new file-history API; falls back to "none found" message if
    // no snapshots exist (e.g. sessions created before file-history was
    // enabled, or file-history disabled via STAR_DISABLE_FILE_CHECKPOINTING).
    //
    // session_id is None: resolve_session_id falls back to a stable per-cwd
    // hash so snapshots taken by write tools (which also pass None) are
    // visible here. This keeps all snapshots for the current project
    // together regardless of UI session.
    let latest = checkpoint_manager::latest_snapshot_id(None)
        .await
        .map_err(|e| format!("Failed to query latest snapshot: {}", e))?;

    if let Some(snapshot_id) = latest {
        let changed = checkpoint_manager::rewind(&snapshot_id, None)
            .await
            .map_err(|e| format!("Failed to rewind to snapshot {}: {}", snapshot_id, e))?;

        let summary = if changed.is_empty() {
            "no files changed (already at this state)".to_string()
        } else {
            format!(
                "restored {} file(s):\n{}",
                changed.len(),
                changed.join("\n")
            )
        };

        let template = i18n::t(
            "cmd.undo.success",
            "✅ 已成功撤销到快照: {id}\n\n{summary}",
            "✅ Reverted to snapshot: {id}\n\n{summary}",
        );
        let content = template
            .replace("{id}", &snapshot_id)
            .replace("{summary}", &summary);
        ctx.state
            .chat_history
            .push(ChatEntry::assistant(content).with_streaming(false));
        return Ok(());
    }

    // No file-history snapshots — show the "none found" message.
    ctx.state.chat_history.push(
        ChatEntry::assistant(i18n::t(
            "cmd.undo.none",
            "⚠️ 没有找到可撤销的快照。",
            "⚠️ No undo snapshots found.",
        ))
        .with_streaming(false),
    );

    Ok(())
}

pub async fn copy(ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    // Find the last assistant message
    let last_assistant_msg = ctx.state.chat_history.iter().rev().find(|entry| {
        matches!(entry.entry_type, ChatEntryType::Assistant) && !entry.content.is_empty()
    });

    if let Some(msg) = last_assistant_msg {
        let content_to_copy = &msg.content;

        match Clipboard::new() {
            Ok(mut clipboard) => {
                if let Err(e) = clipboard.set_text(content_to_copy.clone()) {
                    let prefix = i18n::t(
                        "cmd.copy.error",
                        "无法访问剪贴板: ",
                        "Failed to copy to clipboard: ",
                    );
                    return Err(format!("{}{}", prefix, e));
                }

                ctx.state.chat_history.push(
                    ChatEntry::assistant(i18n::t(
                        "cmd.copy.success",
                        "✅ 上一条回复已复制到剪贴板。",
                        "✅ Copied the last assistant reply to clipboard.",
                    ))
                    .with_streaming(false),
                );
            }
            Err(e) => {
                let prefix = i18n::t(
                    "cmd.copy.error",
                    "无法访问剪贴板: ",
                    "Unable to access clipboard: ",
                );
                return Err(format!("{}{}", prefix, e));
            }
        }
    } else {
        ctx.state.chat_history.push(
            ChatEntry::assistant(i18n::t(
                "cmd.copy.none",
                "⚠️ 未找到可复制的助手回复。",
                "⚠️ No assistant reply available to copy.",
            ))
            .with_streaming(false),
        );
    }

    Ok(())
}

pub async fn compress(ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;

    let _ = ctx
        .agent_tx
        .send(crate::runtime::messages::AgentRequest::Compress { message_id })
        .await;

    // 即时反馈：压缩结果稍后由 agent 流式返回，先给状态提示
    ctx.state.current_status_line = Some("Compressing context...".to_string());

    Ok(())
}
