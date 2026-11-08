//! parity 命令族 —— team_mail（对标 Claude Code 命令面，拆自 parity.rs 2026-11-09）

use crate::commands::execution::{CommandContext, CommandResult};
use crate::commands::parity::core::push_msg;
use crate::commands::parity::core::run;
use crate::runtime::messages::AgentRequest;
use crate::types::ChatEntry;
use std::path::PathBuf;

/// `~/.star/teams` 下的 mailbox 目录
pub(crate) fn teams_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".star")
        .join("teams")
}

/// 扫描所有 team 的 inbox，返回 (team, agent, unread, total)
pub(crate) fn scan_mailboxes() -> Vec<(String, String, usize, usize)> {
    let mgr = crate::core::swarm::mailbox::MailboxManager::new();
    let mut rows = Vec::new();
    let Ok(teams) = std::fs::read_dir(teams_dir()) else {
        return rows;
    };
    for team in teams.flatten() {
        if !team.path().is_dir() {
            continue;
        }
        let team_name = team.file_name().to_string_lossy().to_string();
        let Ok(inboxes) = std::fs::read_dir(team.path().join("inboxes")) else {
            continue;
        };
        for inbox in inboxes.flatten() {
            let file = inbox.file_name().to_string_lossy().to_string();
            let Some(agent) = file.strip_suffix(".json") else {
                continue;
            };
            if let Ok(mb) = mgr.read_mailbox(&team_name, agent) {
                let unread = mb.messages.iter().filter(|m| !m.read).count();
                rows.push((
                    team_name.clone(),
                    agent.to_string(),
                    unread,
                    mb.messages.len(),
                ));
            }
        }
    }
    rows.sort();
    rows
}

pub(crate) fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// `/peers` — 列出本机 swarm mailbox 里可见的对等实例
pub async fn peers(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let rows = scan_mailboxes();
    let mut out = String::from("## Peers\n\n");

    if rows.is_empty() {
        out.push_str(&format!(
            "No peer mailboxes found under `{}`.\n\nPeers appear once a team is created \
             (`/swarm`) or another instance writes to a mailbox.\n",
            teams_dir().display()
        ));
        push_msg(&mut ctx, out);
        return Ok(());
    }

    let want_team = args.first().cloned();
    out.push_str("| team | agent | unread | total |\n|---|---|---:|---:|\n");
    let mut shown = 0usize;
    for (team, agent, unread, total) in &rows {
        if let Some(t) = &want_team {
            if team != t {
                continue;
            }
        }
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            team, agent, unread, total
        ));
        shown += 1;
    }
    if shown == 0 {
        out.push_str(&format!(
            "| _no peers in team `{}`_ | | | |\n",
            want_team.unwrap_or_default()
        ));
    }
    out.push_str("\nSend with `/send [team/]<agent> <message>`.\n");
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/send [team/]<agent> <message>` — 往对等实例的 mailbox 投递一条消息
pub async fn send(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if args.len() < 2 {
        push_msg(
            &mut ctx,
            "Usage: `/send [team/]<agent> <message>`\n\nRun `/peers` to see available targets.",
        );
        return Ok(());
    }

    let target = args[0].clone();
    let body = args[1..].join(" ");
    let rows = scan_mailboxes();

    // 目标可写成 team/agent，也可只写 agent（此时按 mailbox 唯一匹配推断 team）
    let (team, agent) = match target.split_once('/') {
        Some((t, a)) => (t.to_string(), a.to_string()),
        None => {
            let hits: Vec<&(String, String, usize, usize)> =
                rows.iter().filter(|(_, a, _, _)| *a == target).collect();
            match hits.len() {
                1 => (hits[0].0.clone(), target.clone()),
                0 => {
                    push_msg(
                        &mut ctx,
                        format!(
                            "No mailbox for agent `{}`. Use `team/agent`, or run `/peers`.",
                            target
                        ),
                    );
                    return Ok(());
                }
                _ => {
                    let teams: Vec<String> = hits.iter().map(|h| h.0.clone()).collect();
                    push_msg(
                        &mut ctx,
                        format!(
                            "Agent `{}` exists in multiple teams ({}). Use `team/agent`.",
                            target,
                            teams.join(", ")
                        ),
                    );
                    return Ok(());
                }
            }
        }
    };

    deliver_mailbox_message(&mut ctx, &team, &agent, body).await;
    Ok(())
}

/// 真正写 mailbox 的公共实现（`/send`、`/bridge-kick` 共用）
pub(crate) async fn deliver_mailbox_message(
    ctx: &mut CommandContext<'_>,
    team: &str,
    agent: &str,
    body: String,
) {
    use crate::core::swarm::mailbox::{
        generate_message_id, MailboxManager, MailboxMessage, MessageType,
    };

    let broadcast = agent == "*";
    let msg = MailboxMessage {
        id: generate_message_id(),
        from: "main".to_string(),
        to: agent.to_string(),
        message_type: if broadcast {
            MessageType::Broadcast
        } else {
            MessageType::PlainText
        },
        content: body.clone(),
        summary: Some(body.chars().take(80).collect()),
        timestamp_ms: now_ms(),
        read: false,
        color: None,
    };

    let mgr = MailboxManager::new();
    let targets: Vec<String> = if broadcast {
        scan_mailboxes()
            .into_iter()
            .filter(|(t, a, _, _)| t == team && a != "main")
            .map(|(_, a, _, _)| a)
            .collect()
    } else {
        vec![agent.to_string()]
    };

    if targets.is_empty() {
        push_msg(ctx, format!("No recipients in team `{}`.", team));
        return;
    }

    let mut ok = Vec::new();
    let mut failed = Vec::new();
    for to in &targets {
        let mut m = msg.clone();
        m.id = generate_message_id();
        m.to = to.clone();
        match mgr.send_message(team, to, m) {
            Ok(()) => ok.push(to.clone()),
            Err(e) => failed.push(format!("{}: {}", to, e)),
        }
    }

    let mut out = String::new();
    if !ok.is_empty() {
        out.push_str(&format!(
            "📮 Delivered to `{}/{}` ({} char{}).\n",
            team,
            ok.join("`, `"),
            body.len(),
            if body.len() == 1 { "" } else { "s" }
        ));
    }
    for f in &failed {
        out.push_str(&format!("⚠️ {}\n", f));
    }
    push_msg(ctx, out);
}

/// main 实例租约文件：`~/.star/teams/<team>/main-claim.json`
pub(crate) fn claim_file(team: &str) -> PathBuf {
    teams_dir().join(team).join("main-claim.json")
}

/// 判断 pid 是否还活着（Linux 下看 /proc）
pub(crate) fn pid_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{}", pid)).exists()
}

pub(crate) fn read_claim(team: &str) -> Option<serde_json::Value> {
    let raw = std::fs::read_to_string(claim_file(team)).ok()?;
    serde_json::from_str(&raw).ok()
}

/// `/claim-main [team] [--force]` — 声明本进程为该 team 的 main 实例
pub async fn claim_main(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let force = args.iter().any(|a| a == "--force" || a == "-f");
    let team = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .cloned()
        .unwrap_or_else(|| "default".to_string());

    let me = std::process::id();
    let existing = read_claim(&team);

    if let Some(prev) = &existing {
        let pid = prev.get("pid").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let alive = pid != 0 && pid_alive(pid);
        if pid == me {
            push_msg(
                &mut ctx,
                format!(
                    "✅ This process (pid {}) already holds main for `{}`.",
                    me, team
                ),
            );
            return Ok(());
        }
        if alive && !force {
            let cwd = prev.get("cwd").and_then(|v| v.as_str()).unwrap_or("?");
            push_msg(
                &mut ctx,
                format!(
                    "🔒 main for `{}` is held by a live process (pid {}, cwd `{}`).\n\n\
                     Run `/claim-main {} --force` to take over, or `/bridge-kick` it first.",
                    team, pid, cwd, team
                ),
            );
            return Ok(());
        }
    }

    write_claim(&mut ctx, &team, me, existing);
    Ok(())
}

/// 落盘 main 租约并回报结果
pub(crate) fn write_claim(
    ctx: &mut CommandContext<'_>,
    team: &str,
    pid: u32,
    previous: Option<serde_json::Value>,
) {
    let path = claim_file(team);
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            push_msg(
                ctx,
                format!("⚠️ Cannot create `{}`: {}", parent.display(), e),
            );
            return;
        }
    }
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let branch = run("git", &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "-".to_string());
    let payload = serde_json::json!({
        "pid": pid,
        "cwd": cwd,
        "branch": branch,
        "host": std::env::var("HOSTNAME").unwrap_or_else(|_| "localhost".to_string()),
        "claimed_at_ms": now_ms() as u64,
    });
    match serde_json::to_string_pretty(&payload)
        .map_err(|e| e.to_string())
        .and_then(|s| std::fs::write(&path, s).map_err(|e| e.to_string()))
    {
        Ok(()) => {
            let mut out = format!(
                "👑 Claimed **main** for team `{}` (pid {}).\n\n- claim file: `{}`\n- working tree: `{}` on `{}`\n",
                team,
                pid,
                path.display(),
                payload["cwd"].as_str().unwrap_or("?"),
                payload["branch"].as_str().unwrap_or("-")
            );
            if let Some(prev) = previous {
                let old = prev.get("pid").and_then(|v| v.as_u64()).unwrap_or(0);
                out.push_str(&format!("- took over from pid {}\n", old));
            }
            out.push_str("\nPeers addressed as `main` in `/send` now resolve to this instance.\n");
            push_msg(ctx, out);
        }
        Err(e) => push_msg(ctx, format!("⚠️ Failed to write claim: {}", e)),
    }
}

/// `/bridge-kick [team/]<agent>` — 向对等实例投递 ShutdownRequest；
/// `/bridge-kick --release [team]` 释放已失效的 main 租约。
pub async fn bridge_kick(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    use crate::core::swarm::mailbox::{
        generate_message_id, MailboxManager, MailboxMessage, MessageType,
    };

    if args.iter().any(|a| a == "--release") {
        let team = args
            .iter()
            .find(|a| !a.starts_with('-'))
            .cloned()
            .unwrap_or_else(|| "default".to_string());
        let out = match read_claim(&team) {
            None => format!("No main claim recorded for team `{}`.", team),
            Some(prev) => {
                let pid = prev.get("pid").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                if pid != 0 && pid_alive(pid) && pid != std::process::id() {
                    format!(
                        "🔒 pid {} is still alive — kick it first (`/bridge-kick <agent>`) \
                         or use `/claim-main {} --force`.",
                        pid, team
                    )
                } else {
                    match std::fs::remove_file(claim_file(&team)) {
                        Ok(()) => format!("🔓 Released main claim for `{}` (pid {}).", team, pid),
                        Err(e) => format!("⚠️ {}", e),
                    }
                }
            }
        };
        push_msg(&mut ctx, out);
        return Ok(());
    }

    let Some(target) = args.first().cloned() else {
        let rows = scan_mailboxes();
        let mut out = String::from(
            "Usage: `/bridge-kick [team/]<agent>` | `/bridge-kick --release [team]`\n\n",
        );
        if rows.is_empty() {
            out.push_str("No peer mailboxes to kick.\n");
        } else {
            out.push_str("Known peers:\n");
            for (t, a, _, _) in rows {
                out.push_str(&format!("- `{}/{}`\n", t, a));
            }
        }
        push_msg(&mut ctx, out);
        return Ok(());
    };

    let (team, agent) = split_target(&target);
    let msg = MailboxMessage {
        id: generate_message_id(),
        from: "main".to_string(),
        to: agent.clone(),
        message_type: MessageType::ShutdownRequest,
        content: "Shutdown requested by main instance (/bridge-kick).".to_string(),
        summary: Some("shutdown requested".to_string()),
        timestamp_ms: now_ms(),
        read: false,
        color: None,
    };
    let out = match MailboxManager::new().send_message(&team, &agent, msg) {
        Ok(()) => format!(
            "🔌 Sent ShutdownRequest to `{}/{}`.\n\nThe peer disconnects on its next mailbox poll.",
            team, agent
        ),
        Err(e) => format!("⚠️ Failed to kick `{}/{}`: {}", team, agent, e),
    };
    push_msg(&mut ctx, out);
    Ok(())
}

/// 把 `team/agent` 或裸 `agent` 拆成 (team, agent)
pub(crate) fn split_target(target: &str) -> (String, String) {
    match target.split_once('/') {
        Some((t, a)) => (t.to_string(), a.to_string()),
        None => {
            let rows = scan_mailboxes();
            let team = rows
                .iter()
                .find(|(_, a, _, _)| a == target)
                .map(|(t, _, _, _)| t.clone())
                .unwrap_or_else(|| "default".to_string());
            (team, target.to_string())
        }
    }
}

/// `/coordinator` — 协调状态：谁是 main、并行度、活跃 agent 任务、mailbox 对等
pub async fn coordinator(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    match args.first().map(|s| s.to_lowercase()).as_deref() {
        Some("claim") => return claim_main(ctx, args[1..].to_vec()).await,
        Some("release") => {
            let mut rest = vec!["--release".to_string()];
            rest.extend(args[1..].iter().cloned());
            return bridge_kick(ctx, rest).await;
        }
        _ => {}
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let queued = crate::core::remote::queued_count(&cwd).await.unwrap_or(0);
    let peers = scan_mailboxes();
    let cfg = crate::core::coordinator::CoordinatorConfig::default();

    let (tasks, group) = {
        let mut v: Vec<String> = ctx
            .state
            .active_agent_tasks
            .values()
            .map(|t| format!("- `{}` {} — {:?}", t.task_id, t.agent_type, t.status))
            .collect();
        v.sort();
        (v, ctx.state.agent_group_id.clone())
    };

    let mut out = String::from("## Coordinator\n\n");
    out.push_str(&format!(
        "- parallel tool slots: `4` (streaming executor)\n\
         - default worker pool: `{}` workers · task timeout `{}s` · strategy `{:?}`\n\
         - queued background requests: `{}`\n",
        cfg.max_workers, cfg.task_timeout_secs, cfg.load_balancing_strategy, queued
    ));
    if let Some(g) = group {
        out.push_str(&format!("- active agent group: `{}`\n", g));
    }

    out.push_str("\n### main claim\n\n");
    let mut teams: Vec<String> = peers.iter().map(|(t, _, _, _)| t.clone()).collect();
    teams.dedup();
    if teams.is_empty() {
        teams.push("default".to_string());
    }
    for t in &teams {
        match read_claim(t) {
            Some(c) => {
                let pid = c.get("pid").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                out.push_str(&format!(
                    "- `{}` → pid {} ({}) on `{}`\n",
                    t,
                    pid,
                    if pid_alive(pid) { "alive" } else { "stale" },
                    c.get("branch").and_then(|v| v.as_str()).unwrap_or("-")
                ));
            }
            None => out.push_str(&format!("- `{}` → unclaimed\n", t)),
        }
    }

    out.push_str("\n### agent tasks\n\n");
    if tasks.is_empty() {
        out.push_str("_none active_\n");
    } else {
        for t in &tasks {
            out.push_str(t);
            out.push('\n');
        }
    }
    out.push_str(&format!(
        "\n### peers\n\n{} mailbox(es) — see `/peers`\n",
        peers.len()
    ));
    out.push_str("\n`/coordinator claim [team]` · `/coordinator release [team]`\n");
    push_msg(&mut ctx, out);
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// 记忆库 / 凭据保管
// ═══════════════════════════════════════════════════════════════════════════
