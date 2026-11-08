//! parity 命令族 —— remote（对标 Claude Code 命令面，拆自 parity.rs 2026-11-09）

use crate::commands::execution::{CommandContext, CommandResult};
use crate::commands::parity::core::on_off;
use crate::commands::parity::core::push_msg;
use crate::commands::parity::core::redact;
use crate::commands::parity::core::run;
use crate::commands::parity::core::star_dir;
use crate::runtime::messages::AgentRequest;
use crate::types::ChatEntry;
use std::path::PathBuf;

/// 收集本机可用于配对的 LAN 地址
pub(crate) fn lan_addresses() -> Vec<String> {
    let raw = run("hostname", &["-I"])
        .or_else(|_| run("sh", &["-c", "ip -4 -o addr | awk '{print $4}'"]))
        .unwrap_or_default();
    raw.split_whitespace()
        .map(|s| s.split('/').next().unwrap_or(s).to_string())
        .filter(|s| !s.starts_with("127.") && !s.is_empty())
        .collect()
}

/// 渲染 bridge / remote inbox 的真实状态（三个命令共用）
pub(crate) async fn remote_surface(client: &str) -> String {
    use crate::core::bridge::BridgeConfig;
    let cfg = BridgeConfig::from_env();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let inbox = crate::core::remote::inbox_file_path(&cwd);
    let queued = crate::core::remote::queued_count(&cwd).await.unwrap_or(0);

    let mut out = String::new();
    out.push_str("### Bridge (websocket / web UI)\n\n");
    out.push_str(&format!("- enabled: `{}`", on_off(cfg.enabled)));
    if !cfg.enabled {
        out.push_str(" — set `STAR_BRIDGE_ENABLED=1`");
    }
    out.push('\n');
    out.push_str(&format!(
        "- port: `{}` · web UI: `{}` on `{}`\n- auth token: `{}` · JWT secret: `{}`\n\
         - max connections: `{}` · session timeout: `{}s`\n",
        cfg.port,
        on_off(cfg.web_ui_enabled),
        cfg.web_ui_port,
        if cfg.auth_token.is_some() {
            "set"
        } else {
            "unset"
        },
        if cfg.jwt_secret.is_some() {
            "set"
        } else {
            "unset"
        },
        cfg.max_connections,
        cfg.session_timeout_secs
    ));

    out.push_str("\n### Remote inbox (always available)\n\n");
    out.push_str(&format!(
        "- file: `{}`\n- queued requests: `{}`\n",
        inbox.display(),
        queued
    ));

    let addrs = lan_addresses();
    if !addrs.is_empty() && cfg.web_ui_enabled {
        out.push_str(&format!("\n### Reach this host from your {}\n\n", client));
        for a in addrs.iter().take(4) {
            out.push_str(&format!("- `http://{}:{}`\n", a, cfg.web_ui_port));
        }
    }
    out
}

/// `/agent-mail` — Agent 间邮件互通状态（Phase 5.4b）。
///
/// 展示三条真实信息：入站 listener 配置（env 读数的真相，不是猜测）、
/// 出站 peers 列表、以及可选的对端探测（`/agent-mail probe` 逐对端拉
/// well-known 卡片核实对端确实是 agent-mail 服务）。
pub async fn agent_mail(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    use crate::agent::messaging::remote::{MailServerConfig, PeerMailClient};

    let sub = args.first().map(|s| s.as_str()).unwrap_or("status");

    // `/agent-mail agents` — 列出隐式 team 的 live 成员（对标 CC 的
    // `ListAgents`/`/list-agents` 列 live teammates）。读的是会话 worker 已
    // 注册的 hub —— UI 层没有 hub 引用，走运行时快照放在 ChatState 的
    // AgentMail 快照（由 stream.rs 维护）不可用时的 env 兜底说明。
    if sub == "agents" {
        let mut out = String::from("## Agent team members\n\n");
        match ctx.state.agent_team_snapshot.as_ref() {
            Some(snapshot) if !snapshot.is_empty() => {
                out.push_str(&format!(
                    "{:<22} {:<12} {}\n",
                    "IDENTITY", "STATUS", "MAILBOX PENDING"
                ));
                for m in snapshot.iter() {
                    out.push_str(&format!(
                        "{:<22} {:<12} {}\n",
                        m.identity, m.status, m.mailbox_pending
                    ));
                }
                out.push_str(
                    "\nSendMessage target_agent=<identity> to message one; \
                     `lead` is the main session's mailbox.\n",
                );
            }
            _ => {
                out.push_str(
                    "No hub snapshot available in this session (headless mode or the runtime \
                     has not registered any agents yet). Named background agents register on \
                     spawn: run the `Agent` tool with `background: true` and a `name`, then \
                     check again.\n",
                );
            }
        }
        push_msg(&mut ctx, out);
        return Ok(());
    }

    if sub == "probe" {
        // 探测模式：逐对端拉 well-known 卡片（3s 超时/对端）。
        let client = PeerMailClient::from_env();
        if !client.has_peers() {
            push_msg(
                &mut ctx,
                "## AgentMail probe\n\nNo peers configured. Set `STAR_AGENT_PEERS` \
                 (comma-separated base URLs) and restart the session.",
            );
            return Ok(());
        }
        let peers = client.peer_base_urls();
        let cards = crate::agent::messaging::remote::discover_peers(&peers).await;
        let mut out = String::from("## AgentMail peers\n\n");
        for card in cards {
            match card.status {
                Ok(()) => out.push_str(&format!(
                    "- ✅ `{}` — agent-mail v{}, inbox `{}`, auth `{}`\n",
                    card.base_url,
                    card.version.unwrap_or(0),
                    card.inbox_path.as_deref().unwrap_or("?"),
                    card.auth.as_deref().unwrap_or("?"),
                )),
                Err(reason) => out.push_str(&format!("- ❌ `{}` — {}\n", card.base_url, reason)),
            }
        }
        push_msg(&mut ctx, out);
        return Ok(());
    }

    // status（默认）
    let cfg = MailServerConfig::from_env();
    let client = PeerMailClient::from_env();
    let mut out = String::from("## AgentMail（agent 间消息互通）\n\n");
    out.push_str(&format!(
        "- inbound listener: **{}** (`{}:{}`, key **{}**)\n",
        if cfg.enabled { "enabled" } else { "disabled" },
        cfg.host,
        cfg.port,
        if cfg.shared_key.is_some() {
            "set"
        } else {
            "unset"
        },
    ));
    if !cfg.enabled {
        out.push_str(
            "  - enable with `STAR_AGENT_MAIL_ENABLED=1`; it binds loopback only unless \
             `STAR_AGENT_MAIL_HOST` is set (non-loopback binding requires a key)\n",
        );
    }
    let peers = client.peer_base_urls();
    if peers.is_empty() {
        out.push_str(
            "- outbound peers: none (`STAR_AGENT_PEERS` unset) — single-process semantics\n",
        );
    } else {
        out.push_str(&format!(
            "- outbound peers ({}): {} — run `/agent-mail probe` to verify each\n",
            peers.len(),
            peers
                .iter()
                .map(|p| format!("`{}`", p))
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    out.push_str(
        "\n### Semantics\n\n\
         - in-process agents (lead / workers / forks) exchange messages through the \
           internal hub; `SendMessage target_agent=\"lead\"` is the reply path\n\
         - unknown targets error explicitly (no dead-letter files)\n\
         - remote messages land in the local mailbox and are consumed at the start \
           of the agent's next turn; an idle lead is woken by the mail signal\n",
    );
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/remote-control` — 展示本会话可被远程驱动的两条真实通路
pub async fn remote_control(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let mut out = String::from("## Remote control\n\n");
    out.push_str(&remote_surface("device").await);
    out.push_str(
        "\n### Drive this session\n\n\
         - `/remote send <message>` — queue a request into the inbox\n\
         - `/remote status` · `/remote drain` · `/remote protocol`\n\
         - external writers can append JSONL to the inbox file directly; \
         the background poller drains it into this session\n",
    );
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/mobile` — 手机配对指引（读取真实 bridge 配置与 LAN 地址）
pub async fn mobile(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let mut out = String::from("## Connect a mobile device\n\n");
    out.push_str(&remote_surface("phone").await);
    out.push_str(
        "\n### Steps\n\n\
         1. `export STAR_BRIDGE_ENABLED=1` and set `STAR_BRIDGE_AUTH_TOKEN` to a random secret\n\
         2. restart this CLI so the bridge picks up the env\n\
         3. open the web UI URL above on the phone (same Wi-Fi) and paste the token\n\n\
         Without the bridge, a phone can still drive the session by appending to the \
         remote inbox over SSH (`/remote protocol` shows the JSONL shape).\n",
    );
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/desktop` — 桌面客户端连接指引
pub async fn desktop(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let mut out = String::from("## Connect the desktop app\n\n");
    out.push_str(&remote_surface("desktop app").await);
    out.push_str(
        "\n### Steps\n\n\
         1. enable the bridge (`STAR_BRIDGE_ENABLED=1`, `STAR_BRIDGE_AUTH_TOKEN=<secret>`)\n\
         2. point the desktop client at `ws://<host>:<port>` using the port above\n\
         3. verify with `/peers` (mailbox peers) and `/remote status` (queued requests)\n",
    );
    push_msg(&mut ctx, out);
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// 目标 / 后台作业 / 进程
// ═══════════════════════════════════════════════════════════════════════════

/// 项目级 goal 存储（`.star/goals.json`），比默认的 `~/.starcode/goals.json` 更贴合仓库
pub(crate) fn goal_manager() -> Result<crate::core::goal_tracking::GoalManager, String> {
    let path = star_dir()?.join("goals.json");
    let mut mgr =
        crate::core::goal_tracking::GoalManager::new(Some(path.to_string_lossy().as_ref()));
    mgr.load()?;
    Ok(mgr)
}

pub(crate) fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

/// 按 id 前缀解析目标；返回完整 id
pub(crate) fn resolve_goal_id(
    mgr: &crate::core::goal_tracking::GoalManager,
    prefix: &str,
) -> Result<String, String> {
    let hits: Vec<String> = mgr
        .get_all_goals()
        .iter()
        .filter(|g| g.id.starts_with(prefix))
        .map(|g| g.id.clone())
        .collect();
    match hits.len() {
        1 => Ok(hits[0].clone()),
        0 => Err(format!("No goal matching `{}`.", prefix)),
        n => Err(format!(
            "`{}` matches {} goals — use more characters.",
            prefix, n
        )),
    }
}

pub(crate) fn goal_status_icon(status: &crate::core::goal_tracking::GoalStatus) -> &'static str {
    use crate::core::goal_tracking::GoalStatus as S;
    match status {
        S::InProgress => "▶",
        S::Completed => "✅",
        S::Paused => "⏸",
        S::Cancelled => "✖",
    }
}

/// `/daemon` — 本进程内真实运行的后台循环（git 状态、/loop 调度、remote inbox）
pub async fn daemon(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let cfg = crate::core::daemon::DaemonConfig::from_env();
    let queued = crate::core::remote::queued_count(&cwd).await.unwrap_or(0);
    let loop_tasks = crate::core::loops::list_tasks(&cwd)
        .await
        .unwrap_or_default();

    if args.first().map(|s| s.to_lowercase()).as_deref() == Some("tasks") {
        let mut out = String::from("## Daemon: scheduled /loop tasks\n\n");
        if loop_tasks.is_empty() {
            out.push_str(&format!(
                "_no tasks_ — file: `{}`\n\nAdd one with `/loop`.\n",
                crate::core::loops::loops_file_path(&cwd).display()
            ));
        } else {
            for t in &loop_tasks {
                out.push_str(&format!(
                    "- **{}** — every {} min · {} · last run {}\n",
                    t.name,
                    t.interval_minutes,
                    if t.enabled { "enabled" } else { "disabled" },
                    t.last_run_at
                        .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
                        .map(|d| d.format("%m-%d %H:%M").to_string())
                        .unwrap_or_else(|| "never".to_string())
                ));
            }
        }
        push_msg(&mut ctx, out);
        return Ok(());
    }

    let mut out = String::from("## Daemon\n\n### In-process loops (always on)\n\n");
    out.push_str(&format!(
        "| loop | cadence | state |\n|---|---|---|\n\
         | git status | 5s (backs off to 60s) | running |\n\
         | /loop scheduler | 1s tick | {} task(s) registered |\n\
         | remote inbox | 1s tick (500ms budget) | {} queued |\n",
        loop_tasks.len(),
        queued
    ));

    out.push_str("\n### Worker-pool config (`STAR_DAEMON_*`)\n\n");
    out.push_str(&format!(
        "- enabled: `{}` · max workers: `{}`\n- max runtime: `{}s` · health check: `{}s`\n\
         - pid file: `{}` · log file: `{}`\n",
        on_off(cfg.enabled),
        cfg.max_workers,
        cfg.max_runtime_secs,
        cfg.health_check_interval_secs,
        cfg.pid_file.as_deref().unwrap_or("-"),
        cfg.log_file.as_deref().unwrap_or("-")
    ));
    out.push_str("\n`/daemon tasks` lists scheduled work · `/job` shows the queue\n");
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/remote-env` — 显示当前会话有效环境变量（含远程/网桥相关的 STAR_* 变量），
/// 与 `/env` 类似但对齐 Claude Code 的 `/remote-env` 命名。
pub async fn remote_env(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if args.first().map(|s| s.as_str()) == Some("--json") {
        let mut map = serde_json::Map::new();
        for (k, v) in std::env::vars() {
            if k.starts_with("STAR_") || k.starts_with("ANTHROPIC_") {
                map.insert(k.clone(), serde_json::Value::String(redact(&k, &v)));
            }
        }
        push_msg(&mut ctx, serde_json::json!({ "env": map }).to_string());
        return Ok(());
    }
    let mut rows = Vec::new();
    for (k, v) in std::env::vars() {
        if k.starts_with("STAR_") || k.starts_with("ANTHROPIC_") {
            rows.push(format!("- `{}` = {}", k, redact(&k, &v)));
        }
    }
    if rows.is_empty() {
        push_msg(
            &mut ctx,
            "No `STAR_*` / `ANTHROPIC_*` environment variables are set.",
        );
    } else {
        push_msg(
            &mut ctx,
            format!("## Remote environment\n\n{}\n", rows.join("\n")),
        );
    }
    Ok(())
}

/// `/remote-control-server` — 启动/查看远程控制服务器（对标 Claude Code 的 server 模式）。
/// 本项目没有常驻监听 socket，这里给出桥接/daemon 的诚实指引。
pub async fn remote_control_server(
    mut ctx: CommandContext<'_>,
    _args: Vec<String>,
) -> CommandResult {
    let mut out = String::from("## Remote control server\n\n");
    out.push_str(
        "This build does not run a standalone control-plane socket. The nearest equivalents are:\n\n\
         - `/remote status` — the inbox bridge and LAN port (when `STAR_BRIDGE_ENABLED=1`)\n\
         - `/daemon start` — run this CLI as a background daemon that drains the inbox\n\
         - `/rc` / `/rcs` — the client-facing surface for the same bridge\n\n\
         Start a headless daemon with:\n\n\
         ```bash\n\
         STAR_BRIDGE_ENABLED=1 STAR_BRIDGE_AUTH_TOKEN=<secret> starcode-cli daemon\n\
         ```\n",
    );
    push_msg(&mut ctx, out);
    Ok(())
}
