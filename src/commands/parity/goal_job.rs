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

/// `/goal` — 查看/设置当前目标（项目级持久化）
pub async fn goal(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    use crate::core::goal_tracking::{GoalPriority, GoalStatus};

    let mut mgr = match goal_manager() {
        Ok(m) => m,
        Err(e) => {
            push_msg(&mut ctx, format!("⚠️ {}", e));
            return Ok(());
        }
    };

    let sub = args.first().map(|s| s.to_lowercase()).unwrap_or_default();
    let out = match sub.as_str() {
        "" | "list" | "all" => render_goals(&mgr, sub == "all"),
        "done" | "complete" => match args.get(1) {
            None => "Usage: `/goal done <id>`".to_string(),
            Some(p) => match resolve_goal_id(&mgr, p) {
                Err(e) => e,
                Ok(id) => match mgr.update_status(&id, GoalStatus::Completed) {
                    Ok(()) => {
                        let _ = mgr.save();
                        format!("✅ Completed `{}`.", short_id(&id))
                    }
                    Err(e) => format!("⚠️ {}", e),
                },
            },
        },
        "rm" | "delete" => match args.get(1) {
            None => "Usage: `/goal rm <id>`".to_string(),
            Some(p) => match resolve_goal_id(&mgr, p) {
                Err(e) => e,
                Ok(id) => match mgr.delete_goal(&id) {
                    Ok(()) => {
                        let _ = mgr.save();
                        format!("🗑 Deleted `{}`.", short_id(&id))
                    }
                    Err(e) => format!("⚠️ {}", e),
                },
            },
        },
        "progress" => goal_set_progress(&mut mgr, &args),
        _ => {
            let title = args.join(" ");
            let priority = GoalPriority::Medium;
            let id = mgr.create_goal(&title, None, priority);
            format!(
                "🎯 Goal set: **{}** (`{}`)\n\nUpdate with `/goal progress {} 50`, \
                 finish with `/goal done {}`.",
                title,
                short_id(&id),
                short_id(&id),
                short_id(&id)
            )
        }
    };
    push_msg(&mut ctx, out);
    Ok(())
}

pub(crate) fn goal_set_progress(
    mgr: &mut crate::core::goal_tracking::GoalManager,
    args: &[String],
) -> String {
    let (Some(p), Some(v)) = (args.get(1), args.get(2)) else {
        return "Usage: `/goal progress <id> <0-100>`".to_string();
    };
    let Ok(pct) = v.trim_end_matches('%').parse::<u8>() else {
        return format!("`{}` is not a percentage (0-100).", v);
    };
    match resolve_goal_id(mgr, p) {
        Err(e) => e,
        Ok(id) => match mgr.update_progress(&id, pct.min(100)) {
            Ok(()) => {
                let _ = mgr.save();
                format!("📈 `{}` → {}%", short_id(&id), pct.min(100))
            }
            Err(e) => format!("⚠️ {}", e),
        },
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
    out.push_str("\n`/goal done <id>` · `/goal progress <id> <pct>` · `/goal all`\n");
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
