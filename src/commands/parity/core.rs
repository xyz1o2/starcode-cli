//! parity 命令族 —— core（对标 Claude Code 命令面，拆自 parity.rs 2026-11-09）

use crate::commands::execution::{CommandContext, CommandResult};
use crate::runtime::messages::AgentRequest;
use crate::types::ChatEntry;
use std::path::PathBuf;

pub(crate) fn push_msg(ctx: &mut CommandContext<'_>, content: impl Into<String>) {
    ctx.state
        .chat_history
        .push(ChatEntry::assistant(content).with_streaming(false));
}

/// `/lens` —— StarLens 代码认知层：`build [--enrich]` / `status`（默认）/ `update`。
pub async fn lens(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    use crate::core::config::storage::Storage;
    use crate::core::context::lens_service;

    let sub = args.first().map(|s| s.as_str()).unwrap_or("status");

    // build 走 agent（可带 --enrich 用模型补 F/R/S），结果经 StreamMessage 异步回显。
    if sub == "build" {
        let enrich = args.iter().any(|a| a == "--enrich" || a == "enrich");
        let message_id = ctx.state.next_message_id;
        ctx.state.next_message_id += 1;
        ctx.agent_tx
            .send(AgentRequest::LensRebuild { message_id, enrich })
            .await
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    // status / update 本地直接算（不需模型）。
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let storage = Storage::new(cwd);
    let profile = lens_service::scope_profile();
    let out = match sub {
        "status" | "" => match lens_service::check_drift(&storage, profile) {
            Some(d) if d.is_aligned() => "# StarLens status\n\n✅ 对齐，无漂移。".to_string(),
            Some(d) => format!(
                "# StarLens status\n\n⚠️ 漂移 {} 项\n- stale: {:?}\n- missing: {:?}\n- orphan: {:?}\n- unbaselined: {:?}\n\n运行 `/lens update` 增量重算。",
                d.total(), d.stale, d.missing, d.orphan, d.unbaselined
            ),
            None => "# StarLens status\n\n尚无索引，先运行 `/lens build`。".to_string(),
        },
        "update" => match lens_service::update(&storage, profile) {
            Some(d) if d.is_aligned() => "# StarLens update\n\n✅ 本已对齐，无需重算。".to_string(),
            Some(d) => format!(
                "# StarLens update\n\n已增量重算 {} 项（stale {} / unbaselined {} 刷新，missing {} / orphan {} 清理）。",
                d.total(), d.stale.len(), d.unbaselined.len(), d.missing.len(), d.orphan.len()
            ),
            None => "# StarLens update\n\n尚无索引，先运行 `/lens build`。".to_string(),
        },
        _ => "用法：`/lens [build|build --enrich|status|update]`".to_string(),
    };
    push_msg(&mut ctx, out);
    Ok(())
}

/// 把提示交给主对话回路（与 /git-status 等命令一致）
pub(crate) async fn ask_agent(ctx: &mut CommandContext<'_>, prompt: String) -> CommandResult {
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
pub(crate) fn run(program: &str, args: &[&str]) -> Result<String, String> {
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

pub(crate) fn on_off(enabled: bool) -> &'static str {
    if enabled {
        "on"
    } else {
        "off"
    }
}

/// 项目 `.star/` 目录（按需创建）
pub(crate) fn star_dir() -> Result<PathBuf, String> {
    let dir = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .join(".star");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

pub(crate) fn approval_label(mode: &crate::types::ApprovalMode) -> &'static str {
    mode.as_str()
}

/// 敏感环境变量只显示是否设置与长度，不回显明文
pub(crate) fn is_secretish(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    ["KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL", "COOKIE"]
        .iter()
        .any(|k| n.contains(k))
}

pub(crate) fn redact(name: &str, value: &str) -> String {
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
    // 路径必须与代码实际读写的一致：provider_store 用的是 ~/.star/user-settings.json，
    // 不是 settings.json。列错文件，用户照着改了也不生效，只会以为配置被吞了。
    let mut paths: Vec<PathBuf> = Vec::new();
    paths.push(crate::core::config::storage::Storage::global_star_dir().join("user-settings.json"));
    if let Ok(cwd) = std::env::current_dir() {
        paths.push(
            crate::core::config::storage::Storage::new(cwd.clone()).workspace_settings_path(),
        );
        if let Some(ctx_file) = crate::utils::project_context::find_project_context_file(&cwd) {
            paths.push(ctx_file);
        }
    }
    for p in paths {
        out.push_str(&format!(
            "- `{}` {}\n",
            p.display(),
            if p.exists() { "✅" } else { "—" }
        ));
    }
    out.push_str("_Run `starcode doctor` for a parse check of each file._\n");

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

/// Parse the token budget accepted by `/autocompact` (`200k`, `1m`, `500000`).
pub(crate) fn parse_autocompact_tokens(raw: &str) -> Result<usize, String> {
    let lower = raw.trim().to_ascii_lowercase();
    let without_unit = lower.strip_suffix("tokens").unwrap_or(&lower).trim();
    let cleaned = without_unit.replace([',', '_'], "");

    let (number, multiplier) = if let Some(number) = cleaned.strip_suffix('k') {
        (number, 1_000f64)
    } else if let Some(number) = cleaned.strip_suffix('m') {
        (number, 1_000_000f64)
    } else {
        (cleaned.as_str(), 1f64)
    };

    let value = number
        .trim()
        .parse::<f64>()
        .map_err(|_| "Expected a token count such as `200k`, `1m`, or `500000`.".to_string())?;
    if !value.is_finite() || value <= 0.0 {
        return Err("Token count must be a positive number.".to_string());
    }

    let tokens = value * multiplier;
    if !tokens.is_finite() || tokens < 10_000.0 {
        return Err("Auto-compact needs at least 10,000 tokens.".to_string());
    }
    if tokens > 10_000_000.0 {
        return Err("Token count is unreasonably large (maximum 10,000,000).".to_string());
    }
    Ok(tokens.round() as usize)
}

pub(crate) fn format_token_count(tokens: usize) -> String {
    let digits = tokens.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(character);
    }
    out
}

/// `/autocompact [auto|on|off|<tokens>]` — control the automatic compaction
/// threshold. Values are stored in the self-tuning overlay, which the compact
/// manager reads on the next loop iteration, so no restart is needed.
pub async fn autocompact(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    use crate::core::self_tuning::{overrides, ValueSource};

    if args.is_empty() {
        let enabled = crate::agent::context::auto_compact_enabled();
        let (raw_threshold, source) = overrides::resolve("compact.max_tokens");
        let threshold = match source {
            ValueSource::SelfTuned => raw_threshold
                .as_deref()
                .and_then(|value| value.parse::<usize>().ok())
                .map(|tokens| {
                    format!(
                        "{} tokens (set by `/autocompact`)",
                        format_token_count(tokens)
                    )
                })
                .unwrap_or_else(|| "invalid override".to_string()),
            ValueSource::Environment => raw_threshold
                .as_deref()
                .map(|value| format!("{} tokens (`STAR_COMPACT_MAX_TOKENS`)", value))
                .unwrap_or_else(|| "environment override".to_string()),
            ValueSource::Default => "model context policy (auto)".to_string(),
        };

        push_msg(
            &mut ctx,
            format!(
                "## Auto-compact\n\n\
                 - Status: **{}**\n\
                 - Threshold: {}\n\n\
                 Usage: `/autocompact auto|on|off|<tokens>` (for example `/autocompact 200k`).\n\
                 Changes take effect on the next loop iteration.",
                on_off(enabled),
                threshold
            ),
        );
        return Ok(());
    }

    let request = args.join(" ");
    let request = request.trim();
    let lower = request.to_ascii_lowercase();
    let result = (|| -> Result<String, String> {
        match lower.as_str() {
            "auto" | "default" | "reset" => {
                overrides::reset(Some("compact.max_tokens"))?;
                overrides::set_user("compact.auto_enabled", "true", true)?;
                Ok("Auto-compact restored to the model context policy and enabled.".to_string())
            }
            "on" | "enable" | "true" => {
                overrides::set_user("compact.auto_enabled", "true", true)?;
                Ok("Auto-compact enabled.".to_string())
            }
            "off" | "disable" | "false" => {
                overrides::set_user("compact.auto_enabled", "false", true)?;
                Ok("Auto-compact disabled.".to_string())
            }
            _ => {
                let tokens = parse_autocompact_tokens(request)?;
                overrides::set_user("compact.max_tokens", &tokens.to_string(), true)?;
                overrides::set_user("compact.auto_enabled", "true", true)?;
                Ok(format!(
                    "Auto-compact enabled; threshold set to {} tokens.",
                    format_token_count(tokens)
                ))
            }
        }
    })();

    match result {
        Ok(message) => push_msg(
            &mut ctx,
            format!("{}\n\nIt takes effect on the next loop iteration.", message),
        ),
        Err(error) => push_msg(&mut ctx, format!("❌ {}", error)),
    }
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
///
/// 别名不再在这里私养一份：全部走 `approval_mode_from_str` 这份统一口径
/// （settings `defaultMode`、`--permission-mode`、`/permissions` 同源）。
/// 此前这份本地表把 `"auto" => "yolo"`，在 `Auto` 独立成档之后就把
/// `/mode auto` 错切成了全放行。
pub async fn mode(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if args.is_empty() {
        let current = approval_label(&ctx.state.approval_mode);
        let out = format!(
            "# Permission mode\n\nCurrent: **{}**\n\n\
             - `/mode auto` — LLM classifier reviews each action instead of you\n\
             - `/mode default` — ask before risky writes/commands; read-only tools run free\n\
             - `/mode acceptEdits` — also apply file edits and read-only shell commands without asking\n\
             - `/mode plan` — read-only research & planning, no mutating tools\n\
             - `/mode bypassPermissions` — run everything without asking (alias: yolo)\n\
             - `/mode dontAsk` — never prompt; anything that would ask is denied (CI/headless)\n\n\
             _Rules and the deny log live under `/permissions`._",
            current
        );
        push_msg(&mut ctx, out);
        return Ok(());
    }

    let arg = args[0].trim();
    // 旧名（`yolo` / `bypass` / `dangerously-skip-permissions` …）由统一口径
    // `approval_mode_from_str` 认领，这里不再私养映射。
    let mapped = match crate::core::policy::approval_mode_from_str(arg) {
        Some(m) => m.as_str().to_string(),
        None => {
            push_msg(
                &mut ctx,
                format!(
                    "❌ Unknown mode: `{}`. Usage: /mode [auto|default|acceptEdits|plan|bypassPermissions|dontAsk]",
                    arg
                ),
            );
            return Ok(());
        }
    };
    crate::commands::permissions::run(ctx, vec![mapped]).await
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

pub(crate) fn tags_file() -> Result<PathBuf, String> {
    Ok(star_dir()?.join("session_tags.json"))
}

pub(crate) fn load_tags() -> TagMap {
    tags_file()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub(crate) fn save_tags(map: &TagMap) -> Result<(), String> {
    let path = tags_file()?;
    let json = serde_json::to_string_pretty(map).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

pub(crate) fn session_key(ctx: &CommandContext<'_>) -> String {
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
pub(crate) fn parse_toggle(args: &[String], current: bool) -> Result<bool, String> {
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
