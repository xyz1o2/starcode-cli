use crate::commands::execution::{CommandContext, CommandResult};
use chrono::{Local, TimeZone, Utc};
use clap::Subcommand;

#[derive(Subcommand)]
pub enum LoopCommand {
    /// Add a scheduled loop task
    #[command(arg_required_else_help = true)]
    Add {
        /// Unique loop task name
        name: String,
        /// Trigger interval in minutes
        every_minutes: u64,
        /// Prompt to send when task is triggered
        #[arg(required = true)]
        prompt: Vec<String>,
    },
    /// List scheduled loop tasks
    List,
    /// Remove a scheduled loop task by name
    #[command(arg_required_else_help = true)]
    Remove {
        /// Task name
        name: String,
    },
}

pub async fn execute_loop_command(ctx: CommandContext<'_>, cmd: LoopCommand) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    execute_loop_command_in(ctx, cmd, &cwd).await
}

/// `cwd` 显式传入：单测要在临时目录里建/删任务，不能写进开发者自己的
/// `.star/loops.json`。
pub(crate) async fn execute_loop_command_in(
    ctx: CommandContext<'_>,
    cmd: LoopCommand,
    cwd: &std::path::Path,
) -> CommandResult {
    match cmd {
        LoopCommand::Add {
            name,
            every_minutes,
            prompt,
        } => {
            let prompt_text = prompt.join(" ").trim().to_string();
            let task = crate::core::loops::add_task(cwd, name.clone(), every_minutes, prompt_text)
                .await
                .map_err(|e| format!("Failed to add loop task: {}", e))?;

            let next_time = format_ts(task.next_run_at);
            let msg = format!(
                "✅ 已创建 /loop 任务\n\n- 名称: `{}`\n- 间隔: 每 {} 分钟\n- 下次触发: {}\n- 提示词: {}",
                task.name, task.interval_minutes, next_time, task.prompt
            );

            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
            Ok(())
        }
        LoopCommand::List => {
            let tasks = crate::core::loops::list_tasks(cwd)
                .await
                .map_err(|e| format!("Failed to list loop tasks: {}", e))?;

            if tasks.is_empty() {
                ctx.state.chat_history.push(
                    crate::types::ChatEntry::assistant(
                        "当前没有 /loop 任务。使用 `/loop add <name> <every_minutes> <prompt...>` 创建。",
                    )
                    .with_streaming(false),
                );
                return Ok(());
            }

            let now = Utc::now().timestamp();
            let mut lines = vec!["# /loop 任务列表\n".to_string()];
            for t in tasks {
                let status = if t.enabled { "enabled" } else { "disabled" };
                let in_minutes = ((t.next_run_at - now).max(0) as f64 / 60.0).ceil() as i64;
                lines.push(format!(
                    "- `{}` [{}]\n  - every: {} min\n  - next: {} (约 {} 分钟后)\n  - prompt: {}",
                    t.name,
                    status,
                    t.interval_minutes,
                    format_ts(t.next_run_at),
                    in_minutes,
                    t.prompt
                ));
            }

            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(lines.join("\n")).with_streaming(false));
            Ok(())
        }
        LoopCommand::Remove { name } => {
            let removed = crate::core::loops::remove_task(cwd, &name)
                .await
                .map_err(|e| format!("Failed to remove loop task: {}", e))?;

            let msg = if removed {
                format!("✅ 已删除 /loop 任务 `{}`", name)
            } else {
                format!("⚠️ 未找到 /loop 任务 `{}`", name)
            };

            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
            Ok(())
        }
    }
}

fn format_ts(ts: i64) -> String {
    match Local.timestamp_opt(ts, 0).single() {
        Some(dt) => dt.format("%Y-%m-%d %H:%M:%S %Z").to_string(),
        None => ts.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::execution::CommandContext;
    use crate::ui::state::ChatState;

    async fn run_in(cwd: &std::path::Path, cmd: LoopCommand) -> Result<String, String> {
        let mut state = ChatState::test_fixture();
        let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(1);
        execute_loop_command_in(
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

    fn add(name: &str, minutes: u64, prompt: &str) -> LoopCommand {
        LoopCommand::Add {
            name: name.to_string(),
            every_minutes: minutes,
            prompt: prompt.split_whitespace().map(str::to_string).collect(),
        }
    }

    #[tokio::test]
    async fn an_empty_list_explains_how_to_create_one() {
        let dir = tempfile::tempdir().unwrap();
        let out = run_in(dir.path(), LoopCommand::List).await.unwrap();
        assert!(out.contains("当前没有 /loop 任务"), "{out}");
        assert!(
            out.contains("/loop add <name> <every_minutes> <prompt...>"),
            "{out}"
        );
    }

    #[tokio::test]
    async fn adding_a_task_persists_it_and_the_list_shows_it() {
        let dir = tempfile::tempdir().unwrap();
        let out = run_in(dir.path(), add("standup", 30, "summarize the diff"))
            .await
            .unwrap();
        assert!(out.contains("- 名称: `standup`"), "{out}");
        assert!(out.contains("- 间隔: 每 30 分钟"), "{out}");
        assert!(out.contains("- 提示词: summarize the diff"), "{out}");

        assert!(crate::core::loops::loops_file_path(dir.path()).exists());

        let listed = run_in(dir.path(), LoopCommand::List).await.unwrap();
        assert!(listed.starts_with("# /loop 任务列表"), "{listed}");
        assert!(listed.contains("- `standup` [enabled]"), "{listed}");
        assert!(listed.contains("- every: 30 min"), "{listed}");
        assert!(listed.contains("- prompt: summarize the diff"), "{listed}");
    }

    /// 间隔 0 会让 next_run_at 停在过去，调度器每轮都会重新触发 ——
    /// 必须在落盘前拒绝。
    #[tokio::test]
    async fn a_zero_interval_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let err = run_in(dir.path(), add("bad", 0, "loop forever"))
            .await
            .unwrap_err();
        assert!(err.contains("Failed to add loop task"), "{err}");
        assert!(!crate::core::loops::loops_file_path(dir.path()).exists());
    }

    #[tokio::test]
    async fn prompt_arguments_are_joined_with_spaces() {
        let dir = tempfile::tempdir().unwrap();
        run_in(dir.path(), add("multi", 5, "check   the   build"))
            .await
            .unwrap();
        let tasks = crate::core::loops::list_tasks(dir.path()).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].prompt, "check the build");
    }

    #[tokio::test]
    async fn removing_reports_whether_the_task_existed() {
        let dir = tempfile::tempdir().unwrap();
        run_in(dir.path(), add("standup", 30, "summarize"))
            .await
            .unwrap();

        let removed = run_in(
            dir.path(),
            LoopCommand::Remove {
                name: "standup".to_string(),
            },
        )
        .await
        .unwrap();
        assert!(removed.contains("已删除 /loop 任务 `standup`"), "{removed}");

        let again = run_in(
            dir.path(),
            LoopCommand::Remove {
                name: "standup".to_string(),
            },
        )
        .await
        .unwrap();
        assert!(again.contains("未找到 /loop 任务 `standup`"), "{again}");
    }

    #[test]
    fn timestamps_fall_back_to_the_raw_value_when_out_of_range() {
        assert_eq!(format_ts(i64::MAX), i64::MAX.to_string());
        // 合理的时间戳至少带上年份，用户才能判断"下次触发"是不是马上
        assert!(format_ts(1_700_000_000).contains("2023"));
    }
}
