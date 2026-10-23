use crate::commands::execution::{CommandContext, CommandResult};
use crate::runtime::messages::AgentRequest;
use crate::tools::github_pr_comments::GhPrCommentsTool;
use crate::types::ChatEntry;

fn push_assistant(ctx: &mut CommandContext<'_>, content: impl Into<String>) {
    ctx.state
        .chat_history
        .push(ChatEntry::assistant(content).with_streaming(false));
}

fn mask_secret(secret: &str) -> String {
    let len = secret.chars().count();
    if len <= 8 {
        "********".to_string()
    } else {
        let head: String = secret.chars().take(4).collect();
        let tail: String = secret
            .chars()
            .rev()
            .take(4)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        format!("{}...{}", head, tail)
    }
}

pub async fn cost(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let mut lines = vec!["Cost / Usage".to_string()];
    if let Some(usage) = &ctx.state.token_usage {
        lines.push(format!("- Prompt tokens: {}", usage.prompt_tokens));
        lines.push(format!("- Completion tokens: {}", usage.completion_tokens));
        lines.push(format!("- Total tokens: {}", usage.total_tokens));
    } else {
        lines.push("- Token usage: unavailable (no completed response yet)".to_string());
    }

    if ctx.state.total_cost > 0.0 {
        lines.push(format!("- Estimated cost: ${:.6}", ctx.state.total_cost));
    } else {
        lines.push("- Estimated cost: unavailable".to_string());
    }
    lines.push("- Tip: use /stats session for session counters.".to_string());

    push_assistant(&mut ctx, lines.join("\n"));
    Ok(())
}

pub async fn review(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let target = if args.is_empty() {
        "the current repository changes".to_string()
    } else {
        args.join(" ")
    };

    let review_prompt = format!(
        "Run a strict code review for {}.\n\
         Focus on: correctness bugs, behavioral regressions, security issues, and missing tests.\n\
         Return findings ordered by severity.\n\
         Each finding should include file references and concrete remediation.\n\
         If no issues are found, explicitly say so and mention residual risks.\n\
         Do not modify files; review only.",
        target
    );

    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: review_prompt,
        })
        .await
        .map_err(|e| e.to_string())?;

    push_assistant(
        &mut ctx,
        "Started code review. I will return findings once analysis completes.",
    );
    Ok(())
}

fn parse_pr_comments_args(args: &[String]) -> Result<(Option<u64>, Option<String>), String> {
    let mut pr_number: Option<u64> = None;
    let mut repo: Option<String> = None;
    let mut i = 0usize;

    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "-h" | "--help" => {
                return Err("Usage: /pr_comments [--pr <number>] [--repo <owner/repo>]".to_string())
            }
            "-p" | "--pr" => {
                let next = args
                    .get(i + 1)
                    .ok_or_else(|| "missing value for --pr".to_string())?;
                pr_number = Some(
                    next.parse::<u64>()
                        .map_err(|_| format!("invalid PR number: {}", next))?,
                );
                i += 2;
                continue;
            }
            "-r" | "--repo" => {
                let next = args
                    .get(i + 1)
                    .ok_or_else(|| "missing value for --repo".to_string())?;
                repo = Some(next.clone());
                i += 2;
                continue;
            }
            _ if arg.starts_with("--pr=") => {
                let v = arg.trim_start_matches("--pr=");
                pr_number = Some(
                    v.parse::<u64>()
                        .map_err(|_| format!("invalid PR number: {}", v))?,
                );
                i += 1;
                continue;
            }
            _ if arg.starts_with("--repo=") => {
                let v = arg.trim_start_matches("--repo=");
                repo = Some(v.to_string());
                i += 1;
                continue;
            }
            _ => {
                if pr_number.is_none() && arg.chars().all(|c| c.is_ascii_digit()) {
                    pr_number = Some(
                        arg.parse::<u64>()
                            .map_err(|_| format!("invalid PR number: {}", arg))?,
                    );
                } else if repo.is_none() && arg.contains('/') {
                    repo = Some(arg.to_string());
                } else {
                    return Err(format!(
                        "unknown argument: {}\nUsage: /pr_comments [--pr <number>] [--repo <owner/repo>]",
                        arg
                    ));
                }
                i += 1;
            }
        }
    }

    Ok((pr_number, repo))
}

pub async fn pr_comments(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let (pr_number, repo) = parse_pr_comments_args(&args)?;
    let tool = GhPrCommentsTool::new();

    match tool.fetch(pr_number, repo).await {
        Ok(result) => {
            if let Some(err) = result.error {
                push_assistant(
                    &mut ctx,
                    format!("Failed to fetch PR comments: {}", err.message),
                );
                return Ok(());
            }
            let output = result.output.trim();
            if output.is_empty() {
                push_assistant(&mut ctx, "No PR comments found.");
            } else {
                push_assistant(&mut ctx, format!("PR comments:\n\n{}", output));
            }
            Ok(())
        }
        Err(e) => Err(format!("failed to run gh_pr_comments: {}", e)),
    }
}

/// 列出当前仓库的 Pull Requests（依赖 gh CLI）
pub async fn prs(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let limit = args
        .iter()
        .find_map(|a| a.strip_prefix("--limit=").map(|v| v.to_string()))
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(10);

    let output = std::process::Command::new("gh")
        .args(["pr", "list", "--limit", &limit.to_string()])
        .output()
        .map_err(|e| format!("failed to run gh CLI: {} (is gh installed?)", e))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        push_assistant(&mut ctx, format!("Failed to list PRs:\n{}", err));
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        push_assistant(&mut ctx, "No open pull requests found.");
    } else {
        push_assistant(&mut ctx, format!("Open pull requests:\n\n{}", stdout));
    }
    Ok(())
}

pub async fn bug(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let summary = if args.is_empty() {
        "<describe the issue>".to_string()
    } else {
        args.join(" ")
    };
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ".".to_string());
    let model = if ctx.state.current_model.trim().is_empty() {
        "<unknown>"
    } else {
        ctx.state.current_model.as_str()
    };
    let log_path = crate::utils::logging::debug_log_path_display();

    let content = format!(
        "Bug Report Template\n\
         \n\
         Summary: {}\n\
         Version: {}\n\
         Model: {}\n\
         Approval mode: {:?}\n\
         Workspace: {}\n\
         Debug log: {}\n\
         \n\
         Repro Steps:\n\
         1. <step 1>\n\
         2. <step 2>\n\
         3. <actual vs expected>\n\
         \n\
         You can attach recent logs and the exact command/input that triggered the issue.",
        summary,
        env!("CARGO_PKG_VERSION"),
        model,
        ctx.state.approval_mode,
        cwd,
        log_path
    );

    push_assistant(&mut ctx, content);
    Ok(())
}

pub async fn terminal_setup(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let setup = crate::core::terminal_setup::TerminalSetup::detect();

    let content = format!(
        "Terminal Setup\n\
         \n\
         Detected shell: {}\n\
         Config path: {}\n\
         Integration installed: {}\n\
         \n\
         {}\n\
         \n\
         To install shell integration, run:\n\
         /terminal-setup install\n\
         \n\
         To uninstall shell integration, run:\n\
         /terminal-setup uninstall\n\
         \n\
         Optional env vars:\n\
         - STAR_API_KEY=<your_key>\n\
         - STAR_BASE_URL=<provider_url>\n\
         - STAR_MODEL=<model_id>",
        setup.get_shell_name(),
        setup.config_path,
        setup.integration_installed,
        setup.get_setup_instructions()
    );

    push_assistant(&mut ctx, content);
    Ok(())
}

pub async fn vim(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    ctx.state.vim_enabled = !ctx.state.vim_enabled;
    let status = if ctx.state.vim_enabled {
        ctx.state.vim_state.mode = crate::ui::vim::VimMode::Normal;
        "Vim mode: ON (Normal mode)"
    } else {
        "Vim mode: OFF"
    };
    push_assistant(&mut ctx, status);
    Ok(())
}

/// `/mouse` — 开关鼠标捕获。
///
/// 开：滚轮以真实滚轮事件到达，不被终端转成箭头键污染输入历史。
/// 关：**终端原生选区恢复**（捕获开着时终端整个会话都不再自己做选区，而本应用
/// 只在聊天区实现了左键拖选）。
pub async fn mouse(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let want = match args.first().map(|a| a.to_ascii_lowercase()) {
        Some(v) if v == "on" || v == "1" || v == "true" => Some(true),
        Some(v) if v == "off" || v == "0" || v == "false" => Some(false),
        _ => None,
    }
    .unwrap_or(!ctx.state.mouse_capture_enabled);

    if want == ctx.state.mouse_capture_enabled {
        push_assistant(
            &mut ctx,
            if want {
                "Mouse capture already ON"
            } else {
                "Mouse capture already OFF"
            },
        );
        return Ok(());
    }

    match crate::ui::utils::term_recovery::set_mouse_capture(want) {
        Ok(()) => {
            ctx.state.mouse_capture_enabled = want;
            push_assistant(
                &mut ctx,
                if want {
                    "Mouse capture: ON — wheel scrolls directly; terminal text selection is unavailable while it is on (/mouse off to restore)"
                } else {
                    "Mouse capture: OFF — terminal text selection restored; wheel now arrives as arrow keys"
                },
            );
        }
        Err(e) => {
            return Err(format!("Mouse capture toggle failed: {e}"));
        }
    }
    Ok(())
}

#[derive(Debug)]
struct LoginArgs {
    api_key: String,
    base_url: Option<String>,
    model: Option<String>,
}

fn parse_login_args(args: &[String]) -> Result<LoginArgs, String> {
    let mut api_key: Option<String> = None;
    let mut base_url: Option<String> = None;
    let mut model: Option<String> = None;
    let mut i = 0usize;

    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "-h" | "--help" => {
                return Err(
                    "Usage: /login --api-key <key> [--base-url <url>] [--model <id>]".to_string(),
                )
            }
            "-k" | "--api-key" => {
                let next = args
                    .get(i + 1)
                    .ok_or_else(|| "missing value for --api-key".to_string())?;
                api_key = Some(next.clone());
                i += 2;
                continue;
            }
            "-u" | "--base-url" => {
                let next = args
                    .get(i + 1)
                    .ok_or_else(|| "missing value for --base-url".to_string())?;
                base_url = Some(next.clone());
                i += 2;
                continue;
            }
            "-m" | "--model" => {
                let next = args
                    .get(i + 1)
                    .ok_or_else(|| "missing value for --model".to_string())?;
                model = Some(next.clone());
                i += 2;
                continue;
            }
            _ if arg.starts_with("--api-key=") => {
                api_key = Some(arg.trim_start_matches("--api-key=").to_string());
                i += 1;
                continue;
            }
            _ if arg.starts_with("--base-url=") => {
                base_url = Some(arg.trim_start_matches("--base-url=").to_string());
                i += 1;
                continue;
            }
            _ if arg.starts_with("--model=") => {
                model = Some(arg.trim_start_matches("--model=").to_string());
                i += 1;
                continue;
            }
            _ => {
                if api_key.is_none() {
                    api_key = Some(arg.to_string());
                    i += 1;
                } else {
                    return Err(format!("unknown argument: {}", arg));
                }
            }
        }
    }

    let api_key = api_key.ok_or_else(|| {
        "missing API key\nUsage: /login --api-key <key> [--base-url <url>] [--model <id>]"
            .to_string()
    })?;
    Ok(LoginArgs {
        api_key,
        base_url,
        model,
    })
}

pub async fn login(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let parsed = parse_login_args(&args)?;
    let manager = crate::core::config::settings_manager::get_settings_manager()
        .await
        .map_err(|e| e.to_string())?;
    let mut settings = manager
        .load_user_settings()
        .await
        .map_err(|e| e.to_string())?;

    settings.api_key = Some(parsed.api_key.clone());
    if let Some(base_url) = parsed.base_url.clone() {
        settings.base_url = Some(base_url);
    }
    if let Some(model) = parsed.model.clone() {
        settings.default_model = Some(model.clone());
        ctx.state.current_model = model;
    }

    manager
        .save_user_settings(&settings)
        .await
        .map_err(|e| e.to_string())?;

    let mut lines = vec![
        "Login settings saved.".to_string(),
        format!("- API key: {}", mask_secret(&parsed.api_key)),
    ];
    if let Some(url) = parsed.base_url {
        lines.push(format!("- Base URL: {}", url));
    }
    if let Some(model) = parsed.model {
        lines.push(format!("- Default model: {}", model));
    }
    push_assistant(&mut ctx, lines.join("\n"));
    Ok(())
}

// ── /code-review — strict code review ─────────────────────────

pub async fn code_review(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let target = if args.is_empty() {
        "the current repository changes".to_string()
    } else {
        args.join(" ")
    };
    let prompt = format!(
        "Run a strict code review for {}.\n\
         Focus on: correctness bugs, behavioral regressions, security issues, and missing tests.\n\
         Return findings ordered by severity (critical > high > medium > low).\n\
         Each finding must include: file path, line range, root cause, concrete fix.\n\
         If no issues are found, explicitly state so and mention residual risks.\n\
         Do not modify files; review only.",
        target
    );
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: prompt,
        })
        .await
        .map_err(|e| e.to_string())?;
    push_assistant(
        &mut ctx,
        "Started code review. Findings will be returned once analysis completes.",
    );
    Ok(())
}

// ── /security-review — security audit ─────────────────────────

pub async fn security_review(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let target = if args.is_empty() {
        "the current repository changes".to_string()
    } else {
        args.join(" ")
    };
    let prompt = format!(
        "Complete a security review of {}.\n\
         Check for: injection risks, authentication bypass, insecure data handling, \
         dependency vulnerabilities, exposed secrets, unsafe deserialization, \
         privilege escalation paths, and supply chain risks.\n\
         Return findings ordered by severity with file references and mitigations.\n\
         If no issues, explicitly state residual risks.\n\
         Do not modify files; review only.",
        target
    );
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: prompt,
        })
        .await
        .map_err(|e| e.to_string())?;
    push_assistant(
        &mut ctx,
        "Started security review. Findings will be returned once analysis completes.",
    );
    Ok(())
}

// ── /simplify — code simplification ────────────────────────────

pub async fn simplify(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let target = if args.is_empty() {
        "the current repository changes".to_string()
    } else {
        args.join(" ")
    };
    let prompt = format!(
        "Review {} for simplification and efficiency improvements.\n\
         Look for: dead code, duplicated logic, over-complicated patterns, \
         unnecessary allocations, redundant error handling, and cleaner alternatives.\n\
         For each finding, show the current code and the simplified version.\n\
         Only make changes that are safe and preserve behavior.",
        target
    );
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: prompt,
        })
        .await
        .map_err(|e| e.to_string())?;
    push_assistant(
        &mut ctx,
        "Started simplification analysis. Suggestions will be returned once analysis completes.",
    );
    Ok(())
}

// ── /run — trigger project run ─────────────────────────────────

pub async fn run(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let target = if args.is_empty() {
        "the project".to_string()
    } else {
        args.join(" ")
    };
    let prompt = format!(
        "Launch and drive {} to see a change working.\n\
         First look for a project skill or script that already covers launching the app;\n\
         otherwise use the appropriate run pattern for the project type.\n\
         Report the result and any issues encountered.",
        target
    );
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: prompt,
        })
        .await
        .map_err(|e| e.to_string())?;
    push_assistant(&mut ctx, "Running project...");
    Ok(())
}

// ── /verify — drive the real app and observe the change ────────

pub(crate) fn verify_prompt(target: &str) -> String {
    format!(
        "Verify that the change actually works end to end: {}.\n\
         Do not stop at reading code or running static checks. Find the project's run/verify skill \
         or the canonical local run path, launch the app or service, drive the changed behavior, \
         and record what you observed (commands, input, output, logs, screenshots when available).\n\
         If the app cannot be launched, report the exact blocker and fall back to the closest \
         executable check; do not claim success without evidence.\n\
         Do not modify files unless the user explicitly asked for a fix. End with PASS/FAIL, the \
         evidence, and any remaining risk.",
        target
    )
}

pub async fn verify(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let target = if args.is_empty() {
        "the current change".to_string()
    } else {
        args.join(" ")
    };
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: verify_prompt(&target),
        })
        .await
        .map_err(|e| e.to_string())?;
    push_assistant(
        &mut ctx,
        "Started verification. I will report the observed behavior, evidence, and any blocker.",
    );
    Ok(())
}

// ── /feedback — feedback template ──────────────────────────────

pub async fn feedback(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let content = format!(
        "Feedback & Suggestions\n\n\
         StarCode CLI v{}\n\n\
         Please send feedback to the project repository or maintainer.\n\
         \n\
         Common feedback channels:\n\
         - GitHub Issues (bug reports, feature requests)\n\
         - GitHub Discussions (questions, ideas)\n\
         \n\
         When reporting issues, include:\n\
         - Version: {}\n\
         - OS: {}\n\
         - Shell: {}\n\
         - Steps to reproduce\n\
         - Expected vs actual behavior",
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::var("SHELL").unwrap_or_else(|_| "unknown".to_string()),
    );
    push_assistant(&mut ctx, content);
    Ok(())
}

// ── /tasks — show and manage tasks ─────────────────────────────

pub async fn tasks(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    // 无参数：打开交互式任务面板（与 Ctrl+T 相同），而不是倾倒原始 JSON
    ctx.state.task_panel.reload();
    ctx.state.task_panel.manually_hidden = false;
    ctx.state.task_panel.is_visible = true;
    ctx.state.current_status_line = Some("Task panel opened (Ctrl+T to toggle)".to_string());
    Ok(())
}

// ── /workflows — list workflow definitions ─────────────────────

pub async fn workflows(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let cwd = std::env::current_dir().unwrap_or_default();
    workflows_in(&mut ctx, &cwd);
    Ok(())
}

/// `cwd` 走参数：单测要能对着临时目录验证"有/没有 workflow 文件"两种输出。
fn workflows_in(ctx: &mut CommandContext<'_>, cwd: &std::path::Path) {
    let workflows_dir = cwd.join(".claude").join("workflows");
    let mut lines = vec!["Workflows".to_string()];
    if workflows_dir.exists() {
        match std::fs::read_dir(&workflows_dir) {
            Ok(entries) => {
                let mut found = false;
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().is_some_and(|e| e == "md" || e == "js") {
                        lines.push(format!(
                            "- {} ({})",
                            path.file_stem()
                                .map(|s| s.to_string_lossy().to_string())
                                .unwrap_or_default(),
                            path.display()
                        ));
                        found = true;
                    }
                }
                if !found {
                    lines.push("(no workflow files found)".to_string());
                }
            }
            Err(e) => lines.push(format!("Error reading workflows: {}", e)),
        }
    } else {
        lines.push("No .claude/workflows directory found in project.".to_string());
    }
    lines.push("\nAgent workflows are also available via the Workflow tool.".to_string());
    push_assistant(ctx, lines.join("\n"));
}

// ── /context — show context stats ──────────────────────────────

pub async fn context(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let history_len = ctx.state.chat_history.len();
    let token_info = if let Some(usage) = &ctx.state.token_usage {
        format!(
            "Prompt tokens: {}\nCompletion tokens: {}\nTotal tokens: {}",
            usage.prompt_tokens, usage.completion_tokens, usage.total_tokens
        )
    } else {
        "Token usage: not yet available (no completed response)".to_string()
    };
    let model = if ctx.state.current_model.is_empty() {
        "<default>"
    } else {
        &ctx.state.current_model
    };
    let content = format!(
        "Context Stats\n\n\
         Messages in history: {}\n\
         Current model: {}\n\
         {}\n\
         \n\
         Use /compress to compact context if it grows too large.\n\
         Use /cost for cost estimation.",
        history_len, model, token_info
    );
    push_assistant(&mut ctx, content);
    Ok(())
}

// ── /bashes — show recent bash history ─────────────────────────

pub async fn bashes(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let bash_history_path = std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".bash_history"))
        .unwrap_or_default();
    let content = if bash_history_path.exists() {
        match tokio::fs::read_to_string(&bash_history_path).await {
            Ok(raw) => {
                let recent: Vec<&str> = raw.lines().rev().take(20).collect();
                if recent.is_empty() {
                    "No recent bash history found.".to_string()
                } else {
                    format!(
                        "Recent bash history (last {}):\n\n{}",
                        recent.len(),
                        recent
                            .iter()
                            .enumerate()
                            .map(|(i, l)| format!("  {}. {}", i + 1, l))
                            .collect::<Vec<_>>()
                            .join("\n")
                    )
                }
            }
            Err(e) => format!("Failed to read bash history: {}", e),
        }
    } else {
        "No .bash_history file found. Use ! prefix for inline bash execution.".to_string()
    };
    push_assistant(&mut ctx, content);
    Ok(())
}

// ── /lint — trigger lint analysis ──────────────────────────────

pub async fn lint(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let prompt = "Run linting/static analysis on the current project.\n\
                  Auto-detect the appropriate linter (clippy for Rust, eslint for JS/TS, ruff for Python, etc.) \
                  and report any warnings or errors.\n\
                  If no linter is configured, suggest setting one up.";
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: prompt.to_string(),
        })
        .await
        .map_err(|e| e.to_string())?;
    push_assistant(&mut ctx, "Running lint analysis...");
    Ok(())
}

// ── /upgrade — check for updates ───────────────────────────────

pub async fn upgrade(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let content = format!(
        "StarCode CLI v{}\n\n\
         To upgrade:\n\
         - Git install: cd to repo and run `git pull && cargo install --path .`\n\
         - Script install: re-run the install script\n\
         - Check for latest release: visit the project repository\n\
         \n\
         Current binary: {}",
        env!("CARGO_PKG_VERSION"),
        std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "unknown".to_string()),
    );
    push_assistant(&mut ctx, content);
    Ok(())
}

// ── /ide — IDE integration info ────────────────────────────────

pub async fn ide(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let content = "IDE Integration\n\n\
                   StarCode CLI supports IDE integration via:\n\
                   - MCP (Model Context Protocol) server mode\n\
                   - External editor integration (VSCode, JetBrains, etc.)\n\
                   \n\
                   MCP Server:\n\
                   - Start with: starcode-cli mcp serve\n\
                   - Configure in your IDE's MCP settings\n\
                   \n\
                   See /help for more commands."
        .to_string();
    push_assistant(&mut ctx, content);
    Ok(())
}

// ── /forget — remove memory by keyword ─────────────────────────

pub async fn forget(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if args.is_empty() {
        push_assistant(
            &mut ctx,
            "Usage: /forget <keyword or topic>\n\nRemoves memory entries matching the keyword.\nUse /memory show to view current memories first.",
        );
        return Ok(());
    }
    let keyword = args.join(" ");
    let prompt = format!(
        "Remove any memory entries related to: \"{}\".\n\
         Use the memory tool to update the memory file.\n\
         Only remove the specific entries that match this topic.",
        keyword
    );
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: prompt,
        })
        .await
        .map_err(|e| e.to_string())?;
    push_assistant(&mut ctx, format!("Forgetting: {}...", keyword));
    Ok(())
}

pub async fn logout(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let clear_all = args.iter().any(|a| a == "--all");
    let has_unknown = args.iter().any(|a| a != "--all");
    if has_unknown {
        return Err("Usage: /logout [--all]".to_string());
    }

    let manager = crate::core::config::settings_manager::get_settings_manager()
        .await
        .map_err(|e| e.to_string())?;
    let mut settings = manager
        .load_user_settings()
        .await
        .map_err(|e| e.to_string())?;

    settings.api_key = None;
    if clear_all {
        settings.base_url = None;
        settings.default_model = None;
        settings.active_provider_id = None;
    }

    manager
        .save_user_settings(&settings)
        .await
        .map_err(|e| e.to_string())?;

    if clear_all {
        push_assistant(
            &mut ctx,
            "Logged out and cleared api_key/base_url/default_model from user settings.",
        );
    } else {
        push_assistant(&mut ctx, "Logged out (API key removed from user settings).");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn ctx_parts() -> (
        crate::ui::state::ChatState,
        mpsc::Sender<AgentRequest>,
        mpsc::Receiver<AgentRequest>,
    ) {
        let (tx, rx) = mpsc::channel(4);
        (crate::ui::state::ChatState::test_fixture(), tx, rx)
    }

    fn last(state: &crate::ui::state::ChatState) -> String {
        state
            .chat_history
            .last()
            .map(|entry| entry.content.clone())
            .unwrap_or_default()
    }

    /// 走到模型的命令必须：先递增 message_id 再发送，且只发一条 SendMessage。
    /// 复用 id 会让下一轮的 `Done` 被当成重复而吞掉。
    ///
    /// `expected_id` 由调用方在构造 future **之前**取，因为构造之后
    /// `state` 就被可变借走了。
    async fn expect_single_send(
        fut: impl std::future::Future<Output = CommandResult>,
        rx: &mut mpsc::Receiver<AgentRequest>,
        expected_id: u64,
    ) -> String {
        fut.await.unwrap();
        match rx.try_recv() {
            Ok(AgentRequest::SendMessage {
                message_id,
                message,
            }) => {
                assert_eq!(
                    message_id, expected_id,
                    "the id must be taken before sending"
                );
                assert!(rx.try_recv().is_err(), "exactly one request per command");
                message
            }
            other => panic!("expected SendMessage, got {other:?}"),
        }
    }

    // ── mask_secret ──────────────────────────────────────────────

    #[test]
    fn short_secrets_are_fully_masked() {
        assert_eq!(mask_secret(""), "********");
        assert_eq!(mask_secret("abc"), "********");
        // 8 个字符还是会把大半露出来，所以边界定在 8
        assert_eq!(mask_secret("12345678"), "********");
    }

    #[test]
    fn long_secrets_keep_only_the_ends() {
        let masked = mask_secret("sk-ant-api03-abcdefghijklmnop");
        assert!(masked.starts_with("sk-a"), "{masked}");
        assert!(masked.ends_with("mnop"), "{masked}");
        assert!(masked.contains("..."), "{masked}");
        assert!(!masked.contains("api03"), "{masked}");
    }

    // ── /cost & /stats 的口径 ───────────────────────────────────

    #[tokio::test]
    async fn cost_reports_token_usage_and_estimated_cost() {
        let (mut state, tx, _rx) = ctx_parts();
        state.token_usage = Some(crate::types::StarUsage {
            prompt_tokens: 1000,
            completion_tokens: 250,
            total_tokens: 1250,
            ..Default::default()
        });
        state.total_cost = 0.0123456;

        cost(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            Vec::new(),
        )
        .await
        .unwrap();

        let out = last(&state);
        assert!(out.contains("- Prompt tokens: 1000"), "{out}");
        assert!(out.contains("- Completion tokens: 250"), "{out}");
        assert!(out.contains("- Total tokens: 1250"), "{out}");
        assert!(out.contains("- Estimated cost: $0.012346"), "{out}");
        assert!(out.contains("/stats session"), "{out}");
    }

    /// 没有用量时不能印出 "0 tokens"，那会被读成"这次真的没花 token"。
    #[tokio::test]
    async fn cost_says_unavailable_when_nothing_was_measured() {
        let (mut state, tx, _rx) = ctx_parts();
        state.token_usage = None;
        state.total_cost = 0.0;

        cost(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            Vec::new(),
        )
        .await
        .unwrap();

        let out = last(&state);
        assert!(out.contains("Token usage: unavailable"), "{out}");
        assert!(out.contains("Estimated cost: unavailable"), "{out}");
    }

    // ── 走模型的评审类命令 ──────────────────────────────────────

    #[tokio::test]
    async fn review_defaults_to_repository_changes_and_accepts_a_target() {
        let (mut state, tx, mut rx) = ctx_parts();
        let expected_id = state.next_message_id;

        let prompt = expect_single_send(
            review(
                CommandContext {
                    state: &mut state,
                    agent_tx: &tx,
                },
                Vec::new(),
            ),
            &mut rx,
            expected_id,
        )
        .await;
        assert!(
            prompt.contains("the current repository changes"),
            "{prompt}"
        );
        assert!(prompt.contains("Do not modify files"), "{prompt}");
        assert!(last(&state).contains("Started code review"));

        let (mut state, tx, mut rx) = ctx_parts();
        let expected_id = state.next_message_id;

        let prompt = expect_single_send(
            review(
                CommandContext {
                    state: &mut state,
                    agent_tx: &tx,
                },
                vec!["src".to_string(), "auth.rs".to_string()],
            ),
            &mut rx,
            expected_id,
        )
        .await;
        assert!(prompt.contains("src auth.rs"), "{prompt}");
    }

    #[tokio::test]
    async fn code_review_demands_file_and_line_references() {
        let (mut state, tx, mut rx) = ctx_parts();
        let expected_id = state.next_message_id;

        let prompt = expect_single_send(
            code_review(
                CommandContext {
                    state: &mut state,
                    agent_tx: &tx,
                },
                Vec::new(),
            ),
            &mut rx,
            expected_id,
        )
        .await;
        assert!(
            prompt.contains("critical > high > medium > low"),
            "{prompt}"
        );
        assert!(prompt.contains("file path, line range"), "{prompt}");
        assert!(prompt.contains("Do not modify files"), "{prompt}");
    }

    #[tokio::test]
    async fn security_review_names_the_attack_classes_to_check() {
        let (mut state, tx, mut rx) = ctx_parts();
        let expected_id = state.next_message_id;

        let prompt = expect_single_send(
            security_review(
                CommandContext {
                    state: &mut state,
                    agent_tx: &tx,
                },
                Vec::new(),
            ),
            &mut rx,
            expected_id,
        )
        .await;
        for needle in [
            "injection risks",
            "authentication bypass",
            "exposed secrets",
            "supply chain",
            "Do not modify files",
        ] {
            assert!(prompt.contains(needle), "missing {needle}:\n{prompt}");
        }
        assert!(last(&state).contains("Started security review"));
    }

    #[tokio::test]
    async fn simplify_asks_for_before_after_and_preserved_behavior() {
        let (mut state, tx, mut rx) = ctx_parts();
        let expected_id = state.next_message_id;

        let prompt = expect_single_send(
            simplify(
                CommandContext {
                    state: &mut state,
                    agent_tx: &tx,
                },
                Vec::new(),
            ),
            &mut rx,
            expected_id,
        )
        .await;
        assert!(
            prompt.contains("show the current code and the simplified version"),
            "{prompt}"
        );
        assert!(prompt.contains("preserve behavior"), "{prompt}");
    }

    #[tokio::test]
    async fn run_and_lint_forward_their_prompts() {
        let (mut state, tx, mut rx) = ctx_parts();
        let expected_id = state.next_message_id;

        let prompt = expect_single_send(
            run(
                CommandContext {
                    state: &mut state,
                    agent_tx: &tx,
                },
                vec!["the web app".to_string()],
            ),
            &mut rx,
            expected_id,
        )
        .await;
        assert!(prompt.contains("the web app"), "{prompt}");
        assert!(last(&state).contains("Running project"));

        let (mut state, tx, mut rx) = ctx_parts();
        let expected_id = state.next_message_id;

        let prompt = expect_single_send(
            lint(
                CommandContext {
                    state: &mut state,
                    agent_tx: &tx,
                },
                Vec::new(),
            ),
            &mut rx,
            expected_id,
        )
        .await;
        assert!(prompt.contains("clippy for Rust"), "{prompt}");
        assert!(prompt.contains("eslint"), "{prompt}");
        assert!(last(&state).contains("Running lint analysis"));
    }

    /// `/forget` 不带关键词时不能把"清空所有记忆"发出去 —— 那是一条不可逆指令。
    #[tokio::test]
    async fn forget_without_a_keyword_never_reaches_the_model() {
        let (mut state, tx, mut rx) = ctx_parts();
        forget(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            Vec::new(),
        )
        .await
        .unwrap();

        assert!(rx.try_recv().is_err(), "no request expected");
        let out = last(&state);
        assert!(out.contains("Usage: /forget <keyword or topic>"), "{out}");
        assert!(out.contains("/memory show"), "{out}");
    }

    #[tokio::test]
    async fn forget_with_a_keyword_asks_for_a_scoped_removal() {
        let (mut state, tx, mut rx) = ctx_parts();
        let expected_id = state.next_message_id;

        let prompt = expect_single_send(
            forget(
                CommandContext {
                    state: &mut state,
                    agent_tx: &tx,
                },
                vec!["deploy".to_string(), "steps".to_string()],
            ),
            &mut rx,
            expected_id,
        )
        .await;
        assert!(prompt.contains("\"deploy steps\""), "{prompt}");
        assert!(
            prompt.contains("Only remove the specific entries"),
            "{prompt}"
        );
        assert!(last(&state).contains("Forgetting: deploy steps"));
    }

    // ── /tasks & /context ───────────────────────────────────────

    /// `/tasks` 打开交互面板，不往聊天里倾倒 JSON。
    #[tokio::test]
    async fn tasks_opens_the_panel_instead_of_printing_a_list() {
        let (mut state, tx, _rx) = ctx_parts();
        state.task_panel.is_visible = false;
        state.task_panel.manually_hidden = true;
        let before = state.chat_history.len();

        tasks(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            Vec::new(),
        )
        .await
        .unwrap();

        assert!(state.task_panel.is_visible);
        assert!(!state.task_panel.manually_hidden);
        assert_eq!(state.chat_history.len(), before, "no chat entry expected");
        assert!(state
            .current_status_line
            .as_deref()
            .unwrap_or_default()
            .contains("Task panel opened"));
    }

    #[tokio::test]
    async fn context_reports_history_and_token_usage() {
        let (mut state, tx, _rx) = ctx_parts();
        state.chat_history.push(ChatEntry::user("hi"));
        // 命令自己会再追加一条报告，所以先记下它看到的长度
        let expected_messages = state.chat_history.len();
        state.current_model = "claude-sonnet-5-5".to_string();
        state.token_usage = Some(crate::types::StarUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
            ..Default::default()
        });

        context(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            Vec::new(),
        )
        .await
        .unwrap();

        let out = last(&state);
        assert!(
            out.contains(&format!("Messages in history: {expected_messages}")),
            "{out}"
        );
        assert!(out.contains("Current model: claude-sonnet-5-5"), "{out}");
        assert!(out.contains("Total tokens: 15"), "{out}");
        assert!(out.contains("/compress"), "{out}");
    }

    #[tokio::test]
    async fn context_degrades_to_a_placeholder_model_name() {
        let (mut state, tx, _rx) = ctx_parts();
        state.current_model.clear();
        state.token_usage = None;

        context(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            Vec::new(),
        )
        .await
        .unwrap();

        let out = last(&state);
        assert!(out.contains("Current model: <default>"), "{out}");
        assert!(out.contains("Token usage: not yet available"), "{out}");
    }

    // ── /workflows ──────────────────────────────────────────────

    #[test]
    fn workflows_reports_a_missing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx, _rx) = ctx_parts();
        let mut ctx = CommandContext {
            state: &mut state,
            agent_tx: &tx,
        };
        workflows_in(&mut ctx, dir.path());
        let out = last(&state);
        assert!(
            out.contains("No .claude/workflows directory found"),
            "{out}"
        );
    }

    #[test]
    fn workflows_lists_markdown_and_js_files_only() {
        let dir = tempfile::tempdir().unwrap();
        let wf = dir.path().join(".claude").join("workflows");
        std::fs::create_dir_all(&wf).unwrap();
        std::fs::write(wf.join("deploy.md"), "steps").unwrap();
        std::fs::write(wf.join("build.js"), "steps").unwrap();
        std::fs::write(wf.join("notes.txt"), "ignored").unwrap();

        let (mut state, tx, _rx) = ctx_parts();
        let mut ctx = CommandContext {
            state: &mut state,
            agent_tx: &tx,
        };
        workflows_in(&mut ctx, dir.path());
        let out = last(&state);
        assert!(out.contains("- deploy ("), "{out}");
        assert!(out.contains("- build ("), "{out}");
        assert!(!out.contains("notes"), "{out}");
    }

    #[test]
    fn an_empty_workflows_directory_says_so() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".claude").join("workflows")).unwrap();

        let (mut state, tx, _rx) = ctx_parts();
        let mut ctx = CommandContext {
            state: &mut state,
            agent_tx: &tx,
        };
        workflows_in(&mut ctx, dir.path());
        assert!(last(&state).contains("(no workflow files found)"));
    }

    // ── /pr_comments 参数解析 ───────────────────────────────────

    #[test]
    fn pr_comments_accepts_flags_and_positional_forms() {
        assert_eq!(parse_pr_comments_args(&[]).unwrap(), (None, None));
        assert_eq!(
            parse_pr_comments_args(&["--pr".into(), "42".into()]).unwrap(),
            (Some(42), None)
        );
        assert_eq!(
            parse_pr_comments_args(&["-p".into(), "42".into()]).unwrap(),
            (Some(42), None)
        );
        assert_eq!(
            parse_pr_comments_args(&["--pr=42".into()]).unwrap(),
            (Some(42), None)
        );
        // 位置参数：纯数字是 PR 号，含 `/` 的是仓库
        assert_eq!(
            parse_pr_comments_args(&["42".into(), "org/repo".into()]).unwrap(),
            (Some(42), Some("org/repo".to_string()))
        );
        assert_eq!(
            parse_pr_comments_args(&["--repo=org/repo".into()]).unwrap(),
            (None, Some("org/repo".to_string()))
        );
    }

    #[test]
    fn pr_comments_rejects_bad_input_with_a_usage_hint() {
        let err = parse_pr_comments_args(&["--help".into()]).unwrap_err();
        assert!(err.starts_with("Usage: /pr_comments"), "{err}");

        let err = parse_pr_comments_args(&["--pr".into()]).unwrap_err();
        assert!(err.contains("missing value for --pr"), "{err}");

        let err = parse_pr_comments_args(&["--pr".into(), "abc".into()]).unwrap_err();
        assert!(err.contains("invalid PR number: abc"), "{err}");

        // 第二个非数字/非仓库参数没法解释，必须报错而不是静默丢弃
        let err =
            parse_pr_comments_args(&["42".into(), "org/repo".into(), "extra".into()]).unwrap_err();
        assert!(err.contains("unknown argument: extra"), "{err}");
    }

    // ── /login 参数解析 ─────────────────────────────────────────

    #[test]
    fn login_parses_key_url_and_model_in_every_form() {
        let a = parse_login_args(&["sk-123".into()]).unwrap();
        assert_eq!(a.api_key, "sk-123");
        assert!(a.base_url.is_none() && a.model.is_none());

        let a = parse_login_args(&[
            "--api-key".into(),
            "sk-123".into(),
            "--base-url".into(),
            "https://api.example.com".into(),
            "--model".into(),
            "m1".into(),
        ])
        .unwrap();
        assert_eq!(a.api_key, "sk-123");
        assert_eq!(a.base_url.as_deref(), Some("https://api.example.com"));
        assert_eq!(a.model.as_deref(), Some("m1"));

        let a = parse_login_args(&[
            "--api-key=sk-9".into(),
            "--base-url=https://x".into(),
            "--model=m2".into(),
        ])
        .unwrap();
        assert_eq!(a.api_key, "sk-9");
        assert_eq!(a.base_url.as_deref(), Some("https://x"));
        assert_eq!(a.model.as_deref(), Some("m2"));
    }

    #[test]
    fn login_without_a_key_explains_the_syntax() {
        let err = parse_login_args(&[]).unwrap_err();
        assert!(err.contains("missing API key"), "{err}");
        assert!(err.contains("Usage: /login --api-key <key>"), "{err}");

        let err = parse_login_args(&["--api-key".into()]).unwrap_err();
        assert!(err.contains("missing value for --api-key"), "{err}");

        // 两个位置参数里第二个没人认领
        let err = parse_login_args(&["sk-1".into(), "extra".into()]).unwrap_err();
        assert!(err.contains("unknown argument: extra"), "{err}");
    }

    /// 帮助入口不能用 "missing API key" 敷衍过去 —— 那是错误，不是帮助。
    #[test]
    fn login_help_returns_the_usage_line() {
        let err = parse_login_args(&["-h".into()]).unwrap_err();
        assert!(err.starts_with("Usage: /login"), "{err}");
    }

    // ── /logout 参数校验 ────────────────────────────────────────

    #[tokio::test]
    async fn logout_rejects_unknown_flags_before_touching_settings() {
        let (mut state, tx, _rx) = ctx_parts();
        let err = logout(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            vec!["--everything".to_string()],
        )
        .await
        .unwrap_err();
        assert_eq!(err, "Usage: /logout [--all]");
    }

    // ── 静态文本类命令 ──────────────────────────────────────────

    #[tokio::test]
    async fn bug_renders_a_fillable_report_with_environment_details() {
        let (mut state, tx, _rx) = ctx_parts();
        state.current_model = "claude-sonnet-5-5".to_string();

        bug(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            vec!["crash".to_string(), "on".to_string(), "start".to_string()],
        )
        .await
        .unwrap();

        let out = last(&state);
        assert!(out.contains("Bug Report Template"), "{out}");
        assert!(out.contains("Summary: crash on start"), "{out}");
        assert!(out.contains(env!("CARGO_PKG_VERSION")), "{out}");
        assert!(out.contains("Model: claude-sonnet-5-5"), "{out}");
        assert!(out.contains("Debug log:"), "{out}");
        assert!(out.contains("Repro Steps:"), "{out}");
    }

    #[tokio::test]
    async fn feedback_and_upgrade_report_the_build_identity() {
        let (mut state, tx, _rx) = ctx_parts();
        feedback(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            Vec::new(),
        )
        .await
        .unwrap();
        let out = last(&state);
        assert!(out.contains("Feedback & Suggestions"), "{out}");
        assert!(out.contains(env!("CARGO_PKG_VERSION")), "{out}");
        assert!(out.contains(std::env::consts::OS), "{out}");

        upgrade(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            Vec::new(),
        )
        .await
        .unwrap();
        let out = last(&state);
        assert!(out.contains("To upgrade:"), "{out}");
        assert!(out.contains("Current binary:"), "{out}");
    }

    #[tokio::test]
    async fn terminal_setup_and_ide_explain_how_to_configure() {
        let (mut state, tx, _rx) = ctx_parts();
        terminal_setup(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            Vec::new(),
        )
        .await
        .unwrap();
        let out = last(&state);
        assert!(out.contains("Terminal Setup"), "{out}");
        assert!(out.contains("Detected shell:"), "{out}");
        assert!(out.contains("/terminal-setup install"), "{out}");
        assert!(out.contains("STAR_API_KEY"), "{out}");

        ide(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            Vec::new(),
        )
        .await
        .unwrap();
        let out = last(&state);
        assert!(out.contains("IDE Integration"), "{out}");
        assert!(out.contains("starcode-cli mcp serve"), "{out}");
    }

    /// `/vim` 是开关：开的时候必须同时把模式切回 Normal，否则光标停在上次的
    /// Insert 里，用户按 Esc 才有反应。
    #[tokio::test]
    async fn vim_toggles_and_resets_to_normal_mode() {
        let (mut state, tx, _rx) = ctx_parts();
        state.vim_enabled = false;
        state.vim_state.mode = crate::ui::vim::VimMode::Insert;

        vim(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            Vec::new(),
        )
        .await
        .unwrap();
        assert!(state.vim_enabled);
        assert_eq!(state.vim_state.mode, crate::ui::vim::VimMode::Normal);
        assert!(last(&state).contains("Vim mode: ON"));

        vim(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            Vec::new(),
        )
        .await
        .unwrap();
        assert!(!state.vim_enabled);
        assert!(last(&state).contains("Vim mode: OFF"));
    }

    #[test]
    fn verify_prompt_requires_runtime_evidence_not_just_static_checks() {
        let prompt = verify_prompt("the login form");
        assert!(prompt.contains("the login form"));
        assert!(prompt.contains("launch the app or service"));
        assert!(prompt.contains("Do not stop at reading code or running static checks"));
        assert!(prompt.contains("Do not modify files"));
        assert!(prompt.contains("PASS/FAIL"));
    }

    #[tokio::test]
    async fn verify_sends_the_prompt_and_reports_started_state() {
        let mut state = crate::ui::state::ChatState::new();
        let (agent_tx, mut agent_rx) = mpsc::channel(1);

        verify(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            vec!["src/login.rs".to_string()],
        )
        .await
        .unwrap();

        match agent_rx.recv().await {
            Some(AgentRequest::SendMessage { message, .. }) => {
                assert!(message.contains("src/login.rs"));
                assert!(message.contains("end to end"));
            }
            other => panic!("expected SendMessage, got {:?}", other),
        }
        let reply = &state.chat_history.last().unwrap().content;
        assert!(reply.contains("Started verification"));
        assert!(reply.contains("observed behavior"));
    }
}
