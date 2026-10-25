#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_assignments,
    unused_must_use
)]

mod agent;
mod commands;
mod core;
mod llm;
mod runtime;
mod tools;
mod types;
mod ui;
mod utils;

use clap::{Parser, Subcommand, ValueEnum};
use std::sync::Arc;

use crate::types::ChatEntry;

#[derive(Subcommand)]
enum Commands {
    /// Manage MCP (Model Context Protocol) servers
    Mcp {
        #[command(subcommand)]
        command: crate::commands::mcp::McpCommand,
    },
    /// Git operations with AI assistance
    Git {
        #[command(subcommand)]
        command: crate::commands::git::GitCommand,
    },
    /// Initialize a new project with STAR.md
    Init,
    /// Diagnose environment: config files, provider credentials, toolchain, logs
    ///
    /// 不启动 TUI 也能跑——环境类失败恰恰发生在配置坏掉、TUI 起不来的时候。
    /// 报告内容与 `/doctor` 完全一致（共用 build_report）。
    Doctor,
}

#[derive(Debug, Clone, ValueEnum)]
enum HeadlessOutputFormat {
    /// 每行一条最终对话条目（`ChatEntry` 的 JSON）。信息量有限：只有助手正文。
    Jsonl,
    /// 纯文本，只有助手正文。
    Text,
    /// 每行一条**事件**（agent 的完整过程：工具轨迹、增量文本、token 用量、
    /// 错误），最后跟一条 `result` 终态事件（带 usage / cost / is_error）。
    ///
    /// 这是给外部 harness 用的格式 —— `jsonl` 只有最终答案，没法知道 agent
    /// 中间做了什么。
    StreamJson,
}

#[derive(Parser)]
// name/bin_name 在 main() 里按 argv[0] 覆盖（见 utils::invocation）；这里给的是兜底值。
#[command(name = crate::utils::invocation::CANONICAL_PROGRAM_NAME)]
#[command(version)] // 没有这行 clap 不认 --version，`sc --version` 会以退出码 2 失败
#[command(
    about = "A conversational AI CLI tool powered by star (OpenAI-compatible) with text editor capabilities"
)]
struct CliArgs {
    /// Initial message to send to star
    #[arg(value_parser)]
    message: Vec<String>,

    /// Set working directory
    #[arg(short = 'd', long = "directory", default_value = ".")]
    directory: String,

    /// API key (or set STAR_API_KEY env var)
    #[arg(short = 'k', long = "api-key")]
    api_key: Option<String>,

    /// API base URL (or set STAR_BASE_URL env var)
    #[arg(short = 'u', long = "base-url")]
    base_url: Option<String>,

    /// AI model to use
    #[arg(short = 'm', long = "model")]
    model: Option<String>,

    /// Process a single prompt and exit (headless mode)
    #[arg(short = 'p', long = "prompt", visible_alias = "print")]
    prompt: Option<String>,

    /// Output format for headless mode
    ///
    /// - `jsonl`（默认）：每行一条最终对话条目。只有助手正文，工具轨迹和
    ///   token 用量都拿不到。
    /// - `text`：纯文本，只有助手正文。
    /// - `stream-json`：每行一条**事件**（工具轨迹、逐字增量、token 用量、错误），
    ///   最后跟一条 `result` 终态行（带 usage / cost / is_error / session_id）。
    ///   这是给外部 harness（CI、编排器、IDE 插件）用的格式。
    #[arg(long = "output-format", default_value = "jsonl", value_enum)]
    output_format: HeadlessOutputFormat,

    /// Maximum number of tool execution rounds (default: 400)
    #[arg(long = "max-tool-rounds", default_value = "400")]
    max_tool_rounds: u32,

    /// Maximum number of agent turns (default: 200)
    #[arg(long = "max-turns", default_value = "200")]
    max_turns: i32,

    /// Skip all permission prompts (dangerous!)
    #[arg(long = "dangerously-skip-permissions")]
    dangerously_skip_permissions: bool,

    /// Approval mode: default | plan | yolo | acceptEdits | bypassPermissions
    #[arg(long = "permission-mode", visible_alias = "approval-mode")]
    permission_mode: Option<String>,

    /// Resume a previous session by ID, or latest session if no ID provided
    #[arg(long = "resume", short = 'r', num_args = 0..=1, default_missing_value = "latest")]
    resume: Option<String>,

    /// Continue the most recent session (shortcut for `--resume latest`)
    #[arg(long = "continue", short = 'c', conflicts_with = "resume")]
    continue_session: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

struct ResolvedResumeSession {
    id: String,
    session: crate::utils::session_manager::Session,
}

/// Resolve CLI resume input to the one canonical JSON snapshot that will own the session.
/// Exact IDs win; only explicit non-auto IDs may use the historical `auto-` fallback.
async fn resolve_resume_session(requested_id: &str) -> Result<ResolvedResumeSession, String> {
    let exact_id = if requested_id == "latest" {
        crate::utils::session_manager::read_latest_session_id()
            .await
            .map_err(|error| format!("Failed to read latest session: {}", error))?
            .ok_or_else(|| "No sessions found. Start a new session first.".to_string())?
    } else {
        requested_id.to_string()
    };

    match crate::utils::session_manager::load_session(&exact_id).await {
        Ok(session) => Ok(ResolvedResumeSession {
            id: exact_id,
            session,
        }),
        Err(exact_error) if requested_id != "latest" && !exact_id.starts_with("auto-") => {
            let fallback_id = format!("auto-{}", exact_id);
            crate::utils::session_manager::load_session(&fallback_id)
                .await
                .map(|session| ResolvedResumeSession {
                    id: fallback_id,
                    session,
                })
                .map_err(|_| format!("Failed to load session '{}': {}", exact_id, exact_error))
        }
        Err(error) => Err(format!("Failed to load session '{}': {}", exact_id, error)),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // On Windows, set console output code page to UTF-8 so Chinese/non-ASCII
    // characters render correctly in cmd.exe (which defaults to CP936/GBK).
    // This must be done before any println!/eprintln! output.
    #[cfg(windows)]
    let _saved_output_cp = crate::ui::win32::set_console_output_utf8();

    // Load environment variables
    dotenvy::dotenv().ok();

    // sc / starcode / starcode-cli 是同一个二进制的三个入口，usage 和报错要回显
    // 本次实际敲的那个名字，所以 name 在这里按 argv[0] 覆盖掉派生宏里的兜底值。
    let args = {
        use clap::{CommandFactory, FromArgMatches};
        let prog = crate::utils::invocation::program_name();
        let matches = CliArgs::command()
            .name(prog.clone())
            .bin_name(prog)
            .get_matches();
        CliArgs::from_arg_matches(&matches).unwrap_or_else(|e| e.exit())
    };

    // Handle subcommands first
    match args.command {
        Some(Commands::Mcp { command }) => {
            match crate::commands::mcp::execute_mcp_command(command).await {
                Ok(msg) => println!("{}", msg),
                Err(e) => eprintln!("Error: {}", e),
            }
            return Ok(());
        }
        Some(Commands::Git { command }) => {
            match crate::commands::git::execute_git_command(command).await {
                Ok(_) => return Ok(()),
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Some(Commands::Init) => {
            let cwd = std::env::current_dir()?;
            match crate::commands::init::generate_initial_context(&cwd) {
                Ok(path) => {
                    println!("✅ Successfully created context file at {}", path.display());
                    println!("💡 Tip: Run `starcode` and use `/init` for a full AI analysis.");
                }
                Err(e) => {
                    eprintln!("❌ {}", e);
                    if e.contains("already exists") {
                        println!(
                            "💡 Tip: Run `starcode` and use `/init` to improve the existing file."
                        );
                    } else {
                        std::process::exit(1);
                    }
                }
            }
            return Ok(());
        }
        Some(Commands::Doctor) => {
            // doctor 必须能在 TUI 起不来时用，所以这里不做任何终端初始化，
            // 也不初始化 Config/StarAgent——那正是它要诊断的东西。
            println!("{}", crate::commands::doctor::run_cli().await);
            return Ok(());
        }
        None => {}
    }

    // Change directory if specified
    if args.directory != "." {
        std::env::set_current_dir(&args.directory)?;
    }

    // 并行加载配置：settings_manager, provider_store 可以同时进行
    let (settings_manager_result, provider_store_result, cwd) = tokio::join!(
        core::config::settings_manager::get_settings_manager(),
        async {
            let store = crate::core::config::provider_store::ProviderStore::new();
            (store.load().await.unwrap_or_default(), store)
        },
        async { std::env::current_dir() }
    );

    let settings_manager = settings_manager_result.map_err(|e| e as Box<dyn std::error::Error>)?;
    let cwd = cwd?;
    let cwd_clone = cwd.clone(); // 克隆一份用于后台初始化
    let (provider_config, _provider_store) = provider_store_result;

    // settings 依赖 settings_manager，但可以在加载后立即初始化 i18n
    let settings = settings_manager
        .load_user_settings()
        .await
        .map_err(|e| e as Box<dyn std::error::Error>)?;
    core::i18n::init(settings.ui_language.as_deref(), &cwd);
    // 持久化的思考力度档位在这里就落到会话状态里：TUI 和 headless 都要经过
    // 这一段，构造请求时由 `llm::thinking` 按 provider 方言翻成对应字段。
    if let Some(effort) = settings
        .thinking_effort
        .as_deref()
        .and_then(llm::thinking::parse_effort)
    {
        llm::thinking::set_session_effort(&effort);
    }
    let provider_resolution =
        crate::core::config::provider_resolution::resolve_effective_provider_settings(
            crate::core::config::provider_resolution::ProviderResolutionInputs {
                cli_model: args.model.clone(),
                cli_base_url: args.base_url.clone(),
                cli_api_key: args.api_key.clone(),
                ..Default::default()
            },
            &provider_config,
            &settings,
        );

    let effective_provider_id = provider_resolution.provider_id.clone();
    let model = provider_resolution.model.value.clone();

    let api_key = if let Some(key) = provider_resolution.api_key.value.clone() {
        if let Some(provider_id) = effective_provider_id.as_deref() {
            utils::logging::append_agent_log_line(&format!(
                "API key loaded from provider '{}': {}",
                provider_id, "yes"
            ));
        }
        key
    } else {
        // No API key provided - if in headless mode, exit with error; if interactive, allow to continue but warn
        if args.prompt.is_some() {
            eprintln!(
                "❌ Error: API key required. Set STAR_API_KEY environment variable, use --api-key flag, or set \"apiKey\" field in ~/.star/user-settings.json"
            );
            std::process::exit(1);
        } else {
            // No API key but in interactive mode - set to a placeholder to allow UI to start
            "API_KEY_NOT_SET".to_string()
        }
    };

    let base_url = match provider_resolution.base_url.value.clone() {
        Some(url) => url,
        None => "BASE_URL_NOT_SET".to_string(),
    };

    let is_openai_compatible = if provider_resolution.openai_compatible_source.kind
        == crate::core::config::provider_resolution::SRC_RUNTIME_DEFAULT_OPENAI_COMPATIBLE
    {
        None
    } else {
        Some(provider_resolution.openai_compatible)
    };

    let cli_approval_mode = if args.dangerously_skip_permissions {
        Some(crate::core::policy::types::ApprovalMode::Yolo)
    } else if let Some(mode) = args.permission_mode.as_deref() {
        Some(parse_cli_approval_mode(mode).map_err(|e| {
            let err: Box<dyn std::error::Error> = e.into();
            err
        })?)
    } else {
        // settings.json 里的 `permissions.defaultMode`（对标 Claude Code）。
        // 优先级低于 CLI flag，高于内置默认值。
        crate::core::policy::SettingsPermissions::from_project(&cwd)
            .default_mode
            .as_deref()
            .and_then(crate::core::policy::approval_mode_from_str)
    };

    // `--continue` 是 `--resume latest` 的别名（对标 CC 的 `-c`）。
    // 之前只有 `-r`，而"接着上次继续"恰恰是最常用的那条路径 —— 用户被迫先知道
    // 自己的 session id 才能续上。
    let requested_resume = if args.continue_session {
        Some("latest")
    } else {
        args.resume.as_deref()
    };
    let resumed_session = match requested_resume {
        Some(requested_id) => Some(resolve_resume_session(requested_id).await.map_err(
            |error| {
                eprintln!("{}", error);
                let boxed: Box<dyn std::error::Error> = error.into();
                boxed
            },
        )?),
        None => None,
    };

    // 创建 Config：成功解析的 canonical 文件名是整个运行期的会话身份。
    let session_id = resumed_session
        .as_ref()
        .map(|resolved| resolved.id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let config_params = core::config::ConfigParameters {
        session_id,
        resume_session: resumed_session.is_some(),
        sandbox: None,
        target_dir: cwd.clone(),
        debug_mode: false,
        question: None,
        core_tools: None,
        allowed_tools: None,
        exclude_tools: None,
        tool_discovery_command: None,
        tool_call_command: None,
        mcp_server_command: None,
        mcp_servers: None,
        user_memory: None,
        star_md_file_count: None,
        star_md_file_paths: None,
        approval_mode: cli_approval_mode,
        show_memory_usage: None,
        context_file_name: None,
        accessibility: None,
        telemetry: None,
        usage_statistics_enabled: None,
        // settings.json 的 `fileFiltering` —— 之前这里恒为 None，
        // `Config::file_filtering()` 拿到的永远是硬编码默认值。
        file_filtering: settings.file_filtering.clone(),
        checkpointing: None,
        proxy: None,
        disable_model_router_for_auth: None,
        cwd,
        bug_command: None,
        model: model.clone().unwrap_or_default(),
        max_session_turns: Some(args.max_turns),
        list_sessions: None,
        delete_session: None,
        list_extensions: None,
        enabled_extensions: None,
        enable_extension_reloading: None,
        allowed_mcp_servers: None,
        blocked_mcp_servers: None,
        allowed_environment_variables: None,
        blocked_environment_variables: None,
        enable_environment_variable_redaction: None,
        no_browser: None,
        summarize_tool_output: None,
        folder_trust: None,
        ide_mode: None,
        load_memory_from_include_directories: None,
        import_format: None,
        discovery_max_dirs: None,
        compression_threshold: None,
        context_window: settings.context_window.map(|v| v as usize),
        interactive: None,
        pty_info: None,
        trusted_folder: None,
        use_ripgrep: None,
        enable_interactive_shell: None,
        skip_next_speaker_check: None,
        extension_management: None,
        enable_prompt_completion: None,
        truncate_tool_output_threshold: None,
        truncate_tool_output_lines: None,
        enable_tool_output_truncation: None,
        use_write_todos: None,
        output: None,
        codebase_investigator_settings: None,
        introspection_agent_settings: None,
        continue_on_failed_api_call: None,
        retry_fetch_errors: None,
        enable_shell_output_efficiency: None,
        shell_tool_inactivity_timeout: None,
        fake_responses: None,
        record_responses: None,
        disable_yolo_mode: None,
        mcp_enabled: None,
        enable_hooks: None,
        hooks: None,
        project_hooks: None,
        preview_features: None,
        enable_agents: None,
        skills_support: None,
        disabled_skills: None,
        experimental_jit_context: None,
        recursion_depth: None,
        // `-p` 没有 UI 订阅确认总线，策略引擎据此把 AskUser 直接判成拒绝。
        headless: args.prompt.is_some().then_some(true),
    };

    if let Some(prompt) = args.prompt {
        // Headless mode: initialize config synchronously, then process prompt.
        // 与交互模式同样有初始化超时：否则网络/文件系统卡住时 -p 会永久挂起，
        // CI 里只能靠外层 timeout 强杀，拿不到任何错误信息。
        let mut config = core::config::Config::new(config_params);
        match tokio::time::timeout(init_timeout_duration(), config.initialize()).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                utils::logging::append_agent_log_line(&format!("Config 初始化失败: {}", e));
                return Err(format!("Config 初始化失败: {}", e).into());
            }
            Err(_) => {
                let secs = init_timeout_duration().as_secs();
                utils::logging::append_agent_log_line(&format!(
                    "[INIT] Config::initialize timed out ({} seconds)",
                    secs
                ));
                return Err(format!("Config 初始化超时（{} 秒）", secs).into());
            }
        }
        let config = Arc::new(config);

        if api_key == "API_KEY_NOT_SET" {
            utils::logging::append_agent_log_line("Error: API key required for headless mode.");
            eprintln!("❌ Error: API key required for headless mode.");
            std::process::exit(1);
        }

        process_prompt_headless(
            &api_key,
            base_url,
            model,
            args.max_tool_rounds,
            is_openai_compatible,
            effective_provider_id.clone(),
            config.clone(),
            resumed_session,
            args.output_format.clone(),
            prompt,
        )
        .await
        .map_err(|e| e as Box<dyn std::error::Error>)?;
    } else {
        // Interactive mode: initialize terminal first, then agent in background
        let initial_message = args.message.join(" ");
        let initial_session_id = resumed_session.as_ref().map(|resolved| resolved.id.clone());
        let initial_session = resumed_session.map(|resolved| {
            eprintln!("Resumed session: {}", resolved.id);
            resolved.session
        });
        let initial_history = initial_session
            .as_ref()
            .map(|session| session.chat_history.clone())
            .unwrap_or_default();
        let initial_usage = initial_session
            .as_ref()
            .and_then(|session| session.last_usage.clone());
        let restored_context = initial_session.as_ref().map(|session| {
            (
                crate::utils::session_manager::agent_messages_for_restore(session),
                session.pending_local_context.clone().unwrap_or_default(),
            )
        });

        // Spawn heavy init as a background task with timeout
        let (init_tx, init_rx) = tokio::sync::oneshot::channel::<
            Result<(agent::StarAgent, Arc<core::config::Config>), String>,
        >();
        let api_key2 = api_key.clone();
        let model2 = model.clone();
        let base_url2 = base_url.clone();
        let config_params2 = config_params.clone();
        let restored_context2 = restored_context.clone();
        let _cwd2 = cwd_clone;
        let effective_provider_id2 = effective_provider_id.clone();
        tokio::spawn(async move {
            // Overall timeout for initialization (30 seconds - reduced from 60)
            let init_timeout = init_timeout_duration();

            let result = tokio::time::timeout(init_timeout, async {
                utils::logging::append_agent_log_line("[INIT] Starting Config::new...");
                let mut config = core::config::Config::new(config_params2);

                utils::logging::append_agent_log_line("[INIT] Starting Config::initialize...");
                let config = match config.initialize().await {
                    Ok(_) => {
                        utils::logging::append_agent_log_line(
                            "[INIT] Config::initialize completed",
                        );
                        Ok(Arc::new(config))
                    }
                    Err(e) => {
                        utils::logging::append_agent_log_line(&format!(
                            "[INIT] Config::initialize failed: {}",
                            e
                        ));
                        Err(format!("Config 初始化失败: {}", e))
                    }
                }?;

                utils::logging::append_agent_log_line("[INIT] Starting StarAgent::new...");
                let mut agent = agent::StarAgent::new_with_provider(
                    &api_key2,
                    model2,
                    base_url2,
                    Some(args.max_tool_rounds),
                    is_openai_compatible,
                    Some(config.clone()),
                    // provider id 必须一起进 client：Settings 面板靠它显示供应商，
                    // 协议推断和 key 环境变量解析也都靠它，缺了会静默走错网关。
                    effective_provider_id2,
                )
                .await
                .map_err(|e| {
                    utils::logging::append_agent_log_line(&format!(
                        "[INIT] StarAgent::new failed: {}",
                        e
                    ));
                    format!("Agent 初始化失败: {}", e)
                })?;

                if let Some((messages, pending_local_context)) = restored_context2 {
                    agent.replace_session_context(messages, pending_local_context);
                }

                utils::logging::append_agent_log_line("[INIT] StarAgent::new completed");
                // 显式标注：oneshot 通道收的是 String 错误，下游只 format! 它，
                // 不再能靠 Err(e) 的传递反推出类型。
                Ok::<_, String>((agent, config))
            })
            .await;

            match result {
                Ok(Ok(pair)) => {
                    utils::logging::append_agent_log_line(
                        "[INIT] Initialization completed successfully",
                    );
                    let _ = init_tx.send(Ok(pair));
                }
                Ok(Err(e)) => {
                    utils::logging::append_agent_log_line(&format!(
                        "[INIT] Initialization failed: {}",
                        e
                    ));
                    // 同超时分支：错误本身往往只是表象（"API key required" 之类），
                    // 真正的来源链记在 agent.log 里，给用户一个可查的位置。
                    let log_hint = init_log_hint();
                    let _ = init_tx.send(Err(format!("{}。{}", e, log_hint)));
                }
                Err(_) => {
                    let secs = init_timeout_duration().as_secs();
                    utils::logging::append_agent_log_line(&format!(
                        "[INIT] Initialization timed out ({} seconds)",
                        secs
                    ));
                    // 超时消息必须指向日志位置：否则用户只看到一句无信息的报错，
                    // 不知道 [INIT] 面包屑记在哪里，而那里才有具体卡在哪一步。
                    let log_hint = init_log_hint();
                    let _ = init_tx.send(Err(format!("初始化超时（{} 秒）。{}", secs, log_hint)));
                }
            }
        });

        // Enter UI immediately with loading screen, then receive init result
        crate::ui::app::runtime::run_app(
            init_rx,
            initial_message,
            initial_history,
            initial_usage,
            initial_session_id,
        )
        .await
        .map_err(|e| e as Box<dyn std::error::Error>)?;
    }

    Ok(())
}

/// 初始化超时：交互模式与 headless 模式共用，保证两条入口的容错口径一致。
/// 默认 30 秒；排查慢启动时可设 `STAR_INIT_TIMEOUT_SECS` 临时放宽。
fn init_timeout_duration() -> tokio::time::Duration {
    const DEFAULT_SECS: u64 = 30;
    let secs = std::env::var("STAR_INIT_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v >= 5)
        .unwrap_or(DEFAULT_SECS);
    tokio::time::Duration::from_secs(secs)
}

/// 给用户的日志位置提示。日志被 STAR_LOG_ENABLED=0 关掉时不能指过去——
/// 文件里什么都没有，指了也查不到东西。
fn init_log_hint() -> String {
    if utils::logging::is_log_enabled() {
        format!("详见 {}", utils::logging::agent_log_path_display())
    } else {
        "日志已被 STAR_LOG_ENABLED=0 关闭，设置 STAR_LOG_ENABLED=1 后重试可获取详细信息".to_string()
    }
}

fn parse_cli_approval_mode(
    input: &str,
) -> Result<crate::core::policy::types::ApprovalMode, String> {
    // 别名口径和 `permissions.defaultMode`、`/permissions <mode>` 共用一份。
    crate::core::policy::approval_mode_from_str(input).ok_or_else(|| {
        format!(
            "invalid --permission-mode `{}` (expected: default|plan|yolo|acceptEdits|bypassPermissions)",
            input
        )
    })
}

/// Headless mode: process a single prompt and output results
/// headless 的 `--output-format stream-json` 执行路径。
///
/// # 输出契约
///
/// 每行一个 JSON 对象：
///
/// - **事件行** —— agent 的完整过程。`type` 取值见
///   `agent::messaging::events::AgentEvent` 的手写序列化器：`text_delta` /
///   `reasoning_delta` / `trace` / `tool_started` / `tool_finished` /
///   `tool_progress` / `message` / `error` / `turn_finished` / `done` /
///   `stats_update`。
/// - **`result` 行** —— 终态，**总是最后一行**。带 `usage`（token 用量）、
///   `cost_usd`（估算费用）、`is_error`、`duration_ms`、`session_id`。
///   外部 harness 靠这一行判断"跑完了没有"——它不需要猜"最后一个事件是不是
///   结束"。
///
/// # 为什么要有 `result` 行
///
/// `jsonl` 格式只输出最终助手正文，外部拿不到 token 数、成本，也分不清
/// "正常结束"和"跑一半崩了"。终态行把这三件事一次说清。
///
/// # 为什么事件要**流式**输出
///
/// agent 的工具调用可能持续几十秒。等整轮跑完再一次性吐出来，外部 harness
/// 就没法实时展示进度 —— 那和没有事件流没区别。
async fn run_headless_stream_json(
    agent: &mut agent::StarAgent,
    prompt: &str,
    resumed_session_id: Option<&str>,
    snapshot_history: &mut Vec<ChatEntry>,
    restored_usage: Option<crate::types::StarUsage>,
) -> Result<(), Box<dyn std::error::Error>> {
    use futures::StreamExt;

    let started = std::time::Instant::now();
    // 模型名要在流开始**之前**取走：流持有 `agent` 的可变借用，之后再借
    // `agent`（算费用、取会话快照）就冲突了。
    let model_name = agent.get_client().model.clone();

    // 用 `StarAgent` 的**合并流**（`StreamingChunk`），不是 `Agent` 的
    // `AgentEvent` 流 —— 后者只有 Branch A，缺实时文本增量。
    //
    // agent 内部有两条并行分支：Branch A 是 `AgentEvent`（工具轨迹、错误、
    // 最终消息），Branch B 是 `StreamingChunk` 直接流（逐字文本、思考增量）。
    // UI 消费的是 `select()` 合并后的结果，headless 必须消费同一个合并流，
    // 否则外部拿到的只有工具轨迹、没有模型说了什么。
    // 起一个后台 inbox 消费器 —— headless 没有 UI 事件循环，TUI 那边的
    // `spawn_schedule_pollers` 在这里不存在。外部 harness 往 inbox 写的
    // 消息、批准的工具、取消请求，都由它处理。
    //
    // 顺带把总线标成"有人能回答"：没有它，等待层会认为没有订阅者而直接
    // 拒绝工具确认，外部 harness 的批准就永远送不到。
    let abort_flag = agent.abort_handle();
    let bus = agent.runtime_message_bus();
    if let Some(bus) = bus.as_ref() {
        bus.set_external_responder(true);
    }
    let remote_session_id = resumed_session_id.map(str::to_string);
    // 消费器**不后台跑** —— headless 是一次性执行的，后台任务活不过主流程
    // （主流程 5 秒就结束，任务随之被取消，外部消息永远等不到响应）。
    //
    // 所以改成：主流程跑完后，**同步**再收几轮 inbox。外部 harness 的
    // 「发消息 / 批准工具 / 取消」在这个窗口里生效。
    let remote_ctx = HeadlessRemoteContext {
        cwd: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
        abort_flag,
        bus,
        session_id: remote_session_id,
    };

    let mut stream = agent.process_user_message_streaming(prompt);
    let mut final_response = String::new();
    let mut latest_usage: Option<crate::types::StarUsage> = None;
    let mut entries: Vec<ChatEntry> = Vec::new();
    let mut is_error = false;
    let mut error_detail: Option<String> = None;
    // 从增量累积的助手正文（纯文本回答没有 `Message` 事件）。
    let mut streamed_text = String::new();

    while let Some(chunk_result) = stream.next().await {
        match chunk_result {
            Ok(event) => {
                match &event {
                    crate::agent::messaging::AgentEvent::Message(content) => {
                        final_response = content.clone();
                    }
                    // 纯文本回答没有 `Message` 事件 —— 那个只在有工具调用时发。
                    // 正文要从 `TextDelta` 累积。
                    crate::agent::messaging::AgentEvent::TextDelta(content) => {
                        streamed_text.push_str(content);
                    }
                    crate::agent::messaging::AgentEvent::StatsUpdate { token_usage } => {
                        if token_usage.is_some() {
                            latest_usage = token_usage.clone();
                        }
                    }
                    crate::agent::messaging::AgentEvent::Error(err) => {
                        is_error = true;
                        error_detail = Some(err.clone());
                    }
                    _ => {}
                }
                println!("{}", serde_json::to_string(&event)?);
            }
            Err(err) => {
                is_error = true;
                error_detail = Some(err.to_string());
                let payload = serde_json::json!({
                    "type": "error",
                    "content": err.to_string(),
                });
                println!("{}", payload);
                break;
            }
        }
    }

    // 流已经跑完，显式放掉它对 `agent` 的可变借用。
    drop(stream);

    // `Message` 事件（有工具调用时）优先；否则用增量累积出来的正文。
    if final_response.trim().is_empty() {
        final_response = streamed_text.clone();
    }
    if !final_response.trim().is_empty() {
        entries.push(ChatEntry::assistant(final_response.clone()));
    }

    let usage = latest_usage.or(restored_usage);
    let cost_usd = usage
        .as_ref()
        .map(|u| crate::ui::utils::cost::compute_response_cost(u, &model_name));

    // 会话落盘：headless 之前只有 `--resume` 时才写。
    //
    // 现在默认**也**写 —— 外部 harness 跑完一次就能 `--resume` 接着聊，或者事后
    // 读回完整对话。这是外部 harness 场景下最基本的能力。
    //
    // 但它有个副作用：headless 的会话会混进用户的会话列表，`/continue` 可能
    // 捡到一次 headless 运行。所以**新建**的 headless 会话默认不落盘，需要
    // 显式开 `STAR_HEADLESS_SESSION=1`；`--resume` 指定的会话照旧写回
    // （那是用户点名要续的）。
    let persist_new_session = std::env::var("STAR_HEADLESS_SESSION")
        .ok()
        .map(|v| {
            let v = v.trim().to_lowercase();
            !(v == "0" || v == "false" || v == "off")
        })
        .unwrap_or(false);

    let session_id = resumed_session_id.map(str::to_string).or_else(|| {
        persist_new_session.then(|| format!("headless-{}", chrono::Local::now().timestamp()))
    });
    if let Some(id) = session_id.as_deref() {
        snapshot_history.push(ChatEntry::user(prompt.to_string()));
        snapshot_history.extend(entries.iter().cloned());
        let (messages, pending_local_context) = agent.session_context_snapshot();
        let _ = crate::utils::session_manager::save_session_snapshot(
            id,
            snapshot_history,
            Some(messages),
            Some(pending_local_context),
            usage.clone(),
        )
        .await;
    }

    // 主流程跑完后再收几轮 inbox —— 外部 harness 批准工具 / 追加消息的窗口。
    // 必须**在** result 行之前：result 是终态信号，harness 看到它就知道
    // "跑完了"，之后的响应就没意义了。
    // 远程消息的用量要**替换**掉主流程的用量 —— result 是终态信号，
    // 它必须反映"这一整次 headless 运行"的真实消耗，而不是主 prompt 那一轮。
    let remote_usage = drain_headless_remote_window(&remote_ctx, agent).await;
    if let Some(remote) = remote_usage {
        latest_usage = Some(remote);
    }

    let mut result = serde_json::json!({
        "type": "result",
        "is_error": is_error,
        "duration_ms": started.elapsed().as_millis() as u64,
        "usage": usage,
        "cost_usd": cost_usd,
        "session_id": session_id,
    });
    if let Some(detail) = error_detail {
        result["error"] = serde_json::Value::String(detail);
    }
    println!("{}", result);

    Ok(())
}

/// headless 模式下的远程 inbox 消费上下文。
struct HeadlessRemoteContext {
    cwd: std::path::PathBuf,
    abort_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    bus: Option<std::sync::Arc<crate::core::confirmation_bus::MessageBus>>,
    session_id: Option<String>,
}

/// headless 模式下的远程 inbox 消费。
///
/// # 为什么 headless 需要自己处理
///
/// TUI 那边有 `runtime::background::spawn_schedule_pollers` 在轮询 inbox，
/// 但它只在**UI 事件循环**里启动（`ui/app/runtime.rs:653`）。headless 走
/// 的是另一条路，压根没有那个循环 —— 所以外部 harness 往 inbox 写消息，
/// headless 永远看不见。
///
/// # 为什么是**同步**收尾而不是后台任务
///
/// headless 是一次性执行的：主流程跑完进程就退出。后台任务活不过主流程
/// （主流程 5 秒结束，任务随之被取消，外部消息永远等不到响应）。
///
/// 所以改成：主流程跑完后，**同步**再收几轮 inbox。外部 harness 的
/// 「发消息 / 批准工具 / 取消」在这个窗口里生效。
///
/// # 四个动作
///
/// - `send_message` —— 用独立 agent 跑一轮，结果写进 session。
/// - `approve_tool` —— 往消息总线发 `ToolConfirmationResponse`（主 agent
///   正在等的那个确认）。
/// - `cancel` —— 置位主 agent 的 abort 标志。
/// - `ping` —— 回一条 `pong`。
async fn drain_headless_remote_once(
    ctx: &HeadlessRemoteContext,
    agent: &mut agent::StarAgent,
) -> (usize, Option<crate::types::StarUsage>) {
    use crate::core::remote::{
        RemoteResponse, ACTION_APPROVE_TOOL, ACTION_CANCEL, ACTION_PING, ACTION_SEND_MESSAGE,
    };
    use std::sync::atomic::Ordering;

    let drained = match crate::core::remote::drain_requests(&ctx.cwd).await {
        Ok(drained) => drained,
        Err(err) => {
            crate::utils::logging::append_debug_log_line(&format!(
                "[REMOTE] headless poll failed: {err}"
            ));
            return (0, None);
        }
    };

    let count = drained.accepted.len();
    let mut usage: Option<crate::types::StarUsage> = None;
    if count > 0 || !drained.rejected.is_empty() {
        crate::utils::logging::append_debug_log_line(&format!(
            "[REMOTE] headless poll: {count} accepted, {} rejected",
            drained.rejected.len()
        ));
    }

    for req in drained.accepted {
        let response = match req.action.as_str() {
            ACTION_PING => {
                RemoteResponse::ok(&req.protocol, req.request_id.clone(), ACTION_PING, "pong")
            }
            ACTION_CANCEL => {
                ctx.abort_flag.store(true, Ordering::SeqCst);
                RemoteResponse::ok(
                    &req.protocol,
                    req.request_id.clone(),
                    ACTION_CANCEL,
                    "已请求中止",
                )
            }
            ACTION_APPROVE_TOOL => {
                let tool_call_id = req.message.trim().to_string();
                let approved = req.approved.unwrap_or(true);
                match approve_via_bus(ctx.bus.as_ref(), &tool_call_id, approved).await {
                    Ok(detail) => RemoteResponse::ok(
                        &req.protocol,
                        req.request_id.clone(),
                        ACTION_APPROVE_TOOL,
                        detail,
                    ),
                    Err(detail) => RemoteResponse::error(
                        &req.protocol,
                        req.request_id.clone(),
                        ACTION_APPROVE_TOOL,
                        detail,
                    ),
                }
            }
            ACTION_SEND_MESSAGE => {
                let (detail, turn_usage) =
                    run_remote_message(agent, ctx.session_id.as_deref(), &req.message).await;
                if turn_usage.is_some() {
                    usage = turn_usage;
                }
                RemoteResponse::ok(
                    &req.protocol,
                    req.request_id.clone(),
                    ACTION_SEND_MESSAGE,
                    detail,
                )
            }
            other => RemoteResponse::error(
                &req.protocol,
                req.request_id.clone(),
                other,
                "headless 执行层不认识这个动作",
            ),
        };
        let _ = crate::core::remote::append_response(&ctx.cwd, &response).await;
    }

    (count, usage)
}

/// 主流程结束后，同步收几轮 inbox。
///
/// 窗口默认 3 秒（3 轮 × 1 秒）。想更长就设 `STAR_REMOTE_DRAIN_SECS`。
///
/// # 为什么要留这个窗口
///
/// 外部 harness 的典型用法是：启动 headless → 往 inbox 写「批准这个工具」
/// → 等 agent 继续。如果 headless 跑完就退出，批准永远送不到。
///
/// 窗口是**尽力而为**的：agent 跑得快时它已经结束了，批准就没有意义了
/// （工具已经执行或已被拒）。要真正等批准，应该用 `--dangerously-skip-permissions`
/// 或者在 TUI 里跑。
async fn drain_headless_remote_window(
    ctx: &HeadlessRemoteContext,
    agent: &mut agent::StarAgent,
) -> Option<crate::types::StarUsage> {
    let rounds = std::env::var("STAR_REMOTE_DRAIN_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(3)
        .min(120);

    if rounds == 0 {
        return None;
    }
    let mut latest_usage: Option<crate::types::StarUsage> = None;
    for _ in 0..rounds {
        let (count, usage) = drain_headless_remote_once(ctx, agent).await;
        if usage.is_some() {
            latest_usage = usage;
        }
        if count == 0 {
            // inbox 空了再等一轮就收工 —— 没有消息时不必耗满整个窗口。
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }
    latest_usage
}

/// 通过消息总线应答一次挂起的工具确认。
async fn approve_via_bus(
    bus: Option<&std::sync::Arc<crate::core::confirmation_bus::MessageBus>>,
    correlation_id: &str,
    approved: bool,
) -> Result<String, String> {
    use crate::core::confirmation_bus::types::{Message, MessageBusType, ToolConfirmationResponse};
    use crate::types::ToolConfirmationOutcome;

    let Some(bus) = bus else {
        return Err("没有消息总线 —— 工具确认无法应答".to_string());
    };
    if correlation_id.is_empty() {
        return Err("缺少 correlation_id".to_string());
    }

    bus.respond_external(Message::ToolConfirmationResponse(
        ToolConfirmationResponse {
            message_type: MessageBusType::ToolConfirmationResponse,
            correlation_id: correlation_id.to_string(),
            confirmed: approved,
            requires_user_confirmation: None,
            outcome: Some(if approved {
                ToolConfirmationOutcome::ProceedOnce
            } else {
                ToolConfirmationOutcome::Cancel
            }),
            feedback: None,
        },
    ))
    .await;

    Ok(if approved { "已批准" } else { "已拒绝" }.to_string())
}

/// 用**主 agent** 跑一轮远程消息，结果写进 session。
///
/// # 为什么复用主 agent 而不是新造一个
///
/// 早先这里新造一个 `StarAgent` —— 但它的 `build_agent_runtime_artifacts` 会
/// 重新注册整套工具（含 MCP 工具），既慢又容易在工具注册阶段卡住。
///
/// 复用主 agent 没有这个问题：`Agent::session_messages` 是持久的，`run_stream`
/// 每次调用都会把 system 消息重建好、把上一轮的历史接上，所以连续调用就是
/// 连续的多轮对话。而且远程消息**本来就应该**接在当前会话后面 ——
/// 外部 harness 发的消息是对话的一部分，不是另起一段。
async fn run_remote_message(
    agent: &mut agent::StarAgent,
    session_id: Option<&str>,
    message: &str,
) -> (String, Option<crate::types::StarUsage>) {
    use futures::StreamExt;

    crate::utils::logging::append_debug_log_line(
        "[REMOTE] running remote message on the main agent",
    );

    let mut stream = agent.process_user_message_streaming(message);
    let mut final_response = String::new();
    let mut streamed_text = String::new();
    let mut entries: Vec<ChatEntry> = Vec::new();
    let mut failed: Option<String> = None;
    let mut usage: Option<crate::types::StarUsage> = None;

    while let Some(event_result) = stream.next().await {
        match event_result {
            Ok(event) => {
                match &event {
                    crate::agent::messaging::AgentEvent::Message(content) => {
                        final_response = content.clone();
                    }
                    crate::agent::messaging::AgentEvent::StatsUpdate { token_usage } => {
                        if token_usage.is_some() {
                            usage = token_usage.clone();
                        }
                    }
                    crate::agent::messaging::AgentEvent::TextDelta(content) => {
                        streamed_text.push_str(content);
                    }
                    crate::agent::messaging::AgentEvent::Error(err) => {
                        failed = Some(err.clone());
                    }
                    _ => {}
                }
                match serde_json::to_string(&event) {
                    Ok(line) => println!("{line}"),
                    // 序列化失败不该让远程消息整条失败 —— 事件本身已经拿到了。
                    Err(err) => {
                        crate::utils::logging::append_debug_log_line(&format!(
                            "[REMOTE] failed to serialize event: {err}"
                        ));
                    }
                }
            }
            Err(err) => {
                failed = Some(err.to_string());
                break;
            }
        }
    }

    if let Some(err) = failed {
        return (format!("处理失败: {err}"), usage);
    }

    if final_response.trim().is_empty() {
        final_response = streamed_text.clone();
    }
    if !final_response.trim().is_empty() {
        entries.push(ChatEntry::assistant(final_response.clone()));
    }

    if let Some(id) = session_id {
        let mut history = vec![ChatEntry::user(message.to_string())];
        history.extend(entries);
        let _ =
            crate::utils::session_manager::save_session_snapshot(id, &history, None, None, None)
                .await;
    }

    let detail = if final_response.trim().is_empty() {
        "已处理（无输出）".to_string()
    } else {
        final_response
    };
    (detail, usage)
}

async fn process_prompt_headless(
    api_key: &str,
    base_url: String,
    model: Option<String>,
    max_tool_rounds: u32,
    is_openai_compatible: Option<bool>,
    provider_id: Option<String>,
    config: Arc<core::config::Config>,
    resumed_session: Option<ResolvedResumeSession>,
    output_format: HeadlessOutputFormat,
    prompt: String, // 直接接收 prompt 参数
) -> Result<(), Box<dyn std::error::Error>> {
    let prompt = prompt.trim();
    let resumed_session_id = resumed_session.as_ref().map(|resolved| resolved.id.clone());
    let mut snapshot_history = resumed_session
        .as_ref()
        .map(|resolved| resolved.session.chat_history.clone())
        .unwrap_or_default();
    let restored_usage = resumed_session
        .as_ref()
        .and_then(|resolved| resolved.session.last_usage.clone());

    let mut agent = agent::StarAgent::new_with_provider(
        api_key,
        model,
        base_url,
        Some(max_tool_rounds),
        is_openai_compatible,
        Some(config),
        provider_id,
    )
    .await
    .map_err(|e| e as Box<dyn std::error::Error>)?;

    if let Some(resolved) = resumed_session.as_ref() {
        let messages = crate::utils::session_manager::agent_messages_for_restore(&resolved.session);
        let pending_local_context = resolved
            .session
            .pending_local_context
            .clone()
            .unwrap_or_default();
        agent.replace_session_context(messages, pending_local_context);
    }

    // MCP 挂了不该让整个 headless 运行失败（用户可能根本没配 MCP），但完全
    // 吞掉会让"工具突然不可用"无法排查——把失败记进 agent.log 并在输出里提示。
    if let Err(e) = agent.initialize_mcp().await {
        let msg = format!(
            "[MCP] headless 初始化失败（继续运行，MCP 工具将不可用）: {}",
            e
        );
        utils::logging::append_agent_log_line(&msg);
        eprintln!("⚠️ MCP 初始化失败（已忽略，MCP 工具不可用）: {}", e);
    }

    // `stream-json` 走自己的路径：边跑边把事件吐出去，而不是等整轮跑完。
    //
    // 这条路径必须**先于**下面的非流式分支分流 —— 非流式那条会把除最终
    // 助手正文以外的事件全丢掉（`Agent::run` 的 `_ => {}`），正是 stream-json
    // 要提供的东西。
    if matches!(output_format, HeadlessOutputFormat::StreamJson) {
        return run_headless_stream_json(
            &mut agent,
            prompt,
            resumed_session_id.as_deref(),
            &mut snapshot_history,
            restored_usage,
        )
        .await;
    }

    // Process the user message
    let (chat_entries, latest_usage) = agent
        .process_user_message_with_usage(prompt)
        .await
        .map_err(|e| e as Box<dyn std::error::Error>)?;

    if let Some(id) = resumed_session_id {
        snapshot_history.push(ChatEntry::user(prompt));
        snapshot_history.extend(chat_entries.iter().cloned());
        let (messages, pending_local_context) = agent.session_context_snapshot();
        crate::utils::session_manager::save_session_snapshot(
            &id,
            &snapshot_history,
            Some(messages),
            Some(pending_local_context),
            latest_usage.or(restored_usage),
        )
        .await
        .map_err(|error| -> Box<dyn std::error::Error> { error })?;
    }

    match output_format {
        HeadlessOutputFormat::Jsonl => {
            for entry in &chat_entries {
                println!("{}", serde_json::to_string(entry)?);
            }
        }
        // `StreamJson` 在上面就分流走了 —— 到这里说明控制流出错了。
        HeadlessOutputFormat::StreamJson => {
            eprintln!("internal error: stream-json should have been handled earlier");
        }
        HeadlessOutputFormat::Text => {
            let mut has_assistant = false;
            for entry in &chat_entries {
                if entry.entry_type == crate::types::ChatEntryType::Assistant
                    && !entry.content.trim().is_empty()
                {
                    if has_assistant {
                        println!();
                    }
                    println!("{}", entry.content);
                    has_assistant = true;
                }
            }

            if !has_assistant {
                for entry in &chat_entries {
                    if !entry.content.trim().is_empty() {
                        println!("{}", entry.content);
                    }
                }
            }
        }
    }

    Ok(())
}
