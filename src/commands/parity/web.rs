//! parity 命令族 —— web（对标 Claude Code 命令面，拆自 parity.rs 2026-11-09）

use crate::commands::execution::{CommandContext, CommandResult};
use crate::commands::parity::core::approval_label;
use crate::commands::parity::core::on_off;
use crate::commands::parity::core::push_msg;
use crate::commands::parity::core::redact;
use crate::commands::parity::core::run;
use crate::commands::parity::git_github::repo_slug;
use crate::commands::parity::memory::store_row;
use crate::runtime::messages::AgentRequest;
use crate::types::ChatEntry;
use std::path::PathBuf;

/// 渲染一行「环境变量 | 当前值 | 说明」表格行（值按名字脱敏）
pub(crate) fn env_row(name: &str, note: &str) -> String {
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
           `STAR_MODEL_FALLBACK_MAX_RETRIES` belong to the multi-model `ModelFallbackManager`, \
           which is constructed but only reached via `STAR_MODEL_FALLBACK_*` gating; the live \
           429 path uses the two single-value variables above.\n",
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
    let provider_id = crate::ui::utils::status::current_provider_id(ctx.state);
    let window = ctx
        .state
        .context_window_override
        .or_else(|| {
            crate::agent::model_catalog::get_cached_context_window(
                provider_id.as_deref(),
                &ctx.state.current_model,
            )
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
pub(crate) async fn probe_chrome(port: u16) -> Result<Vec<crate::core::chrome::PageInfo>, String> {
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
pub(crate) fn chrome_limits() -> String {
    "\n### Scope in this build\n\n\
     - Working: connect over CDP and list targets (tabs, workers, extensions).\n\
     - Not implemented: navigate, click, type, screenshot, network log, console log, \
       wait-for-element — the manager returns a stub or an error for those actions, so they are \
       not exposed here.\n\
     - For real browser automation, add a DevTools MCP server with `/mcp add` and let the model \
       drive it; `WebFetch` already covers \"read this page\".\n"
        .to_string()
}
