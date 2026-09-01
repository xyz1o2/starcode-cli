use crate::core::i18n;
use crate::core::mcp::context_server;
pub use crate::core::mcp::{MCPServerConfig, TransportConfig};
use crate::core::mcp::{WindsurfMcpConfig, WindsurfMcpServer};
use clap::Subcommand;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum McpCommand {
    /// Add an MCP server
    #[command(arg_required_else_help = true)]
    Add {
        /// Name for the MCP server
        name: String,
        /// Transport type (stdio, http, sse, streamable_http)
        #[arg(short = 't', long = "transport", default_value = "stdio")]
        transport: String,
        /// Command to run for stdio transport
        #[arg(long = "command")]
        command: Option<String>,
        /// Arguments for the command (single string; split by space in future)
        #[arg(long = "args")]
        args: Option<String>,
        /// URL for http/streamable_http transport
        #[arg(long = "url")]
        url: Option<String>,
        /// Environment variables (repeatable): KEY=VALUE
        #[arg(long = "env")]
        env: Vec<String>,
        /// Mark this server disabled
        #[arg(long = "disabled", default_value_t = false)]
        disabled: bool,
    },
    /// Remove an MCP server
    #[command(arg_required_else_help = true)]
    Remove {
        /// Name of the MCP server to remove
        name: String,
    },

    /// Import MCP servers from a config file (Windsurf format)
    #[command(arg_required_else_help = true)]
    Import {
        /// Path to mcp_config.json
        path: String,
        /// Do not persist env values
        #[arg(long = "strip-env", default_value_t = false)]
        strip_env: bool,
    },

    /// Export MCP servers to a config file (Windsurf format)
    #[command(arg_required_else_help = true)]
    Export {
        /// Output path (will be overwritten)
        path: String,
        /// Do not write env values
        #[arg(long = "strip-env", default_value_t = false)]
        strip_env: bool,
    },

    /// Install an MCP server (npm package)
    #[command(arg_required_else_help = true)]
    Install {
        /// NPM package name (e.g. @modelcontextprotocol/server-memory)
        package: String,
        /// Optional server name (defaults to package name suffix)
        #[arg(long)]
        name: Option<String>,
        /// Environment variables
        #[arg(long = "env")]
        env: Vec<String>,
    },

    /// List configured MCP servers
    List,
    /// Show MCP server status and discovered tool counts
    Status,
    /// Re-discover MCP servers and refresh tool cache
    Refresh,
    /// List MCP tools (optionally for one server)
    Tools {
        /// Optional server name filter
        server: Option<String>,
    },
    /// List MCP tools with descriptions
    Desc {
        /// Optional server name filter
        server: Option<String>,
    },
    /// List MCP tools with input schemas
    Schema {
        /// Optional server name filter
        server: Option<String>,
    },
    /// Start StarCode CLI as an MCP server (context engine)
    ///
    /// Exposes semantic code search and call chain tracing
    /// as MCP tools for external agents (Claude Code, Cursor, etc.).
    /// Uses stdio transport (compatible with all MCP clients).
    Serve,
}

async fn import_windsurf_config(
    path: &str,
    strip_env: bool,
) -> Result<String, crate::core::mcp::McpError> {
    let p = PathBuf::from(path);
    let content = tokio::fs::read_to_string(&p).await?;
    let cfg: WindsurfMcpConfig = serde_json::from_str(&content)?;

    let mut project_cfg = crate::core::mcp::load_project_mcp_config().await?;
    for (name, mut s) in cfg.mcp_servers {
        if strip_env {
            s.env = None;
        }
        project_cfg.mcp_servers.insert(name, s);
    }
    crate::core::mcp::save_project_mcp_config(&project_cfg).await?;
    Ok(i18n::t(
        "cmd.mcp.import.success",
        "Imported MCP servers from: {path}",
        "Imported MCP servers from: {path}",
    )
    .replace("{path}", path))
}

async fn export_windsurf_config(
    path: &str,
    strip_env: bool,
) -> Result<String, crate::core::mcp::McpError> {
    let mut project_cfg = crate::core::mcp::load_project_mcp_config().await?;
    if strip_env {
        for (_, s) in project_cfg.mcp_servers.iter_mut() {
            s.env = None;
        }
    }
    let s = serde_json::to_string_pretty(&project_cfg)?;
    tokio::fs::write(path, s).await?;
    Ok(i18n::t(
        "cmd.mcp.export.success",
        "Exported MCP servers to: {path}",
        "Exported MCP servers to: {path}",
    )
    .replace("{path}", path))
}

pub async fn add_mcp_server(
    name: String,
    config: MCPServerConfig,
) -> Result<String, crate::core::mcp::McpError> {
    let mut project_cfg = crate::core::mcp::load_project_mcp_config().await?;
    let s = WindsurfMcpServer {
        command: config.transport.command.clone().or(config.command.clone()),
        args: config.transport.args.clone().or(config.args.clone()),
        env: config.transport.env.clone().or(config.env.clone()),
        disabled: config.disabled,
        transport_type: Some(config.transport.transport_type.clone()),
        url: config.transport.url.clone(),
    };
    project_cfg.mcp_servers.insert(name.clone(), s);
    crate::core::mcp::save_project_mcp_config(&project_cfg).await?;
    Ok(i18n::t(
        "cmd.mcp.add.success",
        "Added MCP server: {name}",
        "Added MCP server: {name}",
    )
    .replace("{name}", &name))
}

pub async fn remove_mcp_server(name: &str) -> Result<String, crate::core::mcp::McpError> {
    let mut project_cfg = crate::core::mcp::load_project_mcp_config().await?;
    project_cfg.mcp_servers.remove(name);
    crate::core::mcp::save_project_mcp_config(&project_cfg).await?;
    Ok(i18n::t(
        "cmd.mcp.remove.success",
        "Removed MCP server: {name}",
        "Removed MCP server: {name}",
    )
    .replace("{name}", name))
}

async fn build_live_mcp_manager() -> Result<crate::core::mcp::MCPManager, crate::core::mcp::McpError>
{
    let manager = crate::core::mcp::MCPManager::new();
    manager.initialize_mcp_servers().await?;
    Ok(manager)
}

async fn render_tools_snapshot(
    server_filter: Option<&str>,
    include_desc: bool,
    include_schema: bool,
) -> Result<String, crate::core::mcp::McpError> {
    let manager = build_live_mcp_manager().await?;
    let discover_errors = manager.discover_all().await;
    let servers = manager.list_server_names().await;
    if servers.is_empty() {
        return Ok(i18n::t(
            "cmd.mcp.list.empty",
            "No MCP servers configured.",
            "No MCP servers configured.",
        ));
    }

    let mut out = String::new();
    for server in servers {
        if let Some(filter) = server_filter {
            if server != filter {
                continue;
            }
        }

        out.push_str(&format!("[{}]\n", server));

        match manager.list_tools(&server).await {
            Ok(tools) => {
                if tools.is_empty() {
                    out.push_str(&i18n::t(
                        "cmd.mcp.tools.no_tools",
                        "  (no tools)\n",
                        "  (no tools)\n",
                    ));
                } else {
                    for tool in tools {
                        if include_desc {
                            out.push_str(&format!("- {}: {}\n", tool.name, tool.description));
                        } else {
                            out.push_str(&format!("- {}\n", tool.name));
                        }
                        if include_schema {
                            let schema = serde_json::to_string_pretty(&tool.input_schema)
                                .unwrap_or_else(|_| "{}".to_string());
                            out.push_str(&format!("  schema:\n{}\n", schema));
                        }
                    }
                }
            }
            Err(e) => {
                out.push_str(
                    &i18n::t(
                        "cmd.mcp.tools.error",
                        "  Error: {error}\n",
                        "  error: {error}\n",
                    )
                    .replace("{error}", &e.to_string()),
                );
            }
        }

        if let Some(err) = discover_errors.get(&server) {
            out.push_str(
                &i18n::t(
                    "cmd.mcp.tools.discover_error",
                    "  Found error: {error}\n",
                    "  discover_error: {error}\n",
                )
                .replace("{error}", err),
            );
        }
        out.push('\n');
    }

    if out.trim().is_empty() {
        if let Some(filter) = server_filter {
            return Ok(i18n::t(
                "cmd.mcp.tools.server_not_found",
                "Server not found: {name}",
                "Server not found: {name}",
            )
            .replace("{name}", filter));
        }
        return Ok(i18n::t(
            "cmd.mcp.list.empty",
            "No MCP servers configured.",
            "No MCP servers configured.",
        ));
    }
    Ok(out.trim_end().to_string())
}

async fn render_status_snapshot() -> Result<String, crate::core::mcp::McpError> {
    let cfg = crate::core::mcp::load_project_mcp_config().await?;
    let configured_total = cfg.mcp_servers.len();
    let configured_enabled = cfg
        .mcp_servers
        .values()
        .filter(|s| !s.disabled.unwrap_or(false))
        .count();
    let configured_disabled = configured_total.saturating_sub(configured_enabled);

    let manager = build_live_mcp_manager().await?;
    let discover_errors: HashMap<String, String> = manager.discover_all().await;
    let servers = manager.list_server_names().await;

    if servers.is_empty() {
        return Ok(i18n::t(
            "cmd.mcp.status.empty",
            "MCP Status\n- configured: {total} (enabled: {enabled}, disabled: {disabled})\n- active: 0",
            "MCP Status\n- configured: {total} (enabled: {enabled}, disabled: {disabled})\n- active: 0",
        )
        .replace("{total}", &configured_total.to_string())
        .replace("{enabled}", &configured_enabled.to_string())
        .replace("{disabled}", &configured_disabled.to_string()));
    }

    let mut out = String::new();
    out.push_str(&i18n::t(
        "cmd.mcp.status.header",
        "MCP Status\n",
        "MCP Status\n",
    ));
    out.push_str(
        &i18n::t(
            "cmd.mcp.status.configured",
            "- configured: {total} (enabled: {enabled}, disabled: {disabled})\n",
            "- configured: {total} (enabled: {enabled}, disabled: {disabled})\n",
        )
        .replace("{total}", &configured_total.to_string())
        .replace("{enabled}", &configured_enabled.to_string())
        .replace("{disabled}", &configured_disabled.to_string()),
    );
    out.push_str(
        &i18n::t(
            "cmd.mcp.status.active",
            "- active: {count}\n",
            "- active: {count}\n",
        )
        .replace("{count}", &servers.len().to_string()),
    );
    out.push_str(&i18n::t(
        "cmd.mcp.status.servers_header",
        "Servers:\n",
        "Servers:\n",
    ));

    for server in servers {
        if let Some(err) = discover_errors.get(&server) {
            out.push_str(
                &i18n::t(
                    "cmd.mcp.status.server_error",
                    "  - {name}: error ({error})\n",
                    "  - {name}: error ({error})\n",
                )
                .replace("{name}", &server)
                .replace("{error}", err),
            );
            continue;
        }

        match manager.list_tools(&server).await {
            Ok(tools) => {
                out.push_str(
                    &i18n::t(
                        "cmd.mcp.status.server_connected",
                        "  - {name}: connected ({count} tools)\n",
                        "  - {name}: connected ({count} tools)\n",
                    )
                    .replace("{name}", &server)
                    .replace("{count}", &tools.len().to_string()),
                );
            }
            Err(e) => {
                out.push_str(
                    &i18n::t(
                        "cmd.mcp.status.server_error",
                        "  - {name}: error ({error})\n",
                        "  - {name}: error ({error})\n",
                    )
                    .replace("{name}", &server)
                    .replace("{error}", &e.to_string()),
                );
            }
        }
    }

    Ok(out.trim_end().to_string())
}

pub async fn execute_mcp_command(
    command: McpCommand,
) -> Result<String, crate::core::mcp::McpError> {
    match command {
        McpCommand::List => {
            let config = crate::core::mcp::load_project_mcp_config().await?;
            if config.mcp_servers.is_empty() {
                return Ok(i18n::t(
                    "cmd.mcp.list.empty",
                    "No MCP servers configured.",
                    "No MCP servers configured.",
                ));
            }
            let mut out = String::new();
            out.push_str(&i18n::t(
                "cmd.mcp.list.header",
                "Configured MCP Servers:\n",
                "Configured MCP Servers:\n",
            ));
            for (name, server) in config.mcp_servers {
                out.push_str(
                    &i18n::t(
                        "cmd.mcp.list.item",
                        "- {name}: {command} ({transport})\n",
                        "- {name}: {command} ({transport})\n",
                    )
                    .replace("{name}", &name)
                    .replace("{command}", server.command.as_deref().unwrap_or("-"))
                    .replace(
                        "{transport}",
                        server.transport_type.as_deref().unwrap_or("-"),
                    ),
                );
                if let Some(args) = &server.args {
                    out.push_str(
                        &i18n::t("cmd.mcp.list.args", "  Args: {args}\n", "  Args: {args}\n")
                            .replace("{args}", &format!("{:?}", args)),
                    );
                }
                if server.disabled.unwrap_or(false) {
                    out.push_str(&i18n::t(
                        "cmd.mcp.list.disabled",
                        "  (Disabled)\n",
                        "  (Disabled)\n",
                    ));
                }
            }
            Ok(out)
        }
        McpCommand::Status => render_status_snapshot().await,
        McpCommand::Refresh => {
            let manager = build_live_mcp_manager().await?;
            let errors = manager.discover_all().await;
            let server_count = manager.list_server_names().await.len();
            if errors.is_empty() {
                Ok(i18n::t(
                    "cmd.mcp.refresh.success",
                    "Refreshed MCP servers successfully ({count} servers).",
                    "Refreshed MCP servers successfully ({count} servers).",
                )
                .replace("{count}", &server_count.to_string()))
            } else {
                let details = errors
                    .into_iter()
                    .map(|(name, err)| format!("{}: {}", name, err))
                    .collect::<Vec<_>>()
                    .join("; ");
                Ok(i18n::t(
                    "cmd.mcp.refresh.partial",
                    "Refreshed MCP with partial failures ({count} servers): {details}",
                    "Refreshed MCP with partial failures ({count} servers): {details}",
                )
                .replace("{count}", &server_count.to_string())
                .replace("{details}", &details))
            }
        }
        McpCommand::Tools { server } => {
            render_tools_snapshot(server.as_deref(), false, false).await
        }
        McpCommand::Desc { server } => render_tools_snapshot(server.as_deref(), true, false).await,
        McpCommand::Schema { server } => render_tools_snapshot(server.as_deref(), true, true).await,
        McpCommand::Serve => {
            context_server::run_stdio_server().await?;
            Ok(String::new())
        }
        McpCommand::Add {
            name,
            transport,
            command,
            args,
            url,
            env,
            disabled,
        } => {
            let transport_type = transport;
            let args_vec = args.as_deref().map(|s| {
                s.split_whitespace()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
            });

            let mut env_map: std::collections::HashMap<String, String> =
                std::collections::HashMap::new();
            for item in env {
                let s = item.trim();
                if s.is_empty() {
                    continue;
                }
                let Some((k, v)) = s.split_once('=') else {
                    return Err(i18n::t(
                        "cmd.mcp.error.invalid_env",
                        "invalid --env value: {value}",
                        "invalid --env value: {value}",
                    )
                    .replace("{value}", s)
                    .into());
                };
                let k = k.trim();
                if k.is_empty() {
                    return Err(i18n::t(
                        "cmd.mcp.error.invalid_env_key",
                        "invalid --env key: {value}",
                        "invalid --env key: {value}",
                    )
                    .replace("{value}", s)
                    .into());
                }
                env_map.insert(k.to_string(), v.to_string());
            }
            let env_opt = if env_map.is_empty() {
                None
            } else {
                Some(env_map)
            };

            let transport_cfg = TransportConfig {
                transport_type: transport_type.clone(),
                command: command.clone(),
                args: args_vec,
                env: env_opt.clone(),
                url: url.clone(),
                headers: None,
            };

            if transport_type == "stdio"
                && transport_cfg
                    .command
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .is_empty()
            {
                return Err(i18n::t(
                    "cmd.mcp.error.stdio_requires_command",
                    "stdio transport requires --command",
                    "stdio transport requires --command",
                )
                .into());
            }
            if (transport_type == "http" || transport_type == "streamable_http")
                && transport_cfg.url.as_deref().unwrap_or("").trim().is_empty()
            {
                return Err(i18n::t(
                    "cmd.mcp.error.http_requires_url",
                    "http/streamable_http transport requires --url",
                    "http/streamable_http transport requires --url",
                )
                .into());
            }

            let config = MCPServerConfig {
                name: name.clone(),
                transport: transport_cfg,
                command: None,
                args: None,
                env: env_opt,
                disabled: if disabled { Some(true) } else { None },
            };

            add_mcp_server(name, config).await
        }
        McpCommand::Remove { name } => remove_mcp_server(&name).await,
        McpCommand::Import { path, strip_env } => import_windsurf_config(&path, strip_env).await,
        McpCommand::Export { path, strip_env } => export_windsurf_config(&path, strip_env).await,
        McpCommand::Install { package, name, env } => {
            let server_name = name.unwrap_or_else(|| {
                let parts: Vec<&str> = package.split('/').collect();
                let pkg_str = package.as_str();
                let last = parts.last().unwrap_or(&pkg_str);
                last.replace("server-", "").replace("mcp-", "").to_string()
            });

            let args_vec = Some(vec!["-y".to_string(), package]);

            let mut env_map: std::collections::HashMap<String, String> =
                std::collections::HashMap::new();
            for item in env {
                let s = item.trim();
                if s.is_empty() {
                    continue;
                }
                let Some((k, v)) = s.split_once('=') else {
                    return Err(i18n::t(
                        "cmd.mcp.error.invalid_env",
                        "invalid --env value: {value}",
                        "invalid --env value: {value}",
                    )
                    .replace("{value}", s)
                    .into());
                };
                env_map.insert(k.trim().to_string(), v.to_string());
            }
            let env_opt = if env_map.is_empty() {
                None
            } else {
                Some(env_map)
            };

            let transport_cfg = TransportConfig {
                transport_type: "stdio".to_string(),
                command: Some("npx".to_string()),
                args: args_vec,
                env: env_opt.clone(),
                url: None,
                headers: None,
            };

            let config = MCPServerConfig {
                name: server_name.clone(),
                transport: transport_cfg,
                command: None,
                args: None,
                env: env_opt,
                disabled: None,
            };

            add_mcp_server(server_name, config).await
        }
    }
}
