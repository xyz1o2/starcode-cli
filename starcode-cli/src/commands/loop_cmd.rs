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

    match cmd {
        LoopCommand::Add {
            name,
            every_minutes,
            prompt,
        } => {
            let prompt_text = prompt.join(" ").trim().to_string();
            let task = crate::core::loops::add_task(&cwd, name.clone(), every_minutes, prompt_text)
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
            let tasks = crate::core::loops::list_tasks(&cwd)
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
            let removed = crate::core::loops::remove_task(&cwd, &name)
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
