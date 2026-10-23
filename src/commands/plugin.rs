use crate::commands::execution::{CommandContext, CommandResult};
use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum PluginCommand {
    /// List installed plugins
    List,
    /// Inspect a plugin in detail
    #[command(arg_required_else_help = true)]
    Inspect {
        /// Plugin name
        name: String,
    },
    /// Validate plugin runtime manifests
    Validate {
        /// Optional plugin name
        name: Option<String>,
    },
    /// Install plugin from local path or git source
    #[command(arg_required_else_help = true)]
    Install {
        /// Source path / git URL / GitHub shorthand (owner/repo)
        source: String,
        /// Optional plugin name override
        #[arg(long)]
        name: Option<String>,
        /// Git branch/tag/commit (only for git sources)
        #[arg(long = "ref")]
        git_ref: Option<String>,
    },
    /// Remove installed plugin
    #[command(arg_required_else_help = true)]
    Remove {
        /// Plugin name
        name: String,
    },
    /// Enable an installed plugin
    #[command(arg_required_else_help = true)]
    Enable {
        /// Plugin name
        name: String,
    },
    /// Disable an installed plugin
    #[command(arg_required_else_help = true)]
    Disable {
        /// Plugin name
        name: String,
    },
    /// Update installed plugin(s) to latest version
    Update {
        /// Plugin name to update; omit to update all installed plugins
        name: Option<String>,
    },
    /// Run a plugin slash command explicitly
    #[command(arg_required_else_help = true)]
    Run {
        /// Command name (as registered by the plugin, without leading slash)
        command: String,
        /// Arguments to pass to the command
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// List registered hooks from all plugins (or one specific plugin)
    Hooks {
        /// Optional plugin name filter
        name: Option<String>,
    },
    /// List plugin-provided slash commands
    Commands {
        /// Optional plugin name filter
        name: Option<String>,
    },
    /// List plugin-provided agent tools
    Tools {
        /// Optional plugin name filter
        name: Option<String>,
    },
}

pub async fn execute_plugin_command(ctx: CommandContext<'_>, cmd: PluginCommand) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;

    match cmd {
        PluginCommand::List => {
            let plugins = crate::core::plugins::resolve_installed_plugins(&cwd)
                .await
                .map_err(|e| format!("failed to list plugins: {}", e))?;

            if plugins.is_empty() {
                ctx.state.chat_history.push(
                    crate::types::ChatEntry::assistant("No plugins installed.".to_string())
                        .with_streaming(false),
                );
                return Ok(());
            }

            let mut lines = vec!["# Plugins\n".to_string()];
            for p in plugins {
                lines.push(format!(
                    "- `{}` [{}]{}\n  - source: {}\n  - installed_at: {}\n  - runtime: {}",
                    p.entry.name,
                    p.entry.install_type,
                    if p.entry.enabled { "" } else { " (disabled)" },
                    p.entry.source,
                    p.entry.installed_at,
                    describe_runtime_status(&p)
                ));
                if let Some(path) = p.manifest_path.as_ref() {
                    lines.push(format!("  - manifest: {}", path.display()));
                }
                if let Some(runtime) = p.runtime_manifest.as_ref() {
                    if let Some(version) = runtime.version.as_deref().filter(|v| !v.is_empty()) {
                        lines.push(format!("  - version: {}", version));
                    }
                    if let Some(description) = runtime
                        .description
                        .as_deref()
                        .filter(|v| !v.trim().is_empty())
                    {
                        lines.push(format!("  - description: {}", trim_line(description)));
                    }
                    let commands = runtime
                        .commands
                        .iter()
                        .filter(|command| command.enabled != Some(false))
                        .filter(|command| !command.name.trim().is_empty())
                        .map(|command| format!("/{}", command.name))
                        .collect::<Vec<_>>();
                    if !commands.is_empty() {
                        lines.push(format!("  - commands: {}", commands.join(", ")));
                    }
                    let tools = runtime
                        .tools
                        .iter()
                        .filter(|tool| tool.enabled != Some(false))
                        .filter(|tool| !tool.name.trim().is_empty())
                        .map(|tool| tool.name.trim().to_string())
                        .collect::<Vec<_>>();
                    if !tools.is_empty() {
                        lines.push(format!("  - tools: {}", tools.join(", ")));
                    }
                    let init_count = runtime
                        .lifecycle
                        .enabled_stage_count(crate::core::plugins::PluginLifecycleStage::Init);
                    let shutdown_count = runtime
                        .lifecycle
                        .enabled_stage_count(crate::core::plugins::PluginLifecycleStage::Shutdown);
                    if init_count > 0 || shutdown_count > 0 {
                        lines.push(format!(
                            "  - lifecycle: init={}, shutdown={}",
                            init_count, shutdown_count
                        ));
                    }
                }
                if let Some(error) = p.runtime_error.as_deref() {
                    lines.push(format!("  - runtime_error: {}", trim_line(error)));
                }
                if !p.runtime_warnings.is_empty() {
                    lines.push(format!(
                        "  - runtime_warning_count: {}",
                        p.runtime_warnings.len()
                    ));
                    for warning in p.runtime_warnings.iter().take(6) {
                        lines.push(format!("  - warning: {}", trim_line(warning)));
                    }
                    if p.runtime_warnings.len() > 6 {
                        lines.push(format!(
                            "  - warning: ... 还有 {} 条",
                            p.runtime_warnings.len() - 6
                        ));
                    }
                }
            }

            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(lines.join("\n")).with_streaming(false));
            Ok(())
        }
        PluginCommand::Inspect { name } => {
            let plugin = crate::core::plugins::inspect_plugin(&cwd, &name)
                .await
                .map_err(|e| format!("failed to inspect plugin: {}", e))?;

            let msg = match plugin {
                Some(plugin) => render_plugin_detail(&plugin),
                None => format!("⚠️ 未找到插件 `{}`", name),
            };

            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
            Ok(())
        }
        PluginCommand::Validate { name } => {
            let plugins = match name.as_deref() {
                Some(name) => {
                    let plugin = crate::core::plugins::inspect_plugin(&cwd, name)
                        .await
                        .map_err(|e| format!("failed to validate plugin: {}", e))?;
                    match plugin {
                        Some(plugin) => vec![plugin],
                        None => {
                            ctx.state.chat_history.push(
                                crate::types::ChatEntry::assistant(format!(
                                    "⚠️ 未找到插件 `{}`",
                                    name
                                ))
                                .with_streaming(false),
                            );
                            return Ok(());
                        }
                    }
                }
                None => crate::core::plugins::resolve_installed_plugins(&cwd)
                    .await
                    .map_err(|e| format!("failed to validate plugins: {}", e))?,
            };

            if plugins.is_empty() {
                ctx.state.chat_history.push(
                    crate::types::ChatEntry::assistant("No plugins installed.".to_string())
                        .with_streaming(false),
                );
                return Ok(());
            }

            let msg = render_plugin_validation_report(&plugins, name.as_deref());
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
            Ok(())
        }
        PluginCommand::Install {
            source,
            name,
            git_ref,
        } => {
            let source_path = PathBuf::from(&source);
            let is_local_source = source_path.exists();

            let plugin_name = match name {
                Some(n) if !n.trim().is_empty() => crate::core::plugins::normalize_plugin_name(&n),
                _ => {
                    if is_local_source {
                        let stem = source_path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .ok_or("invalid source name")?;
                        crate::core::plugins::normalize_plugin_name(stem)
                    } else {
                        crate::core::plugins::infer_plugin_name_from_git_source(&source)
                            .ok_or("invalid git source name")?
                    }
                }
            };

            if plugin_name.is_empty() {
                return Err("invalid plugin name".to_string());
            }

            let entry = if is_local_source {
                if git_ref.is_some() {
                    return Err("`--ref` 仅适用于 git 源安装".to_string());
                }
                crate::core::plugins::install_plugin_local(
                    &cwd,
                    &source_path,
                    &plugin_name,
                    crate::core::plugins::SCOPE_PROJECT,
                )
                .await
                .map_err(|e| format!("failed to install plugin: {}", e))?
            } else {
                let resolved = crate::core::plugins::resolve_plugin_git_source(&source);
                crate::core::plugins::install_plugin_git(
                    &cwd,
                    &resolved,
                    &plugin_name,
                    git_ref.as_deref(),
                    crate::core::plugins::SCOPE_PROJECT,
                )
                .await
                .map_err(|e| format!("failed to install plugin: {}", e))?
            };

            let inspection = crate::core::plugins::inspect_plugin(&cwd, &entry.name)
                .await
                .map_err(|e| format!("failed to inspect plugin runtime: {}", e))?;

            let msg = format!(
                "✅ 插件安装完成\n\n- name: `{}`\n- type: {}\n- source: {}\n- ref: {}\n- runtime: {}{}{}",
                entry.name,
                entry.install_type,
                entry.source,
                git_ref.unwrap_or_else(|| "-".to_string()),
                match inspection.as_ref() {
                    Some(plugin) => describe_runtime_status(&plugin),
                    None => "unknown".to_string(),
                },
                summarize_lifecycle_result(
                    crate::core::plugins::run_plugin_lifecycle_for_plugin(
                        &cwd,
                        &entry.name,
                        crate::core::plugins::PluginLifecycleStage::Init,
                    )
                    .await
                    .ok()
                    .as_deref(),
                    "init",
                ),
                inspection
                    .as_ref()
                    .map(summarize_runtime_warnings)
                    .unwrap_or_default()
            );
            let _ = ctx
                .agent_tx
                .send(crate::runtime::messages::AgentRequest::PluginToolsRefresh)
                .await;
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
            Ok(())
        }
        PluginCommand::Remove { name } => {
            let shutdown_results = crate::core::plugins::run_plugin_lifecycle_for_plugin(
                &cwd,
                &name,
                crate::core::plugins::PluginLifecycleStage::Shutdown,
            )
            .await
            .ok();
            let removed = crate::core::plugins::remove_plugin(&cwd, &name)
                .await
                .map_err(|e| format!("failed to remove plugin: {}", e))?;

            let msg = if removed {
                format!(
                    "✅ 已删除插件 `{}`{}",
                    name,
                    summarize_lifecycle_result(shutdown_results.as_deref(), "shutdown")
                )
            } else {
                format!("⚠️ 未找到插件 `{}`", name)
            };

            if removed {
                let _ = ctx
                    .agent_tx
                    .send(crate::runtime::messages::AgentRequest::PluginToolsRefresh)
                    .await;
            }

            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
            Ok(())
        }
        PluginCommand::Enable { name } => {
            let updated = crate::core::plugins::set_plugin_enabled(&cwd, &name, true)
                .await
                .map_err(|e| format!("failed to enable plugin: {}", e))?;

            let Some(updated) = updated else {
                ctx.state.chat_history.push(
                    crate::types::ChatEntry::assistant(format!("⚠️ 未找到插件 `{}`", name))
                        .with_streaming(false),
                );
                return Ok(());
            };

            let inspection = crate::core::plugins::inspect_plugin(&cwd, &name)
                .await
                .map_err(|e| format!("failed to inspect plugin runtime: {}", e))?;
            let init_results = if updated.changed {
                crate::core::plugins::run_plugin_lifecycle_for_plugin(
                    &cwd,
                    &name,
                    crate::core::plugins::PluginLifecycleStage::Init,
                )
                .await
                .ok()
            } else {
                None
            };

            let msg = if updated.changed {
                format!(
                    "✅ 已启用插件 `{}`\n\n- runtime: {}{}{}",
                    name,
                    inspection
                        .as_ref()
                        .map(describe_runtime_status)
                        .unwrap_or_else(|| "unknown".to_string()),
                    summarize_lifecycle_result(init_results.as_deref(), "init"),
                    inspection
                        .as_ref()
                        .map(summarize_runtime_warnings)
                        .unwrap_or_default()
                )
            } else {
                format!(
                    "ℹ️ 插件 `{}` 已经是启用状态\n\n- runtime: {}{}",
                    name,
                    inspection
                        .as_ref()
                        .map(describe_runtime_status)
                        .unwrap_or_else(|| "unknown".to_string()),
                    inspection
                        .as_ref()
                        .map(summarize_runtime_warnings)
                        .unwrap_or_default()
                )
            };

            let _ = ctx
                .agent_tx
                .send(crate::runtime::messages::AgentRequest::PluginToolsRefresh)
                .await;
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
            Ok(())
        }
        PluginCommand::Update { name } => {
            match name {
                Some(n) => {
                    let result = crate::core::plugins::update_plugin(&cwd, &n)
                        .await
                        .map_err(|e| e.to_string())?;
                    let msg = render_update_result(&result);
                    let _ = ctx
                        .agent_tx
                        .send(crate::runtime::messages::AgentRequest::PluginToolsRefresh)
                        .await;
                    ctx.state
                        .chat_history
                        .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
                }
                None => {
                    let plugins = crate::core::plugins::resolve_installed_plugins(&cwd)
                        .await
                        .map_err(|e| format!("failed to list plugins: {}", e))?;

                    if plugins.is_empty() {
                        ctx.state.chat_history.push(
                            crate::types::ChatEntry::assistant("No plugins installed.".to_string())
                                .with_streaming(false),
                        );
                        return Ok(());
                    }

                    let mut lines = vec!["# Plugin Update\n".to_string()];
                    let mut any_changed = false;
                    for plugin in &plugins {
                        let result = crate::core::plugins::update_plugin(&cwd, &plugin.entry.name)
                            .await
                            .map_err(|e| e.to_string())?;
                        if result.success {
                            any_changed = true;
                        }
                        lines.push(render_update_result(&result));
                    }

                    if any_changed {
                        let _ = ctx
                            .agent_tx
                            .send(crate::runtime::messages::AgentRequest::PluginToolsRefresh)
                            .await;
                    }

                    ctx.state.chat_history.push(
                        crate::types::ChatEntry::assistant(lines.join("\n")).with_streaming(false),
                    );
                }
            }
            Ok(())
        }
        PluginCommand::Run { command, args } => {
            let result =
                crate::core::plugins::execute_plugin_command(&cwd, &command, &args).await?;

            let msg = match result {
                None => format!("⚠️ 未找到插件命令 `{}`", command),
                Some(result) => {
                    let status = if result.success { "✅" } else { "❌" };
                    let mut lines = vec![format!(
                        "{} 插件命令 `/{}`（来自 `{}`）{}",
                        status,
                        result.command_name,
                        result.plugin_name,
                        if result.timed_out {
                            " — 已超时".to_string()
                        } else if let Some(code) = result.exit_code {
                            format!(" — exit {}", code)
                        } else {
                            String::new()
                        }
                    )];
                    if !result.stdout.is_empty() {
                        lines.push(String::new());
                        lines.push("```".to_string());
                        lines.push(result.stdout.clone());
                        lines.push("```".to_string());
                    }
                    if !result.stderr.is_empty() {
                        lines.push(String::new());
                        lines.push(format!("stderr: {}", trim_line(&result.stderr)));
                    }
                    lines.join("\n")
                }
            };
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
            Ok(())
        }
        PluginCommand::Hooks { name } => {
            let hooks = crate::core::plugins::discover_plugin_hooks(&cwd)
                .await
                .map_err(|e| format!("failed to discover hooks: {}", e))?;

            let hooks: Vec<_> = match name.as_deref() {
                Some(n) => {
                    let prefix = format!("plugin:{}", n);
                    hooks.into_iter().filter(|h| h.source == prefix).collect()
                }
                None => hooks,
            };

            let msg = if hooks.is_empty() {
                "(no plugin hooks registered)".to_string()
            } else {
                let mut lines = vec!["# Plugin Hooks\n".to_string()];
                for h in &hooks {
                    lines.push(format!(
                        "- `[{}]` {} | source={} | timeout={}s | blocking={}",
                        h.event, h.name, h.source, h.timeout_secs, h.blocking
                    ));
                }
                lines.join("\n")
            };
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
            Ok(())
        }
        PluginCommand::Commands { name } => {
            let commands = crate::core::plugins::discover_plugin_commands(&cwd)
                .await
                .map_err(|e| format!("failed to discover commands: {}", e))?;

            let commands: Vec<_> = match name.as_deref() {
                Some(n) => commands
                    .into_iter()
                    .filter(|c| c.plugin_name == n)
                    .collect(),
                None => commands,
            };

            let msg = if commands.is_empty() {
                "(no plugin commands registered)".to_string()
            } else {
                let mut lines = vec!["# Plugin Commands\n".to_string()];
                for c in &commands {
                    let desc = if c.description.is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", c.description)
                    };
                    lines.push(format!(
                        "- `/{}`{} | source={} | timeout={}s",
                        c.name, desc, c.source, c.timeout_secs
                    ));
                }
                lines.join("\n")
            };
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
            Ok(())
        }
        PluginCommand::Tools { name } => {
            let tools = crate::core::plugins::discover_plugin_tools(&cwd)
                .await
                .map_err(|e| format!("failed to discover tools: {}", e))?;

            let tools: Vec<_> = match name.as_deref() {
                Some(n) => tools.into_iter().filter(|t| t.plugin_name == n).collect(),
                None => tools,
            };

            let msg = if tools.is_empty() {
                "(no plugin tools registered)".to_string()
            } else {
                let mut lines = vec!["# Plugin Tools\n".to_string()];
                for t in &tools {
                    let desc = if t.description.is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", t.description)
                    };
                    lines.push(format!(
                        "- `{}`{} | source={} | permission={}",
                        t.name,
                        desc,
                        t.source,
                        t.required_permission.as_str()
                    ));
                }
                lines.join("\n")
            };
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
            Ok(())
        }
        PluginCommand::Disable { name } => {
            let inspection_before = crate::core::plugins::inspect_plugin(&cwd, &name)
                .await
                .map_err(|e| format!("failed to inspect plugin runtime: {}", e))?;
            let shutdown_results = if inspection_before
                .as_ref()
                .map(|plugin| plugin.entry.enabled)
                .unwrap_or(false)
            {
                crate::core::plugins::run_plugin_lifecycle_for_plugin(
                    &cwd,
                    &name,
                    crate::core::plugins::PluginLifecycleStage::Shutdown,
                )
                .await
                .ok()
            } else {
                None
            };
            let updated = crate::core::plugins::set_plugin_enabled(&cwd, &name, false)
                .await
                .map_err(|e| format!("failed to disable plugin: {}", e))?;

            let Some(updated) = updated else {
                ctx.state.chat_history.push(
                    crate::types::ChatEntry::assistant(format!("⚠️ 未找到插件 `{}`", name))
                        .with_streaming(false),
                );
                return Ok(());
            };

            let msg = if updated.changed {
                format!(
                    "✅ 已禁用插件 `{}`{}",
                    name,
                    summarize_lifecycle_result(shutdown_results.as_deref(), "shutdown")
                )
            } else {
                format!("ℹ️ 插件 `{}` 已经是禁用状态", name)
            };

            let _ = ctx
                .agent_tx
                .send(crate::runtime::messages::AgentRequest::PluginToolsRefresh)
                .await;
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
            Ok(())
        }
    }
}

fn render_update_result(result: &crate::core::plugins::UpdatePluginResult) -> String {
    if result.success {
        let output_part = if result.output.is_empty() {
            String::new()
        } else {
            format!("\n  - output: {}", trim_line(&result.output))
        };
        format!(
            "✅ `{}` [{}] updated{}",
            result.plugin_name, result.install_type, output_part
        )
    } else {
        let error_part = result
            .error
            .as_deref()
            .map(|e| format!("\n  - error: {}", trim_line(e)))
            .unwrap_or_default();
        format!(
            "❌ `{}` [{}] update failed{}",
            result.plugin_name, result.install_type, error_part
        )
    }
}

fn describe_runtime_status(plugin: &crate::core::plugins::ResolvedPlugin) -> String {
    if !plugin.root_exists {
        return "missing plugin directory".to_string();
    }

    if let Some(runtime) = plugin.runtime_manifest.as_ref() {
        let mut summary = format!(
            "manifest detected, {} hook(s), {} command(s), {} tool(s), {} lifecycle command(s)",
            runtime.enabled_hook_count(),
            runtime.enabled_command_count(),
            runtime.enabled_tool_count(),
            runtime.enabled_lifecycle_count()
        );
        if !plugin.runtime_warnings.is_empty() {
            summary.push_str(&format!(", {} warning(s)", plugin.runtime_warnings.len()));
        }
        return summary;
    }

    if plugin.runtime_error.is_some() {
        return "manifest error".to_string();
    }

    "no runtime manifest".to_string()
}

fn render_plugin_detail(plugin: &crate::core::plugins::ResolvedPlugin) -> String {
    let mut lines = vec![format!("# Plugin `{}`", plugin.entry.name), String::new()];

    lines.push("- Summary".to_string());
    lines.push(format!("- enabled: {}", plugin.entry.enabled));
    lines.push(format!("- install_type: {}", plugin.entry.install_type));
    lines.push(format!("- source: {}", trim_line(&plugin.entry.source)));
    lines.push(format!("- installed_at: {}", plugin.entry.installed_at));
    lines.push(format!("- root: {}", plugin.root.display()));
    lines.push(format!("- root_exists: {}", plugin.root_exists));
    lines.push(format!(
        "- manifest: {}",
        plugin
            .manifest_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".to_string())
    ));
    lines.push(format!("- runtime: {}", describe_runtime_status(plugin)));
    lines.push(format!(
        "- runtime_error: {}",
        plugin
            .runtime_error
            .as_deref()
            .map(trim_line)
            .unwrap_or_else(|| "-".to_string())
    ));
    lines.push(format!(
        "- runtime_warning_count: {}",
        plugin.runtime_warnings.len()
    ));

    lines.push(String::new());
    lines.push("## Manifest".to_string());
    if let Some(runtime) = plugin.runtime_manifest.as_ref() {
        lines.push("- detected: true".to_string());
        lines.push(format!(
            "- name: {}",
            optional_display(runtime.name.as_deref())
        ));
        lines.push(format!(
            "- version: {}",
            optional_display(runtime.version.as_deref())
        ));
        lines.push(format!(
            "- description: {}",
            optional_display(runtime.description.as_deref())
        ));

        lines.push(String::new());
        lines.push("## Hooks".to_string());
        let mut hook_lines = Vec::new();
        let mut events = runtime.hooks.iter().collect::<Vec<_>>();
        events.sort_by(|(left, _), (right, _)| left.cmp(right));
        for (event, specs) in events {
            for (index, spec) in specs.iter().enumerate() {
                let (command, name, timeout_secs, blocking, enabled) = match spec {
                    crate::core::plugins::PluginHookSpec::Command(command) => {
                        (command.trim().to_string(), None, 20_u64, false, true)
                    }
                    crate::core::plugins::PluginHookSpec::Detailed(spec) => (
                        spec.command.trim().to_string(),
                        spec.name
                            .as_deref()
                            .map(str::trim)
                            .filter(|name| !name.is_empty())
                            .map(ToOwned::to_owned),
                        spec.timeout.unwrap_or(20).max(1),
                        spec.blocking.unwrap_or(false),
                        spec.enabled != Some(false),
                    ),
                };
                let display_name = name.unwrap_or_else(|| {
                    format!(
                        "plugin:{}:{}:{}",
                        plugin.entry.name,
                        event.trim(),
                        index + 1
                    )
                });
                hook_lines.push(format!(
                    "- [{}] {} | enabled={} | timeout={}s | blocking={} | command={} | source=plugin:{} | working_dir={}",
                    event.trim(),
                    display_name,
                    enabled,
                    timeout_secs,
                    blocking,
                    trim_line(&command),
                    plugin.entry.name,
                    plugin.root.display()
                ));
            }
        }
        if hook_lines.is_empty() {
            lines.push("- none".to_string());
        } else {
            lines.extend(hook_lines);
        }

        lines.push(String::new());
        lines.push("## Commands".to_string());
        if runtime.commands.is_empty() {
            lines.push("- none".to_string());
        } else {
            for (index, command) in runtime.commands.iter().enumerate() {
                let command_name = command.name.trim();
                let display_name = if command_name.is_empty() {
                    format!("<unnamed command #{}>", index + 1)
                } else {
                    format!("/{}", command_name)
                };
                lines.push(format!(
                    "- {} | enabled={} | timeout={}s | description={} | command={} | source=plugin:{} | working_dir={}",
                    display_name,
                    command.enabled != Some(false),
                    command.timeout.unwrap_or(120).max(1),
                    optional_display(Some(command.description.as_str())),
                    trim_line(&command.command),
                    plugin.entry.name,
                    plugin.root.display()
                ));
            }
        }

        lines.push(String::new());
        lines.push("## Tools".to_string());
        if runtime.tools.is_empty() {
            lines.push("- none".to_string());
        } else {
            for (index, tool) in runtime.tools.iter().enumerate() {
                let tool_name = tool.name.trim();
                let display_name = if tool_name.is_empty() {
                    format!("<unnamed tool #{}>", index + 1)
                } else {
                    tool_name.to_string()
                };
                let args = if tool.args.is_empty() {
                    "-".to_string()
                } else {
                    serde_json::to_string(&tool.args).unwrap_or_else(|_| format!("{:?}", tool.args))
                };
                lines.push(format!(
                    "- {} | enabled={} | permission={} | description={} | command={} | args={} | source=plugin:{} | working_dir={}",
                    display_name,
                    tool.enabled != Some(false),
                    tool.required_permission.as_str(),
                    optional_display(Some(tool.description.as_str())),
                    trim_line(&tool.command),
                    args,
                    plugin.entry.name,
                    plugin.root.display()
                ));
            }
        }

        lines.push(String::new());
        lines.push("## Lifecycle".to_string());
        let mut lifecycle_lines = Vec::new();
        for (stage_label, specs) in [
            ("init", runtime.lifecycle.init.iter()),
            ("shutdown", runtime.lifecycle.shutdown.iter()),
        ] {
            for (index, spec) in specs.enumerate() {
                let spec = spec.command_spec();
                let display_name = spec
                    .name
                    .as_deref()
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| {
                        format!("plugin:{}:{}:{}", plugin.entry.name, stage_label, index + 1)
                    });
                lifecycle_lines.push(format!(
                    "- [{}] {} | enabled={} | timeout={}s | command={} | source=plugin:{} | working_dir={}",
                    stage_label,
                    display_name,
                    spec.enabled != Some(false),
                    spec.timeout.unwrap_or(30).max(1),
                    trim_line(&spec.command),
                    plugin.entry.name,
                    plugin.root.display()
                ));
            }
        }
        if lifecycle_lines.is_empty() {
            lines.push("- none".to_string());
        } else {
            lines.extend(lifecycle_lines);
        }
    } else {
        lines.push("- detected: false".to_string());
    }

    lines.push(String::new());
    lines.push("## Warnings".to_string());
    if plugin.runtime_warnings.is_empty() {
        lines.push("- none".to_string());
    } else {
        for warning in &plugin.runtime_warnings {
            lines.push(format!("- {}", trim_line(warning)));
        }
    }

    lines.join("\n")
}

fn render_plugin_validation_report(
    plugins: &[crate::core::plugins::ResolvedPlugin],
    selected_name: Option<&str>,
) -> String {
    let plugins_with_errors = plugins
        .iter()
        .filter(|plugin| plugin.runtime_error.is_some())
        .count();
    let plugins_with_warnings = plugins
        .iter()
        .filter(|plugin| !plugin.runtime_warnings.is_empty())
        .count();
    let total_warnings = plugins
        .iter()
        .map(|plugin| plugin.runtime_warnings.len())
        .sum::<usize>();
    let healthy_plugins = plugins
        .iter()
        .filter(|plugin| plugin.runtime_error.is_none() && plugin.runtime_warnings.is_empty())
        .count();
    let disabled_plugins = plugins
        .iter()
        .filter(|plugin| !plugin.entry.enabled)
        .count();

    let mut lines = vec![match selected_name {
        Some(name) => format!("# Plugin Validation `{}`", name),
        None => "# Plugin Validation".to_string(),
    }];
    lines.push(String::new());
    lines.push("- Summary".to_string());
    lines.push(format!("- plugins: {}", plugins.len()));
    lines.push(format!("- healthy: {}", healthy_plugins));
    lines.push(format!("- plugins_with_errors: {}", plugins_with_errors));
    lines.push(format!(
        "- plugins_with_warnings: {}",
        plugins_with_warnings
    ));
    lines.push(format!("- total_warnings: {}", total_warnings));
    lines.push(format!("- disabled: {}", disabled_plugins));

    let problem_plugins = plugins
        .iter()
        .filter(|plugin| plugin.runtime_error.is_some() || !plugin.runtime_warnings.is_empty())
        .collect::<Vec<_>>();

    lines.push(String::new());
    lines.push("## Result".to_string());
    if problem_plugins.is_empty() {
        lines.push("- 未发现 runtime 问题。".to_string());
        return lines.join("\n");
    }

    lines.push("- 发现 runtime 问题，建议优先处理 `error`，再处理 `warning`。".to_string());
    lines.push("- 可继续运行 `/plugin inspect <name>` 查看单插件详细结构。".to_string());

    lines.push(String::new());
    lines.push("## Problems".to_string());
    for plugin in problem_plugins {
        let status = if plugin.runtime_error.is_some() {
            "error"
        } else {
            "warning"
        };
        lines.push(format!(
            "- `{}` | status={} | enabled={} | runtime={}",
            plugin.entry.name,
            status,
            plugin.entry.enabled,
            describe_runtime_status(plugin)
        ));
        if let Some(error) = plugin.runtime_error.as_deref() {
            lines.push(format!("  - error: {}", trim_line(error)));
        }
        for warning in &plugin.runtime_warnings {
            lines.push(format!("  - warning: {}", trim_line(warning)));
        }
    }

    lines.join("\n")
}

fn optional_display(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(trim_line)
        .unwrap_or_else(|| "-".to_string())
}

fn trim_line(s: &str) -> String {
    let one_line = s.replace('\n', " ").replace('\r', " ");
    if one_line.chars().count() <= 180 {
        one_line
    } else {
        format!("{}...", one_line.chars().take(180).collect::<String>())
    }
}

fn summarize_lifecycle_result(
    results: Option<&[crate::core::plugins::PluginLifecycleExecution]>,
    stage: &str,
) -> String {
    let Some(results) = results else {
        return String::new();
    };
    if results.is_empty() {
        return String::new();
    }

    let failed = results.iter().filter(|result| !result.success).count();
    if failed == 0 {
        format!("\n- lifecycle_{}: {} command(s) ok", stage, results.len())
    } else {
        format!(
            "\n- lifecycle_{}: {} ok, {} failed",
            stage,
            results.len().saturating_sub(failed),
            failed
        )
    }
}

fn summarize_runtime_warnings(plugin: &crate::core::plugins::ResolvedPlugin) -> String {
    if plugin.runtime_warnings.is_empty() {
        return String::new();
    }

    let mut lines = Vec::new();
    lines.push(format!(
        "\n- runtime_warnings: {}",
        plugin.runtime_warnings.len()
    ));
    for warning in plugin.runtime_warnings.iter().take(4) {
        lines.push(format!("- warning: {}", trim_line(warning)));
    }
    if plugin.runtime_warnings.len() > 4 {
        lines.push(format!(
            "- warning: ... 还有 {} 条",
            plugin.runtime_warnings.len() - 4
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::plugins::{
        PluginEntry, PluginLifecycleExecution, PluginRuntimeCommand, PluginRuntimeManifest,
        PluginRuntimeTool, ResolvedPlugin,
    };

    fn plugin(name: &str) -> ResolvedPlugin {
        ResolvedPlugin {
            entry: PluginEntry {
                name: name.to_string(),
                source: "local:./plugins/demo".to_string(),
                install_type: "local".to_string(),
                installed_at: 1_700_000_000,
                enabled: true,
                scope: "project".to_string(),
            },
            root: PathBuf::from("/tmp/demo-plugin"),
            root_exists: true,
            manifest_path: Some(PathBuf::from("/tmp/demo-plugin/plugin.json")),
            runtime_manifest: None,
            runtime_error: None,
            runtime_warnings: Vec::new(),
        }
    }

    fn runtime_with_one_command_and_tool() -> PluginRuntimeManifest {
        PluginRuntimeManifest {
            name: Some("demo".to_string()),
            commands: vec![PluginRuntimeCommand {
                name: "deploy".to_string(),
                description: "Ship it".to_string(),
                command: "echo deploy".to_string(),
                timeout: None,
                enabled: None,
            }],
            tools: vec![PluginRuntimeTool {
                name: "lookup".to_string(),
                description: "Look something up".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
                command: "echo lookup".to_string(),
                args: Vec::new(),
                required_permission: Default::default(),
                enabled: None,
            }],
            ..Default::default()
        }
    }

    fn lifecycle(name: &str, success: bool) -> PluginLifecycleExecution {
        PluginLifecycleExecution {
            name: name.to_string(),
            stage: crate::core::plugins::PluginLifecycleStage::Init,
            plugin_name: "demo".to_string(),
            source: "local".to_string(),
            command: "true".to_string(),
            stdout: String::new(),
            stderr: String::new(),
            exit_code: Some(if success { 0 } else { 1 }),
            success,
            timed_out: false,
            error: None,
        }
    }

    /// 运行状态是 `/plugin list` 唯一的健康信号，四种情况必须互不混淆。
    #[test]
    fn runtime_status_distinguishes_the_four_states() {
        let mut missing = plugin("demo");
        missing.root_exists = false;
        assert_eq!(
            describe_runtime_status(&missing),
            "missing plugin directory"
        );

        let mut broken = plugin("demo");
        broken.runtime_error = Some("bad json".to_string());
        assert_eq!(describe_runtime_status(&broken), "manifest error");

        assert_eq!(
            describe_runtime_status(&plugin("demo")),
            "no runtime manifest"
        );

        let mut healthy = plugin("demo");
        healthy.runtime_manifest = Some(runtime_with_one_command_and_tool());
        assert_eq!(
            describe_runtime_status(&healthy),
            "manifest detected, 0 hook(s), 1 command(s), 1 tool(s), 0 lifecycle command(s)"
        );

        healthy.runtime_warnings = vec!["a".to_string(), "b".to_string()];
        assert!(describe_runtime_status(&healthy).ends_with("2 warning(s)"));
    }

    #[test]
    fn validation_report_counts_and_groups_problems() {
        let mut clean = plugin("clean");
        clean.runtime_manifest = Some(PluginRuntimeManifest::default());

        let mut warn = plugin("warn");
        warn.runtime_warnings = vec!["timeout too large".to_string()];

        let mut broken = plugin("broken");
        broken.entry.enabled = false;
        broken.runtime_error = Some("manifest is not valid JSON".to_string());

        let report = render_plugin_validation_report(&[clean, warn, broken], None);
        assert!(report.starts_with("# Plugin Validation\n"), "{report}");
        assert!(report.contains("- plugins: 3"), "{report}");
        assert!(report.contains("- healthy: 1"), "{report}");
        assert!(report.contains("- plugins_with_errors: 1"), "{report}");
        assert!(report.contains("- plugins_with_warnings: 1"), "{report}");
        assert!(report.contains("- total_warnings: 1"), "{report}");
        assert!(report.contains("- disabled: 1"), "{report}");
        assert!(report.contains("## Problems"), "{report}");
        assert!(
            report.contains("`broken` | status=error | enabled=false"),
            "{report}"
        );
        assert!(
            report.contains("`warn` | status=warning | enabled=true"),
            "{report}"
        );
        assert!(
            !report.contains("`clean` |"),
            "healthy plugins stay out of Problems:\n{report}"
        );
    }

    /// 全部健康时不该出现空的 Problems 小节 —— 空标题会让用户以为输出被截断了。
    #[test]
    fn a_clean_validation_report_says_so_and_is_scoped_by_name() {
        let report = render_plugin_validation_report(&[plugin("demo")], Some("demo"));
        assert!(report.starts_with("# Plugin Validation `demo`"), "{report}");
        assert!(report.contains("- plugins_with_errors: 0"), "{report}");
        assert!(report.contains("未发现 runtime 问题"), "{report}");
        assert!(!report.contains("## Problems"), "{report}");
    }

    #[test]
    fn lifecycle_summaries_count_successes_and_failures() {
        assert_eq!(summarize_lifecycle_result(None, "init"), "");
        assert_eq!(summarize_lifecycle_result(Some(&[]), "init"), "");

        let all_ok = vec![lifecycle("a", true), lifecycle("b", true)];
        assert_eq!(
            summarize_lifecycle_result(Some(&all_ok), "init"),
            "\n- lifecycle_init: 2 command(s) ok"
        );

        let mixed = vec![lifecycle("a", true), lifecycle("b", false)];
        assert_eq!(
            summarize_lifecycle_result(Some(&mixed), "shutdown"),
            "\n- lifecycle_shutdown: 1 ok, 1 failed"
        );
    }

    #[test]
    fn warning_summaries_are_capped_with_a_remainder() {
        let mut demo = plugin("demo");
        assert_eq!(summarize_runtime_warnings(&demo), "");

        demo.runtime_warnings = (0..6).map(|i| format!("w{i}")).collect();
        let summary = summarize_runtime_warnings(&demo);
        assert!(summary.contains("- runtime_warnings: 6"), "{summary}");
        assert!(summary.contains("- warning: w0"), "{summary}");
        assert!(summary.contains("- warning: w3"), "{summary}");
        assert!(!summary.contains("- warning: w4"), "{summary}");
        assert!(summary.contains("还有 2 条"), "{summary}");
    }

    #[test]
    fn optional_display_collapses_blank_values() {
        assert_eq!(optional_display(None), "-");
        assert_eq!(optional_display(Some("   ")), "-");
        assert_eq!(optional_display(Some("  hello ")), "hello");
    }

    #[test]
    fn trim_line_flattens_newlines_and_caps_length() {
        assert_eq!(trim_line("a\nb"), "a b");
        let long = "y".repeat(400);
        assert_eq!(trim_line(&long).chars().count(), 183);
        assert!(trim_line(&long).ends_with("..."));
    }

    /// detail 是人用来判断"这个插件到底加载了什么"的视图，字段一个都不能少。
    #[test]
    fn plugin_detail_reports_identity_and_runtime_sections() {
        let mut demo = plugin("demo");
        demo.runtime_manifest = Some(runtime_with_one_command_and_tool());
        demo.runtime_warnings = vec!["unknown key: foo".to_string()];

        let detail = render_plugin_detail(&demo);
        assert!(detail.starts_with("# Plugin `demo`"), "{detail}");
        assert!(detail.contains("- enabled: true"), "{detail}");
        assert!(detail.contains("- install_type: local"), "{detail}");
        assert!(detail.contains("- runtime: manifest detected"), "{detail}");
        assert!(detail.contains("- runtime_warning_count: 1"), "{detail}");
        assert!(detail.contains("## Manifest"), "{detail}");

        let mut missing = plugin("gone");
        missing.root_exists = false;
        missing.manifest_path = None;
        let detail = render_plugin_detail(&missing);
        assert!(detail.contains("root_exists: false"), "{detail}");
        assert!(detail.contains("missing plugin directory"), "{detail}");
    }

    #[test]
    fn update_results_report_success_and_failure_without_dumping_multiline_output() {
        let ok = crate::core::plugins::UpdatePluginResult {
            plugin_name: "demo".to_string(),
            install_type: "git".to_string(),
            success: true,
            output: "Already up to date.\n".to_string(),
            error: None,
        };
        let rendered = render_update_result(&ok);
        assert!(
            rendered.starts_with("✅ `demo` [git] updated"),
            "{rendered}"
        );
        assert!(!rendered.contains('\n') || rendered.lines().count() == 2);

        let failed = crate::core::plugins::UpdatePluginResult {
            plugin_name: "demo".to_string(),
            install_type: "git".to_string(),
            success: false,
            output: String::new(),
            error: Some("network\ndown".to_string()),
        };
        let rendered = render_update_result(&failed);
        assert!(
            rendered.starts_with("❌ `demo` [git] update failed"),
            "{rendered}"
        );
        assert!(rendered.contains("network down"), "{rendered}");
    }
}
