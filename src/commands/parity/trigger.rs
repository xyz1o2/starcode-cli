//! parity 命令族 —— trigger（对标 Claude Code 命令面，拆自 parity.rs 2026-11-09）

use crate::commands::execution::{CommandContext, CommandResult};
use crate::commands::parity::core::push_msg;
use crate::commands::parity::core::run;
use crate::commands::parity::git_github::gh_ready;
use crate::commands::parity::git_github::repo_slug;
use crate::commands::parity::goal_job::render_jobs;
use crate::commands::parity::remote::remote_surface;
use crate::runtime::messages::AgentRequest;
use crate::types::ChatEntry;
use std::path::PathBuf;

/// `/schedule [add <desc> [in <secs>|at <HH:MM>]|list|remove <id>]` — 会话内定时触发
/// （对标 Claude Code 的 /schedule、/triggers）。实现挂在 `.star/triggers.json`，
/// 每个 trigger 是一次性定时，到期时通过 `AgentRequest::SendMessage` 把描述注入主对话。
/// 本项目没有完整的 cron 引擎，这里用会话内轻量调度器覆盖"给未来的自己发一条
/// 消息"的核心语义；持久化 JSON 保证重启后仍能列出（不会在重启后补发）。
pub async fn schedule(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let first = args.first().map(|s| s.to_lowercase()).unwrap_or_default();
    if first == "add" {
        if args.len() < 2 {
            push_msg(
                &mut ctx,
                "Usage: `/schedule add <message> [in <secs>]` or `/schedule add <message> at <HH:MM>`",
            );
            return Ok(());
        }
        let (message, delay) = split_schedule_args(&args[1..]);
        if message.trim().is_empty() {
            push_msg(&mut ctx, "❌ Nothing to schedule: the message is empty.");
            return Ok(());
        }
        match delay {
            Ok(secs) => {
                let id = crate::core::trigger_scheduler::add_trigger(&message, secs);
                let when = format_schedule_when(secs);
                push_msg(
                    &mut ctx,
                    format!("⏰ Scheduled trigger `{}` to fire **{}**.", id, when),
                );
                // 会话内定时：后台 task 睡眠 secs 后把消息注入主对话。触发前重新
                // 校验 `.star/triggers.json` 里仍有这个 id —— 这样 `/schedule remove`
                // 能真正取消已排程的 task（否则被删掉的触发照样会发）。
                let tx = ctx.agent_tx.clone();
                let fire_msg = message.clone();
                let fire_id = id.clone();
                let message_id = ctx.state.next_message_id;
                ctx.state.next_message_id += 1;
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                    // 离线模式下不能悄悄把消息发出去（那会绕过 `enqueue_user_message` 里的
                    // offline 拦截）。等回到在线再发，有上限；超时就放弃并留下日志。
                    if !wait_until_online(&fire_id).await {
                        return;
                    }
                    // take_trigger 是"取出并删除"：既充当取消检查，也避免重复触发
                    if !crate::core::trigger_scheduler::take_trigger(&fire_id) {
                        crate::utils::logging::append_debug_log_line(&format!(
                            "[SCHEDULE] Trigger {} was removed before firing",
                            fire_id
                        ));
                        return;
                    }
                    let _ = tx
                        .send(crate::runtime::messages::AgentRequest::SendMessage {
                            message_id,
                            message: fire_msg,
                        })
                        .await;
                });
            }
            Err(e) => push_msg(&mut ctx, format!("❌ {}", e)),
        }
        return Ok(());
    }
    if first == "remove" || first == "rm" {
        let Some(id) = args.get(1) else {
            push_msg(&mut ctx, "Usage: `/schedule remove <id>`");
            return Ok(());
        };
        match crate::core::trigger_scheduler::remove_trigger(id) {
            true => push_msg(&mut ctx, format!("🗑 Removed trigger `{}`.", id)),
            false => push_msg(&mut ctx, format!("❌ No trigger with id `{}`.", id)),
        }
        return Ok(());
    }
    if first == "list" || first.is_empty() {
        let rows = crate::core::trigger_scheduler::list_triggers();
        let body = if rows.is_empty() {
            "No triggers scheduled.".to_string()
        } else {
            let mut lines = vec!["| id | message | fires at |".to_string()];
            lines.push("|---|---|---|".to_string());
            for (id, message, eta) in rows {
                lines.push(format!("| `{}` | {} | {} |", id, message, eta));
            }
            lines.join("\n")
        };
        push_msg(
            &mut ctx,
            format!(
                "## Scheduled triggers\n\n{}\n\nUse `/schedule add <message> [in <secs>|at <HH:MM>]` and `/schedule remove <id>`.",
                body
            ),
        );
        return Ok(());
    }
    push_msg(&mut ctx, "Usage: `/schedule [add|list|remove]`");
    Ok(())
}

/// `/triggers` — 列出已调度的触发（alias of `/schedule list`）。
pub async fn triggers(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    schedule(ctx, args).await
}

/// 把 `/schedule add` 的参数切成「消息」+「延迟」两部分。
///
/// 尾部的 `in <secs>` / `at <HH:MM>` 是时间修饰，不属于消息正文；早先的实现只过滤
/// 掉 `in`/`at` 这两个词本身，把秒数或时刻留在了消息里（`add ping in 30` → "ping 30"）。
/// 这里按第一个时间关键字切分：之前是消息，之后是延迟参数。
pub(crate) fn split_schedule_args(args: &[String]) -> (String, Result<u64, String>) {
    let split_at = args
        .iter()
        .position(|a| a.eq_ignore_ascii_case("in") || a.eq_ignore_ascii_case("at"));
    match split_at {
        Some(idx) => (args[..idx].join(" "), parse_schedule_delay(&args[idx..])),
        None => (args.join(" "), Ok(DEFAULT_SCHEDULE_DELAY_SECS)),
    }
}

/// `/schedule add` 未指定时间时的默认延迟。
pub(crate) const DEFAULT_SCHEDULE_DELAY_SECS: u64 = 60;

/// 触发到期时若处于离线模式，最多等多久回到在线。
pub(crate) const SCHEDULE_OFFLINE_WAIT_SECS: u64 = 1_800;
pub(crate) const SCHEDULE_OFFLINE_POLL_SECS: u64 = 15;

/// 到期触发落在离线模式里时的处理：轮询等待回到在线，返回 `true` 表示可以发送。
///
/// 超过 `SCHEDULE_OFFLINE_WAIT_SECS` 仍离线就放弃 —— 并把触发从存储里摘掉，否则它会
/// 以一条永不到期的过期记录留在 `.star/triggers.json` 里，直到 stale 清理才消失。
pub(crate) async fn wait_until_online(fire_id: &str) -> bool {
    if !crate::core::offline::is_offline() {
        return true;
    }
    let mut waited = 0;
    while crate::core::offline::is_offline() {
        if waited >= SCHEDULE_OFFLINE_WAIT_SECS {
            crate::core::trigger_scheduler::remove_trigger(fire_id);
            crate::utils::logging::append_debug_log_line(&format!(
                "[SCHEDULE] Trigger {} dropped: still offline after {}s",
                fire_id, waited
            ));
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_secs(SCHEDULE_OFFLINE_POLL_SECS)).await;
        waited += SCHEDULE_OFFLINE_POLL_SECS;
    }
    crate::utils::logging::append_debug_log_line(&format!(
        "[SCHEDULE] Trigger {} held {}s for offline mode, firing now",
        fire_id, waited
    ));
    true
}

/// 把 `in <secs>` / `at <HH:MM>`（今天最近的时刻，已过则算明天）解析成相对秒数。
///
/// 调用方 `split_schedule_args` 保证 `args[0]` 是时间关键字，但这里仍显式拒绝其他词 ——
/// 让这个函数单独看也是完备的，而不是悄悄退回默认延迟（那样 `every 5m` 会被当成
/// "60 秒后"，用户拿不到任何提示）。
pub(crate) fn parse_schedule_delay(args: &[String]) -> Result<u64, String> {
    let keyword = args
        .first()
        .ok_or_else(|| "expected `in <secs>` or `at <HH:MM>`".to_string())?;
    let value = args.get(1);
    if keyword.eq_ignore_ascii_case("in") {
        let secs = value
            .ok_or_else(|| "`in` needs a seconds value".to_string())?
            .parse::<u64>()
            .map_err(|_| "invalid seconds value".to_string())?;
        if secs == 0 {
            return Err("`in` needs a value greater than 0".to_string());
        }
        return Ok(secs);
    }
    if keyword.eq_ignore_ascii_case("at") {
        let hhmm = value.ok_or_else(|| "`at` needs a time like 14:30".to_string())?;
        return crate::core::trigger_scheduler::secs_until_hhmm(hhmm);
    }
    Err(format!(
        "Unknown time modifier `{}` — use `in <secs>` or `at <HH:MM>`",
        keyword
    ))
}

pub(crate) fn format_schedule_when(secs: u64) -> String {
    if secs < 60 {
        format!("in {}s", secs)
    } else {
        format!("in {}s (≈{} min)", secs, secs / 60)
    }
}

/// `/detach` — 对标 Claude Code /detach（把前台任务转后台）。
///
/// 本项目没有"前台转后台"的运行时通道：`AgentRequest` 里没有把当前 turn 移交给
/// 后台 runner 的变体，所以这里**不做任何破坏性动作**，只报告当前状态和可用替代。
/// 早先的实现会顺手发一条 `Abort`，那等于悄悄丢掉在飞的请求 —— 与 detach 的语义
/// 相反（用户想让它继续跑，只是不想盯着），所以已移除。
pub async fn detach(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let mut out = String::from("## Detach\n\n");
    if ctx.state.is_processing || ctx.state.is_streaming {
        out.push_str(
            "- a request is in flight and **keeps running** — `/detach` does not interrupt it\n\
             - press `Esc` if you actually want to abort the current turn\n",
        );
    } else {
        out.push_str("- nothing is running right now\n");
    }
    out.push_str(
        "\nThis build has no \"promote foreground task to background\" channel, so a turn \
         cannot be handed off mid-flight. The background facilities that do exist:\n\n\
         - `/daemon start` — run this CLI headless, draining the remote inbox\n\
         - `/pipes`, `/pipe-status` — inspect the inbox / daemon / job surfaces\n\
         - `/remote` — queue a message for a detached instance to pick up\n\
         - `/schedule add <message> in <secs>` — hand work to your future self\n",
    );
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/pipes` — 列出本会话的后台通道（远程 inbox、bridge 端口、daemon）。
pub async fn pipes(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let mut out = String::from("## Pipes\n\n");
    let remote = remote_surface("remote").await;
    out.push_str(&remote);
    out.push_str("\n### Background jobs\n\n");
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let jobs = render_jobs(&cwd).await;
    out.push_str(&jobs);
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/pipe-status` — 显示某个后台通道的状态；无参数时列出全部可用通道。
pub async fn pipe_status(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let name = args.first().map(|s| s.to_lowercase()).unwrap_or_default();
    if name.is_empty() {
        push_msg(
            &mut ctx,
            "## Pipe status\n\n\
             - `remote` — remote inbox & bridge surface (`/remote status`)\n\
             - `daemon` — background daemon process (`/daemon status`)\n\
             - `jobs` — tracked background jobs (`/job list`)\n\n\
             Usage: `/pipe-status <name>`",
        );
        return Ok(());
    }
    match name.as_str() {
        "remote" => {
            let mut out = String::from("## Remote pipe\n\n");
            out.push_str(&remote_surface("remote").await);
            push_msg(&mut ctx, out);
        }
        "daemon" => {
            let mut out = String::from("## Daemon pipe\n\n");
            // pgrep 会匹配到**正在跑的这个 TUI 自己**，直接展示等于永远报告
            // "daemon 在跑"。所以先剔除自身 pid 再判断。
            // 模式覆盖全部入口名（sc / starcode / starcode-cli 是同一个二进制），
            // 只认 starcode-cli 会漏掉用简称启动的实例。
            let own = std::process::id().to_string();
            let pattern = crate::utils::invocation::process_match_pattern();
            let raw = run("pgrep", &["-f", &pattern]).unwrap_or_default();
            let others: Vec<&str> = raw
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty() && *l != own)
                .collect();
            if others.is_empty() {
                out.push_str(&format!(
                    "- no other StarCode process is running (this TUI is pid {})\n",
                    own
                ));
            } else {
                out.push_str(&format!(
                    "- {} other StarCode process(es), excluding this TUI (pid {}):\n```\n{}\n```\n",
                    others.len(),
                    own,
                    others.join("\n")
                ));
                out.push_str(
                    "- a match is not proof of a daemon: any second StarCode process \
                     (another TUI, a headless `-p` run) shows up here too\n",
                );
            }
            push_msg(&mut ctx, out);
        }
        "jobs" => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            push_msg(
                &mut ctx,
                format!("## Jobs pipe\n\n{}", render_jobs(&cwd).await),
            );
        }
        other => push_msg(
            &mut ctx,
            format!(
                "Unknown pipe `{}`. Known: `remote`, `daemon`, `jobs`.",
                other
            ),
        ),
    }
    Ok(())
}

/// `/break-cache` — 给下一条消息打上"重建上下文"标记（对标 Claude Code /break-cache）。
///
/// 真实效果由 `logic.rs::enqueue_user_message` 兑现：发送前调用
/// `prompts::loader::invalidate_cache()` 丢掉进程内的提示词文件缓存，使系统提示重新读盘，
/// 然后清除标记（一次性）。注意这**不**动 provider 侧的 prompt caching —— 那个由
/// `rig_adapter` 的 `cache_control` 控制，本命令碰不到。
pub async fn break_cache(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    ctx.state.break_cache_next = true;
    push_msg(
        &mut ctx,
        "🛠 Cache break armed for the next message.\n\n\
         Before that message is sent, the in-process prompt cache is dropped so every \
         system-prompt and tool-description `.md` is re-read from disk, and a \
         `[CACHE-BREAK]` breadcrumb is written to `.star/logs/starcode_debug.log`. \
         Useful right after editing a prompt file under `~/.starcode/prompts/` or \
         `./.star/prompts/`.\n\n\
         Provider-side prompt caching is separate and unaffected.",
    );
    Ok(())
}

/// `/autofix-pr` — 为当前分支开一个 PR（对标 Claude Code /autofix-pr）。
///
/// 注意：Claude Code 的同名命令带一个「check → fix → push」自动循环，本 build **没有**
/// 那个循环（没有 CI 轮询器，也没有把失败日志喂回 agent 的驱动器）。所以这里只做能做到的
/// 部分：开 PR，然后指出后续该用哪些命令，措辞不假装有循环。
pub async fn autofix_pr(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    if let Err(e) = gh_ready() {
        push_msg(
            &mut ctx,
            format!(
                "## Autofix PR\n\n`gh` CLI is not available ({}), so `/autofix-pr` cannot open a PR. \
                 Install the GitHub CLI and run `gh auth login`, then retry.\n\n\
                 In this build the command opens a PR for the current branch; it does not run an \
                 automated check→fix→push loop.",
                e
            ),
        );
        return Ok(());
    }
    let repo = repo_slug().ok_or("Not inside a GitHub repository")?;
    let branch = run("git", &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
    let branch = branch.trim();
    let pr = run(
        "gh",
        &[
            "pr",
            "create",
            "--title",
            "Autofix: current branch",
            "--body",
            "PR opened by /autofix-pr.",
        ],
    );
    match pr {
        Ok(url) => {
            let url = url.trim().lines().last().unwrap_or("").to_string();
            push_msg(
                &mut ctx,
                format!(
                    "## Autofix PR\n\n- repo: `{}`\n- branch: `{}`\n- PR: {}\n\n\
                     The PR is open. There is **no automated fix loop** in this build — \
                     nothing polls the checks or pushes follow-up commits on its own. \
                     To iterate: `/subscribe-pr` to watch the checks, then fix and \
                     `/commit-push-pr` (or `/commit`) to update the branch.\n",
                    repo,
                    branch,
                    if url.is_empty() { "(no URL)" } else { &url }
                ),
            );
        }
        Err(e) => push_msg(
            &mut ctx,
            format!(
                "## Autofix PR\n\nPR creation failed: {}\n\nMake sure the current branch is \
                 pushed and that `gh auth status` shows an authenticated account.",
                e
            ),
        ),
    }
    Ok(())
}

/// `/thinkback-play` — 回放最近一次 chain-of-thought（对标 Claude Code /think-back play）。
pub async fn thinkback_play(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    push_msg(
        &mut ctx,
        "## Think-back playback\n\n\
         The think-back recorder keeps a rolling transcript of the last thinking traces. \
         In this build the recorder is not attached to a live session (see `/think-back` \
         for the persistence store), so there is nothing to play back right now.\n\n\
         `N/A` — no recent chain-of-thought recorded for this session.",
    );
    Ok(())
}

/// `/force-snip` — 立即触发一次上下文压缩（对标 Claude Code /force-snip）。
/// 复用 `/compress` 的底层压缩，并显示压缩前后的 token 变化。
pub async fn force_snip(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let before_tokens = ctx
        .state
        .chat_history
        .iter()
        .map(|e| e.content.len())
        .sum::<usize>()
        / 4;
    // 与 `/compress` 走同一个底层：发一条 AgentRequest::Compress（异步返回，
    // 所以精确的压缩后 token 只能由 agent 流式回传），状态行给即时反馈。
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    let _ = ctx
        .agent_tx
        .send(crate::runtime::messages::AgentRequest::Compress {
            message_id,
            instructions: crate::commands::utility::compact_instructions(&args),
        })
        .await;
    ctx.state.current_status_line = Some("Compressing context...".to_string());
    push_msg(
        &mut ctx,
        format!(
            "✂️ Forced context snip requested (~{} estimated chars before). The agent streams the compacted result back; check `/debug state` for exact token counts.",
            before_tokens
        ),
    );
    Ok(())
}
