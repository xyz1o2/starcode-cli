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
    push_assistant(
        &mut ctx,
        "Vim mode is not implemented yet in this TUI. Use current keybindings and /help for now.",
    );
    Ok(())
}

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
    let workflows_dir = std::env::current_dir()
        .unwrap_or_default()
        .join(".claude")
        .join("workflows");
    let mut lines = vec!["Workflows".to_string()];
    if workflows_dir.exists() {
        match std::fs::read_dir(&workflows_dir) {
            Ok(entries) => {
                let mut found = false;
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().map_or(false, |e| e == "md" || e == "js") {
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
    push_assistant(&mut ctx, lines.join("\n"));
    Ok(())
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
