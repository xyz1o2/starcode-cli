//! parity 命令族 —— git_github（对标 Claude Code 命令面，拆自 parity.rs 2026-11-09）

use crate::commands::execution::{CommandContext, CommandResult};
use crate::commands::parity::core::push_msg;
use crate::commands::parity::core::run;
use crate::commands::parity::core::star_dir;
use crate::runtime::messages::AgentRequest;
use crate::types::ChatEntry;
use std::path::PathBuf;

/// `gh` 是否可用（未安装时给出一致的提示）
pub(crate) fn gh_ready() -> Result<(), String> {
    run("gh", &["--version"]).map(|_| ()).map_err(|_| {
        "GitHub CLI (`gh`) not found. Install it from https://cli.github.com/ and run \
         `gh auth login`."
            .to_string()
    })
}

/// 当前仓库 slug（`owner/repo`），来自 `gh repo view`
pub(crate) fn repo_slug() -> Option<String> {
    run(
        "gh",
        &[
            "repo",
            "view",
            "--json",
            "nameWithOwner",
            "-q",
            ".nameWithOwner",
        ],
    )
    .ok()
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
}

/// `/issue [list | new <title…> | view <n>]` — 通过 `gh` 管理 issue。
pub async fn issue(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if let Err(e) = gh_ready() {
        push_msg(&mut ctx, format!("❌ {}", e));
        return Ok(());
    }
    let Some(slug) = repo_slug() else {
        push_msg(
            &mut ctx,
            "❌ Not inside a GitHub repository (or `gh` is not authenticated).",
        );
        return Ok(());
    };

    let sub = args
        .first()
        .map(|s| s.to_lowercase())
        .unwrap_or_else(|| "list".into());
    let rest: Vec<String> = args.iter().skip(1).cloned().collect();

    let out = match sub.as_str() {
        "list" => match run("gh", &["issue", "list", "--limit", "20"]) {
            Ok(t) if t.is_empty() => format!("No open issues in `{}`.", slug),
            Ok(t) => format!("# Open issues — {}\n\n```\n{}\n```", slug, t),
            Err(e) => format!("❌ {}", e),
        },
        "view" => {
            let Some(n) = rest.first() else {
                push_msg(&mut ctx, "Usage: `/issue view <number>`");
                return Ok(());
            };
            match run("gh", &["issue", "view", n.trim_start_matches('#')]) {
                Ok(t) => format!("```\n{}\n```", t),
                Err(e) => format!("❌ {}", e),
            }
        }
        "new" | "create" => {
            let title = rest.join(" ").trim().to_string();
            if title.is_empty() {
                push_msg(&mut ctx, "Usage: `/issue new <title>`");
                return Ok(());
            }
            // 创建 issue 会对外发布内容 —— 只给出待确认的命令，不直接执行。
            format!(
                "About to open an issue on `{}`:\n\n- title: **{}**\n\n\
                 This publishes to GitHub, so run it yourself when ready:\n\
                 ```bash\ngh issue create --repo {} --title {:?} --body \"\"\n```",
                slug, title, slug, title
            )
        }
        other => format!(
            "Unknown subcommand `{}`. Try `/issue list`, `/issue view <n>`, `/issue new <title>`.",
            other
        ),
    };
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/subscribe-pr [<pr> | rm <pr>]` — 订阅 PR 状态跟踪。
///
/// 与 `GitPrSubscribe` 工具共用 `.star/pr_subscriptions/`，纯本地记录，
/// 不改动 GitHub 上的通知设置。
pub async fn subscribe_pr(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let dir = star_dir()?.join("pr_subscriptions");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let first = args.first().map(|s| s.to_lowercase()).unwrap_or_default();
    if first == "rm" || first == "remove" {
        let Some(n) = args.get(1) else {
            push_msg(&mut ctx, "Usage: `/subscribe-pr rm <number>`");
            return Ok(());
        };
        let n = n.trim_start_matches('#');
        let mut removed = 0usize;
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.ends_with(&format!("_{}.json", n)) && std::fs::remove_file(e.path()).is_ok()
                {
                    removed += 1;
                }
            }
        }
        push_msg(
            &mut ctx,
            format!("🗑️ Removed {} subscription(s) for PR #{}.", removed, n),
        );
        return Ok(());
    }

    // 无参数：列出已订阅的 PR
    if args.is_empty() {
        let mut rows: Vec<String> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let Ok(text) = std::fs::read_to_string(e.path()) else {
                    continue;
                };
                let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                rows.push(format!(
                    "- `{}` #{} — {} _(since {})_",
                    v["repo"].as_str().unwrap_or("?"),
                    v["pr_number"].as_u64().unwrap_or(0),
                    v["last_state"].as_str().unwrap_or("unknown"),
                    v["subscribed_at"].as_str().unwrap_or("?"),
                ));
            }
        }
        rows.sort();
        let out = if rows.is_empty() {
            "No PR subscriptions yet. Add one with `/subscribe-pr <number>`.".to_string()
        } else {
            format!(
                "# PR subscriptions\n\n{}\n\n_Stored in `{}`._",
                rows.join("\n"),
                dir.display()
            )
        };
        push_msg(&mut ctx, out);
        return Ok(());
    }

    // 订阅指定 PR
    let Some(n) = args[0].trim_start_matches('#').parse::<u64>().ok() else {
        push_msg(&mut ctx, "Usage: `/subscribe-pr <number>`");
        return Ok(());
    };
    if let Err(e) = gh_ready() {
        push_msg(&mut ctx, format!("❌ {}", e));
        return Ok(());
    }
    let Some(slug) = repo_slug() else {
        push_msg(&mut ctx, "❌ Not inside a GitHub repository.");
        return Ok(());
    };

    let state = run(
        "gh",
        &[
            "pr",
            "view",
            &n.to_string(),
            "--json",
            "state,title,url",
            "-q",
            "[.state, .title, .url] | join(\" | \")",
        ],
    )
    .unwrap_or_else(|e| format!("unknown ({})", e));

    let payload = serde_json::json!({
        "repo": slug,
        "pr_number": n,
        "subscribed_at": chrono::Utc::now().to_rfc3339(),
        "last_state": state,
    });
    let file = dir.join(format!("{}_{}.json", slug.replace('/', "_"), n));
    std::fs::write(
        &file,
        serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;

    push_msg(
        &mut ctx,
        format!(
            "🔔 Subscribed to `{}` #{}\n\n- state: {}\n- record: `{}`\n\n\
             _Local tracking only — GitHub notification settings are untouched._",
            slug,
            n,
            state,
            file.display()
        ),
    );
    Ok(())
}

/// `/install-github-app` — GitHub 集成状态与安装入口。
///
/// starcode 没有自己的托管 GitHub App；这里报告 `gh` 认证状态、已安装的
/// App，以及本地可用的 GitHub 能力。
pub async fn install_github_app(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let mut out = String::from("# GitHub integration\n\n");

    match gh_ready() {
        Err(e) => out.push_str(&format!("- `gh`: ❌ {}\n", e)),
        Ok(()) => {
            let version = run("gh", &["--version"])
                .unwrap_or_default()
                .lines()
                .next()
                .unwrap_or("gh")
                .to_string();
            out.push_str(&format!("- `gh`: ✅ {}\n", version));
            match run("gh", &["auth", "status"]) {
                Ok(t) | Err(t) => {
                    for line in t.lines().filter(|l| !l.trim().is_empty()).take(4) {
                        out.push_str(&format!("  - {}\n", line.trim()));
                    }
                }
            }
            out.push_str(&format!(
                "- repo: `{}`\n",
                repo_slug().unwrap_or_else(|| "not a GitHub repo".into())
            ));
            if let Ok(apps) = run(
                "gh",
                &[
                    "api",
                    "user/installations",
                    "--jq",
                    ".installations[] | \"\\(.app_slug) → \\(.account.login)\"",
                ],
            ) {
                if !apps.trim().is_empty() {
                    out.push_str("\n## Installed GitHub Apps\n");
                    for line in apps.lines().take(15) {
                        out.push_str(&format!("- {}\n", line));
                    }
                }
            }
        }
    }

    out.push_str(
        "\n## Notes\n\
         - starcode ships no hosted GitHub App, so there is nothing to authorize here.\n\
         - Browse/install apps at <https://github.com/apps>.\n\
         - Local GitHub features work through `gh`: `/pr-comments`, `/issue`, `/subscribe-pr`.\n",
    );
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/install-slack-app` — Slack 集成状态。
///
/// 没有托管 Slack App，但通知系统支持 webhook；这里报告是否已配置。
pub async fn install_slack_app(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let webhook = std::env::var("STAR_SLACK_WEBHOOK_URL")
        .ok()
        .or_else(|| std::env::var("SLACK_WEBHOOK_URL").ok());

    let mut out = String::from("# Slack integration\n\n");
    match &webhook {
        Some(url) => {
            let host = url.split('/').nth(2).unwrap_or("hooks.slack.com");
            out.push_str(&format!("- webhook: ✅ configured (`{}`)\n", host));
        }
        None => out.push_str("- webhook: ❌ not configured\n"),
    }
    out.push_str(&format!(
        "- notifications recorded this session: `{}`\n",
        ctx.state.notifications.get_all().len()
    ));
    out.push_str(
        "\n## Set it up\n\
         1. Create an Incoming Webhook: <https://api.slack.com/messaging/webhooks>\n\
         2. Export it before launching starcode:\n\
         ```bash\nexport STAR_SLACK_WEBHOOK_URL='https://hooks.slack.com/services/…'\n```\n\
         3. Use a `Notification` hook to post events: `/hooks add slack Notification \
         \"curl -s -X POST -d '{\\\"text\\\":\\\"starcode\\\"}' $STAR_SLACK_WEBHOOK_URL\"`\n\
         \n_There is no hosted starcode Slack app to authorize._\n",
    );
    push_msg(&mut ctx, out);
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// 对等实例 / 远程控制
// ═══════════════════════════════════════════════════════════════════════════

/// `/commit-push-pr` — 提交并推送，然后打开一个 PR（对标 Claude Code /commit-push-pr）。
/// 底层复用 git.rs 的 `GitCommand::CommitAndPush`（AI 生成提交信息并提交、推送），
/// PR 创建通过探测 `gh` CLI；`gh` 不可用或仓库没有远程时给出诚实提示。
pub async fn commit_push_pr(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if args.first().map(|s| s.as_str()) == Some("--help")
        || args.first().map(|s| s.as_str()) == Some("-h")
    {
        push_msg(
            &mut ctx,
            "`/commit-push-pr` — commit changes, push to remote and open a pull request.\n\n\
             Uses the same AI commit-message generation as `/commit-and-push`, then runs `gh pr create`.\n\
             Requires the `gh` CLI for the PR step.",
        );
        return Ok(());
    }
    let commit_and_push = crate::commands::git_wrapper(args.clone()).await;
    if let Err(e) = commit_and_push {
        push_msg(
            &mut ctx,
            format!(
                "❌ Commit/push failed: {}\n\n`/commit-push-pr` did not open a PR.",
                e
            ),
        );
        return Ok(());
    }
    if let Err(e) = gh_ready() {
        push_msg(
            &mut ctx,
            format!(
                "✅ Committed and pushed.\n\n⚠️ `gh` CLI is not available ({}), so no PR was opened. Install `gh` and run `gh pr create` manually.",
                e
            ),
        );
        return Ok(());
    }
    match run("gh", &["pr", "create", "--fill"]) {
        Ok(pr_url) => {
            let url = pr_url.trim().lines().last().unwrap_or("").to_string();
            if url.is_empty() {
                push_msg(&mut ctx, "✅ Committed and pushed. `gh pr create` ran (no URL returned).");
            } else {
                push_msg(&mut ctx, format!("✅ PR opened: {}", url));
            }
        }
        Err(e) => push_msg(
            &mut ctx,
            format!(
                "✅ Committed and pushed, but PR creation failed: {}\n\nRun `gh pr create` manually.",
                e
            ),
        ),
    }
    Ok(())
}
