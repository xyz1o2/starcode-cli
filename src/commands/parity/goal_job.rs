//! parity 命令族 —— goal_job（对标 Claude Code 命令面，拆自 parity.rs 2026-11-09）

use crate::commands::execution::{CommandContext, CommandResult};
use crate::commands::parity::core::on_off;
use crate::commands::parity::core::push_msg;
use crate::commands::parity::core::run;
use crate::commands::parity::remote::goal_manager;
use crate::commands::parity::remote::goal_status_icon;
use crate::commands::parity::remote::resolve_goal_id;
use crate::commands::parity::remote::short_id;
use crate::runtime::messages::AgentRequest;
use crate::types::ChatEntry;
use std::path::PathBuf;

/// 参数先校验再加载存储；`--` 或 `add` 可显式创建保留词开头的标题。
#[derive(Debug, PartialEq, Eq)]
enum GoalCommand {
    Help,
    List { include_done: bool },
    Create(String),
    Complete(String),
    Remove(String),
    Progress(String, u8),
}

impl GoalCommand {
    fn parse(args: &[String]) -> Result<Self, String> {
        use crate::core::i18n;
        let Some(first) = args.first() else {
            return Ok(Self::List {
                include_done: false,
            });
        };
        let sub = first.to_lowercase();
        let valid_id = || args.get(1).is_some_and(|id| !id.trim().is_empty());
        match sub.as_str() {
            "help" | "--help" | "-h" if args.len() == 1 => Ok(Self::Help),
            "list" | "all" if args.len() == 1 => Ok(Self::List {
                include_done: sub == "all",
            }),
            "done" | "complete" if args.len() == 2 && valid_id() => {
                Ok(Self::Complete(args[1].clone()))
            }
            "rm" | "delete" if args.len() == 2 && valid_id() => Ok(Self::Remove(args[1].clone())),
            "progress" if args.len() == 3 && valid_id() => {
                let value = &args[2];
                let digits = value.strip_suffix('%').unwrap_or(value);
                let progress = digits
                    .parse::<u8>()
                    .ok()
                    .filter(|pct| *pct <= 100 && digits.bytes().all(|b| b.is_ascii_digit()))
                    .ok_or_else(|| {
                        i18n::t1("cmd.goal.invalid_progress").replace("{value}", value)
                    })?;
                Ok(Self::Progress(args[1].clone(), progress))
            }
            "help" | "--help" | "-h" | "list" | "all" | "done" | "complete" | "rm" | "delete"
            | "progress" => Err(i18n::t1("cmd.goal.help")),
            _ => {
                let title_args = if matches!(sub.as_str(), "add" | "--") {
                    &args[1..]
                } else if first.starts_with('-') {
                    return Err(i18n::t1("cmd.goal.help"));
                } else {
                    args
                };
                let title = title_args.join(" ").trim().to_string();
                if title.is_empty() {
                    Err(i18n::t1("cmd.goal.help"))
                } else {
                    Ok(Self::Create(title))
                }
            }
        }
    }
}

/// `/goal` — 查看或修改项目目标，只有持久化成功才显示成功消息。
pub async fn goal(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let out = goal_output(&args, goal_manager).unwrap_or_else(|error| error);
    push_msg(&mut ctx, out);
    Ok(())
}

fn goal_output(
    args: &[String],
    load: impl FnOnce() -> Result<crate::core::goal_tracking::GoalManager, String>,
) -> Result<String, String> {
    use crate::core::goal_tracking::{GoalPriority, GoalStatus};
    use crate::core::i18n;

    let command = GoalCommand::parse(args)?;
    if command == GoalCommand::Help {
        return Ok(i18n::t1("cmd.goal.help"));
    }
    let mut mgr = load()?;
    match command {
        GoalCommand::Help => unreachable!(),
        GoalCommand::List { include_done } => Ok(render_goals(&mgr, include_done)),
        GoalCommand::Create(title) => {
            let id = mgr.create_goal(&title, None, GoalPriority::Medium)?;
            Ok(i18n::t1("cmd.goal.created")
                .replace("{title}", &title)
                .replace("{id}", &short_id(&id)))
        }
        GoalCommand::Complete(prefix) => {
            let id = resolve_goal_id(&mgr, &prefix)?;
            mgr.update_status(&id, GoalStatus::Completed)?;
            Ok(i18n::t1("cmd.goal.completed").replace("{id}", &short_id(&id)))
        }
        GoalCommand::Remove(prefix) => {
            let id = resolve_goal_id(&mgr, &prefix)?;
            mgr.delete_goal(&id)?;
            Ok(i18n::t1("cmd.goal.deleted").replace("{id}", &short_id(&id)))
        }
        GoalCommand::Progress(prefix, progress) => {
            let id = resolve_goal_id(&mgr, &prefix)?;
            mgr.update_progress(&id, progress)?;
            Ok(i18n::t1("cmd.goal.progress_updated")
                .replace("{id}", &short_id(&id))
                .replace("{progress}", &progress.to_string()))
        }
    }
}

#[cfg(test)]
mod goal_tests {
    use super::*;

    #[test]
    fn parser_handles_help_reserved_title_and_strict_arguments() {
        assert_eq!(
            GoalCommand::parse(&[]).unwrap(),
            GoalCommand::List {
                include_done: false
            }
        );
        assert_eq!(
            GoalCommand::parse(&["--help".into()]).unwrap(),
            GoalCommand::Help
        );
        assert_eq!(
            GoalCommand::parse(&["add".into(), "progress".into()]).unwrap(),
            GoalCommand::Create("progress".into())
        );
        assert_eq!(
            GoalCommand::parse(&["--".into(), "done".into(), "my".into(), "goal".into()]).unwrap(),
            GoalCommand::Create("done my goal".into())
        );
        assert!(GoalCommand::parse(&["done".into()]).is_err());
        assert!(GoalCommand::parse(&["list".into(), "unexpected".into()]).is_err());
        for value in ["101", "255", "-1", "30%%"] {
            assert!(GoalCommand::parse(&["progress".into(), "id".into(), value.into()]).is_err());
        }
        assert_eq!(
            GoalCommand::parse(&["progress".into(), "id".into(), "100%".into()]).unwrap(),
            GoalCommand::Progress("id".into(), 100)
        );
    }

    #[test]
    fn help_and_invalid_arguments_do_not_load_storage() {
        assert!(goal_output(&["--help".into()], || panic!("help must not load storage")).is_ok());
        assert!(goal_output(&["done".into()], || panic!(
            "invalid input must not load storage"
        ))
        .is_err());
    }

    #[test]
    fn create_reports_persistence_failure_without_success_text() {
        use crate::core::goal_tracking::GoalManager;
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, "not a directory").unwrap();
        let path = blocker.join("goals.json");
        let result = goal_output(&["A goal".into()], || {
            let mut mgr = GoalManager::new(Some(path.to_str().unwrap()));
            mgr.load()?;
            Ok(mgr)
        });
        assert!(result.unwrap_err().contains("Failed to create directory"));
    }
}

pub(crate) fn render_goals(
    mgr: &crate::core::goal_tracking::GoalManager,
    include_done: bool,
) -> String {
    use crate::core::goal_tracking::GoalStatus;
    let mut goals: Vec<_> = mgr.get_all_goals();
    if !include_done {
        goals.retain(|g| g.status != GoalStatus::Completed && g.status != GoalStatus::Cancelled);
    }
    goals.sort_by_key(|g| -g.created_at);

    if goals.is_empty() {
        return format!(
            "No {}goals yet.\n\nSet one with `/goal <what you are working towards>`.",
            if include_done { "" } else { "active " }
        );
    }

    let mut out = String::from("## Goals\n\n");
    for g in goals {
        out.push_str(&format!(
            "- {} `{}` **{}** — {}%{}\n",
            goal_status_icon(&g.status),
            short_id(&g.id),
            g.title,
            g.progress,
            if g.milestones.is_empty() {
                String::new()
            } else {
                let done = g.milestones.iter().filter(|m| m.completed).count();
                format!(" · milestones {}/{}", done, g.milestones.len())
            }
        ));
    }
    out.push_str(
        "\n`/goal done <id>` · `/goal rm <id>` · `/goal progress <id> <pct>` · `/goal all`\n",
    );
    out
}

/// `/job` — 后台作业：读队列（`.star/remote/inbox.jsonl`）与完成标记（`.star/completed_tasks/`）
pub async fn job(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let sub = args.first().map(|s| s.to_lowercase()).unwrap_or_default();

    match sub.as_str() {
        "" | "list" | "status" => {
            let out = render_jobs(&cwd).await;
            push_msg(&mut ctx, out);
        }
        "add" | "submit" => {
            let body = args[1..].join(" ");
            if body.is_empty() {
                push_msg(&mut ctx, "Usage: `/job add <task description>`");
                return Ok(());
            }
            let out = match crate::core::remote::queue_message(
                &cwd,
                format!("[BackgroundTask: {}]\n{}", body, body),
                Some("slash-job".to_string()),
            )
            .await
            {
                Ok(()) => format!(
                    "📥 Queued background job: **{}**\n\nThe background poller picks it up on its \
                     next tick; watch it with `/job` or `/monitor`.",
                    body
                ),
                Err(e) => format!("⚠️ {}", e),
            };
            push_msg(&mut ctx, out);
        }
        "clear" => {
            let dir = cwd.join(".star").join("completed_tasks");
            let mut removed = 0usize;
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for e in entries.flatten() {
                    if e.path().extension().and_then(|s| s.to_str()) == Some("done")
                        && std::fs::remove_file(e.path()).is_ok()
                    {
                        removed += 1;
                    }
                }
            }
            push_msg(
                &mut ctx,
                format!(
                    "🧹 Cleared {} completion marker(s) in `{}`.",
                    removed,
                    dir.display()
                ),
            );
        }
        _ => push_msg(&mut ctx, "Usage: `/job` | `/job add <task>` | `/job clear`"),
    }
    Ok(())
}

/// 渲染队列中的作业与完成标记
pub(crate) async fn render_jobs(cwd: &std::path::Path) -> String {
    let inbox = crate::core::remote::inbox_file_path(cwd);
    let mut out = String::from("## Background jobs\n\n");

    let raw = tokio::fs::read_to_string(&inbox).await.unwrap_or_default();
    let pending: Vec<crate::core::remote::RemoteRequest> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();

    out.push_str(&format!(
        "**Queued** ({}) — `{}`\n\n",
        pending.len(),
        inbox.display()
    ));
    if pending.is_empty() {
        out.push_str("_queue empty_\n");
    } else {
        for (i, r) in pending.iter().enumerate().take(20) {
            let first = r.message.lines().next().unwrap_or("").trim();
            out.push_str(&format!(
                "{}. `{}` {} — {}\n",
                i + 1,
                if r.source.is_empty() { "?" } else { &r.source },
                chrono::DateTime::from_timestamp(r.created_at, 0)
                    .map(|d| d.format("%m-%d %H:%M").to_string())
                    .unwrap_or_default(),
                first.chars().take(90).collect::<String>()
            ));
        }
    }

    let done_dir = cwd.join(".star").join("completed_tasks");
    let mut done: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&done_dir) {
        for e in entries.flatten() {
            if e.path().extension().and_then(|s| s.to_str()) == Some("done") {
                done.push(
                    e.path()
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string(),
                );
            }
        }
    }
    done.sort();
    out.push_str(&format!("\n**Completed one-shots** ({})\n\n", done.len()));
    for d in done.iter().take(20) {
        out.push_str(&format!("- ✅ {}\n", d));
    }
    if done.is_empty() {
        out.push_str("_none_\n");
    }
    out.push_str("\n`/job add <task>` queues one · `/job clear` drops completion markers\n");
    out
}

/// `/monitor` — 一次性快照：本进程资源、同源进程、后台队列、活跃 agent 任务
pub async fn monitor(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let me = std::process::id();
    let queued = crate::core::remote::queued_count(&cwd).await.unwrap_or(0);

    let self_row = run("ps", &["-o", "pid,pcpu,rss,etime", "-p", &me.to_string()])
        .ok()
        .and_then(|s| s.lines().nth(1).map(|l| l.trim().to_string()));
    let siblings = run(
        "sh",
        &[
            "-c",
            "ps -eo pid,pcpu,rss,etime,comm --no-headers | grep -i star | grep -v grep",
        ],
    )
    .unwrap_or_default();

    let agent_lines = {
        let mut v: Vec<String> = ctx
            .state
            .active_agent_tasks
            .values()
            .map(|t| {
                format!(
                    "- `{}` {} — {:?} · {} tool call(s) · {} tok",
                    t.task_id, t.agent_type, t.status, t.tool_use_count, t.tokens
                )
            })
            .collect();
        v.sort();
        v
    };
    let streaming = ctx.state.is_streaming;
    let tool_running = ctx.state.tool_started_at.len();

    let mut out = String::from("## Monitor\n\n### This session\n\n");
    out.push_str(&format!("- pid: `{}`\n", me));
    if let Some(row) = self_row {
        let f: Vec<&str> = row.split_whitespace().collect();
        if f.len() >= 4 {
            out.push_str(&format!(
                "- cpu: `{}%` · rss: `{} MB` · uptime: `{}`\n",
                f[1],
                f[2].parse::<u64>().unwrap_or(0) / 1024,
                f[3]
            ));
        }
    }
    out.push_str(&format!(
        "- streaming: `{}` · tools in flight: `{}`\n- queued background requests: `{}`\n",
        on_off(streaming),
        tool_running,
        queued
    ));

    out.push_str("\n### Agent tasks\n\n");
    if agent_lines.is_empty() {
        out.push_str("_none active_\n");
    } else {
        for l in &agent_lines {
            out.push_str(l);
            out.push('\n');
        }
    }

    out.push_str("\n### Related processes\n\n");
    let rows: Vec<&str> = siblings.lines().take(12).collect();
    if rows.is_empty() {
        out.push_str("_none found_\n");
    } else {
        out.push_str("```\nPID    %CPU  RSS(kB)  ELAPSED   COMMAND\n");
        for r in rows {
            out.push_str(r.trim());
            out.push('\n');
        }
        out.push_str("```\n");
    }
    push_msg(&mut ctx, out);
    Ok(())
}
