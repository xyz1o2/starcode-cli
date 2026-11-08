//! parity 命令族 —— ultra（对标 Claude Code 命令面，拆自 parity.rs 2026-11-09）

use crate::commands::execution::{CommandContext, CommandResult};
use crate::commands::parity::core::approval_label;
use crate::commands::parity::core::ask_agent;
use crate::commands::parity::core::push_msg;
use crate::commands::parity::core::run;
use crate::runtime::messages::AgentRequest;
use crate::types::ChatEntry;
use std::path::PathBuf;

/// `/ultraplan` —— 深度规划：切到 plan 模式并要求模型产出分阶段计划。
///
/// 无参数时报告 [`UltraplanConfig`] 的触发关键字与当前权限模式。
pub async fn ultraplan(mut ctx: CommandContext<'_>, args: &[String]) -> CommandResult {
    use crate::core::ultraplan::UltraplanConfig;

    let task = args.join(" ").trim().to_string();
    let cfg = UltraplanConfig::default();

    if task.is_empty() {
        let mode = approval_label(&ctx.state.approval_mode).to_string();
        let out = format!(
            "## Ultraplan\n\nUsage: `/ultraplan <task>`\n\n\
             - permission mode: **{}** (ultraplan runs in `plan`)\n\
             - auto-trigger keywords: {}\n\
             - max plan depth: {}\n\n\
             Produces a phased plan (phases → steps → files → tools), an effort estimate and a\n\
             risk list, without touching the working tree. Approve it with `/mode default`.\n",
            mode,
            cfg.auto_trigger_keywords
                .iter()
                .map(|k| format!("`{}`", k))
                .collect::<Vec<_>>()
                .join(", "),
            cfg.max_plan_depth,
        );
        push_msg(&mut ctx, out);
        return Ok(());
    }

    // 深度规划必须只读：先切 plan 模式再把任务交给模型。
    crate::commands::permissions::run(
        CommandContext {
            state: &mut *ctx.state,
            agent_tx: ctx.agent_tx,
        },
        vec!["plan".to_string()],
    )
    .await?;

    let prompt = format!(
        "Ultraplan request — produce a deep implementation plan for the task below. \
         Do not modify any files; research first with read-only tools.\n\n\
         Task: {task}\n\n\
         Structure the answer as:\n\
         1. Understanding — what exists today, with concrete file:line references.\n\
         2. Phases (at most {depth}) — each with an ordered list of steps; per step name the \
         files to touch and the tools needed.\n\
         3. Effort — total estimate, complexity (low/medium/high/very-high) and your confidence.\n\
         4. Risks — severity plus a mitigation for each.\n\
         5. Verification — the exact build/test/lint commands that prove the plan landed.\n\n\
         Call out anything you could not verify instead of assuming it.",
        task = task,
        depth = cfg.max_plan_depth
    );
    ask_agent(&mut ctx, prompt).await
}

/// `/ultrareview` —— 深度审查：先取真实 diff 范围，再要求模型做「发现 + 自证」两轮。
///
/// 与 `/review` 的区别是明确的审查维度和「无法复现的发现必须丢弃」要求。
pub async fn ultrareview(mut ctx: CommandContext<'_>, args: &[String]) -> CommandResult {
    let explicit = args.join(" ").trim().to_string();

    // 真实变更范围：优先未提交改动，否则退回最近一次提交。
    let unstaged = run("git", &["diff", "--stat"]).unwrap_or_default();
    let staged = run("git", &["diff", "--cached", "--stat"]).unwrap_or_default();
    let dirty = !unstaged.is_empty() || !staged.is_empty();
    let scope_cmd = if dirty {
        "git diff HEAD"
    } else {
        "git show HEAD"
    };
    let files = if dirty {
        run("git", &["diff", "HEAD", "--name-only"]).unwrap_or_default()
    } else {
        run("git", &["show", "--name-only", "--format=", "HEAD"]).unwrap_or_default()
    };
    let file_list: Vec<&str> = files.lines().filter(|l| !l.trim().is_empty()).collect();

    let target = if !explicit.is_empty() {
        explicit.clone()
    } else if file_list.is_empty() {
        push_msg(
            &mut ctx,
            "No uncommitted changes and no files in `HEAD` to review.\n\n\
             Usage: `/ultrareview [path|range|description]`",
        );
        return Ok(());
    } else {
        format!("the {} changed file(s) in `{}`", file_list.len(), scope_cmd)
    };

    let scope_note = if explicit.is_empty() {
        format!(
            "Scope: `{}`\nFiles:\n{}\n\n",
            scope_cmd,
            file_list
                .iter()
                .take(40)
                .map(|f| format!("- `{}`", f))
                .collect::<Vec<_>>()
                .join("\n")
        )
    } else {
        String::new()
    };

    let prompt = format!(
        "Ultrareview — deep, adversarial review of {target}. Review only; do not modify files.\n\n\
         {scope_note}\
         Pass 1 — find. Read the actual code (not just the diff hunks) and look for:\n\
         - correctness: wrong logic, off-by-one, error paths that swallow failures\n\
         - regressions: callers/tests that the change breaks\n\
         - concurrency: shared state, await points holding locks, cancellation\n\
         - security: injection, path traversal, secret exposure, missing authz\n\
         - resource use: unbounded growth, blocking calls in async paths\n\
         - test coverage: behaviour changed with no test proving it\n\n\
         Pass 2 — verify. For each candidate finding, construct the concrete input or state that \
         triggers it and trace it to the wrong output. Drop every finding you cannot substantiate \
         that way, and say how many you dropped.\n\n\
         Report survivors most-severe first as `file:line — defect — failure scenario — fix`. \
         If nothing survives, say so plainly and list the residual risks you could not rule out.",
        target = target,
        scope_note = scope_note
    );
    ask_agent(&mut ctx, prompt).await
}

// ═══════════════════════════════════════════════════════════════════════════
// Briefings / side channel
// ═══════════════════════════════════════════════════════════════════════════

/// `/btw` —— 旁路提问：走 [`NoteKind::Aside`]，不进入主对话上下文。
pub async fn btw(mut ctx: CommandContext<'_>, args: &[String]) -> CommandResult {
    let question = args.join(" ").trim().to_string();
    if question.is_empty() {
        push_msg(
            &mut ctx,
            "Usage: `/btw <question>`\n\n\
             Answers a one-off question in a side channel: the question and the answer never \
             enter the main conversation context, so the current task is not disturbed.",
        );
        return Ok(());
    }
    crate::commands::extended::request_note(
        &mut ctx,
        crate::runtime::messages::NoteKind::Aside,
        Some(question),
    )
    .await
}

/// `/think-back` —— 回放本次会话里真实记录的 thinking 轨迹。
///
/// 数据来自 `ChatEntry::reasoning_content`，纯本地读取，不调用模型。
pub async fn think_back(mut ctx: CommandContext<'_>, args: &[String]) -> CommandResult {
    let traces: Vec<(usize, String)> = ctx
        .state
        .chat_history
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            e.reasoning_content
                .as_ref()
                .map(|r| r.trim().to_string())
                .filter(|r| !r.is_empty())
                .map(|r| (i, r))
        })
        .collect();

    if traces.is_empty() {
        push_msg(
            &mut ctx,
            "No thinking traces in this session.\n\n\
             Traces are only recorded when the active model streams reasoning — check \
             `/model` support and that thinking is enabled.",
        );
        return Ok(());
    }

    let arg = args.first().map(|s| s.as_str()).unwrap_or("");
    let (selected, header): (Vec<&(usize, String)>, String) = match arg {
        "" | "last" => (
            traces.iter().rev().take(1).collect(),
            "latest trace".to_string(),
        ),
        "all" => (traces.iter().collect(), format!("{} traces", traces.len())),
        n => match n.parse::<usize>() {
            Ok(k) if k >= 1 && k <= traces.len() => (
                vec![&traces[traces.len() - k]],
                format!("trace #{} from the end", k),
            ),
            _ => {
                push_msg(
                    &mut ctx,
                    format!(
                        "Usage: `/think-back [last|all|<n>]` — {} trace(s) available.",
                        traces.len()
                    ),
                );
                return Ok(());
            }
        },
    };

    let mut out = format!("## Thinking replay — {}\n\n", header);
    for (idx, text) in selected {
        let words = text.split_whitespace().count();
        out.push_str(&format!(
            "### entry {} · {} words\n\n> {}\n\n",
            idx,
            words,
            text.replace('\n', "\n> ")
        ));
    }
    out.push_str(&format!(
        "_{} trace(s) in this session · `/think-back all` for every one._\n",
        traces.len()
    ));
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/brief` —— 项目简报：先采集真实仓库事实，再让模型写简报。
pub async fn brief(mut ctx: CommandContext<'_>, args: &[String]) -> CommandResult {
    let focus = args.join(" ").trim().to_string();
    let facts = repo_facts();

    let prompt = format!(
        "Write a project brief for this repository{focus}. Read the code you need with read-only \
         tools before writing; do not modify anything.\n\n\
         Cover: what the project is and who it is for; how it is structured (top-level modules and \
         what each owns); how to build, test and run it; the conventions a new contributor must \
         follow; and what is currently in flight (uncommitted work, recent commits).\n\
         Ground every claim in files you actually read and cite them as `path:line`. \
         Say explicitly when something is unclear rather than guessing.\n\n\
         ## Collected facts\n{facts}",
        focus = if focus.is_empty() {
            String::new()
        } else {
            format!(", focused on {}", focus)
        },
        facts = facts
    );
    ask_agent(&mut ctx, prompt).await
}

/// 采集仓库客观事实（供 /brief、/weekly-report 作为 grounding）。
pub(crate) fn repo_facts() -> String {
    let mut out = String::new();
    if let Ok(cwd) = std::env::current_dir() {
        out.push_str(&format!("- cwd: `{}`\n", cwd.display()));
    }
    if let Ok(branch) = run("git", &["rev-parse", "--abbrev-ref", "HEAD"]) {
        out.push_str(&format!("- branch: `{}`\n", branch));
    }
    if let Ok(remote) = run("git", &["remote", "get-url", "origin"]) {
        out.push_str(&format!("- origin: `{}`\n", remote));
    }
    if let Ok(count) = run("git", &["rev-list", "--count", "HEAD"]) {
        out.push_str(&format!("- commits: {}\n", count));
    }
    if let Ok(status) = run("git", &["status", "--porcelain"]) {
        let n = status.lines().filter(|l| !l.trim().is_empty()).count();
        out.push_str(&format!("- uncommitted changes: {} file(s)\n", n));
    }
    if let Ok(log) = run("git", &["log", "-10", "--pretty=%h %s"]) {
        if !log.is_empty() {
            out.push_str("- recent commits:\n");
            for line in log.lines() {
                out.push_str(&format!("  - {}\n", line));
            }
        }
    }
    let manifests = [
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "go.mod",
        "pom.xml",
        "Makefile",
        "CLAUDE.md",
        "README.md",
    ];
    let present: Vec<&str> = manifests
        .iter()
        .copied()
        .filter(|f| std::path::Path::new(f).exists())
        .collect();
    if !present.is_empty() {
        out.push_str(&format!("- manifests present: {}\n", present.join(", ")));
    }
    if let Ok(tree) = run("git", &["ls-files"]) {
        let files: Vec<&str> = tree.lines().collect();
        out.push_str(&format!("- tracked files: {}\n", files.len()));
        let mut tops: Vec<&str> = files
            .iter()
            .filter_map(|f| f.split('/').next())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        tops.truncate(25);
        out.push_str(&format!("- top level: {}\n", tops.join(", ")));
    }
    out
}

/// `/weekly-report` —— 用真实 git 历史生成周报。
///
/// `/weekly-report [days] [--me]`：默认 7 天；`--me` 只统计当前 git user。
pub async fn weekly_report(mut ctx: CommandContext<'_>, args: &[String]) -> CommandResult {
    let mut days: i64 = 7;
    let mut only_me = false;
    for a in args {
        match a.as_str() {
            "--me" | "me" | "--mine" => only_me = true,
            other => {
                if let Ok(d) = other.trim_end_matches('d').parse::<i64>() {
                    if d > 0 {
                        days = d;
                    }
                }
            }
        }
    }

    let since = format!("--since={} days ago", days);
    let author = run("git", &["config", "user.name"]).unwrap_or_default();
    let mut log_args: Vec<String> = vec![
        "log".into(),
        since.clone(),
        "--pretty=%h|%an|%ad|%s".into(),
        "--date=short".into(),
    ];
    if only_me && !author.is_empty() {
        log_args.push(format!("--author={}", author));
    }
    let log_ref: Vec<&str> = log_args.iter().map(|s| s.as_str()).collect();
    let log = run("git", &log_ref).unwrap_or_default();

    if log.trim().is_empty() {
        push_msg(
            &mut ctx,
            format!(
                "No commits in the last {} day(s){}.\n\n\
                 Usage: `/weekly-report [days] [--me]`",
                days,
                if only_me {
                    format!(" by `{}`", author)
                } else {
                    String::new()
                }
            ),
        );
        return Ok(());
    }

    let commits: Vec<&str> = log.lines().collect();
    let mut stat_args: Vec<String> = vec!["diff".into(), "--shortstat".into()];
    if let Ok(oldest) = run(
        "git",
        &["log", &since, "--pretty=%H", "--reverse", "--max-count=1"],
    ) {
        if !oldest.is_empty() {
            stat_args.push(format!("{}^..HEAD", oldest));
        }
    }
    let stat_ref: Vec<&str> = stat_args.iter().map(|s| s.as_str()).collect();
    let shortstat = run("git", &stat_ref).unwrap_or_default();

    let mut facts = format!(
        "- window: last {} day(s)\n- commits: {}\n{}",
        days,
        commits.len(),
        if shortstat.is_empty() {
            String::new()
        } else {
            format!("- churn: {}\n", shortstat)
        }
    );
    if only_me {
        facts.push_str(&format!("- author filter: `{}`\n", author));
    }
    facts.push_str("- commit log (`hash|author|date|subject`):\n");
    for line in commits.iter().take(200) {
        facts.push_str(&format!("  - {}\n", line));
    }

    let prompt = format!(
        "Write a weekly engineering report from the git history below. Group the commits into \
         themes rather than listing them one by one, and for each theme say what changed and why \
         it matters. Then add: work in progress (uncommitted or clearly unfinished), and risks or \
         follow-ups the log implies. Keep it factual — the log is the only evidence; if you need \
         detail it does not contain, read the repository, and mark anything you could not confirm.\n\n\
         ## Facts\n{}",
        facts
    );
    ask_agent(&mut ctx, prompt).await
}
