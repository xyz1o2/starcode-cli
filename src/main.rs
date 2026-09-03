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
    /// Run the eval harness (mechanism layer and/or live tasks)
    Eval {
        /// Path to the tasks JSON file
        #[arg(long, default_value = "eval/tasks.json")]
        tasks: String,
        /// Path to write the JSON report to
        #[arg(long, default_value = ".star/eval-results.json")]
        out: String,
        /// Path to write a markdown report to (optional)
        #[arg(long)]
        report_md: Option<String>,
        /// Number of trials per task
        #[arg(long, default_value = "1")]
        trials: usize,
        /// Save baseline to path (optional, defaults to .star/eval-baseline.json)
        #[arg(long)]
        save_baseline: Option<String>,
        /// Compare against a baseline file (optional)
        #[arg(long)]
        baseline: Option<String>,
    },
}

#[derive(Debug, Clone, ValueEnum)]
enum HeadlessOutputFormat {
    Jsonl,
    Text,
}

#[derive(Parser)]
#[command(name = "starcode-cli")]
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

    #[command(subcommand)]
    command: Option<Commands>,
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

    let args = CliArgs::parse();

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
        Some(Commands::Eval {
            tasks,
            out,
            report_md,
            trials,
            save_baseline,
            baseline,
        }) => {
            let cwd = std::env::current_dir()?;
            let tasks_path = cwd.join(tasks);
            let out_path = cwd.join(out);
            let report =
                crate::agent::eval_harness::run_eval(&tasks_path, &out_path, trials).await?;

            let mut lines = vec![format!(
                "Eval: {} total, {} passed, {} failed ({:.1}% pass)",
                report.summary.total,
                report.summary.passed,
                report.summary.failed,
                report.summary.pass_rate * 100.0
            )];

            if let Some(live) = &report.live_results {
                let passed = live.iter().filter(|r| r.passed).count();
                let skipped = live.iter().filter(|r| !r.executed).count();
                lines.push(format!(
                    "Live: {}/{} passed, {} skipped",
                    passed,
                    live.len(),
                    skipped
                ));
                for r in live.iter().filter(|r| r.executed && !r.passed) {
                    lines.push(format!("  ❌ {} — {}", r.id, r.failed_rules.join(", ")));
                }
            }

            if let Some(md) = &report_md {
                let md_path = cwd.join(md);
                if let Some(parent) = md_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let md_text = crate::agent::eval_harness::eval_report_to_markdown(&report);
                std::fs::write(&md_path, md_text)?;
                lines.push(format!("Report: {}", md_path.display()));
            }

            if let Some(bl) = &baseline {
                let bl_path = cwd.join(bl);
                match crate::agent::eval_harness::compare_baseline(&report, &bl_path).await {
                    Ok(deltas) if deltas.is_empty() => {
                        lines.push("Baseline: no regressions.".to_string())
                    }
                    Ok(deltas) => {
                        let changes: Vec<String> = deltas
                            .iter()
                            .map(|d| {
                                let tag = match &d.change {
                                    crate::agent::eval_harness::BaselineChange::Regression {
                                        ..
                                    } => "REGRESSION",
                                    crate::agent::eval_harness::BaselineChange::Removed => {
                                        "REMOVED"
                                    }
                                    crate::agent::eval_harness::BaselineChange::New => "NEW",
                                };
                                format!("{}({})", d.task_id, tag)
                            })
                            .collect();
                        lines.push(format!(
                            "Baseline: {} changes — {}",
                            deltas.len(),
                            changes.join(", ")
                        ));
                    }
                    Err(e) => lines.push(format!("Baseline compare failed: {e}")),
                }
            }

            if let Some(sb) = &save_baseline {
                let sb_path = cwd.join(sb);
                match crate::agent::eval_harness::save_baseline(&report, &sb_path).await {
                    Ok(_) => lines.push(format!("Baseline saved: {}", sb_path.display())),
                    Err(e) => lines.push(format!("Baseline save failed: {e}")),
                }
            }

            println!("{}", lines.join("\n"));
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
        None
    };

    // 创建 Config
    // When resuming, reuse the same session ID so auto-save on exit overwrites the same file.
    // --resume (no value) → load latest session
    // --resume auto-xxx → load specified session
    // --resume xxx → load specified session (with auto- prefix if needed)
    let (session_id, resume_session_id) = if let Some(ref id) = args.resume {
        let lookup_id = if id == "latest" {
            // Load latest session - will be resolved later
            "latest".to_string()
        } else {
            // Use the ID as-is (user may pass with or without auto- prefix)
            id.clone()
        };
        // For Config.session_id, strip auto- prefix to get the base UUID
        let base_id = lookup_id.strip_prefix("auto-").unwrap_or(&lookup_id);
        (base_id.to_string(), Some(lookup_id))
    } else {
        (uuid::Uuid::new_v4().to_string(), None)
    };
    let config_params = core::config::ConfigParameters {
        session_id,
        resume_session: resume_session_id.is_some(),
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
        file_filtering: None,
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
        context_window: None,
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
    };

    if let Some(prompt) = args.prompt {
        // Headless mode: initialize config synchronously, then process prompt
        let mut config = core::config::Config::new(config_params);
        config.initialize().await.map_err(|e| {
            utils::logging::append_agent_log_line(&format!("Config 初始化失败: {}", e));
            eprintln!("❌ Config 初始化失败: {}", e);
            e as Box<dyn std::error::Error>
        })?;
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
            config.clone(),
            args.output_format.clone(),
            prompt,
        )
        .await
        .map_err(|e| e as Box<dyn std::error::Error>)?;
    } else {
        // Interactive mode: initialize terminal first, then agent in background
        let initial_message = args.message.join(" ");
        let initial_history = if let Some(ref lookup_id) = resume_session_id {
            let resolved_id = if lookup_id == "latest" {
                // Load the latest session
                match crate::utils::session_manager::read_latest_session_id().await {
                    Ok(Some(id)) => id,
                    Ok(None) => {
                        eprintln!("No sessions found. Start a new session first.");
                        std::process::exit(1);
                    }
                    Err(e) => {
                        eprintln!("Failed to read latest session: {}", e);
                        std::process::exit(1);
                    }
                }
            } else {
                lookup_id.clone()
            };
            match crate::utils::session_manager::load_session(&resolved_id).await {
                Ok(session) => {
                    eprintln!("Resumed session: {}", resolved_id);
                    session.chat_history
                }
                Err(e) => {
                    // Try with auto- prefix if not already present
                    let alt_id = if resolved_id.starts_with("auto-") {
                        resolved_id.clone()
                    } else {
                        format!("auto-{}", resolved_id)
                    };
                    match crate::utils::session_manager::load_session(&alt_id).await {
                        Ok(session) => {
                            eprintln!("Resumed session: {}", alt_id);
                            session.chat_history
                        }
                        Err(_) => {
                            eprintln!("Failed to load session '{}': {}", resolved_id, e);
                            eprintln!("\nAvailable sessions:");
                            match crate::utils::session_manager::list_session_summaries().await {
                                Ok(summaries) => {
                                    if summaries.is_empty() {
                                        eprintln!("  (none)");
                                    } else {
                                        for s in summaries.iter().take(10) {
                                            eprintln!("  {} - {}", s.id, s.title);
                                        }
                                        if summaries.len() > 10 {
                                            eprintln!("  ... and {} more", summaries.len() - 10);
                                        }
                                    }
                                }
                                Err(_) => eprintln!("  (failed to list sessions)"),
                            }
                            std::process::exit(1);
                        }
                    }
                }
            }
        } else {
            Vec::new()
        };

        // Spawn heavy init as a background task with timeout
        let (init_tx, init_rx) = tokio::sync::oneshot::channel::<
            Result<(agent::StarAgent, Arc<core::config::Config>), String>,
        >();
        let api_key2 = api_key.clone();
        let model2 = model.clone();
        let base_url2 = base_url.clone();
        let config_params2 = config_params.clone();
        let _cwd2 = cwd_clone;
        tokio::spawn(async move {
            // Overall timeout for initialization (30 seconds - reduced from 60)
            let init_timeout = tokio::time::Duration::from_secs(30);

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
                agent::StarAgent::new(
                    &api_key2,
                    model2,
                    base_url2,
                    Some(args.max_tool_rounds),
                    is_openai_compatible,
                    Some(config.clone()),
                )
                .await
                .map(|agent| {
                    utils::logging::append_agent_log_line("[INIT] StarAgent::new completed");
                    (agent, config)
                })
                .map_err(|e| {
                    utils::logging::append_agent_log_line(&format!(
                        "[INIT] StarAgent::new failed: {}",
                        e
                    ));
                    format!("Agent 初始化失败: {}", e)
                })
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
                    let _ = init_tx.send(Err(e));
                }
                Err(_) => {
                    utils::logging::append_agent_log_line(
                        "[INIT] Initialization timed out (30 seconds)",
                    );
                    let _ = init_tx.send(Err("初始化超时（30秒）".to_string()));
                }
            }
        });

        // Enter UI immediately with loading screen, then receive init result
        crate::ui::app::runtime::run_app(init_rx, initial_message, initial_history)
            .await
            .map_err(|e| e as Box<dyn std::error::Error>)?;
    }

    Ok(())
}

fn parse_cli_approval_mode(
    input: &str,
) -> Result<crate::core::policy::types::ApprovalMode, String> {
    let normalized = input.trim().to_lowercase();
    match normalized.as_str() {
        "default" | "acceptedits" => Ok(crate::core::policy::types::ApprovalMode::Default),
        "plan" | "readonly" => Ok(crate::core::policy::types::ApprovalMode::Plan),
        "yolo" | "bypasspermissions" => Ok(crate::core::policy::types::ApprovalMode::Yolo),
        _ => Err(format!(
            "invalid --permission-mode `{}` (expected: default|plan|yolo|acceptEdits|bypassPermissions)",
            input
        )),
    }
}

/// Headless mode: process a single prompt and output results
async fn process_prompt_headless(
    api_key: &str,
    base_url: String,
    model: Option<String>,
    max_tool_rounds: u32,
    is_openai_compatible: Option<bool>,
    config: Arc<core::config::Config>,
    output_format: HeadlessOutputFormat,
    prompt: String, // 直接接收 prompt 参数
) -> Result<(), Box<dyn std::error::Error>> {
    let prompt = prompt.trim();

    let mut agent = agent::StarAgent::new(
        api_key,
        model,
        base_url,
        Some(max_tool_rounds),
        is_openai_compatible,
        Some(config),
    )
    .await
    .map_err(|e| e as Box<dyn std::error::Error>)?;

    // Initialize MCP servers for headless mode (non-fatal)
    let _ = agent.initialize_mcp().await;

    // Process the user message
    let chat_entries = agent
        .process_user_message(prompt)
        .await
        .map_err(|e| e as Box<dyn std::error::Error>)?;

    match output_format {
        HeadlessOutputFormat::Jsonl => {
            for entry in &chat_entries {
                println!("{}", serde_json::to_string(entry)?);
            }
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
