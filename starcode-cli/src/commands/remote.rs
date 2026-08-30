use crate::commands::execution::{CommandContext, CommandResult};
use clap::Subcommand;

#[derive(Subcommand)]
pub enum RemoteCommand {
    /// Show remote control inbox status
    Status,
    /// Queue a remote control message
    #[command(arg_required_else_help = true)]
    Send {
        /// Message content
        #[arg(required = true)]
        message: Vec<String>,
        /// Optional source label
        #[arg(long)]
        source: Option<String>,
    },
    /// Show protocol format
    Protocol,
    /// Drain remote inbox manually
    Drain,
}

pub async fn execute_remote_command(ctx: CommandContext<'_>, cmd: RemoteCommand) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;

    match cmd {
        RemoteCommand::Status => {
            let path = crate::core::remote::inbox_file_path(&cwd);
            let queued = crate::core::remote::queued_count(&cwd)
                .await
                .map_err(|e| format!("failed to read remote inbox: {}", e))?;

            let msg = format!(
                "📡 Remote Control 状态\n\n- inbox: {}\n- queued: {}",
                path.display(),
                queued
            );
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
            Ok(())
        }
        RemoteCommand::Send { message, source } => {
            let msg = message.join(" ").trim().to_string();
            crate::core::remote::queue_message(&cwd, msg.clone(), source.clone())
                .await
                .map_err(|e| format!("failed to queue remote message: {}", e))?;

            let queued = crate::core::remote::queued_count(&cwd)
                .await
                .map_err(|e| format!("failed to read remote inbox: {}", e))?;

            let out = format!(
                "✅ 已写入 remote inbox\n\n- message: {}\n- source: {}\n- queued: {}",
                msg,
                source.unwrap_or_else(|| "-".to_string()),
                queued
            );
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(out).with_streaming(false));
            Ok(())
        }
        RemoteCommand::Protocol => {
            let path = crate::core::remote::inbox_file_path(&cwd);
            let msg = format!(
                "# Remote Protocol v1\n\n写入文件（每行一个 JSON 对象）:\n- `{}`\n\n示例:\n```json\n{}\n```",
                path.display(),
                crate::core::remote::protocol_example()
            );
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
            Ok(())
        }
        RemoteCommand::Drain => {
            let drained = crate::core::remote::drain_requests(&cwd)
                .await
                .map_err(|e| format!("failed to drain remote inbox: {}", e))?;

            let mut lines = vec![format!(
                "📥 已消费 remote inbox: accepted={}, rejected={}",
                drained.accepted.len(),
                drained.rejected.len()
            )];
            for req in drained.accepted.iter().take(10) {
                lines.push(format!(
                    "- [{}] {}",
                    if req.source.trim().is_empty() {
                        "remote"
                    } else {
                        &req.source
                    },
                    req.message
                ));
            }
            if drained.accepted.len() > 10 {
                lines.push(format!("- ... 其余 {} 条省略", drained.accepted.len() - 10));
            }
            for err in drained.rejected.iter().take(5) {
                lines.push(format!("- rejected: {}", err));
            }
            if drained.rejected.len() > 5 {
                lines.push(format!(
                    "- ... 其余 {} 条错误省略",
                    drained.rejected.len() - 5
                ));
            }

            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(lines.join("\n")).with_streaming(false));
            Ok(())
        }
    }
}
