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
    execute_remote_command_in(ctx, cmd, &cwd).await
}

/// `cwd` 走参数而不是 `current_dir()`：单测在临时目录里读写 remote inbox，
/// 否则会把消息写进开发者自己的 `.star/remote/inbox.jsonl`。
pub(crate) async fn execute_remote_command_in(
    ctx: CommandContext<'_>,
    cmd: RemoteCommand,
    cwd: &std::path::Path,
) -> CommandResult {
    match cmd {
        RemoteCommand::Status => {
            let path = crate::core::remote::inbox_file_path(cwd);
            let queued = crate::core::remote::queued_count(cwd)
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
            crate::core::remote::queue_message(cwd, msg.clone(), source.clone())
                .await
                .map_err(|e| format!("failed to queue remote message: {}", e))?;

            let queued = crate::core::remote::queued_count(cwd)
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
            let path = crate::core::remote::inbox_file_path(cwd);
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
            let drained = crate::core::remote::drain_requests(cwd)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::execution::CommandContext;
    use crate::ui::state::ChatState;

    async fn run_in(cwd: &std::path::Path, cmd: RemoteCommand) -> Result<String, String> {
        let mut state = ChatState::test_fixture();
        let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(1);
        execute_remote_command_in(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            cmd,
            cwd,
        )
        .await?;
        Ok(state
            .chat_history
            .last()
            .map(|entry| entry.content.clone())
            .unwrap_or_default())
    }

    fn send(message: &str, source: Option<&str>) -> RemoteCommand {
        RemoteCommand::Send {
            message: message.split_whitespace().map(str::to_string).collect(),
            source: source.map(str::to_string),
        }
    }

    #[tokio::test]
    async fn status_reports_an_empty_inbox_without_creating_it() {
        let dir = tempfile::tempdir().unwrap();
        let out = run_in(dir.path(), RemoteCommand::Status).await.unwrap();

        assert!(out.contains("📡 Remote Control 状态"), "{out}");
        assert!(out.contains("- queued: 0"), "{out}");
        assert!(out.contains("inbox.jsonl"), "{out}");
        // 只读命令不该顺手建目录
        assert!(!crate::core::remote::inbox_file_path(dir.path()).exists());
    }

    #[tokio::test]
    async fn send_queues_the_message_and_reports_the_new_depth() {
        let dir = tempfile::tempdir().unwrap();
        let out = run_in(dir.path(), send("check the build", Some("ci-bot")))
            .await
            .unwrap();

        assert!(out.contains("- message: check the build"), "{out}");
        assert!(out.contains("- source: ci-bot"), "{out}");
        assert!(out.contains("- queued: 1"), "{out}");

        let out = run_in(dir.path(), send("and the tests", None))
            .await
            .unwrap();
        assert!(out.contains("- source: -"), "{out}");
        assert!(out.contains("- queued: 2"), "{out}");
    }

    /// 空消息如果写进 inbox，每一轮 drain 都会被拒一次 —— 必须挡在写之前。
    #[tokio::test]
    async fn an_empty_message_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let err = run_in(dir.path(), send("   ", None)).await.unwrap_err();

        assert!(err.contains("failed to queue remote message"), "{err}");
        assert!(err.contains("message cannot be empty"), "{err}");
        assert!(!crate::core::remote::inbox_file_path(dir.path()).exists());
    }

    #[tokio::test]
    async fn protocol_shows_the_v1_envelope_and_the_inbox_path() {
        let dir = tempfile::tempdir().unwrap();
        let out = run_in(dir.path(), RemoteCommand::Protocol).await.unwrap();

        assert!(out.contains("# Remote Protocol v1"), "{out}");
        assert!(out.contains("starcode.remote.v1"), "{out}");
        assert!(out.contains("inbox.jsonl"), "{out}");
        assert!(out.contains("send_message"), "{out}");
    }

    #[tokio::test]
    async fn drain_separates_accepted_requests_from_rejected_lines() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(crate::core::remote::remote_dir(dir.path())).unwrap();
        let path = crate::core::remote::inbox_file_path(dir.path());
        std::fs::write(
            &path,
            concat!(
                r#"{"protocol":"starcode.remote.v1","action":"send_message","source":"ci","message":"fix main"}"#,
                "\n",
                r#"{"protocol":"starcode.remote.v1","action":"send_message","message":"run tests"}"#,
                "\n",
                "not json\n",
                r#"{"protocol":"starcode.remote.v1","action":"pause","message":"halt"}"#,
                "\n",
            ),
        )
        .unwrap();

        let out = run_in(dir.path(), RemoteCommand::Drain).await.unwrap();
        assert!(out.contains("accepted=2, rejected=2"), "{out}");
        assert!(out.contains("- [ci] fix main"), "{out}");
        assert!(out.contains("- [remote] run tests"), "{out}");
        assert!(out.contains("line 3: invalid json"), "{out}");
        assert!(out.contains("unsupported action `pause`"), "{out}");

        // drain 是一次性消费：紧接着再 drain 什么也拿不到
        let empty = run_in(dir.path(), RemoteCommand::Drain).await.unwrap();
        assert!(empty.contains("accepted=0, rejected=0"), "{empty}");
    }

    #[tokio::test]
    async fn drain_on_a_missing_inbox_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let out = run_in(dir.path(), RemoteCommand::Drain).await.unwrap();
        assert!(out.contains("accepted=0, rejected=0"), "{out}");
    }

    /// 一次刷进一万条时不能把整个聊天记录撑爆：列表截断，但计数是准的。
    #[tokio::test]
    async fn drain_truncates_long_lists_but_keeps_the_totals() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(crate::core::remote::remote_dir(dir.path())).unwrap();
        let path = crate::core::remote::inbox_file_path(dir.path());
        let mut content = String::new();
        for i in 0..12 {
            content.push_str(&format!(
                "{{\"protocol\":\"starcode.remote.v1\",\"action\":\"send_message\",\"message\":\"m{}\"}}\n",
                i
            ));
        }
        std::fs::write(&path, content).unwrap();

        let out = run_in(dir.path(), RemoteCommand::Drain).await.unwrap();
        assert!(out.contains("accepted=12, rejected=0"), "{out}");
        assert!(out.contains("其余 2 条省略"), "{out}");
        assert!(!out.contains("m11"), "{out}");
    }
}
