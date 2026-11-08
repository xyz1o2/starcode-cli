//! parity 命令族 —— diag（对标 Claude Code 命令面，拆自 parity.rs 2026-11-09）

use crate::commands::execution::{CommandContext, CommandResult};
use crate::commands::parity::core::approval_label;
use crate::commands::parity::core::ask_agent;
use crate::commands::parity::core::autocompact;
use crate::commands::parity::core::format_token_count;
use crate::commands::parity::core::on_off;
use crate::commands::parity::core::parse_autocompact_tokens;
use crate::commands::parity::core::push_msg;
use crate::commands::parity::core::run;
use crate::commands::parity::core::star_dir;
use crate::commands::parity::git_github::repo_slug;
use crate::commands::parity::memory::dir_stats;
use crate::commands::parity::memory::human_bytes;
use crate::commands::parity::memory::store_row;
use crate::commands::parity::team_mail::now_ms;
use crate::commands::parity::trigger::format_schedule_when;
use crate::commands::parity::trigger::parse_schedule_delay;
use crate::commands::parity::trigger::split_schedule_args;
use crate::commands::parity::trigger::DEFAULT_SCHEDULE_DELAY_SECS;
use crate::runtime::messages::AgentRequest;
use crate::types::ChatEntry;
use std::path::PathBuf;

/// `/insights` — 本次会话的使用洞察（token、缓存命中、工具分布、耗时）。
pub async fn insights(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let s = &ctx.state;
    let user_msgs = s
        .chat_history
        .iter()
        .filter(|e| matches!(e.entry_type, crate::types::ChatEntryType::User))
        .count();
    let assistant_msgs = s
        .chat_history
        .iter()
        .filter(|e| matches!(e.entry_type, crate::types::ChatEntryType::Assistant) && !e.is_welcome)
        .count();
    let tool_entries = s
        .chat_history
        .iter()
        .filter(|e| e.tool_call.is_some())
        .count();
    let thinking_blocks = s
        .chat_history
        .iter()
        .filter(|e| e.reasoning_content.is_some())
        .count();

    let mut tool_counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for t in &s.tools_used {
        *tool_counts.entry(t.as_str()).or_insert(0) += 1;
    }
    let mut top: Vec<(&str, usize)> = tool_counts.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));

    let mut out = String::from("# Session insights\n\n## Volume\n");
    out.push_str(&format!(
        "- messages: **{}** from you, **{}** from the model\n- tool calls: **{}**\n- thinking blocks: **{}**\n",
        user_msgs, assistant_msgs, tool_entries, thinking_blocks
    ));

    out.push_str("\n## Cost & tokens\n");
    out.push_str(&format!(
        "- context tokens: **{}**\n- cumulative cost: **${:.4}**\n",
        s.token_count, s.total_cost
    ));
    if let Some(usage) = s
        .token_usage
        .as_ref()
        .filter(|usage| usage.cache_telemetry_reported)
    {
        out.push_str(&format!(
            "- cache reads: **{}** · writes: **{}**\n",
            usage.cache_read_tokens, usage.cache_creation_tokens
        ));
    }

    if let Some(usage) = &s.token_usage {
        out.push_str(&format!(
            "- last turn: prompt **{}**, completion **{}**, total **{}**\n",
            usage.prompt_tokens, usage.completion_tokens, usage.total_tokens
        ));
    }

    out.push_str("\n## Tools\n");
    if top.is_empty() {
        out.push_str("_No tools used yet._\n");
    } else {
        for (name, n) in top.iter().take(10) {
            out.push_str(&format!("- `{}` × {}\n", name, n));
        }
    }

    out.push_str("\n## Setup\n");
    out.push_str(&format!(
        "- model `{}` · effort `{}` · approval `{}`\n- fast mode `{}` · poor mode `{}` · sandbox `{}`\n",
        s.current_model,
        s.thinking_effort.display_name(),
        approval_label(&s.approval_mode),
        on_off(s.fast_mode),
        on_off(s.poor_mode),
        on_off(s.sandbox_enabled),
    ));

    push_msg(&mut ctx, out);
    Ok(())
}

/// `/heapdump` — 进程资源快照（RSS/线程/句柄）。真正的堆剖析需要外部
/// profiler，这里给出可复制到 issue 里的运行时占用数据。
pub async fn heapdump(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let mut out = String::from("# Process snapshot\n\n");
    out.push_str(&format!("- pid: `{}`\n", std::process::id()));

    #[cfg(target_os = "linux")]
    {
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if line.starts_with("VmRSS")
                    || line.starts_with("VmPeak")
                    || line.starts_with("VmSize")
                    || line.starts_with("Threads")
                {
                    let mut parts = line.splitn(2, ':');
                    let k = parts.next().unwrap_or("").trim();
                    let v = parts.next().unwrap_or("").trim();
                    out.push_str(&format!("- {}: `{}`\n", k, v));
                }
            }
        }
        if let Ok(fds) = std::fs::read_dir("/proc/self/fd") {
            out.push_str(&format!("- open fds: `{}`\n", fds.count()));
        }
    }

    let s = &ctx.state;
    out.push_str("\n## In-memory UI state\n");
    out.push_str(&format!(
        "- chat entries: `{}`\n- render cache entries: `{}`\n- paste segments: `{}`\n\
         - command history: `{}`\n- active agent tasks: `{}`\n- kill ring: `{}`\n",
        s.chat_history.len(),
        s.rendered_cache.len(),
        s.paste_segments.len(),
        s.command_history.len(),
        s.active_agent_tasks.len(),
        s.kill_ring.len(),
    ));
    let chars: usize = s.chat_history.iter().map(|e| e.content.len()).sum();
    out.push_str(&format!("- transcript bytes held: `{}`\n", chars));
    out.push_str("\n_For a real heap profile run under `valgrind --tool=massif` or a Rust allocator profiler._\n");

    push_msg(&mut ctx, out);
    Ok(())
}

/// `/debug-tool-call` — 检查最近的工具调用（参数 + 结果），可传序号回看更早的。
pub async fn debug_tool_call(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    // 先在一个作用域里算好文本，再 push（避免与 chat_history 的不可变借用冲突）
    let out = {
        let calls: Vec<&ChatEntry> = ctx
            .state
            .chat_history
            .iter()
            .filter(|e| e.tool_call.is_some())
            .collect();

        if calls.is_empty() {
            "No tool calls in this session yet.".to_string()
        } else if args.first().map(|s| s.to_lowercase()).as_deref() == Some("list") {
            let mut out = format!("# Tool calls ({} total)\n\n", calls.len());
            for (i, e) in calls.iter().rev().enumerate().take(30) {
                let tc = e.tool_call.as_ref().unwrap();
                let status = match &e.tool_result {
                    Some(r) if r.success => "✅",
                    Some(_) => "❌",
                    None => "…",
                };
                out.push_str(&format!(
                    "{:>3}. {} `{}`\n",
                    i + 1,
                    status,
                    tc.function.name
                ));
            }
            out.push_str("\n_Inspect one with `/debug-tool-call <n>`._\n");
            out
        } else {
            let back = args
                .first()
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(1)
                .max(1);
            match calls.iter().rev().nth(back - 1) {
                None => format!(
                    "Only {} tool call(s) recorded — `{}` is out of range.",
                    calls.len(),
                    back
                ),
                Some(entry) => render_tool_call(entry, back),
            }
        }
    };
    push_msg(&mut ctx, out);
    Ok(())
}

pub(crate) fn render_tool_call(entry: &ChatEntry, back: usize) -> String {
    let tc = entry.tool_call.as_ref().expect("filtered on tool_call");
    let pretty_args = serde_json::from_str::<serde_json::Value>(&tc.function.arguments)
        .and_then(|v| serde_json::to_string_pretty(&v))
        .unwrap_or_else(|_| tc.function.arguments.clone());

    let mut out = format!(
        "# Tool call #{} from the end\n\n- name: `{}`\n- id: `{}`\n- type: `{}`\n\n## Arguments\n```json\n{}\n```\n",
        back, tc.function.name, tc.id, tc.call_type, pretty_args
    );
    match &entry.tool_result {
        Some(r) => {
            out.push_str(&format!(
                "\n## Result — {}\n",
                if r.success {
                    "success ✅"
                } else {
                    "failure ❌"
                }
            ));
            if let Some(err) = &r.error {
                out.push_str(&format!("\n**error:** {}\n", err));
            }
            if let Some(o) = &r.output {
                let clipped: String = o.chars().take(2000).collect();
                let ell = if o.chars().count() > 2000 {
                    "\n…[truncated]"
                } else {
                    ""
                };
                out.push_str(&format!("\n```\n{}{}\n```\n", clipped, ell));
            }
        }
        None => out.push_str("\n_No result recorded (still running or cancelled)._\n"),
    }
    out
}

// ═══════════════════════════════════════════════════════════════════════════
// GitHub / 协作平台
// ═══════════════════════════════════════════════════════════════════════════

/// `/artifacts` 认识的产物位置（key、路径、是否目录、说明）
pub(crate) fn artifact_locations() -> Vec<(&'static str, PathBuf, bool, &'static str)> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let star = cwd.join(".star");
    let home_star = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".star");
    vec![
        (
            "transcript",
            star.join("transcript.jsonl"),
            false,
            "full session transcript (JSONL)",
        ),
        ("logs", star.join("logs"), true, "debug/agent logs"),
        (
            "checkpoints",
            star.join("file-history"),
            true,
            "file snapshots behind /rewind",
        ),
        (
            "team-runs",
            star.join("agent-teams").join("runs"),
            true,
            "agent team run records and patches",
        ),
        (
            "agents",
            star.join("agents"),
            true,
            "project sub-agent definitions",
        ),
        ("skills", star.join("skills"), true, "project skills"),
        (
            "commands",
            star.join("commands"),
            true,
            "project slash commands",
        ),
        (
            "extensions",
            star.join("extensions"),
            true,
            "plugins installed into this project",
        ),
        (
            "user-extensions",
            home_star.join("extensions"),
            true,
            "plugins installed for your user",
        ),
        (
            "memory",
            star.join("memory"),
            true,
            "classified memory store",
        ),
        (
            "reports",
            star.join("reports"),
            true,
            "reports written by commands like /perf-issue",
        ),
        (
            "tmp",
            star.join("tmp"),
            true,
            "scratch space (safe to delete)",
        ),
    ]
}

/// `/artifacts [key]` — 列出本地真实产物；带 key 时展开该位置最近的文件。
pub async fn artifacts(mut ctx: CommandContext<'_>, args: &[String]) -> CommandResult {
    let locations = artifact_locations();

    if let Some(key) = args.first().map(|s| s.to_lowercase()) {
        let Some((name, path, is_dir, note)) = locations
            .iter()
            .find(|(k, _, _, _)| *k == key.as_str())
            .cloned()
        else {
            push_msg(
                &mut ctx,
                format!(
                    "Unknown artifact set `{}`. Known keys: {}",
                    key,
                    locations
                        .iter()
                        .map(|(k, _, _, _)| format!("`{}`", k))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
            return Ok(());
        };
        push_msg(&mut ctx, render_artifact_set(name, &path, is_dir, note));
        return Ok(());
    }

    let mut out =
        String::from("## Artifacts\n\n| set | path | size | state |\n|---|---|---|---|\n");
    for (key, path, is_dir, _) in &locations {
        out.push_str(&store_row(key, path, *is_dir));
    }
    out.push_str("\n| set | holds |\n|---|---|\n");
    for (key, _, _, note) in &locations {
        out.push_str(&format!("| `{}` | {} |\n", key, note));
    }
    out.push_str(
        "\nExpand one with `/artifacts <set>` · `/rewind` restores checkpoints · \
         `/agents team runs` lists team runs in detail.\n",
    );
    push_msg(&mut ctx, out);
    Ok(())
}

/// 展开一个产物位置：目录列最近修改的文件，单文件给大小与时间。
pub(crate) fn render_artifact_set(
    name: &str,
    path: &std::path::Path,
    is_dir: bool,
    note: &str,
) -> String {
    if !path.exists() {
        return format!(
            "## Artifacts · {}\n\n{}\n\n`{}` does not exist yet — nothing has written to it.\n",
            name,
            note,
            path.display()
        );
    }

    let stamp = |m: &std::fs::Metadata| -> String {
        m.modified()
            .ok()
            .map(|t| {
                chrono::DateTime::<chrono::Local>::from(t)
                    .format("%Y-%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_else(|| "-".to_string())
    };

    if !is_dir {
        let meta = std::fs::metadata(path).ok();
        return format!(
            "## Artifacts · {}\n\n{}\n\n- path: `{}`\n- size: {}\n- modified: {}\n",
            name,
            note,
            path.display(),
            meta.as_ref()
                .map(|m| human_bytes(m.len()))
                .unwrap_or_else(|| "-".into()),
            meta.as_ref().map(stamp).unwrap_or_else(|| "-".into())
        );
    }

    let mut files: Vec<(PathBuf, u64, std::time::SystemTime)> = Vec::new();
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            match e.metadata() {
                Ok(m) if m.is_dir() => stack.push(e.path()),
                Ok(m) => {
                    let t = m.modified().unwrap_or(std::time::UNIX_EPOCH);
                    files.push((e.path(), m.len(), t));
                }
                Err(_) => {}
            }
        }
    }
    files.sort_by_key(|f| std::cmp::Reverse(f.2));
    render_artifact_files(name, path, note, &files)
}

/// 渲染产物文件表（按修改时间倒序，最多 25 行）
pub(crate) fn render_artifact_files(
    name: &str,
    root: &std::path::Path,
    note: &str,
    files: &[(PathBuf, u64, std::time::SystemTime)],
) -> String {
    let total: u64 = files.iter().map(|f| f.1).sum();
    let mut out = format!(
        "## Artifacts · {}\n\n{}\n\n- path: `{}`\n- {} file(s), {}\n\n",
        name,
        note,
        root.display(),
        files.len(),
        human_bytes(total)
    );
    if files.is_empty() {
        out.push_str("Directory exists but is empty.\n");
        return out;
    }
    out.push_str("| modified | size | file |\n|---|---|---|\n");
    for (p, size, time) in files.iter().take(25) {
        let rel = p.strip_prefix(root).unwrap_or(p);
        out.push_str(&format!(
            "| {} | {} | `{}` |\n",
            chrono::DateTime::<chrono::Local>::from(*time).format("%Y-%m-%d %H:%M"),
            human_bytes(*size),
            rel.display()
        ));
    }
    if files.len() > 25 {
        out.push_str(&format!("\n_{} more not shown._\n", files.len() - 25));
    }
    out
}

/// 一条诊断结果
pub(crate) struct Check {
    pub(crate) ok: bool,
    pub(crate) name: &'static str,
    pub(crate) detail: String,
}

pub(crate) fn check(ok: bool, name: &'static str, detail: impl Into<String>) -> Check {
    Check {
        ok,
        name,
        detail: detail.into(),
    }
}

/// 目录可写探测：真写一个临时文件再删掉。
pub(crate) fn writable(dir: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let probe = dir.join(format!(".star-write-probe-{}", now_ms()));
    std::fs::write(&probe, b"ok").map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

/// `/torch` — 本地自检：环境、provider、磁盘、外部工具、会话状态。
pub async fn torch(mut ctx: CommandContext<'_>, _args: &[String]) -> CommandResult {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut checks: Vec<Check> = Vec::new();

    checks.push(check(
        true,
        "build",
        format!(
            "starcode-cli {} · {} {}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
    ));

    match run("git", &["rev-parse", "--abbrev-ref", "HEAD"]) {
        Ok(branch) => {
            let dirty = run("git", &["status", "--porcelain"])
                .map(|s| s.lines().count())
                .unwrap_or(0);
            checks.push(check(
                true,
                "git repository",
                format!("branch `{}` · {} changed file(s)", branch, dirty),
            ));
        }
        Err(e) => checks.push(check(false, "git repository", e)),
    }

    let store = crate::core::config::provider_store::ProviderStore::new();
    match store.load().await {
        Ok(cfg) => {
            let pid = cfg.active_provider_id.clone();
            let key_ok = pid
                .as_ref()
                .and_then(|p| cfg.providers.get(p))
                .and_then(|p| p.api_key.clone())
                .map(|k| !k.is_empty() && k != crate::llm::API_KEY_NOT_SET)
                .unwrap_or(false)
                || std::env::var("STAR_API_KEY").is_ok();
            checks.push(check(
                key_ok,
                "provider credentials",
                format!(
                    "provider `{}` · {} configured provider(s) · key {}",
                    pid.as_deref().unwrap_or("-"),
                    cfg.providers.len(),
                    if key_ok { "present" } else { "missing" }
                ),
            ));
        }
        Err(e) => checks.push(check(false, "provider credentials", e)),
    }

    checks.push(check(
        !ctx.state.current_model.is_empty(),
        "active model",
        if ctx.state.current_model.is_empty() {
            "not selected — run `/model`".to_string()
        } else {
            ctx.state.current_model.clone()
        },
    ));

    match crate::core::mcp::load_project_mcp_config().await {
        Ok(mcp) => {
            let enabled = mcp
                .mcp_servers
                .values()
                .filter(|s| !s.disabled.unwrap_or(false))
                .count();
            checks.push(check(
                true,
                "mcp config",
                format!(
                    "parsed · {} server(s), {} enabled · runtime ready: {}",
                    mcp.mcp_servers.len(),
                    enabled,
                    on_off(ctx.state.mcp_ready)
                ),
            ));
        }
        Err(e) => checks.push(check(false, "mcp config", e.to_string())),
    }

    match writable(&cwd.join(".star")) {
        Ok(()) => checks.push(check(
            true,
            "`.star/` writable",
            cwd.join(".star").display().to_string(),
        )),
        Err(e) => checks.push(check(false, "`.star/` writable", e)),
    }

    let mut missing: Vec<&str> = Vec::new();
    let mut found: Vec<&str> = Vec::new();
    for (bin, arg) in [
        ("git", "--version"),
        ("gh", "--version"),
        ("rg", "--version"),
        ("cargo", "--version"),
        ("node", "--version"),
        ("python3", "--version"),
    ] {
        if run(bin, &[arg]).is_ok() {
            found.push(bin);
        } else {
            missing.push(bin);
        }
    }
    checks.push(check(
        !found.is_empty(),
        "external tools",
        format!(
            "available: {}{}",
            if found.is_empty() {
                "none".to_string()
            } else {
                found.join(", ")
            },
            if missing.is_empty() {
                String::new()
            } else {
                format!(" · missing: {}", missing.join(", "))
            }
        ),
    ));

    let log_enabled = std::env::var("STAR_LOG_ENABLED")
        .map(|v| v != "0" && v.to_lowercase() != "false")
        .unwrap_or(true);
    checks.push(check(
        true,
        "logging",
        format!(
            "debug log {} · transcript {}",
            on_off(log_enabled),
            on_off(crate::ui::utils::transcript::transcript_enabled_from_env())
        ),
    ));

    checks.push(check(
        ctx.state.plugin_errors.is_empty(),
        "plugins",
        if ctx.state.plugin_errors.is_empty() {
            format!(
                "{} installed, no load errors",
                ctx.state.plugin_installed.len()
            )
        } else {
            format!(
                "{} load error(s): {}",
                ctx.state.plugin_errors.len(),
                ctx.state
                    .plugin_errors
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        },
    ));

    checks.push(check(
        true,
        "approval mode",
        format!(
            "{:?} · thinking effort {:?} · auto-continue {}",
            ctx.state.approval_mode,
            ctx.state.thinking_effort,
            on_off(ctx.state.auto_continue_enabled)
        ),
    ));

    let turns = ctx
        .state
        .chat_history
        .iter()
        .filter(|e| e.entry_type == crate::types::ChatEntryType::User)
        .count();
    let tokens = ctx
        .state
        .token_usage
        .as_ref()
        .map(|u| u.total_tokens)
        .unwrap_or(0);
    checks.push(check(
        true,
        "session",
        format!(
            "{} user turn(s) · {} history entr(ies) · {} token(s) · ${:.4}",
            turns,
            ctx.state.chat_history.len(),
            tokens,
            ctx.state.total_cost
        ),
    ));

    push_msg(&mut ctx, render_checks(&checks));
    Ok(())
}

/// 把自检结果渲染成表格：先给通过/失败计数，再逐项列出。
pub(crate) fn render_checks(checks: &[Check]) -> String {
    let failed = checks.iter().filter(|c| !c.ok).count();
    let passed = checks.len() - failed;

    let mut out = String::from("## /torch — environment self-check\n\n");
    out.push_str(&format!(
        "{} **{} passed, {} failed** out of {} checks.\n\n",
        if failed == 0 { "✅" } else { "⚠️" },
        passed,
        failed,
        checks.len()
    ));
    out.push_str("| | Check | Detail |\n|---|---|---|\n");
    for c in checks {
        out.push_str(&format!(
            "| {} | {} | {} |\n",
            if c.ok { "✅" } else { "❌" },
            c.name,
            c.detail.replace('\n', " ").replace('|', "\\|")
        ));
    }

    if failed > 0 {
        out.push_str("\nFailed checks are reported as-is — nothing is repaired automatically.\n");
    }
    out.push_str(
        "\nThis command only inspects the local environment: binaries on `PATH`, the provider \
         config on disk, the parsed MCP config and whether `.star/` accepts a write. It does not \
         contact any provider or validate that an API key is accepted.\n",
    );
    out
}

// ═══════════════════════════════════════════════════════════════════════════
// Performance report
// ═══════════════════════════════════════════════════════════════════════════

/// 汇总一份可直接粘贴到 issue 里的性能快照（全部来自本地真实状态）。
pub(crate) fn perf_report(ctx: &CommandContext<'_>) -> String {
    let st = &ctx.state;
    let mut out = String::new();

    out.push_str("### Build\n");
    out.push_str(&format!(
        "- starcode-cli {} · {}/{} · {} build\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        if cfg!(debug_assertions) {
            "debug (unoptimized — expect much slower rendering and tool I/O)"
        } else {
            "release"
        }
    ));
    if let Some(area) = st.last_chat_area {
        out.push_str(&format!(
            "- chat viewport: {}x{} cells · {} rendered item(s)\n",
            area.width,
            area.height,
            st.last_item_heights.len()
        ));
    }

    out.push_str("\n### Model\n");
    out.push_str(&format!(
        "- model: `{}` · provider: `{}`\n",
        if st.current_model.is_empty() {
            "-"
        } else {
            st.current_model.as_str()
        },
        st.current_provider_id.as_deref().unwrap_or("-")
    ));
    out.push_str(&format!(
        "- thinking effort: {:?} · fast mode: {} · poor mode: {}\n",
        st.thinking_effort,
        on_off(st.fast_mode),
        on_off(st.poor_mode)
    ));

    out.push_str("\n### Timeouts in effect\n");
    for (var, default) in [
        ("STAR_LLM_TIMEOUT", "120"),
        ("STAR_CONNECT_TIMEOUT", "30"),
        (
            "STAR_TOOL_TIMEOUT_SECS",
            "240 smart_edit/skill, 120 Bash, 180 other",
        ),
    ] {
        out.push_str(&format!(
            "- `{}`: {}\n",
            var,
            match std::env::var(var) {
                Ok(v) => format!("{}s (set)", v),
                Err(_) => format!("{} (default)", default),
            }
        ));
    }

    out.push_str(&perf_report_runtime(ctx));
    out
}

/// 报告的运行时部分：会话计数、上下文占用、在飞工具、磁盘增长。
pub(crate) fn perf_report_runtime(ctx: &CommandContext<'_>) -> String {
    let st = &ctx.state;
    let mut out = String::from("\n### Session counters\n");

    let provider_id = crate::ui::utils::status::current_provider_id(st);
    let window = st
        .context_window_override
        .or_else(|| {
            crate::agent::model_catalog::get_cached_context_window(
                provider_id.as_deref(),
                &st.current_model,
            )
        })
        .or_else(|| {
            std::env::var("STAR_CONTEXT_WINDOW")
                .ok()
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(128_000);
    let used = st.token_usage.as_ref().map(|u| u.total_tokens).unwrap_or(0);
    let tool_entries = st
        .chat_history
        .iter()
        .filter(|e| e.tool_call.is_some())
        .count();
    out.push_str(&format!(
        "- history: {} entr(ies), {} with a tool call\n",
        st.chat_history.len(),
        tool_entries
    ));
    out.push_str(&format!(
        "- tokens: {} of ~{} window ({:.1}%) · cost ${:.4}\n",
        used,
        window,
        (used as f64 / window as f64) * 100.0,
        st.total_cost
    ));
    if let Some(u) = st.token_usage.as_ref() {
        out.push_str(&format!(
            "- last usage split: {} prompt / {} completion / {} cache-read\n",
            u.prompt_tokens, u.completion_tokens, u.cache_read_tokens
        ));
    }
    out.push_str(&format!(
        "- auto-compact: {} (fires at 92% of the window)\n",
        on_off(
            std::env::var("STAR_AUTO_COMPACT")
                .map(|v| v != "0" && v.to_lowercase() != "false")
                .unwrap_or(true)
        )
    ));

    out.push_str("\n### In flight\n");
    out.push_str(&format!(
        "- processing: {} · streaming: {}\n",
        on_off(st.is_processing),
        on_off(st.is_streaming)
    ));
    if let Some(t) = st.processing_started_at {
        out.push_str(&format!(
            "- current turn elapsed: {:.1}s\n",
            t.elapsed().as_secs_f64()
        ));
    }
    if let Some(t) = st.last_token_time {
        out.push_str(&format!(
            "- since last token: {:.1}s{}\n",
            t.elapsed().as_secs_f64(),
            if t.elapsed().as_secs() > 30 {
                " ⚠️ stalled"
            } else {
                ""
            }
        ));
    }
    if st.tool_started_at.is_empty() {
        out.push_str("- no tool call is currently running\n");
    } else {
        for (id, started) in st.tool_started_at.iter() {
            out.push_str(&format!(
                "- tool `{}` running for {:.1}s\n",
                id,
                started.elapsed().as_secs_f64()
            ));
        }
    }

    out.push_str(&perf_report_disk(ctx));
    out
}

/// 报告的磁盘部分：日志/转录/`.star` 体积——大文件会直接拖慢启动与追加写。
pub(crate) fn perf_report_disk(ctx: &CommandContext<'_>) -> String {
    let mut out = String::from("\n### On-disk growth\n");
    let size_of = |p: &std::path::Path| -> String {
        std::fs::metadata(p)
            .map(|m| human_bytes(m.len()))
            .unwrap_or_else(|_| "absent".to_string())
    };

    let debug_log = crate::utils::logging::debug_log_path();
    let agent_log = crate::utils::logging::agent_log_path();
    out.push_str(&format!(
        "- debug log ({}): `{}` — {}\n",
        on_off(crate::utils::logging::is_log_enabled()),
        debug_log.display(),
        size_of(&debug_log)
    ));
    out.push_str(&format!(
        "- agent log: `{}` — {}\n",
        agent_log.display(),
        size_of(&agent_log)
    ));
    match ctx.state.transcript_path.as_ref() {
        Some(p) => out.push_str(&format!(
            "- transcript ({}): `{}` — {}\n",
            on_off(ctx.state.transcript_enabled),
            p.display(),
            size_of(p)
        )),
        None => out.push_str(&format!(
            "- transcript: {} (no file opened this session)\n",
            on_off(ctx.state.transcript_enabled)
        )),
    }
    if let Ok(cwd) = std::env::current_dir() {
        let star = cwd.join(".star");
        if star.exists() {
            let (n, b) = dir_stats(&star);
            out.push_str(&format!("- `.star/`: {} file(s) / {}\n", n, human_bytes(b)));
        }
    }
    out
}

/// `/perf-issue [save]` — 采集本地性能快照，生成可粘贴的 issue 正文。
///
/// 只读本地状态与文件大小，不上传任何内容；`save` 落盘到 `.star/`。
/// 提交 issue 需要用户自己执行给出的 `gh` 命令（对外发布不代劳）。
pub async fn perf_issue(mut ctx: CommandContext<'_>, args: &[String]) -> CommandResult {
    let body = perf_report(&ctx);
    let sub = args.first().map(|s| s.to_lowercase()).unwrap_or_default();

    if sub == "save" {
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let path = star_dir()?.join(format!("perf-report-{}.md", stamp));
        let file_body = format!(
            "# starcode-cli performance report\n\nGenerated {}\n\n{}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            body
        );
        std::fs::write(&path, &file_body).map_err(|e| e.to_string())?;
        push_msg(
            &mut ctx,
            format!(
                "## /perf-issue\n\nWrote the report to `{}` ({}).\n\nAttach it to an issue \
                 yourself — this command never uploads anything.\n",
                path.display(),
                human_bytes(file_body.len() as u64)
            ),
        );
        return Ok(());
    }

    let mut out = String::from("## /perf-issue — local performance snapshot\n\n");
    out.push_str(&body);
    out.push_str("\n### What to add before filing\n");
    out.push_str(
        "- what felt slow (typing, streaming, a specific tool, startup) and how long it took\n\
         - whether it reproduces on a fresh session (`/clear`) and in a release build\n\
         - the last few lines of the debug log around the slow moment\n",
    );
    match repo_slug() {
        Some(slug) => out.push_str(&format!(
            "\nFile it against `{}` when ready (publishes to GitHub, so run it yourself):\n\
             ```bash\ngh issue create --repo {} --title \"perf: <what was slow>\" \
             --body-file .star/perf-report-*.md\n```\n\
             Run `/perf-issue save` first to produce that file.\n",
            slug, slug
        )),
        None => out.push_str(
            "\n`gh` could not resolve a GitHub repository here, so no issue command is offered. \
             `/perf-issue save` still writes the report to `.star/`.\n",
        ),
    }
    out.push_str(
        "\nNothing here is measured by a profiler: these are the counters and file sizes the \
         session already tracks. Per-turn latency history is not retained, so only the current \
         turn shows elapsed time.\n",
    );
    push_msg(&mut ctx, out);
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Verifiers
// ═══════════════════════════════════════════════════════════════════════════

/// 一条检测到的校验命令：来源清单 + 用途 + 命令行。
pub(crate) struct Verifier {
    pub(crate) kind: &'static str,
    pub(crate) source: String,
    pub(crate) command: String,
}

/// 只根据仓库里真实存在的清单文件推断校验命令，不猜测不存在的工具链。
pub(crate) fn detect_verifiers(root: &std::path::Path) -> Vec<Verifier> {
    let mut v: Vec<Verifier> = Vec::new();
    let has = |name: &str| root.join(name).exists();
    let mut add = |kind: &'static str, source: &str, command: &str| {
        v.push(Verifier {
            kind,
            source: source.to_string(),
            command: command.to_string(),
        })
    };

    if has("Cargo.toml") {
        add("build", "Cargo.toml", "cargo check --all-targets");
        add("test", "Cargo.toml", "cargo test");
        if run("cargo", &["clippy", "--version"]).is_ok() {
            add(
                "lint",
                "Cargo.toml",
                "cargo clippy --all-targets -- -D warnings",
            );
        }
        if run("cargo", &["fmt", "--version"]).is_ok() {
            add("format", "Cargo.toml", "cargo fmt --check");
        }
    }

    if has("package.json") {
        let pm = if has("pnpm-lock.yaml") {
            "pnpm"
        } else if has("yarn.lock") {
            "yarn"
        } else if has("bun.lockb") {
            "bun"
        } else {
            "npm"
        };
        let scripts: Vec<String> = std::fs::read_to_string(root.join("package.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|j| j.get("scripts").cloned())
            .and_then(|s| s.as_object().map(|m| m.keys().cloned().collect()))
            .unwrap_or_default();
        for (key, kind) in [
            ("build", "build"),
            ("test", "test"),
            ("lint", "lint"),
            ("typecheck", "types"),
            ("format", "format"),
        ] {
            if scripts.iter().any(|s| s == key) {
                let prefix = if pm == "npm" { "npm run" } else { pm };
                add(kind, "package.json scripts", &format!("{} {}", prefix, key));
            }
        }
    }

    v.extend(detect_verifiers_more(root));
    v
}

/// Python / Go / Make / pre-commit 部分的检测。
pub(crate) fn detect_verifiers_more(root: &std::path::Path) -> Vec<Verifier> {
    let mut v: Vec<Verifier> = Vec::new();
    let has = |name: &str| root.join(name).exists();
    let mut add = |kind: &'static str, source: &str, command: &str| {
        v.push(Verifier {
            kind,
            source: source.to_string(),
            command: command.to_string(),
        })
    };

    if has("pyproject.toml") || has("requirements.txt") || has("setup.py") {
        let src = if has("pyproject.toml") {
            "pyproject.toml"
        } else if has("requirements.txt") {
            "requirements.txt"
        } else {
            "setup.py"
        };
        if run("pytest", &["--version"]).is_ok() {
            add("test", src, "pytest -q");
        }
        if run("ruff", &["--version"]).is_ok() {
            add("lint", src, "ruff check .");
        }
        if run("mypy", &["--version"]).is_ok() {
            add("types", src, "mypy .");
        }
    }

    if has("go.mod") {
        add("build", "go.mod", "go build ./...");
        add("test", "go.mod", "go test ./...");
        add("lint", "go.mod", "go vet ./...");
    }

    for mk in ["Makefile", "makefile", "GNUmakefile"] {
        if !has(mk) {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(root.join(mk)) {
            let targets: Vec<String> = text
                .lines()
                .filter_map(|l| l.split_once(':').map(|(t, _)| t.trim().to_string()))
                .filter(|t| {
                    !t.is_empty() && !t.starts_with('.') && !t.contains(char::is_whitespace)
                })
                .collect();
            for (target, kind) in [
                ("build", "build"),
                ("test", "test"),
                ("check", "build"),
                ("lint", "lint"),
                ("fmt", "format"),
            ] {
                if targets.iter().any(|t| t == target) {
                    add(kind, mk, &format!("make {}", target));
                }
            }
        }
        break;
    }

    if has(".pre-commit-config.yaml") && run("pre-commit", &["--version"]).is_ok() {
        add(
            "lint",
            ".pre-commit-config.yaml",
            "pre-commit run --all-files",
        );
    }
    v
}

/// 把 `## Verification` 段落写进项目指令文件（存在则整段替换，否则追加）。
/// 该文件会被 `load_project_context` 注入 system prompt，所以写进去就是真的生效。
pub(crate) fn write_verification_section(
    path: &std::path::Path,
    section: &str,
) -> Result<bool, String> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let replaced = if let Some(start) = existing.find("\n## Verification") {
        let after = &existing[start + 1..];
        let end = after
            .char_indices()
            .skip(1)
            .find(|(i, _)| after[*i..].starts_with("\n## "))
            .map(|(i, _)| start + 1 + i + 1);
        let mut next = String::from(&existing[..start + 1]);
        next.push_str(section);
        if let Some(end) = end {
            next.push('\n');
            next.push_str(&existing[end..]);
        }
        std::fs::write(path, next).map_err(|e| e.to_string())?;
        true
    } else {
        let mut next = existing;
        if !next.is_empty() && !next.ends_with('\n') {
            next.push('\n');
        }
        if !next.is_empty() {
            next.push('\n');
        }
        next.push_str(section);
        std::fs::write(path, next).map_err(|e| e.to_string())?;
        false
    };
    Ok(replaced)
}

/// 渲染写入指令文件的段落正文。
pub(crate) fn verification_section(verifiers: &[Verifier]) -> String {
    let mut s = String::from("## Verification\n\nRun these before reporting work as done:\n\n");
    for v in verifiers {
        s.push_str(&format!("- {}: `{}`\n", v.kind, v.command));
    }
    s.push_str("\nIf a command fails, fix the cause rather than skipping the check.\n");
    s
}

/// `/init-verifiers [apply|run]` — 从项目清单推断校验命令。
///
/// - 无参数：列出检测结果与将写入的内容；
/// - `apply`：把 `## Verification` 段落写进项目指令文件（真正进 system prompt）；
/// - `run`：把命令交给主回路，由 agent 用 Bash 工具依次执行（不阻塞 UI）。
pub async fn init_verifiers(mut ctx: CommandContext<'_>, args: &[String]) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let verifiers = detect_verifiers(&cwd);
    let sub = args.first().map(|s| s.to_lowercase()).unwrap_or_default();

    if verifiers.is_empty() {
        push_msg(
            &mut ctx,
            format!(
                "## /init-verifiers\n\nNo verification commands could be inferred from `{}`.\n\n\
                 Detection only looks at manifests that actually exist: `Cargo.toml`, \
                 `package.json` scripts, `pyproject.toml`/`requirements.txt`/`setup.py`, \
                 `go.mod`, a `Makefile` target list and `.pre-commit-config.yaml` — and for \
                 Python/pre-commit it also requires the tool to be on `PATH`.\n\n\
                 Add the commands by hand to `STAR.md` under a `## Verification` heading; that \
                 file is injected into the system prompt.\n",
                cwd.display()
            ),
        );
        return Ok(());
    }

    let section = verification_section(&verifiers);

    match sub.as_str() {
        "apply" => {
            let path = crate::utils::project_context::find_project_context_file(&cwd)
                .unwrap_or_else(|| cwd.join("STAR.md"));
            let replaced = write_verification_section(&path, &section)?;
            push_msg(
                &mut ctx,
                format!(
                    "## /init-verifiers — applied\n\n{} the `## Verification` section in `{}`.\n\n\
                     ```markdown\n{}```\n\nThis file is loaded into the system prompt on the next \
                     turn, so the commands above become part of the agent's instructions.\n",
                    if replaced { "Replaced" } else { "Added" },
                    path.display(),
                    section
                ),
            );
            Ok(())
        }
        "run" => {
            let list = verifiers
                .iter()
                .map(|v| format!("- {}: `{}`", v.kind, v.command))
                .collect::<Vec<_>>()
                .join("\n");
            push_msg(
                &mut ctx,
                format!(
                    "Running the detected verifiers via the Bash tool:\n\n{}\n",
                    list
                ),
            );
            ask_agent(
                &mut ctx,
                format!(
                    "Run these project verification commands in order with the Bash tool, one \
                     call each, and report each command's exit status plus the first failing \
                     output. Do not change any files.\n\n{}",
                    list
                ),
            )
            .await
        }
        _ => {
            push_msg(&mut ctx, render_verifiers(&cwd, &verifiers, &section));
            Ok(())
        }
    }
}

/// 检测结果表格 + 目标文件状态 + 下一步。
pub(crate) fn render_verifiers(
    root: &std::path::Path,
    verifiers: &[Verifier],
    section: &str,
) -> String {
    let mut out = String::from(
        "## /init-verifiers\n\n| purpose | command | inferred from |\n|---|---|---|\n",
    );
    for v in verifiers {
        out.push_str(&format!(
            "| {} | `{}` | {} |\n",
            v.kind, v.command, v.source
        ));
    }

    let target = crate::utils::project_context::find_project_context_file(root);
    match &target {
        Some(p) => {
            let has_section = std::fs::read_to_string(p)
                .map(|t| t.contains("## Verification"))
                .unwrap_or(false);
            out.push_str(&format!(
                "\nTarget file: `{}` — {}.\n",
                p.display(),
                if has_section {
                    "already has a `## Verification` section, `apply` replaces it"
                } else {
                    "no `## Verification` section yet, `apply` appends one"
                }
            ));
        }
        None => out.push_str(&format!(
            "\nNo project instruction file yet — `apply` creates `{}`.\n",
            root.join("STAR.md").display()
        )),
    }

    out.push_str(&format!(
        "\nWhat `apply` writes:\n\n```markdown\n{}```\n",
        section
    ));
    out.push_str(
        "\n`/init-verifiers apply` writes it · `/init-verifiers run` asks the agent to execute \
         the commands with the Bash tool. Detection reads manifests only; commands are never run \
         by this command itself, and nothing enforces them automatically — the instruction file \
         is what makes the agent honour them.\n",
    );
    out
}

// ═══════════════════════════════════════════════════════════════════════════
// Onboarding
// ═══════════════════════════════════════════════════════════════════════════

/// 一个上手步骤：是否完成 + 现状 + 未完成时要跑的命令。
pub(crate) struct Step {
    pub(crate) done: bool,
    pub(crate) title: &'static str,
    pub(crate) detail: String,
    pub(crate) action: &'static str,
}

/// `/onboarding` — 按真实配置状态给出上手清单。
///
/// 每一项都读磁盘或会话状态，不做假勾选；未完成项直接给出对应命令。
pub async fn onboarding(mut ctx: CommandContext<'_>, _args: &[String]) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let mut steps: Vec<Step> = Vec::new();

    let store = crate::core::config::provider_store::ProviderStore::new();
    let cfg = store.load().await.ok();
    let (provider_done, provider_detail) = match cfg.as_ref() {
        Some(c) => {
            let active = c.active_provider_id.clone();
            let key_ok = active
                .as_ref()
                .and_then(|p| c.providers.get(p))
                .and_then(|p| p.api_key.clone())
                .map(|k| !k.is_empty() && k != crate::llm::API_KEY_NOT_SET)
                .unwrap_or(false)
                || std::env::var("STAR_API_KEY").is_ok();
            (
                key_ok,
                format!(
                    "{} provider(s) configured · active `{}` · key {}",
                    c.providers.len(),
                    active.as_deref().unwrap_or("-"),
                    if key_ok { "present" } else { "missing" }
                ),
            )
        }
        None => (false, "provider config could not be read".to_string()),
    };
    steps.push(Step {
        done: provider_done,
        title: "Connect a provider",
        detail: provider_detail,
        action: "/provider",
    });

    steps.push(Step {
        done: !ctx.state.current_model.is_empty(),
        title: "Pick a model",
        detail: if ctx.state.current_model.is_empty() {
            "no model selected".to_string()
        } else {
            format!("`{}`", ctx.state.current_model)
        },
        action: "/model",
    });

    steps.extend(onboarding_project_steps(&ctx, &cwd).await);
    push_msg(&mut ctx, render_steps(&steps));
    Ok(())
}

/// 上手清单里与项目相关的步骤（指令文件、MCP、技能/子代理、插件、审批模式）。
pub(crate) async fn onboarding_project_steps(
    ctx: &CommandContext<'_>,
    cwd: &std::path::Path,
) -> Vec<Step> {
    let mut steps: Vec<Step> = Vec::new();

    let context_file = crate::utils::project_context::find_project_context_file(cwd);
    steps.push(Step {
        done: context_file.is_some(),
        title: "Describe the project",
        detail: match &context_file {
            Some(p) => format!("`{}` is loaded into the system prompt", p.display()),
            None => "no STAR.md / STARCODE.md / CLAUDE.md in this tree".to_string(),
        },
        action: "/init",
    });

    let mcp_count = crate::core::mcp::load_project_mcp_config()
        .await
        .map(|c| c.mcp_servers.len())
        .unwrap_or(0);
    steps.push(Step {
        done: mcp_count > 0,
        title: "Add MCP servers (optional)",
        detail: if mcp_count == 0 {
            "no servers in `.mcp.json`".to_string()
        } else {
            format!(
                "{} server(s) declared · runtime ready: {}",
                mcp_count,
                on_off(ctx.state.mcp_ready)
            )
        },
        action: "/mcp",
    });

    let count_dir = |p: PathBuf| -> usize {
        std::fs::read_dir(p)
            .map(|d| d.flatten().filter(|e| e.path().is_file()).count())
            .unwrap_or(0)
    };
    let agents = count_dir(cwd.join(".star/agents"));
    steps.push(Step {
        done: agents > 0,
        title: "Define subagents (optional)",
        detail: format!("{} definition(s) in `.star/agents`", agents),
        action: "/agents",
    });

    let skills = count_dir(cwd.join(".star/skills"));
    steps.push(Step {
        done: skills > 0,
        title: "Install skills (optional)",
        detail: format!("{} file(s) in `.star/skills`", skills),
        action: "/skills",
    });

    steps.push(Step {
        done: !ctx.state.plugin_installed.is_empty(),
        title: "Install plugins (optional)",
        detail: format!(
            "{} installed · {} load error(s)",
            ctx.state.plugin_installed.len(),
            ctx.state.plugin_errors.len()
        ),
        action: "/plugin",
    });

    steps.push(Step {
        done: true,
        title: "Choose an approval mode",
        detail: format!(
            "currently `{}` — this is the gate that actually blocks tools",
            approval_label(&ctx.state.approval_mode)
        ),
        action: "/mode",
    });

    steps
}

/// 渲染上手清单：完成度 + 逐项状态 + 第一个待办。
pub(crate) fn render_steps(steps: &[Step]) -> String {
    let done = steps.iter().filter(|s| s.done).count();
    let mut out = format!(
        "## /onboarding\n\n{} of {} steps look set up.\n\n| | step | state | command |\n|---|---|---|---|\n",
        done,
        steps.len()
    );
    for s in steps {
        out.push_str(&format!(
            "| {} | {} | {} | `{}` |\n",
            if s.done { "✅" } else { "⬜" },
            s.title,
            s.detail.replace('|', "\\|"),
            s.action
        ));
    }

    match steps.iter().find(|s| !s.done) {
        Some(next) => out.push_str(&format!(
            "\nNext: **{}** — run `{}`.\n",
            next.title, next.action
        )),
        None => out.push_str("\nEverything on the checklist is configured.\n"),
    }
    out.push_str(
        "\nSteps marked optional only affect extra capabilities; a provider, a model and (for \
         project-specific behaviour) an instruction file are what actually change how the agent \
         works. This command inspects state and links to the real commands — it does not change \
         anything on its own.\n",
    );
    out
}

// ═══════════════════════════════════════════════════════════════════════════
// Agents platform
// ═══════════════════════════════════════════════════════════════════════════

/// 读取一个 agent 定义目录，返回 (文件名, description 首行)。
pub(crate) fn agent_defs_in(dir: &std::path::Path) -> Vec<(String, String)> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = rd
        .flatten()
        .filter(|e| e.path().is_file())
        .filter(|e| {
            matches!(
                e.path().extension().and_then(|x| x.to_str()),
                Some("md") | Some("markdown")
            )
        })
        .map(|e| {
            let name = e
                .path()
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let desc = std::fs::read_to_string(e.path())
                .ok()
                .and_then(|t| {
                    t.lines()
                        .find(|l| l.trim_start().starts_with("description:"))
                        .map(|l| {
                            l.split_once(':')
                                .map(|(_, v)| v.trim().to_string())
                                .unwrap_or_default()
                        })
                })
                .unwrap_or_default();
            (name, desc)
        })
        .collect();
    out.sort();
    out
}

/// `/agents-platform` — 本地 agent 平台总览：定义、团队预设、最近团队运行。
///
/// 全部来自磁盘上的真实文件（`.star/agents`、`agent-teams.json`、
/// `.star/agent-teams/runs`）；没有云端平台，也没有远端注册表。
pub async fn agents_platform(mut ctx: CommandContext<'_>, _args: &[String]) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let storage = crate::core::config::storage::Storage::new(cwd.clone());

    let project = agent_defs_in(&storage.star_dir().join("agents"));
    let user =
        agent_defs_in(&crate::core::config::storage::Storage::global_star_dir().join("agents"));

    let mut out = String::from("## /agents-platform\n\n### Built-in subagent types\n\n");
    out.push_str("| slug | shown as |\n|---|---|\n");
    for t in [
        crate::core::agents::types::SubagentType::GeneralPurpose,
        crate::core::agents::types::SubagentType::Explorer,
        crate::core::agents::types::SubagentType::Analyzer,
        crate::core::agents::types::SubagentType::Editor,
        crate::core::agents::types::SubagentType::CodeReviewer,
    ] {
        out.push_str(&format!(
            "| `{}` | {} |\n",
            t.as_str(),
            t.user_facing_name()
        ));
    }
    out.push_str(
        "\nOmitting `subagent_type` runs `general_purpose` synchronously; `background: true` \
         makes it async; coordinator mode routes every call through a worker.\n",
    );

    out.push_str(&format!(
        "\n### Custom definitions\n\n- project `{}`: {} file(s)\n- user `{}`: {} file(s)\n",
        storage.star_dir().join("agents").display(),
        project.len(),
        crate::core::config::storage::Storage::global_star_dir()
            .join("agents")
            .display(),
        user.len()
    ));
    if project.is_empty() && user.is_empty() {
        out.push_str("\nNo custom agents yet — create one with `/agents create`.\n");
    } else {
        out.push_str("\n| scope | name | description |\n|---|---|---|\n");
        for (scope, list) in [("project", &project), ("user", &user)] {
            for (name, desc) in list.iter().take(20) {
                out.push_str(&format!(
                    "| {} | `{}` | {} |\n",
                    scope,
                    name,
                    if desc.is_empty() {
                        "—"
                    } else {
                        desc.as_str()
                    }
                ));
            }
        }
    }

    out.push_str(&agents_platform_teams(&storage).await);
    push_msg(&mut ctx, out);
    Ok(())
}

/// 团队预设与最近的团队运行记录。
pub(crate) async fn agents_platform_teams(
    storage: &crate::core::config::storage::Storage,
) -> String {
    let mut out = String::from("\n### Team presets\n\n");
    match crate::commands::agent_team_presets::list_team_presets(storage).await {
        Ok((project, user)) if project.is_empty() && user.is_empty() => {
            out.push_str("None saved — `/agents team save` writes `agent-teams.json`.\n");
        }
        Ok((project, user)) => {
            out.push_str("| scope | team | agents | mode |\n|---|---|---|---|\n");
            for (scope, list) in [("project", &project), ("user", &user)] {
                for t in list.iter().take(20) {
                    out.push_str(&format!(
                        "| {} | `{}` | {} | {} |\n",
                        scope,
                        t.name,
                        if t.agents.is_empty() {
                            "—".to_string()
                        } else {
                            t.agents.join(", ")
                        },
                        t.mode.as_deref().unwrap_or("default")
                    ));
                }
            }
        }
        Err(e) => out.push_str(&format!("Could not read presets: {}\n", e)),
    }

    out.push_str("\n### Recent team runs\n\n");
    match crate::commands::agent_team_support::scan_team_run_records(storage).await {
        Ok(runs) if runs.is_empty() => {
            out.push_str("No runs recorded — `/agents team run` creates them.\n");
        }
        Ok(runs) => {
            out.push_str("| run | when | mode | members | rounds |\n|---|---|---|---|---|\n");
            for r in runs.iter().take(10) {
                let when = chrono::DateTime::from_timestamp(r.created_at, 0)
                    .map(|t| {
                        t.with_timezone(&chrono::Local)
                            .format("%Y-%m-%d %H:%M")
                            .to_string()
                    })
                    .unwrap_or_else(|| "-".to_string());
                out.push_str(&format!(
                    "| `{}` | {} | {} | {} | {} |\n",
                    r.run_id,
                    when,
                    r.mode,
                    r.members.len(),
                    r.rounds
                ));
            }
            out.push_str("\nInspect one with `/agents team show-run <run>`.\n");
        }
        Err(e) => out.push_str(&format!("Could not read runs: {}\n", e)),
    }

    out.push_str(
        "\nEverything above is local: definitions are markdown files, presets are JSON, runs are \
         directories under `.star/agent-teams/runs`. There is no hosted agents platform and \
         nothing is published anywhere. Manage them with `/agents`.\n",
    );
    out
}

// ═══════════════════════════════════════════════════════════════════════════
// Assistant view
// ═══════════════════════════════════════════════════════════════════════════

/// `/assistant` — 当前助手的有效配置：模型、模式、指令来源、可用能力。
///
/// 每一行都读真实来源（会话状态、用户设置、磁盘上的指令文件），并给出改它的命令。
pub async fn assistant(mut ctx: CommandContext<'_>, _args: &[String]) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;

    let style = match crate::core::config::settings_manager::SettingsManager::new() {
        Ok(mgr) => mgr
            .load_user_settings()
            .await
            .ok()
            .and_then(|s| s.output_style)
            .unwrap_or_else(|| "default".to_string()),
        Err(_) => "default (settings unavailable)".to_string(),
    };

    let mut out = String::from(
        "## /assistant\n\n### Model\n\n| setting | value | change with |\n|---|---|---|\n",
    );
    out.push_str(&format!(
        "| model | `{}` | `/model` |\n| provider | `{}` | `/provider` |\n",
        if ctx.state.current_model.is_empty() {
            "-"
        } else {
            ctx.state.current_model.as_str()
        },
        ctx.state.current_provider_id.as_deref().unwrap_or("-")
    ));
    out.push_str(&format!(
        "| thinking | effort {:?}{} | `/effort` |\n",
        ctx.state.thinking_effort,
        match ctx.state.current_model_supports_thinking {
            Some(true) => ", model supports it",
            Some(false) => ", model does not support it",
            None => ", support unknown",
        }
    ));
    out.push_str(&format!(
        "| fast mode | {} | `/fast` |\n| poor mode | {} | `/poor` |\n",
        on_off(ctx.state.fast_mode),
        on_off(ctx.state.poor_mode)
    ));

    out.push_str("\n### Behaviour\n\n| setting | value | change with |\n|---|---|---|\n");
    out.push_str(&format!(
        "| approval mode | `{}` — the gate that actually blocks tools | `/mode` |\n",
        approval_label(&ctx.state.approval_mode)
    ));
    out.push_str(&format!(
        "| output style | `{}` | `/output-style` |\n",
        style
    ));
    out.push_str(&format!(
        "| advisor | {} | `/advisor` |\n| proactive suggestions | {} | `/proactive` |\n",
        on_off(ctx.state.advisor_mode),
        on_off(ctx.state.proactive_suggestions.enabled)
    ));
    out.push_str(&format!(
        "| auto-continue | {} ({} left) | `/autonomy` |\n",
        on_off(ctx.state.auto_continue_enabled),
        ctx.state.auto_continue_remaining
    ));

    out.push_str(&assistant_sources(&ctx, &cwd).await);
    push_msg(&mut ctx, out);
    Ok(())
}

/// 助手的指令来源与可用能力（都按磁盘实况统计）。
pub(crate) async fn assistant_sources(ctx: &CommandContext<'_>, cwd: &std::path::Path) -> String {
    let mut out = String::from("\n### Instruction sources\n\n");
    let paths = crate::utils::project_context::get_context_file_paths(cwd);
    if paths.is_empty() {
        out.push_str("No STAR.md / STARCODE.md / CLAUDE.md found — run `/init` to create one.\n");
    } else {
        out.push_str("| file | size |\n|---|---|\n");
        for p in &paths {
            let size = std::fs::metadata(p)
                .map(|m| human_bytes(m.len()))
                .unwrap_or_else(|_| "—".to_string());
            out.push_str(&format!("| `{}` | {} |\n", p.display(), size));
        }
        let merged = crate::utils::project_context::load_merged_project_context(cwd)
            .map(|t| t.chars().count())
            .unwrap_or(0);
        out.push_str(&format!(
            "\n{} character(s) of project instructions are merged into the system prompt.\n",
            merged
        ));
    }

    let count_files = |p: PathBuf| -> usize {
        std::fs::read_dir(p)
            .map(|d| d.flatten().filter(|e| e.path().is_file()).count())
            .unwrap_or(0)
    };
    let star = cwd.join(".star");
    out.push_str(
        "\n### Capabilities in reach\n\n| capability | state | manage with |\n|---|---|---|\n",
    );
    out.push_str(&format!(
        "| MCP servers | {} | `/mcp` |\n",
        match crate::core::mcp::load_project_mcp_config().await {
            Ok(c) => format!(
                "{} declared · runtime ready: {}",
                c.mcp_servers.len(),
                on_off(ctx.state.mcp_ready)
            ),
            Err(e) => format!("config error: {}", e),
        }
    ));
    out.push_str(&format!(
        "| subagents | {} definition(s) | `/agents` |\n| skills | {} file(s) | `/skills` |\n\
         | plugins | {} installed, {} error(s) | `/plugin` |\n| memory | {} file(s) | `/memory` |\n",
        count_files(star.join("agents")),
        count_files(star.join("skills")),
        ctx.state.plugin_installed.len(),
        ctx.state.plugin_errors.len(),
        count_files(star.join("memory"))
    ));

    out.push_str(
        "\nThis is a read-only view of the running session — nothing here is a separate assistant \
         profile, and switching any row takes effect from the next turn. Tool availability itself \
         is decided by the approval mode plus per-tool policy, not by this view; `/permissions` \
         shows the rules and `/hooks` the interception points.\n",
    );
    out
}

// ═══════════════════════════════════════════════════════════════════════════
// Passes / entitlements
// ═══════════════════════════════════════════════════════════════════════════

/// `/passes` — 本地并没有 pass / 额度体系，这里如实说明并列出真正决定访问权的东西：
/// 每个 provider 的凭证与 base URL（额度由 provider 侧决定，本程序不查询）。
pub async fn passes(mut ctx: CommandContext<'_>, _args: &[String]) -> CommandResult {
    let store = crate::core::config::provider_store::ProviderStore::new();
    let cfg = store.load().await.ok();

    let mut out = String::from("## /passes\n\n");
    out.push_str(
        "There is no pass, plan, seat or credit system in this build: nothing local grants or \
         revokes access. What decides whether a request goes through is the provider credential \
         below plus whatever quota that provider enforces on its side.\n\n",
    );

    match cfg.as_ref() {
        None => out.push_str("Provider config could not be read.\n"),
        Some(c) if c.providers.is_empty() => {
            out.push_str("No providers configured yet — run `/provider` to add one.\n")
        }
        Some(c) => {
            out.push_str(
                "| provider | credential | base URL | selected model |\n|---|---|---|---|\n",
            );
            let mut ids: Vec<&String> = c.providers.keys().collect();
            ids.sort();
            for id in ids {
                let p = &c.providers[id];
                let key = match p.api_key.as_deref() {
                    Some(k) if k == crate::llm::API_KEY_NOT_SET => "placeholder".to_string(),
                    Some("") => "empty".to_string(),
                    Some(k) => format!("set, {} chars", k.chars().count()),
                    None => "absent".to_string(),
                };
                // 候选列表的数量也要显示 —— 只看到当前模型时，用户会以为这家
                // provider 只能选这一个（而实际上 API 可能还返回了别的）。
                let candidate_count = p.candidate_models().len();
                let model_cell = match (p.selected_model.as_deref(), candidate_count) {
                    (Some(model), count) if count > 1 => format!("{model} (+{count} candidates)"),
                    (Some(model), _) => model.to_string(),
                    (None, 0) => "—".to_string(),
                    (None, count) => format!("— ({count} candidates)"),
                };
                out.push_str(&format!(
                    "| {}`{}` | {} | {} | {} |\n",
                    if c.active_provider_id.as_deref() == Some(id.as_str()) {
                        "▸ "
                    } else {
                        ""
                    },
                    id,
                    key,
                    p.base_url.as_deref().unwrap_or("provider default"),
                    model_cell
                ));
            }
            out.push_str(
                "\n`▸` marks the active provider. Keys are never printed, only their length.\n",
            );
        }
    }

    out.push_str(&format!(
        "\nEnvironment overrides in effect: `STAR_API_KEY` {} · `STAR_BASE_URL` {}\n",
        if std::env::var("STAR_API_KEY").is_ok() {
            "set (wins over the stored key)"
        } else {
            "unset"
        },
        std::env::var("STAR_BASE_URL").unwrap_or_else(|_| "unset".to_string())
    ));
    out.push_str(
        "\nFor what a session has actually consumed see `/extra-usage`; for what happens when a \
         provider rate-limits you see `/rate-limit-options`. Neither queries a billing or \
         entitlement API — no such call exists in this build.\n",
    );
    push_msg(&mut ctx, out);
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Stickers
// ═══════════════════════════════════════════════════════════════════════════

/// 本地 ASCII 贴纸；纯粹画在会话里，不联网也不寄实体贴纸。
pub(crate) const STICKERS: &[(&str, &str)] = &[
    (
        "ship",
        "   ___|____\n  /  ship  \\\n~~~~~~~~~~~~~~~\n  it's green",
    ),
    ("bug", "   \\   /\n    (o o)\n   /  V  \\\n  reproduced!"),
    ("star", "      *\n     /|\\\n    * + *\n     \\|/\n      *"),
    (
        "coffee",
        "  ( (\n   ) )\n  ______\n |      |]\n \\      /\n  `----'",
    ),
    (
        "rocket",
        "    /\\\n   /  \\\n  | () |\n  |    |\n /|/\\/\\|\\\n   ^  ^",
    ),
    (
        "green",
        "  +------------+\n  | all tests  |\n  |   passed   |\n  +------------+",
    ),
    ("lgtm", "   ,--.\n  ( oo )  LGTM\n   \\__/"),
];

/// `/stickers [<name>]` — 在会话里画一张本地贴纸；无参数列出可用名字。
pub async fn stickers(mut ctx: CommandContext<'_>, args: &[String]) -> CommandResult {
    let names = STICKERS
        .iter()
        .map(|(n, _)| format!("`{}`", n))
        .collect::<Vec<_>>()
        .join(" · ");

    let Some(raw) = args.first() else {
        push_msg(
            &mut ctx,
            format!(
                "## /stickers\n\n{}\n\nDraw one with `/stickers <name>`.\n\nThese are ASCII \
                 stickers rendered into this transcript. Nothing is sent anywhere and no physical \
                 stickers are ordered — there is no sticker service in this build.\n",
                names
            ),
        );
        return Ok(());
    };

    let key = raw.to_lowercase();
    let Some((name, art)) = STICKERS.iter().find(|(n, _)| *n == key.as_str()).copied() else {
        push_msg(
            &mut ctx,
            format!("No sticker called `{}`. Available: {}", raw, names),
        );
        return Ok(());
    };

    push_msg(&mut ctx, format!("**{}**\n\n```\n{}\n```\n", name, art));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autocompact_tokens_accept_compact_units_and_separators() {
        assert_eq!(parse_autocompact_tokens("200k").unwrap(), 200_000);
        assert_eq!(parse_autocompact_tokens("200K tokens").unwrap(), 200_000);
        assert_eq!(parse_autocompact_tokens("1m").unwrap(), 1_000_000);
        assert_eq!(parse_autocompact_tokens("500000").unwrap(), 500_000);
        assert_eq!(parse_autocompact_tokens("50,000").unwrap(), 50_000);
        assert_eq!(parse_autocompact_tokens("50_000").unwrap(), 50_000);
        assert_eq!(parse_autocompact_tokens("12.5k").unwrap(), 12_500);
    }

    #[test]
    fn autocompact_tokens_reject_small_or_invalid_values() {
        assert!(parse_autocompact_tokens("9k").is_err());
        assert!(parse_autocompact_tokens("0").is_err());
        assert!(parse_autocompact_tokens("-5").is_err());
        assert!(parse_autocompact_tokens("lots").is_err());
        assert!(parse_autocompact_tokens("11m").is_err());
        assert!(parse_autocompact_tokens("").is_err());
    }

    #[test]
    fn autocompact_status_formats_thousands_periods() {
        assert_eq!(format_token_count(999), "999");
        assert_eq!(format_token_count(1_000), "1,000");
        assert_eq!(format_token_count(200_000), "200,000");
        assert_eq!(format_token_count(1_000_000), "1,000,000");
    }

    #[tokio::test]
    async fn autocompact_without_arguments_reports_the_live_policy() {
        let mut state = crate::ui::state::ChatState::new();
        let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(1);
        autocompact(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            Vec::new(),
        )
        .await
        .unwrap();

        let output = &state.chat_history.last().expect("status message").content;
        assert!(output.contains("Auto-compact"), "{output}");
        assert!(output.contains("Threshold"), "{output}");
        assert!(output.contains("/autocompact auto|on|off"), "{output}");
    }

    #[tokio::test]
    async fn autocompact_reports_an_invalid_size() {
        let mut state = crate::ui::state::ChatState::new();
        let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(1);
        autocompact(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            vec!["banana".to_string()],
        )
        .await
        .unwrap();

        let output = &state.chat_history.last().expect("error message").content;
        assert!(output.contains("Expected a token count"), "{output}");
    }

    #[test]
    fn detects_cargo_verifiers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        let found = detect_verifiers(dir.path());
        let commands: Vec<&str> = found.iter().map(|v| v.command.as_str()).collect();
        assert!(commands.contains(&"cargo check --all-targets"));
        assert!(commands.contains(&"cargo test"));
        assert!(found.iter().all(|v| v.source == "Cargo.toml"));
    }

    #[test]
    fn detects_package_json_scripts_only_when_declared() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"build":"tsc","lint":"eslint ."}}"#,
        )
        .unwrap();
        let commands: Vec<String> = detect_verifiers(dir.path())
            .into_iter()
            .map(|v| v.command)
            .collect();
        assert!(commands.contains(&"npm run build".to_string()));
        assert!(commands.contains(&"npm run lint".to_string()));
        assert!(!commands.iter().any(|c| c.contains("test")));
    }

    #[test]
    fn verification_section_is_appended_then_replaced_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("STAR.md");
        std::fs::write(&path, "# Project\n\nintro\n\n## Style\n\nkeep it short\n").unwrap();

        let first = verification_section(&[Verifier {
            kind: "test",
            source: "Cargo.toml".into(),
            command: "cargo test".into(),
        }]);
        assert!(!write_verification_section(&path, &first).unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("## Verification"));
        assert!(text.contains("`cargo test`"));

        let second = verification_section(&[Verifier {
            kind: "lint",
            source: "Cargo.toml".into(),
            command: "cargo clippy".into(),
        }]);
        assert!(write_verification_section(&path, &second).unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("## Verification").count(), 1);
        assert!(text.contains("`cargo clippy`"));
        assert!(!text.contains("`cargo test`"));
        assert!(text.contains("## Style"), "later sections must survive");
    }

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace().map(|w| w.to_string()).collect()
    }

    #[test]
    fn schedule_args_split_message_from_time_modifier() {
        // `in <secs>` / `at <HH:MM>` 是修饰，不能留在消息正文里
        let (msg, delay) = split_schedule_args(&words("run the tests in 30"));
        assert_eq!(msg, "run the tests");
        assert_eq!(delay.unwrap(), 30);

        let (msg, delay) = split_schedule_args(&words("check CI AT 23:59"));
        assert_eq!(msg, "check CI");
        assert!(delay.is_ok(), "`at` must be case-insensitive");

        // 没有修饰时整串都是消息，用默认延迟
        let (msg, delay) = split_schedule_args(&words("ping me"));
        assert_eq!(msg, "ping me");
        assert_eq!(delay.unwrap(), DEFAULT_SCHEDULE_DELAY_SECS);
    }

    #[test]
    fn schedule_delay_rejects_nonsense() {
        assert!(
            parse_schedule_delay(&words("in 0")).is_err(),
            "in 0 is not a delay"
        );
        assert!(parse_schedule_delay(&words("in soon")).is_err());
        assert!(
            parse_schedule_delay(&words("in")).is_err(),
            "`in` needs a value"
        );
        assert!(parse_schedule_delay(&words("at 25:00")).is_err());
        assert!(
            parse_schedule_delay(&words("every 5m")).is_err(),
            "only in/at are supported"
        );
    }

    #[test]
    fn schedule_when_reads_naturally() {
        assert!(format_schedule_when(45).contains("45"));
        assert!(
            format_schedule_when(3_600).contains("60"),
            "minutes for long delays"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Command gaps（对标 Claude Code 的命令面补齐）
// ═══════════════════════════════════════════════════════════════════════════
