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

fn is_copyable_reply(entry: &ChatEntry) -> bool {
    matches!(entry.entry_type, ChatEntryType::Assistant) && !entry.content.trim().is_empty()
}

/// Select the `index`-th most recent assistant reply (1 = latest).
///
/// Claude Code's `/copy [N]` accepts a 1-based reply number; zero and values
/// past the end are errors, not wraparound.
pub(crate) fn nth_latest_assistant(history: &[ChatEntry], index: usize) -> Option<&ChatEntry> {
    if index == 0 {
        return None;
    }
    history
        .iter()
        .rev()
        .filter(|entry| is_copyable_reply(entry))
        .nth(index - 1)
}

pub async fn copy(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let index = match args.first() {
        None => 1,
        Some(raw) => match raw.parse::<usize>() {
            Ok(index) if index > 0 => index,
            _ => {
                ctx.state.chat_history.push(
                    ChatEntry::assistant(
                        "Usage: /copy [N] — copy the Nth most recent assistant reply (default: 1).",
                    )
                    .with_streaming(false),
                );
                return Ok(());
            }
        },
    };

    let available = ctx
        .state
        .chat_history
        .iter()
        .filter(|entry| is_copyable_reply(entry))
        .count();

    if let Some(msg) = nth_latest_assistant(&ctx.state.chat_history, index) {
        let content_to_copy = msg.content.clone();
        match crate::ui::utils::clipboard::copy_to_clipboard(&content_to_copy) {
            Ok(()) => {
                let template = i18n::t(
                    "cmd.copy.success_nth",
                    "✅ 已复制第 {index} 条助手回复（共 {total} 条）到剪贴板。",
                    "✅ Copied assistant reply #{index} of {total} to clipboard.",
                );
                ctx.state.chat_history.push(
                    ChatEntry::assistant(
                        template
                            .replace("{index}", &index.to_string())
                            .replace("{total}", &available.to_string()),
                    )
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
    } else if available == 0 {
        ctx.state.chat_history.push(
            ChatEntry::assistant(i18n::t(
                "cmd.copy.none",
                "⚠️ 未找到可复制的助手回复。",
                "⚠️ No assistant reply available to copy.",
            ))
            .with_streaming(false),
        );
    } else {
        let template = i18n::t(
            "cmd.copy.unavailable",
            "⚠️ 没有第 {index} 条助手回复（共 {total} 条）。",
            "⚠️ No assistant reply #{index} (only {total} available).",
        );
        ctx.state.chat_history.push(
            ChatEntry::assistant(
                template
                    .replace("{index}", &index.to_string())
                    .replace("{total}", &available.to_string()),
            )
            .with_streaming(false),
        );
    }

    Ok(())
}

/// `/compact [instructions]` — text after the command is a focus hint for the
/// summarizer. Empty/whitespace-only text is the same as no hint at all.
pub(crate) fn compact_instructions(args: &[String]) -> Option<String> {
    let instructions = args.join(" ");
    let instructions = instructions.trim();
    if instructions.is_empty() {
        None
    } else {
        Some(instructions.to_string())
    }
}

pub async fn compress(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;

    let _ = ctx
        .agent_tx
        .send(crate::runtime::messages::AgentRequest::Compress {
            message_id,
            instructions: compact_instructions(&args),
        })
        .await;

    // 即时反馈：压缩结果稍后由 agent 流式返回，先给状态提示
    ctx.state.current_status_line = Some("Compressing context...".to_string());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history() -> Vec<ChatEntry> {
        vec![
            ChatEntry::user("question"),
            ChatEntry::assistant("first answer"),
            ChatEntry::assistant(""),
            ChatEntry::user("follow-up"),
            ChatEntry::assistant("second answer"),
        ]
    }

    #[test]
    fn copy_defaults_to_the_latest_reply() {
        let history = history();
        assert_eq!(
            nth_latest_assistant(&history, 1).map(|entry| entry.content.as_str()),
            Some("second answer")
        );
    }

    #[test]
    fn copy_can_select_older_replies_by_one_based_index() {
        let history = history();
        assert_eq!(
            nth_latest_assistant(&history, 2).map(|entry| entry.content.as_str()),
            Some("first answer"),
            "empty assistant entries must not consume a reply number"
        );
        assert!(nth_latest_assistant(&history, 3).is_none());
    }

    #[test]
    fn copy_rejects_zero_and_out_of_range_numbers() {
        let history = history();
        assert!(nth_latest_assistant(&history, 0).is_none());
        assert!(nth_latest_assistant(&history, 99).is_none());
        assert!(nth_latest_assistant(&[], 1).is_none());
    }

    #[test]
    fn compact_instructions_are_optional_and_keep_their_spacing() {
        assert_eq!(compact_instructions(&[]), None);
        assert_eq!(
            compact_instructions(&["  ".to_string(), "\t".to_string()]),
            None,
            "whitespace-only text must not reach the summarizer"
        );
        assert_eq!(
            compact_instructions(&[
                "focus".to_string(),
                "on".to_string(),
                "the".to_string(),
                "migration".to_string()
            ]),
            Some("focus on the migration".to_string())
        );
    }
}
