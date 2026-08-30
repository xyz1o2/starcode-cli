use crate::agent;
use crate::core;
use clap::Subcommand;
use std::sync::Arc;

use crate::types;

#[derive(Subcommand)]
pub enum GitCommand {
    /// Generate AI commit message and push to remote
    CommitAndPush {
        /// Set working directory
        #[arg(short = 'd', long = "directory", default_value = ".")]
        directory: String,

        /// STAR API key (or set STAR_API_KEY env var)
        #[arg(short = 'k', long = "api-key")]
        api_key: Option<String>,

        /// STAR API base URL (or set STAR_BASE_URL env var)
        #[arg(short = 'u', long = "base-url")]
        base_url: Option<String>,

        /// AI model to use
        #[arg(short = 'm', long = "model")]
        model: Option<String>,

        /// Maximum number of tool execution rounds (default: 400)
        #[arg(long = "max-tool-rounds", default_value = "400")]
        max_tool_rounds: u32,
    },
}

pub async fn execute_git_command(command: GitCommand) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        GitCommand::CommitAndPush {
            directory,
            api_key,
            base_url,
            model,
            max_tool_rounds,
        } => {
            if directory != "." {
                std::env::set_current_dir(&directory)?;
            }
            handle_commit_and_push(api_key, base_url, model, max_tool_rounds).await
        }
    }
}

async fn handle_commit_and_push(
    api_key: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
    max_tool_rounds: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let settings_manager = core::config::settings_manager::get_settings_manager()
        .await
        .map_err(|e| e as Box<dyn std::error::Error>)?;
    let settings = settings_manager
        .load_user_settings()
        .await
        .map_err(|e| e as Box<dyn std::error::Error>)?;
    let provider_store = crate::core::config::provider_store::ProviderStore::new();
    let provider_config = provider_store.load().await.ok();
    let effective_provider_id = provider_config
        .as_ref()
        .and_then(|config| config.active_provider_id.clone())
        .filter(|pid| !pid.trim().is_empty())
        .map(|pid| crate::core::config::providers::normalize_provider_id(&pid).unwrap_or(pid));

    // Get API key from args, environment, or settings
    let api_key_env = crate::core::config::providers::normalize_api_key_value(api_key);
    let provider_api_key = provider_config.as_ref().and_then(|config| {
        config.active_provider_id.as_ref().and_then(|active_pid| {
            config.providers.get(active_pid).and_then(|provider| {
                crate::core::config::providers::resolve_runtime_api_key(
                    Some(active_pid),
                    provider.api_key.clone(),
                )
            })
        })
    });
    let api_key = if let Some(key) = api_key_env {
        key
    } else if let Some(key) = provider_api_key {
        key
    } else if let Some(key) =
        crate::core::config::providers::normalize_api_key_value(settings.api_key)
    {
        key
    } else {
        eprintln!(
            "❌ Error: API key required. Set STAR_API_KEY environment variable, use --api-key flag, or set \"apiKey\" field in ~/.star/user-settings.json"
        );
        std::process::exit(1);
    };
    let api_key = if api_key.trim().is_empty() {
        if let Some(provider_id) = effective_provider_id.as_deref() {
            crate::core::config::providers::resolve_runtime_api_key(Some(provider_id), None)
                .unwrap_or(api_key)
        } else {
            crate::core::config::providers::normalize_api_key_value(
                std::env::var("STAR_API_KEY").ok(),
            )
            .unwrap_or(api_key)
        }
    } else {
        api_key
    };

    let mut resolved_base_url = base_url.or_else(|| std::env::var("STAR_BASE_URL").ok());
    if resolved_base_url.is_none() {
        if let Some(ref config) = provider_config {
            if let Some(active_pid) = &config.active_provider_id {
                resolved_base_url = config
                    .providers
                    .get(active_pid)
                    .and_then(|provider| provider.base_url.clone())
                    .filter(|value| !value.trim().is_empty());

                if resolved_base_url.is_none() {
                    resolved_base_url =
                        crate::core::config::providers::get_provider_by_id(active_pid)
                            .and_then(|meta| meta.default_base_url.map(|value| value.to_string()));
                }
            }
        }
    }
    if resolved_base_url.is_none() {
        resolved_base_url = settings.base_url.filter(|value| !value.is_empty());
    }
    if resolved_base_url.is_none() {
        if let Some(provider_id) = effective_provider_id.as_deref() {
            resolved_base_url = crate::core::config::providers::get_provider_by_id(provider_id)
                .and_then(|meta| meta.default_base_url.map(|value| value.to_string()));
        }
    }

    let base_url = match resolved_base_url {
        Some(url) => url,
        None => {
            eprintln!(
                "❌ Error: Base URL required. Set STAR_BASE_URL environment variable, use --base-url flag, or set \"baseUrl\" field in ~/.star/user-settings.json"
            );
            std::process::exit(1);
        }
    };

    let model = model
        .or_else(|| std::env::var("STAR_MODEL").ok())
        .or_else(|| {
            provider_config.as_ref().and_then(|config| {
                config
                    .active_provider_id
                    .as_ref()
                    .and_then(|provider_id| {
                        config
                            .providers
                            .get(provider_id)
                            .and_then(|provider| provider.selected_model.clone())
                    })
                    .or_else(|| config.active_model.clone())
            })
        })
        .or(settings.default_model);


    let is_openai_compatible = effective_provider_id
        .as_deref()
        .and_then(crate::core::config::providers::provider_openai_compatible_mode)
        .or(settings.is_openai_compatible);

    // Create Config for agent
    let cwd = std::env::current_dir()?;
    let config_params = core::config::ConfigParameters {
        session_id: uuid::Uuid::new_v4().to_string(),
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
        approval_mode: None,
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
        model: model
            .clone()
            .unwrap_or_else(|| "star-code-fast-1".to_string()),
        max_session_turns: None,
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

    let mut config = core::config::Config::new(config_params);
    config.initialize().await?;
    let config = Arc::new(config);

    let mut agent = agent::StarAgent::new(
        &api_key,
        model,
        base_url,
        Some(max_tool_rounds),
        is_openai_compatible,
        Some(config),
    )
    .await
    .map_err(|e| e as Box<dyn std::error::Error>)?;

    println!("🤖 Processing commit and push...\n");
    println!("> /commit-and-push\n");

    // Use agent to check git status
    let status_prompt =
        "Check git status and list any uncommitted changes. Run: git status --porcelain";
    let status_entries = agent
        .process_user_message(status_prompt)
        .await
        .map_err(|e| e as Box<dyn std::error::Error>)?;

    let mut has_changes = false;
    for entry in &status_entries {
        if entry.entry_type == types::ChatEntryType::ToolResult
            && !entry.content.trim().is_empty()
            && entry.content.contains(|c: char| !c.is_whitespace())
        {
            has_changes = true;
        }
    }

    if !has_changes {
        println!("❌ No changes to commit. Working directory is clean.");
        std::process::exit(1);
    }

    println!("✅ git status: Changes detected");

    // Stage all changes
    let add_prompt = "Stage all changes with: git add .";
    let _add_entries = agent
        .process_user_message(add_prompt)
        .await
        .map_err(|e| e as Box<dyn std::error::Error>)?;
    println!("✅ git add: Changes staged");

    // Get diff for commit message generation
    let diff_prompt = "Show the staged changes with: git diff --cached";
    let diff_entries = agent
        .process_user_message(diff_prompt)
        .await
        .map_err(|e| e as Box<dyn std::error::Error>)?;

    let mut diff_content = String::new();
    for entry in &diff_entries {
        if entry.entry_type == types::ChatEntryType::ToolResult {
            diff_content = entry.content.clone();
            break;
        }
    }

    // Generate commit message using AI
    let commit_prompt = format!(
        "Based on these git changes:\n{}\n\nGenerate a concise, professional git commit message following conventional commit format (feat:, fix:, docs:, etc.) and keep it under 72 characters. Respond with ONLY the commit message, no additional text.",
        diff_content
    );

    println!("🤖 Generating commit message...");

    let chat_entries = agent
        .process_user_message(&commit_prompt)
        .await
        .map_err(|e| e as Box<dyn std::error::Error>)?;
    let mut commit_message = String::new();

    // Extract the commit message from the AI response
    for entry in chat_entries {
        if entry.entry_type == types::ChatEntryType::Assistant && !entry.content.trim().is_empty() {
            commit_message = entry.content.trim().to_string();
            break;
        }
    }

    if commit_message.is_empty() {
        println!("❌ Failed to generate commit message");
        std::process::exit(1);
    }

    // Clean the commit message
    let clean_commit_message = commit_message.trim_matches(|c| c == '"' || c == '\'');
    println!("✅ Generated commit message: \"{}\"", clean_commit_message);

    // Execute the commit
    let commit_command = format!(
        "git commit -m \"{}\"",
        clean_commit_message.replace('"', "\\\"")
    );
    let commit_prompt = format!("Execute this command: {}", commit_command);
    let commit_entries = agent
        .process_user_message(&commit_prompt)
        .await
        .map_err(|e| e as Box<dyn std::error::Error>)?;

    let mut commit_success = false;
    for entry in &commit_entries {
        if entry.entry_type == types::ChatEntryType::ToolResult
            && !entry.content.contains("error")
            && !entry.content.contains("Error")
        {
            commit_success = true;
            println!("✅ git commit: Commit created successfully");
            break;
        }
    }

    if !commit_success {
        println!("❌ git commit: Failed to create commit");
        std::process::exit(1);
    }

    // Try to push
    let push_prompt = "Push to remote with: git push. If it fails with 'no upstream branch', use: git push -u origin HEAD";
    let push_entries = agent
        .process_user_message(push_prompt)
        .await
        .map_err(|e| e as Box<dyn std::error::Error>)?;

    let mut push_success = false;
    for entry in &push_entries {
        if entry.entry_type == types::ChatEntryType::ToolResult
            && !entry.content.contains("error")
            && !entry.content.contains("Error")
        {
            push_success = true;
            println!("✅ git push: Push successful");
            break;
        }
    }

    if !push_success {
        println!("❌ git push: Push failed");
        std::process::exit(1);
    }

    Ok(())
}
