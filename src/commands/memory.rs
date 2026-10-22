use crate::commands::execution::{CommandContext, CommandResult};
use crate::types::ChatEntry;
use clap::Subcommand;
use std::path::{Path, PathBuf};
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

pub(crate) fn memory_file_path(root: &Path) -> PathBuf {
    root.join(".star").join("memory.md")
}

fn current_memory_path() -> Result<PathBuf, String> {
    Ok(memory_file_path(
        &std::env::current_dir().map_err(|e| e.to_string())?,
    ))
}

/// One memory entry, in the same `- [timestamp] fact` shape the confirmation
/// preview used before.
fn memory_entry_line(timestamp: &str, fact: &str) -> String {
    format!("- [{}] {}\n", timestamp, fact.trim())
}

fn append_memory(existing: &str, entry: &str) -> String {
    let mut content = existing.to_string();
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(entry);
    content
}

/// Append directly to `.star/memory.md`.
///
/// The old implementation fabricated a `save_memory` tool call and a
/// confirmation card, but approval sent `ConfirmTool` to a worker that had never
/// started that tool call — so the fact was silently dropped. Memory writes are
/// local and reversible; doing them here is both simpler and actually works.
async fn append_memory_to_path(path: &Path, fact: &str) -> Result<(), String> {
    let fact = fact.trim();
    if fact.is_empty() {
        return Err("Memory content cannot be empty.".to_string());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    let existing = match fs::read_to_string(path).await {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.to_string()),
    };
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let content = append_memory(&existing, &memory_entry_line(&now, fact));
    fs::write(path, content).await.map_err(|e| e.to_string())
}

async fn show_memory(ctx: CommandContext<'_>) -> CommandResult {
    let path = current_memory_path()?;
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

async fn add_memory(mut ctx: CommandContext<'_>, content: String) -> CommandResult {
    let path = current_memory_path()?;
    match append_memory_to_path(&path, &content).await {
        Ok(()) => {
            ctx.state.chat_history.push(
                ChatEntry::assistant(format!(
                    "✅ Added to project memory: {}\n\nSaved in `{}`.",
                    content.trim(),
                    path.display()
                ))
                .with_streaming(false),
            );
        }
        Err(error) => {
            ctx.state.chat_history.push(
                ChatEntry::assistant(format!("❌ Could not save memory: {}", error))
                    .with_streaming(false),
            );
        }
    }
    Ok(())
}

async fn refresh_memory(mut ctx: CommandContext<'_>) -> CommandResult {
    // memory.md 在每次请求时自动重读（agent/context.rs 按内容哈希检测变化并重注入），
    // 无需手动刷新。这里只做存在性检查并如实告知。
    let path = current_memory_path()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_entries_keep_the_documented_bullet_shape() {
        assert_eq!(
            memory_entry_line("2026-10-22 10:00:00", "  prefer tabs  "),
            "- [2026-10-22 10:00:00] prefer tabs\n"
        );
    }

    #[test]
    fn appending_does_not_glue_entries_together() {
        assert_eq!(
            append_memory("old fact", "- [t] new fact\n"),
            "old fact\n- [t] new fact\n"
        );
        assert_eq!(
            append_memory("old fact\n", "- [t] new fact\n"),
            "old fact\n- [t] new fact\n",
            "an existing trailing newline must not turn into a blank line"
        );
        assert_eq!(
            append_memory("", "- [t] first fact\n"),
            "- [t] first fact\n"
        );
    }

    #[test]
    fn memory_lives_under_the_project_star_directory() {
        let root = Path::new("C:/work/project");
        assert_eq!(memory_file_path(root), root.join(".star").join("memory.md"));
    }

    #[tokio::test]
    async fn appending_creates_the_directory_and_preserves_previous_entries() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = memory_file_path(dir.path());
        append_memory_to_path(&path, "first fact").await.unwrap();
        append_memory_to_path(&path, "second fact").await.unwrap();

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(content.contains("first fact"));
        assert!(content.contains("second fact"));
        assert_eq!(content.lines().count(), 2, "one bullet per fact");
        assert!(content.lines().all(|line| line.starts_with("- [")));
    }

    #[tokio::test]
    async fn empty_memory_is_rejected_without_touching_the_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = memory_file_path(dir.path());
        let error = append_memory_to_path(&path, "   ").await.unwrap_err();
        assert!(error.contains("empty"));
        assert!(!path.exists());
    }
}
