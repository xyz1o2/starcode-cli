//! 对标 Claude Code 的补齐命令实现。
//!
//! 这些命令此前只在 [`crate::commands::system::ALL_COMMANDS`] 里声明
//! （category = "Pending"），执行时返回占位提示。本模块把它们逐个接到
//! 仓库里已有的 `core/*` 管理器与 UI 状态上。
//!
//! 约定：
//! - 只读命令直接把 markdown 结果 push 进 `chat_history`（不进 LLM 上下文）；
//! - 需要模型参与的命令（/brief、/ultrareview…）走 [`ask_agent`]；
//! - 需要旁路生成、不污染主上下文的走 `AgentRequest::GenerateNote`。

use crate::commands::execution::{CommandContext, CommandResult};
use crate::runtime::messages::AgentRequest;
use crate::types::ChatEntry;
use std::path::PathBuf;

fn push_msg(ctx: &mut CommandContext<'_>, content: impl Into<String>) {
    ctx.state
        .chat_history
        .push(ChatEntry::assistant(content).with_streaming(false));
}

/// 把提示交给主对话回路（与 /git-status 等命令一致）
async fn ask_agent(ctx: &mut CommandContext<'_>, prompt: String) -> CommandResult {
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    ctx.agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: prompt,
        })
        .await
        .map_err(|e| e.to_string())
}

/// 同步执行外部命令，返回 stdout；非零退出码带上 stderr。
fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("`{}` not available: {}", program, e))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() {
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if stderr.is_empty() { stdout } else { stderr })
    }
}

fn on_off(enabled: bool) -> &'static str {
    if enabled {
        "on"
    } else {
        "off"
    }
}

/// 项目 `.star/` 目录（按需创建）
fn star_dir() -> Result<PathBuf, String> {
    let dir = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .join(".star");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn approval_label(mode: &crate::types::ApprovalMode) -> &'static str {
    match mode {
        crate::types::ApprovalMode::Default => "default",
        crate::types::ApprovalMode::Plan => "plan",
        crate::types::ApprovalMode::Yolo => "yolo",
    }
}

/// 敏感环境变量只显示是否设置与长度，不回显明文
fn is_secretish(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    ["KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL", "COOKIE"]
        .iter()
        .any(|k| n.contains(k))
}

fn redact(name: &str, value: &str) -> String {
    if is_secretish(name) {
        format!("<set, {} chars>", value.chars().count())
    } else if value.chars().count() > 96 {
        let head: String = value.chars().take(96).collect();
        format!("{}…", head)
    } else {
        value.to_string()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Session / environment
// ═══════════════════════════════════════════════════════════════════════════

/// `/env` — 有效环境变量与运行诊断（对标 Claude Code /env）。
pub async fn env(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let mut out = String::from("# Environment\n\n## Runtime\n");
    out.push_str(&format!(
        "- version: `starcode-cli {}`\n- os: `{} {}`\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    if let Ok(exe) = std::env::current_exe() {
        out.push_str(&format!("- binary: `{}`\n", exe.display()));
    }
    if let Ok(cwd) = std::env::current_dir() {
        out.push_str(&format!("- cwd: `{}`\n", cwd.display()));
    }
    for key in ["SHELL", "TERM", "TERM_PROGRAM", "LANG"] {
        if let Ok(v) = std::env::var(key) {
            out.push_str(&format!("- {}: `{}`\n", key.to_lowercase(), v));
        }
    }

    out.push_str("\n## Session\n");
    out.push_str(&format!(
        "- model: `{}`\n- provider: `{}`\n- approval mode: `{}`\n- thinking effort: `{}`\n",
        ctx.state.current_model,
        ctx.state.current_provider_id.as_deref().unwrap_or("-"),
        approval_label(&ctx.state.approval_mode),
        ctx.state.thinking_effort.display_name()
    ));
    out.push_str(&format!(
        "- sandbox: `{}`\n- fast mode: `{}`\n- extra working dirs: `{}`\n",
        on_off(ctx.state.sandbox_enabled),
        on_off(ctx.state.fast_mode),
        ctx.state.extra_working_dirs.len()
    ));
    if let Some(branch) = &ctx.state.git_branch {
        out.push_str(&format!("- git branch: `{}`\n", branch));
    }

    out.push_str("\n## Config paths\n");
    let mut paths: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".star").join("settings.json"));
        paths.push(home.join(".star").join("skills"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        paths.push(cwd.join(".star"));
        paths.push(cwd.join("config.toml"));
    }
    for p in paths {
        out.push_str(&format!(
            "- `{}` {}\n",
            p.display(),
            if p.exists() { "✅" } else { "—" }
        ));
    }

    out.push_str("\n## Environment variables\n");
    let mut vars: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| {
            let u = k.to_ascii_uppercase();
            u.starts_with("STAR_")
                || u.starts_with("ANTHROPIC_")
                || u.starts_with("OPENAI_")
                || u.starts_with("CLAUDE_")
                || u.ends_with("_PROXY")
                || u == "NO_PROXY"
        })
        .collect();
    vars.sort();
    if vars.is_empty() {
        out.push_str("_none set_\n");
    } else {
        for (k, v) in vars {
            let shown = redact(&k, &v);
            out.push_str(&format!("- `{}` = `{}`\n", k, shown));
        }
    }
    out.push_str("\n_Secret-looking values are redacted._\n");

    push_msg(&mut ctx, out);
    Ok(())
}

/// `/release-notes` — 当前版本信息 + 自上一个 tag 起的提交。
pub async fn release_notes(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let version = env!("CARGO_PKG_VERSION");
    let mut out = format!("# Release notes — starcode-cli v{}\n\n", version);

    // 优先展示仓库内的 changelog
    let changelog = ["CHANGELOG.md", "docs/CHANGELOG.md"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(p).ok());

    if let Some(text) = changelog {
        let head: Vec<&str> = text.lines().take(80).collect();
        out.push_str(&head.join("\n"));
        out.push('\n');
    } else {
        let last_tag = run("git", &["describe", "--tags", "--abbrev=0"]).ok();
        let range = last_tag
            .as_ref()
            .map(|t| format!("{}..HEAD", t))
            .unwrap_or_else(|| "HEAD".to_string());
        out.push_str(&match last_tag {
            Some(ref t) => format!("Changes since `{}`:\n\n", t),
            None => "No git tags found — showing recent commits:\n\n".to_string(),
        });
        match run("git", &["log", "--oneline", "--no-decorate", "-30", &range]) {
            Ok(log) if !log.is_empty() => {
                for line in log.lines() {
                    out.push_str(&format!("- {}\n", line));
                }
            }
            Ok(_) => out.push_str("_No commits in this range._\n"),
            Err(e) => out.push_str(&format!("_git log failed: {}_\n", e)),
        }
    }
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/history` — 浏览本项目的输入历史（`~/.star/history/<cwd-hash>.json`）。
/// `/history <query>` 过滤，`/history clear` 清空。
pub async fn history(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let sub = args.first().map(|s| s.to_lowercase()).unwrap_or_default();

    if sub == "clear" {
        ctx.state.command_history.clear();
        ctx.state.history_index = None;
        crate::core::config::history_store::save_history(&ctx.state.command_history);
        push_msg(&mut ctx, "✅ Input history cleared for this project.");
        return Ok(());
    }

    if sub == "search" || sub == "find" {
        // 交给已有的历史搜索浮层（Ctrl+R 同一入口）
        ctx.state.close_palette();
        ctx.state.open_history_search();
        return Ok(());
    }

    let query = args.join(" ").trim().to_lowercase();
    let entries: Vec<&String> = ctx
        .state
        .command_history
        .iter()
        .filter(|e| query.is_empty() || e.to_lowercase().contains(&query))
        .take(40)
        .collect();

    let mut out = if query.is_empty() {
        format!(
            "# Input history ({} stored)\n\n",
            ctx.state.command_history.len()
        )
    } else {
        format!("# Input history matching `{}`\n\n", query)
    };
    if entries.is_empty() {
        out.push_str("_No matching entries._\n");
    } else {
        for (i, e) in entries.iter().enumerate() {
            let first = e.lines().next().unwrap_or("").trim();
            let extra = e.lines().count().saturating_sub(1);
            let suffix = if extra > 0 {
                format!(" _(+{} lines)_", extra)
            } else {
                String::new()
            };
            out.push_str(&format!("{:>3}. {}{}\n", i + 1, first, suffix));
        }
        out.push_str("\n_↑/↓ in the prompt walks the same history; Ctrl+R searches it. `/history clear` wipes it._\n");
    }
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/mode` — 切换权限/审批模式（对标 Claude Code /mode）。
/// 无参数展示当前模式与可选项；带参数复用 `/permissions` 的切换实现。
pub async fn mode(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if args.is_empty() {
        let current = approval_label(&ctx.state.approval_mode);
        let out = format!(
            "# Permission mode\n\nCurrent: **{}**\n\n\
             - `/mode default` — ask before risky writes/commands (Claude Code: acceptEdits)\n\
             - `/mode plan` — read-only research & planning, no mutating tools\n\
             - `/mode bypass` — run everything without asking (Claude Code: bypassPermissions)\n\n\
             _Rules and the deny log live under `/permissions`._",
            current
        );
        push_msg(&mut ctx, out);
        return Ok(());
    }

    let arg = args[0].to_lowercase();
    let mapped = match arg.as_str() {
        "default" | "build" | "acceptedits" | "accept-edits" => "default",
        "plan" | "readonly" | "read-only" => "plan",
        "bypass" | "yolo" | "auto" | "bypasspermissions" | "bypass-permissions" => "yolo",
        other => {
            push_msg(
                &mut ctx,
                format!(
                    "❌ Unknown mode: `{}`. Usage: /mode [default|plan|bypass]",
                    other
                ),
            );
            return Ok(());
        }
    };
    crate::commands::permissions::run(ctx, vec![mapped.to_string()]).await
}

/// `/output-style` — 设置输出风格（对标 Claude Code /output-style）。
/// 无参数打开选择面板；带参数直接写入用户设置。
pub async fn output_style(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if args.is_empty() {
        ctx.state.close_palette();
        ctx.state
            .open_palette(crate::ui::state::palette::PaletteMode::OutputStyle);
        return Ok(());
    }

    let style = match args[0].to_lowercase().as_str() {
        "default" | "normal" | "balanced" => "default",
        "concise" | "short" | "brief" => "concise",
        "verbose" | "detailed" | "explanatory" => "verbose",
        other => {
            push_msg(
                &mut ctx,
                format!(
                    "❌ Unknown output style: `{}`. Usage: /output-style [default|concise|verbose]",
                    other
                ),
            );
            return Ok(());
        }
    };

    let style_owned = style.to_string();
    match crate::core::config::settings_manager::SettingsManager::new() {
        Ok(mgr) => {
            let mut settings = mgr.load_user_settings().await.unwrap_or_default();
            settings.output_style = Some(style_owned.clone());
            if let Err(e) = mgr.save_user_settings(&settings).await {
                push_msg(&mut ctx, format!("❌ Failed to save settings: {}", e));
                return Ok(());
            }
            push_msg(
                &mut ctx,
                format!(
                    "✅ Output style: **{}** (saved to user settings)",
                    style_owned
                ),
            );
        }
        Err(e) => push_msg(&mut ctx, format!("❌ Settings unavailable: {}", e)),
    }
    Ok(())
}

// ── /tag: 会话标签，持久化在 .star/session_tags.json ────────────────────────

type TagMap = std::collections::HashMap<String, Vec<String>>;

fn tags_file() -> Result<PathBuf, String> {
    Ok(star_dir()?.join("session_tags.json"))
}

fn load_tags() -> TagMap {
    tags_file()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_tags(map: &TagMap) -> Result<(), String> {
    let path = tags_file()?;
    let json = serde_json::to_string_pretty(map).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

fn session_key(ctx: &CommandContext<'_>) -> String {
    ctx.state
        .current_session_title
        .clone()
        .unwrap_or_else(|| "current".to_string())
}

/// `/tag` — 给当前会话打标签（对标 Claude Code /tag）。
/// 无参数列出；`<name>` 添加；`remove <name>` / `clear`。
pub async fn tag(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let key = session_key(&ctx);
    let mut map = load_tags();

    let sub = args.first().map(|s| s.to_lowercase()).unwrap_or_default();
    let msg = match sub.as_str() {
        "" => {
            let tags = map.get(&key).cloned().unwrap_or_default();
            if tags.is_empty() {
                format!(
                    "No tags on this session (`{}`).\n\nAdd one with `/tag <name>`.",
                    key
                )
            } else {
                format!("# Tags — `{}`\n\n{}", key, tags.join(", "))
            }
        }
        "remove" | "rm" | "delete" => {
            let name = args[1..].join(" ").trim().to_string();
            if name.is_empty() {
                "Usage: /tag remove <name>".to_string()
            } else {
                let entry = map.entry(key.clone()).or_default();
                let before = entry.len();
                entry.retain(|t| t != &name);
                if entry.len() == before {
                    format!("Tag `{}` not found on this session.", name)
                } else {
                    save_tags(&map)?;
                    format!("✅ Removed tag `{}`.", name)
                }
            }
        }
        "clear" => {
            map.remove(&key);
            save_tags(&map)?;
            "✅ All tags cleared for this session.".to_string()
        }
        _ => {
            let name = args.join(" ").trim().to_string();
            let entry = map.entry(key.clone()).or_default();
            if entry.iter().any(|t| t == &name) {
                format!("Tag `{}` is already set.", name)
            } else {
                entry.push(name.clone());
                let all = entry.join(", ");
                save_tags(&map)?;
                format!(
                    "🏷️ Tagged session `{}` with `{}`.\n\nTags: {}",
                    key, name, all
                )
            }
        }
    };
    push_msg(&mut ctx, msg);
    Ok(())
}

/// `/keybindings` — 打开快捷键帮助浮层，并输出一份可复制的速查表。
pub async fn keybindings(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    ctx.state.close_palette();
    ctx.state.show_help = true;

    let vim = on_off(ctx.state.vim_enabled);
    let out = format!(
        "# Key bindings\n\n\
         | Keys | Action |\n|---|---|\n\
         | `Enter` | Send message |\n\
         | `Shift+Enter` / `Alt+Enter` | Newline |\n\
         | `Esc` | Cancel stream · twice clears input |\n\
         | `Ctrl+C` | Twice to quit |\n\
         | `Ctrl+O` | Verbose output (transcript) |\n\
         | `Ctrl+T` | Toggle tasks |\n\
         | `Ctrl+R` | Search input history |\n\
         | `↑` / `↓` | Walk input history |\n\
         | `Ctrl+W` / `Ctrl+U` / `Ctrl+K` | Kill word / line-start / line-end |\n\
         | `Ctrl+Y` / `Alt+Y` | Yank · yank-pop |\n\
         | `Tab` | Accept completion / next hint |\n\
         | `/` | Command palette |\n\
         | `@` | File mention |\n\
         | `?` | This help overlay |\n\n\
         Vim mode: **{}** (toggle from the palette). The overlay is now open — press `Esc` to close.",
        vim
    );
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/reload-plugins` — 重新扫描插件与市场注册表。
pub async fn reload_plugins(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    ctx.state.reload_plugins_state().await;
    let installed = ctx.state.plugin_installed.len();
    let marketplaces = ctx.state.plugin_marketplaces.len();
    let errors = ctx.state.plugin_errors.len();
    let mut out = format!(
        "🔄 Plugins reloaded — **{}** installed, **{}** marketplace(s).",
        installed, marketplaces
    );
    if errors > 0 {
        out.push_str(&format!("\n\n⚠️ {} plugin(s) reported errors:\n", errors));
        for (name, err) in ctx.state.plugin_errors.iter().take(5) {
            out.push_str(&format!("- `{}`: {}\n", name, err));
        }
    }
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/statusline` — 查看状态栏当前显示的信息与可切换项。
pub async fn statusline(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let sub = args.first().map(|s| s.to_lowercase()).unwrap_or_default();
    match sub.as_str() {
        "verbose" => {
            ctx.state.ui_verbose = !ctx.state.ui_verbose;
            let now = on_off(ctx.state.ui_verbose);
            push_msg(&mut ctx, format!("Status/tool verbosity: **{}**", now));
            return Ok(());
        }
        "vim" => {
            ctx.state.vim_enabled = !ctx.state.vim_enabled;
            if ctx.state.vim_enabled {
                ctx.state.vim_state = crate::ui::vim::VimState::new();
            }
            let now = on_off(ctx.state.vim_enabled);
            push_msg(&mut ctx, format!("Vim indicator: **{}**", now));
            return Ok(());
        }
        "" => {}
        other => {
            push_msg(
                &mut ctx,
                format!(
                    "❌ Unknown option `{}`. Usage: /statusline [verbose|vim]",
                    other
                ),
            );
            return Ok(());
        }
    }

    let out = format!(
        "# Status line\n\nCurrently showing:\n\
         - model `{}` · provider `{}`\n\
         - approval mode `{}` · thinking effort `{}`\n\
         - context/token usage `{}` tokens · cost `${:.4}`\n\
         - git branch `{}`\n\
         - fast mode `{}` · extra dirs `{}` · sandbox `{}`\n\
         - vim `{}` · verbose `{}` · colorblind `{}`\n\n\
         Toggles: `/statusline verbose`, `/statusline vim`. \
         Theme and colors live under `/theme`.",
        ctx.state.current_model,
        ctx.state.current_provider_id.as_deref().unwrap_or("-"),
        approval_label(&ctx.state.approval_mode),
        ctx.state.thinking_effort.display_name(),
        ctx.state.token_count,
        ctx.state.total_cost,
        ctx.state.git_branch.as_deref().unwrap_or("-"),
        on_off(ctx.state.fast_mode),
        ctx.state.extra_working_dirs.len(),
        on_off(ctx.state.sandbox_enabled),
        on_off(ctx.state.vim_enabled),
        on_off(ctx.state.ui_verbose),
        on_off(ctx.state.colorblind_mode),
    );
    push_msg(&mut ctx, out);
    Ok(())
}

// ── 开关型命令 ─────────────────────────────────────────────────────────────

/// 解析 on/off/toggle 参数
fn parse_toggle(args: &[String], current: bool) -> Result<bool, String> {
    match args
        .first()
        .map(|s| s.to_lowercase())
        .unwrap_or_else(|| "toggle".into())
        .as_str()
    {
        "on" | "enable" | "true" | "yes" => Ok(true),
        "off" | "disable" | "false" | "no" => Ok(false),
        "toggle" | "" => Ok(!current),
        other => Err(other.to_string()),
    }
}

/// `/poor` — 省电模式：关闭记忆抽取与提示建议（对标 Claude Code poor mode）。
pub async fn poor(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let enable = match parse_toggle(&args, ctx.state.poor_mode) {
        Ok(v) => v,
        Err(other) => {
            push_msg(
                &mut ctx,
                format!("❌ Unknown action `{}`. Usage: /poor [on|off]", other),
            );
            return Ok(());
        }
    };
    ctx.state.poor_mode = enable;
    // 省电模式下顺带关掉主动建议（这是最费额外调用的一项）
    ctx.state.proactive_suggestions.enabled = !enable;
    let out = if enable {
        "🪫 Poor mode **on** — memory extraction and prompt suggestions are skipped. \
         Fewer background model calls, lower cost."
            .to_string()
    } else {
        "🔋 Poor mode **off** — memory extraction and prompt suggestions re-enabled.".to_string()
    };
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/proactive` — 主动建议开关。
pub async fn proactive(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let current = ctx.state.proactive_suggestions.enabled;
    let enable = match parse_toggle(&args, current) {
        Ok(v) => v,
        Err(other) => {
            push_msg(
                &mut ctx,
                format!("❌ Unknown action `{}`. Usage: /proactive [on|off]", other),
            );
            return Ok(());
        }
    };
    ctx.state.proactive_suggestions.enabled = enable;
    if !enable {
        ctx.state.proactive_suggestions.suggestions.clear();
    }
    let pending = ctx.state.proactive_suggestions.suggestions.len();
    let out = format!(
        "Proactive suggestions: **{}**{}",
        on_off(enable),
        if enable && pending > 0 {
            format!(" · {} pending", pending)
        } else {
            String::new()
        }
    );
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/advisor` — 顾问模式开关：开启后在提示里要求模型附带下一步建议。
pub async fn advisor(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let enable = match parse_toggle(&args, ctx.state.advisor_mode) {
        Ok(v) => v,
        Err(other) => {
            push_msg(
                &mut ctx,
                format!("❌ Unknown action `{}`. Usage: /advisor [on|off]", other),
            );
            return Ok(());
        }
    };
    ctx.state.advisor_mode = enable;
    let out = if enable {
        "🧭 Advisor mode **on** — responses end with a short \"what I'd do next\" note."
    } else {
        "Advisor mode **off**."
    };
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/autonomy` — 自动继续（auto-continue）面板与开关。
pub async fn autonomy(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if let Some(first) = args.first() {
        if let Ok(rounds) = first.parse::<u32>() {
            ctx.state.auto_continue_enabled = rounds > 0;
            ctx.state.auto_continue_remaining = rounds;
            let out = format!(
                "🤖 Autonomy: auto-continue for **{}** more round(s).",
                rounds
            );
            push_msg(&mut ctx, out);
            return Ok(());
        }
    }

    let enable = match parse_toggle(&args, ctx.state.auto_continue_enabled) {
        Ok(v) => v,
        Err(other) => {
            push_msg(
                &mut ctx,
                format!(
                    "❌ Unknown action `{}`. Usage: /autonomy [on|off|<rounds>]",
                    other
                ),
            );
            return Ok(());
        }
    };
    ctx.state.auto_continue_enabled = enable;
    if !enable {
        ctx.state.auto_continue_remaining = 0;
    }

    let out = format!(
        "# Autonomy\n\n\
         - auto-continue: **{}** (remaining {})\n\
         - approval mode: **{}**\n\
         - sandbox: **{}**\n\
         - poor mode: **{}**\n\n\
         `/autonomy on|off` toggles, `/autonomy <n>` grants n rounds, \
         `/mode bypass` removes approval prompts.",
        on_off(ctx.state.auto_continue_enabled),
        ctx.state.auto_continue_remaining,
        approval_label(&ctx.state.approval_mode),
        on_off(ctx.state.sandbox_enabled),
        on_off(ctx.state.poor_mode),
    );
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/tui` — TUI 增强项面板：一处查看/切换所有界面开关。
pub async fn tui(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if let Some(which) = args.first().map(|s| s.to_lowercase()) {
        let label = match which.as_str() {
            "vim" => {
                ctx.state.vim_enabled = !ctx.state.vim_enabled;
                if ctx.state.vim_enabled {
                    ctx.state.vim_state = crate::ui::vim::VimState::new();
                }
                Some(("vim mode", ctx.state.vim_enabled))
            }
            "verbose" => {
                ctx.state.ui_verbose = !ctx.state.ui_verbose;
                Some(("verbose tool output", ctx.state.ui_verbose))
            }
            "colorblind" => {
                ctx.state.colorblind_mode = !ctx.state.colorblind_mode;
                Some(("colorblind palette", ctx.state.colorblind_mode))
            }
            "transcript" => {
                let now = ctx.state.toggle_transcript_mode();
                Some(("transcript mode", now))
            }
            "preview" => {
                ctx.state.preview_visible = !ctx.state.preview_visible;
                Some(("preview pane", ctx.state.preview_visible))
            }
            _ => None,
        };
        match label {
            Some((name, now)) => {
                let msg = format!("{}: **{}**", name, on_off(now));
                push_msg(&mut ctx, msg);
            }
            None => push_msg(
                &mut ctx,
                format!(
                    "❌ Unknown toggle `{}`. Usage: /tui [vim|verbose|colorblind|transcript|preview]",
                    which
                ),
            ),
        }
        return Ok(());
    }

    let out = format!(
        "# TUI enhancements\n\n\
         | Toggle | State | Command |\n|---|---|---|\n\
         | Vim mode | {} | `/tui vim` |\n\
         | Verbose tool output | {} | `/tui verbose` |\n\
         | Colorblind palette | {} | `/tui colorblind` |\n\
         | Transcript mode | {} | `/tui transcript` (Ctrl+O) |\n\
         | Preview pane | {} | `/tui preview` |\n\n\
         Theme: `{}` — change it with `/theme`.",
        on_off(ctx.state.vim_enabled),
        on_off(ctx.state.ui_verbose),
        on_off(ctx.state.colorblind_mode),
        on_off(ctx.state.is_transcript_mode),
        on_off(ctx.state.preview_visible),
        ctx.state.theme_manager.current().name,
    );
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/network [on|off|toggle]` — 离线/网络开关（对标 Claude Code /network）。
/// 开启后 LLM 发送入口与 Web 工具（WebSearch/WebFetch）拒绝网络请求，
/// 状态栏显示 OFFLINE 指示。
pub async fn network(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let action = args
        .first()
        .map(|s| s.to_lowercase())
        .unwrap_or_else(|| "toggle".into());
    let enable = match action.as_str() {
        "on" | "enable" | "true" => true,
        "off" | "disable" | "false" => false,
        "toggle" => !crate::core::offline::is_offline(),
        other => {
            push_msg(
                &mut ctx,
                format!("❌ Unknown action: {}. Usage: /network [on|off]", other),
            );
            return Ok(());
        }
    };

    crate::core::offline::set_offline(enable);
    ctx.state.network_offline = enable;
    ctx.state.current_status_line = Some(if enable {
        "📴 Offline mode ON".to_string()
    } else {
        "🌐 Offline mode OFF".to_string()
    });
    push_msg(
        &mut ctx,
        format!(
            "{} Offline mode is now **{}**.\n\n{}\n\nUse `/network on` / `/network off` to toggle. The status bar shows `OFFLINE` while it is active.",
            if enable { "📴" } else { "🌐" },
            if enable { "ON" } else { "OFF" },
            if enable {
                "Messages will not be sent and web tools (WebSearch/WebFetch) will refuse requests."
            } else {
                "Messages and web tools are back online."
            }
        ),
    );
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Attachments / context
// ═══════════════════════════════════════════════════════════════════════════

/// `/attach <path>...` — 把文件挂到下一条消息上（对标 Claude Code /attach）。
/// 走与拖拽/粘贴同一套 paste-segment 机制：输入框里只留占位符，
/// 发送时才展开为内容。
pub async fn attach(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if args.is_empty() {
        push_msg(
            &mut ctx,
            "Usage: `/attach <path> [more paths...]`\n\n\
             Attaches files to your next message. You can also drag files into the \
             prompt, or reference them inline with `@path`.",
        );
        return Ok(());
    }

    let mut resolved: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for raw in &args {
        let cleaned = raw.trim().trim_matches('"');
        let path = if let Some(rest) = cleaned.strip_prefix("~/") {
            dirs::home_dir()
                .map(|h| h.join(rest))
                .unwrap_or_else(|| PathBuf::from(cleaned))
        } else {
            PathBuf::from(cleaned)
        };
        if path.is_file() {
            let abs = path.canonicalize().unwrap_or(path);
            resolved.push(abs.to_string_lossy().to_string());
        } else {
            missing.push(cleaned.to_string());
        }
    }

    if resolved.is_empty() {
        push_msg(
            &mut ctx,
            format!("❌ No readable files: {}", missing.join(", ")),
        );
        return Ok(());
    }

    let count = resolved.len();
    crate::ui::events::clipboard_paste::insert_file_paste_block(ctx.state, resolved);

    let mut out = format!("📎 Attached **{}** file(s) to your next message.", count);
    if !missing.is_empty() {
        out.push_str(&format!(
            "\n\n⚠️ Skipped (not found): {}",
            missing.join(", ")
        ));
    }
    push_msg(&mut ctx, out);
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Insights / diagnostics
// ═══════════════════════════════════════════════════════════════════════════

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

    let cache_total = s.cache_read_tokens + s.cache_creation_tokens;
    let cache_rate = if cache_total > 0 {
        (s.cache_read_tokens as f64 / cache_total as f64) * 100.0
    } else {
        0.0
    };

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
        "- context tokens: **{}**\n- cumulative cost: **${:.4}**\n- cache reads: **{}** · writes: **{}** (hit rate **{:.1}%**)\n",
        s.token_count, s.total_cost, s.cache_read_tokens, s.cache_creation_tokens, cache_rate
    ));

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

fn render_tool_call(entry: &ChatEntry, back: usize) -> String {
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

/// `gh` 是否可用（未安装时给出一致的提示）
fn gh_ready() -> Result<(), String> {
    run("gh", &["--version"]).map(|_| ()).map_err(|_| {
        "GitHub CLI (`gh`) not found. Install it from https://cli.github.com/ and run \
         `gh auth login`."
            .to_string()
    })
}

/// 当前仓库 slug（`owner/repo`），来自 `gh repo view`
fn repo_slug() -> Option<String> {
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
/// 与 `git_pr_subscribe` 工具共用 `.star/pr_subscriptions/`，纯本地记录，
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

/// `~/.star/teams` 下的 mailbox 目录
fn teams_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".star")
        .join("teams")
}

/// 扫描所有 team 的 inbox，返回 (team, agent, unread, total)
fn scan_mailboxes() -> Vec<(String, String, usize, usize)> {
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

fn now_ms() -> u128 {
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
async fn deliver_mailbox_message(
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
fn claim_file(team: &str) -> PathBuf {
    teams_dir().join(team).join("main-claim.json")
}

/// 判断 pid 是否还活着（Linux 下看 /proc）
fn pid_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{}", pid)).exists()
}

fn read_claim(team: &str) -> Option<serde_json::Value> {
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
fn write_claim(
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
fn split_target(target: &str) -> (String, String) {
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

/// 收集本机可用于配对的 LAN 地址
fn lan_addresses() -> Vec<String> {
    let raw = run("hostname", &["-I"])
        .or_else(|_| run("sh", &["-c", "ip -4 -o addr | awk '{print $4}'"]))
        .unwrap_or_default();
    raw.split_whitespace()
        .map(|s| s.split('/').next().unwrap_or(s).to_string())
        .filter(|s| !s.starts_with("127.") && !s.is_empty())
        .collect()
}

/// 渲染 bridge / remote inbox 的真实状态（三个命令共用）
async fn remote_surface(client: &str) -> String {
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
fn goal_manager() -> Result<crate::core::goal_tracking::GoalManager, String> {
    let path = star_dir()?.join("goals.json");
    let mut mgr =
        crate::core::goal_tracking::GoalManager::new(Some(path.to_string_lossy().as_ref()));
    mgr.load()?;
    Ok(mgr)
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

/// 按 id 前缀解析目标；返回完整 id
fn resolve_goal_id(
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

fn goal_status_icon(status: &crate::core::goal_tracking::GoalStatus) -> &'static str {
    use crate::core::goal_tracking::GoalStatus as S;
    match status {
        S::InProgress => "▶",
        S::Completed => "✅",
        S::Paused => "⏸",
        S::Cancelled => "✖",
    }
}

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

fn goal_set_progress(mgr: &mut crate::core::goal_tracking::GoalManager, args: &[String]) -> String {
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

fn render_goals(mgr: &crate::core::goal_tracking::GoalManager, include_done: bool) -> String {
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
async fn render_jobs(cwd: &std::path::Path) -> String {
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

/// 项目级分类记忆库根目录：`.star/memory/`
fn classified_dir() -> Result<PathBuf, String> {
    Ok(star_dir()?.join("memory"))
}

fn classified_manager(
) -> Result<crate::core::memory::classification::ClassifiedMemoryManager, String> {
    use crate::core::memory::classification::ClassifiedMemoryManager;
    let mut mgr = ClassifiedMemoryManager::new(classified_dir()?);
    mgr.initialize()?;
    let _ = mgr.load_index();
    Ok(mgr)
}

/// `/local-memory` — 项目级四类型记忆库（user/feedback/project/reference）
pub async fn local_memory(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let mut mgr = match classified_manager() {
        Ok(m) => m,
        Err(e) => {
            push_msg(&mut ctx, format!("⚠️ {}", e));
            return Ok(());
        }
    };

    let sub = args.first().map(|s| s.to_lowercase()).unwrap_or_default();
    let out = match sub.as_str() {
        "" | "list" | "stats" => render_local_memory(&mgr),
        "search" | "recall" => {
            let q = args[1..].join(" ");
            if q.is_empty() {
                "Usage: `/local-memory search <query>`".to_string()
            } else {
                let hits = mgr.recall(&q, 10);
                if hits.is_empty() {
                    format!("No memories matching `{}`.", q)
                } else {
                    let mut s = format!("## Recall: `{}`\n\n", q);
                    for h in hits {
                        s.push_str(&format!(
                            "- [{}] **{}** — `{}`\n",
                            h.memory_type, h.title, h.file_path
                        ));
                    }
                    s
                }
            }
        }
        "add" | "remember" => add_local_memory(&mut mgr, &args[1..]),
        _ => "Usage: `/local-memory` | `/local-memory add <type> <title> :: <content>` | \
              `/local-memory search <query>`\n\nTypes: user · feedback · project · reference"
            .to_string(),
    };
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/local-memory add <type> <title> :: <content>`
fn add_local_memory(
    mgr: &mut crate::core::memory::classification::ClassifiedMemoryManager,
    rest: &[String],
) -> String {
    use crate::core::memory::classification::{ClassifiedMemory, MemoryType};

    if rest.is_empty() {
        return "Usage: `/local-memory add <user|feedback|project|reference> <title> :: <content>`"
            .to_string();
    }
    let Some(mem_type) = MemoryType::from_str(&rest[0]) else {
        return format!(
            "Unknown memory type `{}` — use user, feedback, project or reference.",
            rest[0]
        );
    };
    let body = rest[1..].join(" ");
    let (title, content) = match body.split_once("::") {
        Some((t, c)) => (t.trim().to_string(), c.trim().to_string()),
        None => {
            let t: String = body.chars().take(60).collect();
            (t, body.clone())
        }
    };
    if content.is_empty() {
        return "Nothing to remember — give a title and content.".to_string();
    }

    let now = now_ms() as u64 / 1000;
    let id = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let memory = ClassifiedMemory {
        id: id.clone(),
        memory_type: mem_type.clone(),
        title: title.clone(),
        content,
        tags: Vec::new(),
        relevance_score: 0.5,
        created_at: now,
        accessed_at: now,
        access_count: 0,
        source: Some("slash:/local-memory".to_string()),
    };
    if !mgr.validate_memory(&memory) {
        return "Rejected: memory failed validation (empty or looks like a secret).".to_string();
    }
    match mgr.store(memory) {
        Ok(()) => {
            let _ = mgr.save_index();
            format!("🧠 Stored [{}] **{}** (`{}`).", mem_type, title, id)
        }
        Err(e) => format!("⚠️ {}", e),
    }
}

fn render_local_memory(
    mgr: &crate::core::memory::classification::ClassifiedMemoryManager,
) -> String {
    use crate::core::memory::classification::MemoryType;
    let dir = classified_dir().unwrap_or_default();
    let mut out = format!("## Local memory\n\n`{}`\n\n", dir.display());

    let stats = mgr.stats();
    let mut total = 0usize;
    for t in [
        MemoryType::User,
        MemoryType::Feedback,
        MemoryType::Project,
        MemoryType::Reference,
    ] {
        let n = stats.get(&t.to_string()).copied().unwrap_or(0);
        total += n;
        out.push_str(&format!(
            "- **{}** — {} entr{}\n",
            t,
            n,
            if n == 1 { "y" } else { "ies" }
        ));
    }

    if total == 0 {
        out.push_str("\n_empty_ — add one with `/local-memory add project <title> :: <content>`\n");
        return out;
    }

    out.push_str("\n### Recent\n\n");
    for t in [
        MemoryType::User,
        MemoryType::Feedback,
        MemoryType::Project,
        MemoryType::Reference,
    ] {
        for e in mgr.by_type(&t).into_iter().take(3) {
            out.push_str(&format!("- [{}] **{}** — `{}`\n", t, e.title, e.id));
        }
    }
    out.push_str("\n`/local-memory search <query>` · `/memory-stores` for every store\n");
    out
}

/// 统计目录里的文件数与总字节
fn dir_stats(dir: &std::path::Path) -> (usize, u64) {
    let mut files = 0usize;
    let mut bytes = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            match e.metadata() {
                Ok(m) if m.is_dir() => stack.push(e.path()),
                Ok(m) => {
                    files += 1;
                    bytes += m.len();
                }
                Err(_) => {}
            }
        }
    }
    (files, bytes)
}

fn human_bytes(b: u64) -> String {
    if b < 1024 {
        format!("{} B", b)
    } else if b < 1024 * 1024 {
        format!("{:.1} KB", b as f64 / 1024.0)
    } else {
        format!("{:.1} MB", b as f64 / (1024.0 * 1024.0))
    }
}

/// 一行 store 描述
fn store_row(label: &str, path: &std::path::Path, is_dir: bool) -> String {
    if !path.exists() {
        return format!("| {} | `{}` | — | absent |\n", label, path.display());
    }
    if is_dir {
        let (n, b) = dir_stats(path);
        format!(
            "| {} | `{}` | {} file(s) / {} | present |\n",
            label,
            path.display(),
            n,
            human_bytes(b)
        )
    } else {
        let b = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        format!(
            "| {} | `{}` | {} | present |\n",
            label,
            path.display(),
            human_bytes(b)
        )
    }
}

/// `/memory-stores` — 列出本会话可见的所有记忆/上下文存储
pub async fn memory_stores(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let team = crate::core::team_memory::TeamMemoryConfig::from_env();

    let mut out =
        String::from("## Memory stores\n\n| store | path | size | state |\n|---|---|---|---|\n");
    out.push_str(&store_row(
        "project memory (/memory)",
        &cwd.join(".star").join("memory.md"),
        false,
    ));
    out.push_str(&store_row(
        "classified (/local-memory)",
        &cwd.join(".star").join("memory"),
        true,
    ));
    for name in ["STAR.md", "STARCODE.md", "CLAUDE.md", "AGENTS.md"] {
        out.push_str(&store_row(name, &cwd.join(name), false));
    }
    out.push_str(&store_row(
        "global CLAUDE.md",
        &home.join(".claude").join("CLAUDE.md"),
        false,
    ));
    out.push_str(&store_row(
        "user memdir",
        &home.join(".starcode").join("memory"),
        true,
    ));
    out.push_str(&store_row(
        "session transcripts",
        &home.join(".star").join("transcripts"),
        true,
    ));

    out.push_str("\n### Shared / team memory\n\n");
    out.push_str(&format!(
        "- enabled: `{}` (`STAR_TEAM_MEMORY_ENABLED`)\n- team id: `{}`\n\
         - sync endpoint: `{}`\n- secret scanning: `{}`\n",
        on_off(team.enabled),
        team.team_id.as_deref().unwrap_or("-"),
        team.sync_endpoint.as_deref().unwrap_or("-"),
        on_off(team.secret_scanning)
    ));
    if let Some(id) = &team.team_id {
        out.push_str(&store_row(
            "team memdir",
            &home.join(".starcode").join("memory").join("teams").join(id),
            true,
        ));
    }
    out.push_str(
        "\n`/memory show` edits the project file · `/local-memory` manages the classified store\n",
    );
    push_msg(&mut ctx, out);
    Ok(())
}

/// 本地凭据库（keychain 优先，Linux 上回退到 `~/.star/secure_storage.json`）
fn vault_manager() -> crate::core::secure_storage::SecureStorageManager {
    crate::core::secure_storage::SecureStorageManager::from_env()
}

fn vault_path() -> String {
    std::env::var("STAR_SECURE_STORAGE_PATH").unwrap_or_else(|_| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".star")
            .join("secure_storage.json")
            .display()
            .to_string()
    })
}

/// `/local-vault` — 本机凭据库：list / set / get / rm
pub async fn local_vault(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let mgr = vault_manager();
    let backend = mgr.backend_type();
    let plaintext = matches!(
        backend,
        crate::core::secure_storage::StorageBackend::PlainText
            | crate::core::secure_storage::StorageBackend::Auto
    ) && !cfg!(target_os = "macos");

    let sub = args.first().map(|s| s.to_lowercase()).unwrap_or_default();
    let out = match sub.as_str() {
        "" | "list" | "ls" => {
            let keys = mgr.list().unwrap_or_default();
            let mut s = format!(
                "## Local vault\n\n- backend: `{:?}`{}\n- store: `{}`\n- entries: `{}`\n\n",
                backend,
                if plaintext {
                    " (plaintext fallback)"
                } else {
                    ""
                },
                vault_path(),
                keys.len()
            );
            if keys.is_empty() {
                s.push_str("_empty_ — add one with `/local-vault set <key> <value>`\n");
            } else {
                let mut sorted = keys;
                sorted.sort();
                for k in sorted {
                    s.push_str(&format!("- `{}`\n", k));
                }
            }
            if plaintext {
                s.push_str(
                    "\n⚠️ Values are stored unencrypted on this platform. Prefer env vars or a \
                     system keychain for production secrets.\n",
                );
            }
            s
        }
        "set" | "put" => match (args.get(1), args.get(2)) {
            (Some(k), Some(v)) => match mgr.store(k, v) {
                Ok(()) => format!("🔐 Stored `{}` in the local vault.", k),
                Err(e) => format!("⚠️ {}", e),
            },
            _ => "Usage: `/local-vault set <key> <value>`".to_string(),
        },
        "get" => match args.get(1) {
            None => "Usage: `/local-vault get <key>`".to_string(),
            Some(k) => match mgr.get(k) {
                // Vault 条目一律按机密处理，只回报长度而不回显明文
                Ok(Some(v)) => format!("`{}` = <set, {} chars>", k, v.chars().count()),
                Ok(None) => format!("`{}` is not in the vault.", k),
                Err(e) => format!("⚠️ {}", e),
            },
        },
        "rm" | "delete" => match args.get(1) {
            None => "Usage: `/local-vault rm <key>`".to_string(),
            Some(k) => match mgr.delete(k) {
                Ok(()) => format!("🗑 Removed `{}`.", k),
                Err(e) => format!("⚠️ {}", e),
            },
        },
        _ => "Usage: `/local-vault [list|set <k> <v>|get <k>|rm <k>]`".to_string(),
    };
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/vault` — 远端凭据/设置来源总览；`/vault sync` 拉一次远端设置
pub async fn vault(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if args.first().map(|s| s.to_lowercase()).as_deref() == Some("sync") {
        let endpoint = ctx.state.remote_settings.endpoint.clone();
        let out = match endpoint {
            None => "No remote settings endpoint configured. Set one with `/vault endpoint <url>`."
                .to_string(),
            Some(url) => match ctx.state.remote_settings.sync().await {
                Ok(()) => format!("🔄 Synced remote settings from `{}`.", url),
                Err(e) => format!("⚠️ {}", e),
            },
        };
        push_msg(&mut ctx, out);
        return Ok(());
    }

    if args.first().map(|s| s.to_lowercase()).as_deref() == Some("endpoint") {
        let out = match args.get(1) {
            None => "Usage: `/vault endpoint <url>`".to_string(),
            Some(url) => {
                ctx.state.remote_settings.set_endpoint(url);
                format!(
                    "✅ Remote settings endpoint set to `{}` (this session).",
                    url
                )
            }
        };
        push_msg(&mut ctx, out);
        return Ok(());
    }

    let store = crate::core::config::provider_store::ProviderStore::new();
    let configured = store.configured_provider_ids().await.unwrap_or_default();
    let env_keys: Vec<String> = std::env::vars()
        .map(|(k, _)| k)
        .filter(|k| is_secretish(k) && (k.starts_with("STAR_") || k.contains("API")))
        .collect();

    let (endpoint, last_sync, interval) = {
        let rs = &ctx.state.remote_settings;
        (rs.endpoint.clone(), rs.last_sync, rs.sync_interval_secs)
    };

    let mut out = String::from("## Vaults\n\n### Remote settings channel\n\n");
    out.push_str(&format!(
        "- endpoint: `{}`\n- last sync: `{}`\n- interval: `{}s`\n",
        endpoint.as_deref().unwrap_or("-"),
        last_sync
            .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
            .map(|d| d.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "never".to_string()),
        interval
    ));

    out.push_str("\n### Credential sources\n\n");
    out.push_str(&format!(
        "- provider store: `{}` provider(s) configured — `/provider`\n",
        configured.len()
    ));
    out.push_str(&format!(
        "- local vault: `{}` — `/local-vault`\n",
        vault_path()
    ));
    if env_keys.is_empty() {
        out.push_str("- environment: no secret-looking vars set\n");
    } else {
        let mut sorted = env_keys;
        sorted.sort();
        out.push_str(&format!(
            "- environment: {} secret-looking var(s) — {}\n",
            sorted.len(),
            sorted
                .iter()
                .take(8)
                .map(|k| format!("`{}`", k))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    out.push_str(
        "\nNo hosted vault provider is bundled. Point `STAR_SECURE_STORAGE_PATH` at a mounted \
         secret volume, or feed credentials through the environment.\n\n\
         `/vault endpoint <url>` · `/vault sync`\n",
    );
    push_msg(&mut ctx, out);
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// 技能：搜索 / 学习 / 市场
// ═══════════════════════════════════════════════════════════════════════════

/// 载入项目 + 用户目录下的所有 SKILL.md
async fn load_all_skills() -> Vec<crate::agent::skills::loader::SkillDefinition> {
    use crate::agent::skills::loader::SkillLoader;
    use crate::core::config::storage::Storage;
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let storage = Storage::new(cwd);
    let mut all = SkillLoader::load_skills_from_dir(&storage.project_skills_dir()).await;
    all.extend(SkillLoader::load_skills_from_dir(&Storage::user_skills_dir()).await);
    all
}

/// `/skill-search <query>` — 在已装载技能里做意图归一 + 相关性排序
pub async fn skill_search(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    use crate::core::skill_search::{
        intent_normalize::IntentNormalizer, local_search::LocalSkillSearch,
        local_search::SkillInfo, SkillSearchConfig,
    };

    let query = args.join(" ");
    let skills = load_all_skills().await;

    if query.is_empty() {
        let mut out = format!("## Skill search\n\n{} skill(s) indexed.\n\n", skills.len());
        out.push_str("Usage: `/skill-search <what you want to do>`\n");
        if !skills.is_empty() {
            out.push_str("\nIndexed:\n");
            for s in skills.iter().take(20) {
                out.push_str(&format!("- `{}`\n", s.name));
            }
        }
        push_msg(&mut ctx, out);
        return Ok(());
    }

    let normalizer = IntentNormalizer::new();
    let intent = normalizer.normalize(&query);
    let infos: Vec<SkillInfo> = skills
        .iter()
        .map(|s| SkillInfo {
            id: s.name.clone(),
            name: s.name.clone(),
            description: if s.description.is_empty() {
                s.metadata.when_to_use.clone()
            } else {
                Some(s.description.clone())
            },
        })
        .collect();

    let searcher = LocalSkillSearch::new(SkillSearchConfig::from_env());
    let keyword_query = intent.keywords.join(" ");
    let hits = if keyword_query.is_empty() {
        Vec::new()
    } else {
        searcher.search(&keyword_query, &infos)
    };
    let results = if hits.is_empty() {
        searcher.search(&query, &infos)
    } else {
        hits
    };

    let mut out = format!(
        "## Skill search: `{}`\n\n- intent: `{:?}` ({:.0}% confidence)\n- keywords: {}\n\n",
        query,
        intent.intent_type,
        intent.confidence * 100.0,
        if intent.keywords.is_empty() {
            "-".to_string()
        } else {
            intent.keywords.join(", ")
        }
    );
    if results.is_empty() {
        out.push_str(&format!(
            "No skill scored above the relevance floor across {} indexed skill(s).\n\n\
             Try `/skills list`, or `/skill-store` to install more.\n",
            skills.len()
        ));
    } else {
        for r in &results {
            out.push_str(&format!(
                "- **{}** — {:.0}% ({})\n",
                r.name,
                r.relevance_score * 100.0,
                r.match_reason
            ));
            if let Some(d) = &r.description {
                out.push_str(&format!(
                    "  - {}\n",
                    d.chars().take(120).collect::<String>()
                ));
            }
        }
        out.push_str("\nRun one with `/skill <name>`.\n");
    }
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/skill-learning` —— 技能学习管线状态与本能探测。
///
/// 学习管线（core/skill_learning）目前是进程内的：观察结果不落盘，
/// 也还没有接到会话循环上，所以这里如实报告策略与真实持久化的技能来源，
/// 并提供 `probe` 子命令对真实文本跑一次 InstinctParser。
pub async fn skill_learning(mut ctx: CommandContext<'_>, args: &[String]) -> CommandResult {
    use crate::core::skill_learning::{InstinctParser, LearningPolicy, SkillLearningManager};

    let sub = args.first().map(|s| s.as_str()).unwrap_or("");
    if sub == "probe" {
        let text = args[1..].join(" ");
        if text.trim().is_empty() {
            push_msg(&mut ctx, "Usage: `/skill-learning probe <text>`");
            return Ok(());
        }
        let parser = InstinctParser::new();
        let out = match parser.parse(&text) {
            Some(i) => format!(
                "## Instinct match\n\n- input: `{}`\n- instinct: **{}**\n- trigger: `{}`\n- response: {}\n- priority: {}\n",
                text, i.name, i.trigger, i.response, i.priority
            ),
            None => format!(
                "## Instinct match\n\nNo built-in instinct matched `{}`.\n\n\
                 Built-in patterns: `error_fix` (error/fix/bug), `refactor` (refactor/improve/optimize), \
                 `test` (test/verify/check).\n",
                text
            ),
        };
        push_msg(&mut ctx, out);
        return Ok(());
    }

    let mgr = SkillLearningManager::from_env();
    let policy = LearningPolicy::from_env();
    let skills = load_all_skills().await;
    let learned = mgr.get_all_skills().len();

    let mut out = String::from("## Skill learning\n\n");
    out.push_str(&format!(
        "- pipeline: {} (`STAR_SKILL_LEARNING_ENABLED`)\n\
         - auto learning: {} (`STAR_SKILL_AUTO_LEARNING`)\n\
         - evolution: {} (`STAR_SKILL_EVOLUTION`)\n\n",
        on_off(mgr.is_enabled()),
        on_off(policy.auto_learning_enabled),
        on_off(policy.evolution_enabled),
    ));
    out.push_str("### Policy\n\n");
    out.push_str(&format!(
        "| setting | value | env |\n|---|---|---|\n\
         | min observations | {} | `STAR_SKILL_MIN_OBSERVATIONS` |\n\
         | min success rate | {:.0}% | `STAR_SKILL_MIN_SUCCESS_RATE` |\n\
         | max skills | {} | `STAR_SKILL_MAX_COUNT` |\n\
         | expiry | {} days | `STAR_SKILL_EXPIRY_DAYS` |\n\n",
        policy.min_observations,
        policy.min_success_rate * 100.0,
        policy.max_skills,
        policy.skill_expiry_secs / 86_400,
    ));
    out.push_str(&format!(
        "### State\n\n- learned skills this process: **{}**\n\
         - authored skills on disk: **{}** (`.star/skills`, `~/.star/skills`)\n\n",
        learned,
        skills.len()
    ));
    out.push_str(
        "The observer/generator/evolution pipeline is in-process only — observations are not\n\
         persisted and nothing in the session loop feeds it yet, so the learned count resets\n\
         every launch. Durable skills are the `SKILL.md` files above; author them by hand or\n\
         install them with `/skill-store install <name>`.\n\n\
         Subcommand: `probe <text>` — run the instinct parser against real text.\n",
    );
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/skill-store` —— 技能/扩展商店：浏览、搜索、安装、卸载。
///
/// 接 `core::extensions::marketplace`（真实写入 `~/.star/extensions/`
/// 与 `registry.json`）。
pub async fn skill_store(mut ctx: CommandContext<'_>, args: &[String]) -> CommandResult {
    use crate::core::extensions::marketplace::Marketplace;
    use crate::core::extensions::registry::ExtensionRegistryManager;
    use crate::core::extensions::types::ExtensionType;

    let market = Marketplace::new();
    let registry = ExtensionRegistryManager::new();
    let sub = args.first().map(|s| s.as_str()).unwrap_or("list");

    match sub {
        "install" | "add" => {
            let name = match args.get(1) {
                Some(n) => n.clone(),
                None => {
                    push_msg(&mut ctx, "Usage: `/skill-store install <name>`");
                    return Ok(());
                }
            };
            match market.install(&name).await {
                Ok(r) => push_msg(
                    &mut ctx,
                    format!(
                        "{} **{}** ({}) — {}\n\nReload with `/reload-plugins`.",
                        if r.success { "✓" } else { "✗" },
                        r.name,
                        r.extension_type,
                        r.message
                    ),
                ),
                Err(e) => push_msg(&mut ctx, format!("Install failed: {}", e)),
            }
        }
        "uninstall" | "rm" | "remove" => {
            let name = match args.get(1) {
                Some(n) => n.clone(),
                None => {
                    push_msg(&mut ctx, "Usage: `/skill-store uninstall <name>`");
                    return Ok(());
                }
            };
            match market.uninstall(&name) {
                Ok(r) => push_msg(
                    &mut ctx,
                    format!(
                        "{} **{}** — {}",
                        if r.success { "✓" } else { "✗" },
                        r.name,
                        r.message
                    ),
                ),
                Err(e) => push_msg(&mut ctx, format!("Uninstall failed: {}", e)),
            }
        }
        "search" | "find" => {
            let q = args[1..].join(" ");
            if q.trim().is_empty() {
                push_msg(&mut ctx, "Usage: `/skill-store search <query>`");
                return Ok(());
            }
            let hits = market.search(&q);
            push_msg(
                &mut ctx,
                render_store_entries(&format!("Search: `{}`", q), &hits, &registry),
            );
        }
        "skills" => {
            let hits = market.list_by_type(&ExtensionType::Skill);
            push_msg(&mut ctx, render_store_entries("Skills", &hits, &registry));
        }
        "mcp" => {
            let hits = market.list_by_type(&ExtensionType::Mcp);
            push_msg(
                &mut ctx,
                render_store_entries("MCP servers", &hits, &registry),
            );
        }
        "installed" => {
            let entries = registry.list_all();
            let mut out = String::from("## Installed extensions\n\n");
            if entries.is_empty() {
                out.push_str("None yet. `/skill-store` to browse.\n");
            } else {
                out.push_str("| name | type | version | enabled |\n|---|---|---|---|\n");
                for e in &entries {
                    out.push_str(&format!(
                        "| `{}` | {} | {} | {} |\n",
                        e.name,
                        e.extension_type,
                        e.version,
                        on_off(e.enabled)
                    ));
                }
                out.push_str(&format!(
                    "\nRoot: `{}`\n",
                    ExtensionRegistryManager::global_extensions_dir().display()
                ));
            }
            push_msg(&mut ctx, out);
        }
        "all" => {
            let all = market.list_all();
            push_msg(
                &mut ctx,
                render_store_entries("All entries", &all, &registry),
            );
        }
        _ => {
            let featured = market.list_featured();
            let mut out = render_store_entries("Featured", &featured, &registry);
            out.push_str(
                "\nSubcommands: `all`, `skills`, `mcp`, `search <q>`, `installed`, \
                 `install <name>`, `uninstall <name>`\n",
            );
            push_msg(&mut ctx, out);
        }
    }
    Ok(())
}

/// 渲染商店条目表格，并标注是否已安装。
fn render_store_entries(
    title: &str,
    entries: &[crate::core::extensions::types::MarketplaceEntry],
    registry: &crate::core::extensions::registry::ExtensionRegistryManager,
) -> String {
    let mut out = format!("## Skill store — {}\n\n", title);
    if entries.is_empty() {
        out.push_str("No matching entries.\n");
        return out;
    }
    out.push_str("| name | type | version | installed | description |\n|---|---|---|---|---|\n");
    for e in entries.iter().take(40) {
        out.push_str(&format!(
            "| `{}` | {} | {} | {} | {} |\n",
            e.name,
            e.extension_type,
            e.version,
            if registry.is_installed(&e.name) {
                "✓"
            } else {
                "-"
            },
            e.description.chars().take(70).collect::<String>()
        ));
    }
    if entries.len() > 40 {
        out.push_str(&format!("\n… {} more\n", entries.len() - 40));
    }
    out
}

// ═══════════════════════════════════════════════════════════════════════════
// Deep planning / review
// ═══════════════════════════════════════════════════════════════════════════

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
fn repo_facts() -> String {
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

/// 渲染一行「环境变量 | 当前值 | 说明」表格行（值按名字脱敏）
fn env_row(name: &str, note: &str) -> String {
    let value = std::env::var(name).unwrap_or_default();
    let shown = if value.trim().is_empty() {
        "unset".to_string()
    } else {
        redact(name, &value)
    };
    format!("| `{}` | `{}` | {} |\n", name, shown, note)
}

/// `/rate-limit-options` — 本 build 真实的 429/限流恢复路径与可调项。
/// 事实来源：`RecoveryManager::handle_rate_limit`（切后备 → 30/60/90s 退避 → 放弃）
/// 与 `agent_llm.rs` 的 `SwitchProviderAndRetry` 分支。
pub async fn rate_limit_options(mut ctx: CommandContext<'_>, _args: &[String]) -> CommandResult {
    let model = if ctx.state.current_model.is_empty() {
        "-".to_string()
    } else {
        ctx.state.current_model.clone()
    };
    let provider = ctx
        .state
        .current_provider_id
        .clone()
        .unwrap_or_else(|| "-".to_string());
    let fallback_count = ["STAR_FALLBACK_MODEL", "STAR_FALLBACK_BASE_URL"]
        .iter()
        .filter(|k| {
            std::env::var(k)
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false)
        })
        .count();

    let mut out = format!(
        "## Rate limit options\n\n\
         - current model: `{}`\n- provider: `{}`\n- fallback entries configured: **{}**\n\n\
         ### What happens on a 429\n\n\
         1. A stream error containing `429` or `rate_limit` is classified as `RateLimit`.\n\
         2. While fallback entries remain, the agent retries after switching: \
            `STAR_FALLBACK_MODEL` takes priority, otherwise `STAR_FALLBACK_BASE_URL`.\n\
         3. With neither set, the retry is preceded by a `STAR_RATE_LIMIT_RETRY_SECS` \
            sleep (default 10s).\n\
         4. Once fallbacks are exhausted, the circuit breaker waits 30s, then 60s, then 90s \
            (emitting a heartbeat every 5s so the UI does not look frozen), then stops with \
            \"Rate limit persists after multiple cooldowns\".\n\n\
         ### Current configuration\n\n| variable | value | effect |\n|---|---|---|\n",
        model, provider, fallback_count
    );
    out.push_str(&env_row(
        "STAR_FALLBACK_MODEL",
        "model switched to on 429 (highest priority)",
    ));
    out.push_str(&env_row(
        "STAR_FALLBACK_BASE_URL",
        "endpoint switched to when no fallback model is set",
    ));
    out.push_str(&env_row(
        "STAR_RATE_LIMIT_RETRY_SECS",
        "sleep before retry when nothing to switch to (default 10)",
    ));
    out.push_str(&env_row(
        "STAR_LLM_TIMEOUT",
        "per-request HTTP timeout, seconds (default 120)",
    ));
    out.push_str(&env_row(
        "STAR_CONNECT_TIMEOUT",
        "TCP/TLS connect timeout, seconds (default 30)",
    ));
    out.push_str(&env_row(
        "STAR_TOOL_TIMEOUT_SECS",
        "tool execution cap (defaults: 240 smart_edit/skill, 120 shell, 180 other)",
    ));

    out.push_str(
        "\n### Options when you are being limited\n\n\
         - `/model <name>` — move to a model with separate capacity.\n\
         - `/provider` — configure a second provider, then export `STAR_FALLBACK_BASE_URL` \
           (and a matching key) before launch so 429s roll over automatically.\n\
         - `/compact` — most providers meter tokens per minute; shrinking the prompt lifts the \
           effective request rate.\n\
         - `/extra-usage` — token and cost counters for this session.\n\
         - Wait it out: the cooldown path already retries for you; ESC aborts instead.\n\n\
         ### Not available in this build\n\n\
         - No provider quota API is queried, so remaining allowance and reset time cannot be \
           shown here — check your provider dashboard.\n\
         - `STAR_MODEL_FALLBACK_ENABLED` / `STAR_FALLBACK_MODELS` / `STAR_FALLBACK_BASE_URLS` / \
           `STAR_MODEL_FALLBACK_MAX_RETRIES` are read only by `ModelFallbackManager`, which is \
           never constructed; the live path uses the two single-value variables above.\n",
    );

    push_msg(&mut ctx, out);
    Ok(())
}

/// `/extra-usage` — 本会话真实的 token/成本计数与上下文余量。
/// 只读 UI 状态（`token_usage`、`total_cost`、`cache_*`）与模型上下文窗口缓存。
pub async fn extra_usage(mut ctx: CommandContext<'_>, _args: &[String]) -> CommandResult {
    let model = if ctx.state.current_model.is_empty() {
        "-".to_string()
    } else {
        ctx.state.current_model.clone()
    };
    let provider = ctx
        .state
        .current_provider_id
        .clone()
        .unwrap_or_else(|| "-".to_string());
    let window = ctx
        .state
        .context_window_override
        .or_else(|| {
            crate::agent::model_catalog::get_cached_context_window(&ctx.state.current_model)
        })
        .or_else(|| {
            std::env::var("STAR_CONTEXT_WINDOW")
                .ok()
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(128_000);
    let auto_compact = std::env::var("STAR_AUTO_COMPACT")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(true);

    let tool_calls = ctx
        .state
        .chat_history
        .iter()
        .filter(|e| e.tool_call.is_some())
        .count();
    let mut out = format!(
        "## Usage this session\n\n- model: `{}`\n- provider: `{}`\n- messages: {}\n- tool calls: {}\n\n",
        model,
        provider,
        ctx.state.chat_history.len(),
        tool_calls
    );

    if let Some(usage) = &ctx.state.token_usage {
        let pct = if window > 0 {
            (usage.prompt_tokens as f64 / window as f64) * 100.0
        } else {
            0.0
        };
        out.push_str(&format!(
            "### Tokens (latest response)\n\n\
             | metric | value |\n|---|---|\n\
             | prompt | {} |\n| completion | {} |\n| total | {} |\n\
             | cache read | {} |\n| cache write | {} |\n\n\
             - context window: {} tokens (prompt fills **{:.1}%**)\n",
            usage.prompt_tokens,
            usage.completion_tokens,
            usage.total_tokens,
            ctx.state.cache_read_tokens,
            ctx.state.cache_creation_tokens,
            window,
            pct
        ));
    } else {
        out.push_str(
            "### Tokens\n\nNo completed response yet — counters appear after the first reply.\n",
        );
    }

    out.push_str(&format!(
        "\n### Cost\n\n{}\n\n\
         ### Headroom\n\n\
         - auto-compact: **{}** (`STAR_AUTO_COMPACT`), triggers at 92% of the window\n\
         - `/compact` compacts now · `/context` shows what is taking up the window · \
           `/cost` shows the short form\n\n\
         ### Not available in this build\n\n\
         - No billing, quota, or \"extra usage\" purchase flow: nothing is fetched from the \
           provider, so allowance and overage cannot be reported here.\n\
         - The cost above is a local estimate from token counts and the built-in price table; \
           treat your provider dashboard as authoritative.\n\
         - Usage is not persisted between sessions — the counters reset on restart.\n",
        if ctx.state.total_cost > 0.0 {
            format!("- estimated cost so far: **${:.6}**", ctx.state.total_cost)
        } else {
            "- estimated cost: unavailable (no priced response yet)".to_string()
        },
        on_off(auto_compact)
    ));

    push_msg(&mut ctx, out);
    Ok(())
}

/// `/privacy-settings` — 出网清单：谁会拿到你的数据、何时触发、本地写了什么。
/// 每一行都对应仓库里真实存在的调用点，未接线的模块单独标注。
pub async fn privacy_settings(mut ctx: CommandContext<'_>, _args: &[String]) -> CommandResult {
    let store = crate::core::config::provider_store::ProviderStore::new();
    let cfg = store.load().await.ok();
    let (base_url, key_state) = cfg
        .as_ref()
        .and_then(|c| {
            let pid = c.active_provider_id.clone()?;
            let p = c.providers.get(&pid)?;
            Some((
                p.base_url
                    .clone()
                    .unwrap_or_else(|| "provider default".into()),
                if p.api_key.as_deref().map(|k| !k.is_empty()).unwrap_or(false) {
                    "stored in provider config"
                } else if std::env::var("STAR_API_KEY").is_ok() {
                    "from STAR_API_KEY"
                } else {
                    "not set"
                }
                .to_string(),
            ))
        })
        .unwrap_or_else(|| ("-".to_string(), "unknown".to_string()));

    let mut out = format!(
        "## Privacy\n\n### Where your prompts go\n\n\
         - model: `{}`\n- provider: `{}`\n- endpoint: `{}`\n- API key: {}\n\n\
         Every message, file excerpt, tool result, and system prompt in this session is sent to \
         that endpoint. Nothing else receives conversation content unless a row below fires.\n\n\
         ### Other network destinations\n\n| destination | trigger | content |\n|---|---|---|\n",
        if ctx.state.current_model.is_empty() {
            "-"
        } else {
            &ctx.state.current_model
        },
        ctx.state.current_provider_id.as_deref().unwrap_or("-"),
        base_url,
        key_state
    );
    out.push_str(
        "| `search.brave.com`, `html.duckduckgo.com`, `www.startpage.com` | model calls `WebSearch` | \
           the query string (no API key, scraped HTML) |\n\
         | any host the model picks | model calls `WebFetch` | the URL only |\n\
         | `api.github.com` via the `gh` CLI | `/issue`, `/pr-comments`, `/subscribe-pr`, `/install-github-app` | \
           issue and comment text you pass |\n\
         | plugin marketplace git remotes | `/plugin` install or marketplace add | git clone traffic only |\n\
         | `127.0.0.1:<debug port>` | `/chrome` | localhost CDP request, never leaves the machine |\n",
    );

    let mcp = crate::core::mcp::load_project_mcp_config()
        .await
        .unwrap_or_default();
    let mut remote_mcp = 0usize;
    let mut local_mcp = 0usize;
    let mut mcp_rows = String::new();
    for (name, server) in &mcp.mcp_servers {
        if server.disabled.unwrap_or(false) {
            continue;
        }
        match server.url.as_deref() {
            Some(url) if !url.is_empty() => {
                remote_mcp += 1;
                mcp_rows.push_str(&format!(
                    "| MCP `{}` → `{}` | tool call routed to that server | tool arguments the model sends |\n",
                    name, url
                ));
            }
            _ => local_mcp += 1,
        }
    }
    out.push_str(&mcp_rows);

    let endpoint = ctx.state.remote_settings.endpoint.clone();
    out.push_str(&format!(
        "| remote settings `{}` | `/remote-settings sync` only | no conversation content |\n\
         | MDM server `{}` | `/mdm sync` only | device id |\n\
         | HTTP hooks | only hooks you configure yourself | whatever the hook sends |\n\n\
         - MCP servers configured: **{} local (stdio)**, **{} remote (http/sse)**\n\
         - Local stdio MCP servers run as child processes on this machine; their traffic depends \
           on the server itself.\n\n",
        endpoint.as_deref().unwrap_or("not set"),
        if ctx.state.mdm.enrolled {
            ctx.state
                .mdm
                .server_url
                .clone()
                .unwrap_or_else(|| "enrolled, server unknown".to_string())
        } else {
            "not enrolled".to_string()
        },
        local_mcp,
        remote_mcp
    ));

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    out.push_str(
        "### Telemetry\n\n\
         No usage analytics or traces are transmitted by this build: the analytics manager is \
         never constructed, its HTTP sink only logs what it *would* send, and the Langfuse \
         observer's flush is an explicit no-op. There is no crash reporter and no update ping.\n\n\
         ### What is written to disk\n\n| data | path | state |\n|---|---|---|\n",
    );
    out.push_str(&store_row(
        "session transcript",
        &cwd.join(".star").join("transcript.jsonl"),
        false,
    ));
    out.push_str(&store_row(
        "debug logs",
        &cwd.join(".star").join("logs"),
        true,
    ));
    out.push_str(&store_row(
        "file checkpoints",
        &cwd.join(".star").join("file-history"),
        true,
    ));
    out.push_str(&store_row(
        "project memory",
        &cwd.join(".star").join("memory.md"),
        false,
    ));
    out.push_str(&store_row(
        "credential vault",
        &home.join(".star").join("secure_storage.json"),
        false,
    ));
    out.push_str(
        "\n### Controls that actually take effect\n\n\
         - `/mode` and `/plan` — approval mode is enforced by the worker before every tool run; \
           plan mode also blocks `WebSearch`/`WebFetch` and all writes.\n\
         - `/mcp` — remove a remote server to stop that destination.\n\
         - `/remote-settings` and `/mdm unenroll` — clear the two configurable endpoints.\n\
         - `STAR_LOG_ENABLED=0` stops debug logs; `STAR_TRANSCRIPT=0` stops transcript writing; \
           `STAR_DISABLE_FILE_CHECKPOINTING=1` stops file snapshots.\n\
         - Caveat: rules added with `/permissions` are recorded and displayed, but the tool \
           executor does not consult them — approval mode is the enforcing gate.\n",
    );

    push_msg(&mut ctx, out);
    Ok(())
}

/// `/web-tools` — 列出真实注册的联网工具、它们的出口和唯一生效的门禁。
pub async fn web_tools(mut ctx: CommandContext<'_>, _args: &[String]) -> CommandResult {
    let plan_mode = matches!(ctx.state.approval_mode, crate::types::ApprovalMode::Plan);
    let out = format!(
        "## Web tools\n\n\
         | tool | what it does | credentials |\n|---|---|---|\n\
         | `WebSearch` | scrapes Brave, DuckDuckGo HTML and Startpage result pages, returns titles \
           and snippets | none — no API key, rotating desktop user-agent |\n\
         | `WebFetch` | fetches one URL over HTTP(S) and returns its text for the model | none |\n\n\
         Both are registered unconditionally at startup: the `core_tools` allow-list is `None` at \
         every construction site, so there is no per-tool enable switch to flip.\n\n\
         ### What actually gates them\n\n\
         - approval mode — currently **{}**{}\n\
         - plan mode blocks them outright: the read-only allow-list covers file reads, search and \
           MCP calls, and `WebSearch`/`WebFetch` are not on it\n\
         - `STAR_TOOL_TIMEOUT_SECS` caps a single call (default 180s for these two)\n\n\
         ### Using them\n\n\
         Just ask in plain language — \"search for X\", \"read <url>\" — the model picks the tool. \
         There is no slash command that runs a search directly.\n\n\
         ### Related but not registered\n\n\
         - `web_browser` exists in the source tree but is not added to any registry, so the model \
           cannot call it. `/chrome` covers what is reachable today.\n\
         - `/web-setup` shows which external integrations are configured.\n",
        approval_label(&ctx.state.approval_mode),
        if plan_mode {
            " — web access is denied right now; `/mode default` restores it"
        } else {
            ""
        }
    );
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/web-setup` — 探测每个外部集成的真实状态，并给出下一步命令。
pub async fn web_setup(mut ctx: CommandContext<'_>, _args: &[String]) -> CommandResult {
    let gh_installed = run("gh", &["--version"]).is_ok();
    let gh_authed = gh_installed && run("gh", &["auth", "status"]).is_ok();
    let slug = repo_slug();

    let port = std::env::var("STAR_CHROME_DEBUG_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(9222);
    let chrome_up = probe_chrome(port).await.is_ok();

    let mcp = crate::core::mcp::load_project_mcp_config()
        .await
        .unwrap_or_default();
    let mcp_remote = mcp
        .mcp_servers
        .values()
        .filter(|s| {
            !s.disabled.unwrap_or(false) && s.url.as_deref().map(|u| !u.is_empty()).unwrap_or(false)
        })
        .count();

    let row = |ok: bool, name: &str, detail: String, next: &str| {
        format!(
            "| {} | {} | {} | {} |\n",
            if ok { "✅" } else { "○" },
            name,
            detail,
            next
        )
    };

    let mut out = String::from(
        "## Web integrations\n\n| | integration | state | next step |\n|---|---|---|---|\n",
    );
    out.push_str(&row(
        true,
        "WebSearch / WebFetch",
        "always registered, no key needed".to_string(),
        "`/web-tools`",
    ));
    out.push_str(&row(
        gh_authed,
        "GitHub (`gh` CLI)",
        match (gh_installed, gh_authed, &slug) {
            (false, _, _) => "gh not installed".to_string(),
            (true, false, _) => "installed but not authenticated".to_string(),
            (true, true, Some(s)) => format!("authenticated · repo `{}`", s),
            (true, true, None) => "authenticated · no GitHub remote here".to_string(),
        },
        "`gh auth login` then `/issue`, `/pr-comments`",
    ));
    out.push_str(&row(
        false,
        "Slack",
        "no workspace token is stored by this build".to_string(),
        "`/install-slack-app` prints the manual steps",
    ));
    out.push_str(&row(
        chrome_up,
        "Chrome (CDP)",
        if chrome_up {
            format!("DevTools endpoint answering on port {}", port)
        } else {
            format!("nothing listening on port {}", port)
        },
        "`/chrome` · start Chrome with `--remote-debugging-port`",
    ));
    out.push_str(&row(
        mcp_remote > 0,
        "Remote MCP servers",
        format!("{} http/sse server(s) configured", mcp_remote),
        "`/mcp add`",
    ));
    out.push_str(&row(
        ctx.state.remote_settings.endpoint.is_some(),
        "Remote settings",
        ctx.state
            .remote_settings
            .endpoint
            .clone()
            .unwrap_or_else(|| "no endpoint set".to_string()),
        "`/remote-settings set <url>`",
    ));

    out.push_str(
        "\n- Nothing here is contacted in the background: each row fires only from the command \
         listed next to it.\n\
         - `/privacy-settings` lists every destination and what it receives.\n",
    );
    push_msg(&mut ctx, out);
    Ok(())
}

/// 通过真实的 CDP `/json` 端点探测本机 Chrome，成功返回页面列表。
async fn probe_chrome(port: u16) -> Result<Vec<crate::core::chrome::PageInfo>, String> {
    use crate::core::chrome::{ChromeAction, ChromeMcpConfig, ChromeMcpManager, ChromeResult};
    let mut mgr = ChromeMcpManager::new(ChromeMcpConfig {
        enabled: true,
        debug_port: port,
        extension_id: None,
        auto_connect: false,
    });
    mgr.connect().await?;
    match mgr.execute(ChromeAction::ListPages).await {
        ChromeResult::Pages(pages) => Ok(pages),
        ChromeResult::Error { message } => Err(message),
        _ => Err("unexpected CDP response".to_string()),
    }
}

/// `/chrome [port]` — 连接本机 Chrome 的 DevTools 端点并列出真实标签页。
/// 只有页面列举是真的；导航/点击/截图在本 build 里未实现，明确写出来。
pub async fn chrome(mut ctx: CommandContext<'_>, args: &[String]) -> CommandResult {
    let port = args
        .first()
        .and_then(|a| a.parse::<u16>().ok())
        .or_else(|| {
            std::env::var("STAR_CHROME_DEBUG_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(9222);

    match probe_chrome(port).await {
        Ok(pages) => {
            let mut out = format!(
                "## Chrome · port {}\n\nConnected to the DevTools endpoint. {} target(s):\n\n\
                 | type | title | url |\n|---|---|---|\n",
                port,
                pages.len()
            );
            for p in pages.iter().take(40) {
                let title = if p.title.chars().count() > 60 {
                    format!("{}…", p.title.chars().take(60).collect::<String>())
                } else {
                    p.title.clone()
                };
                let url = if p.url.chars().count() > 80 {
                    format!("{}…", p.url.chars().take(80).collect::<String>())
                } else {
                    p.url.clone()
                };
                out.push_str(&format!("| {} | {} | `{}` |\n", p.type_, title, url));
            }
            out.push_str(&chrome_limits());
            push_msg(&mut ctx, out);
        }
        Err(e) => {
            push_msg(
                &mut ctx,
                format!(
                    "## Chrome · port {}\n\n❌ {}\n\n\
                     Start a browser with remote debugging first, then run `/chrome` again:\n\n\
                     ```\nchrome --remote-debugging-port={}\n```\n\n\
                     Set `STAR_CHROME_DEBUG_PORT` or pass the port: `/chrome 9333`.\n{}",
                    port,
                    e,
                    port,
                    chrome_limits()
                ),
            );
        }
    }
    Ok(())
}

/// Chrome 支持范围的真实说明（连接/列举之外都没实现）
fn chrome_limits() -> String {
    "\n### Scope in this build\n\n\
     - Working: connect over CDP and list targets (tabs, workers, extensions).\n\
     - Not implemented: navigate, click, type, screenshot, network log, console log, \
       wait-for-element — the manager returns a stub or an error for those actions, so they are \
       not exposed here.\n\
     - For real browser automation, add a DevTools MCP server with `/mcp add` and let the model \
       drive it; `WebFetch` already covers \"read this page\".\n"
        .to_string()
}

/// `/artifacts` 认识的产物位置（key、路径、是否目录、说明）
fn artifact_locations() -> Vec<(&'static str, PathBuf, bool, &'static str)> {
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
fn render_artifact_set(name: &str, path: &std::path::Path, is_dir: bool, note: &str) -> String {
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
fn render_artifact_files(
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
struct Check {
    ok: bool,
    name: &'static str,
    detail: String,
}

fn check(ok: bool, name: &'static str, detail: impl Into<String>) -> Check {
    Check {
        ok,
        name,
        detail: detail.into(),
    }
}

/// 目录可写探测：真写一个临时文件再删掉。
fn writable(dir: &std::path::Path) -> Result<(), String> {
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
fn render_checks(checks: &[Check]) -> String {
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
fn perf_report(ctx: &CommandContext<'_>) -> String {
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
fn perf_report_runtime(ctx: &CommandContext<'_>) -> String {
    let st = &ctx.state;
    let mut out = String::from("\n### Session counters\n");

    let window = st
        .context_window_override
        .or_else(|| crate::agent::model_catalog::get_cached_context_window(&st.current_model))
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
fn perf_report_disk(ctx: &CommandContext<'_>) -> String {
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
struct Verifier {
    kind: &'static str,
    source: String,
    command: String,
}

/// 只根据仓库里真实存在的清单文件推断校验命令，不猜测不存在的工具链。
fn detect_verifiers(root: &std::path::Path) -> Vec<Verifier> {
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
fn detect_verifiers_more(root: &std::path::Path) -> Vec<Verifier> {
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
fn write_verification_section(path: &std::path::Path, section: &str) -> Result<bool, String> {
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
fn verification_section(verifiers: &[Verifier]) -> String {
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
fn render_verifiers(root: &std::path::Path, verifiers: &[Verifier], section: &str) -> String {
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
struct Step {
    done: bool,
    title: &'static str,
    detail: String,
    action: &'static str,
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
async fn onboarding_project_steps(ctx: &CommandContext<'_>, cwd: &std::path::Path) -> Vec<Step> {
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
fn render_steps(steps: &[Step]) -> String {
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
fn agent_defs_in(dir: &std::path::Path) -> Vec<(String, String)> {
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
async fn agents_platform_teams(storage: &crate::core::config::storage::Storage) -> String {
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
async fn assistant_sources(ctx: &CommandContext<'_>, cwd: &std::path::Path) -> String {
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
                    p.selected_model.as_deref().unwrap_or("—")
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
const STICKERS: &[(&str, &str)] = &[
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
fn split_schedule_args(args: &[String]) -> (String, Result<u64, String>) {
    let split_at = args
        .iter()
        .position(|a| a.eq_ignore_ascii_case("in") || a.eq_ignore_ascii_case("at"));
    match split_at {
        Some(idx) => (args[..idx].join(" "), parse_schedule_delay(&args[idx..])),
        None => (args.join(" "), Ok(DEFAULT_SCHEDULE_DELAY_SECS)),
    }
}

/// `/schedule add` 未指定时间时的默认延迟。
const DEFAULT_SCHEDULE_DELAY_SECS: u64 = 60;

/// 触发到期时若处于离线模式，最多等多久回到在线。
const SCHEDULE_OFFLINE_WAIT_SECS: u64 = 1_800;
const SCHEDULE_OFFLINE_POLL_SECS: u64 = 15;

/// 到期触发落在离线模式里时的处理：轮询等待回到在线，返回 `true` 表示可以发送。
///
/// 超过 `SCHEDULE_OFFLINE_WAIT_SECS` 仍离线就放弃 —— 并把触发从存储里摘掉，否则它会
/// 以一条永不到期的过期记录留在 `.star/triggers.json` 里，直到 stale 清理才消失。
async fn wait_until_online(fire_id: &str) -> bool {
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
fn parse_schedule_delay(args: &[String]) -> Result<u64, String> {
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

fn format_schedule_when(secs: u64) -> String {
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
    let commit_and_push = super::git_wrapper(args.clone()).await;
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
pub async fn force_snip(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
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
        .send(crate::runtime::messages::AgentRequest::Compress { message_id })
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
