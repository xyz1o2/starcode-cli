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
    if project_cfg.mcp_servers.remove(name).is_none() {
        // 删一个不存在的 server 不能回 "Removed"：用户会以为它被删掉了，
        // 实际可能是拼错名字，或者它根本在 user 作用域（那份不归这条命令改）。
        return Err(format!(
            "No MCP server named `{}` in the project config ({}). Use /mcp list to see the effective scopes.",
            name,
            crate::core::mcp::project_star_mcp_path().display()
        )
        .into());
    }
    crate::core::mcp::save_project_mcp_config(&project_cfg).await?;
    Ok(i18n::t(
        "cmd.mcp.remove.success",
        "Removed MCP server: {name}",
        "Removed MCP server: {name}",
    )
    .replace("{name}", name))
}

/// `--env KEY=VALUE` 的解析。`add` 和 `install` 曾经各抄一份，于是
/// `install` 少了空 key 校验 —— 同一面旗子两种行为，这里只留一份。
fn parse_env_pairs(
    items: &[String],
) -> Result<Option<std::collections::HashMap<String, String>>, String> {
    let mut env_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for item in items {
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
            .replace("{value}", s));
        };
        let k = k.trim();
        if k.is_empty() {
            return Err(i18n::t(
                "cmd.mcp.error.invalid_env_key",
                "invalid --env key: {value}",
                "invalid --env key: {value}",
            )
            .replace("{value}", s));
        }
        env_map.insert(k.to_string(), v.to_string());
    }
    Ok(if env_map.is_empty() {
        None
    } else {
        Some(env_map)
    })
}

/// `install` 没给 `--name` 时的默认名：包名最后一段，去掉常见的
/// `server-` / `mcp-` 前缀，好让 `/mcp tools memory` 这种用法可预测。
fn server_name_for_package(package: &str) -> String {
    let last = package.split('/').next_back().unwrap_or(package).trim();
    last.replace("server-", "").replace("mcp-", "")
}

/// 任何一种 transport 缺了它唯一的必需品都得在落盘前拒绝 —— 写出一个
/// 连不上的 server，用户下次只会看到一条翻译不通的握手错误。
fn validate_transport(
    transport: &str,
    command: Option<&str>,
    url: Option<&str>,
) -> Result<(), String> {
    match transport {
        "stdio" if command.unwrap_or("").trim().is_empty() => Err(i18n::t(
            "cmd.mcp.error.stdio_requires_command",
            "stdio transport requires --command",
            "stdio transport requires --command",
        )),
        "http" | "streamable_http" | "sse" if url.unwrap_or("").trim().is_empty() => Err(i18n::t(
            "cmd.mcp.error.http_requires_url",
            "http/sse/streamable_http transport requires --url",
            "http/sse/streamable_http transport requires --url",
        )),
        _ => Ok(()),
    }
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
    // 统计**生效**的配置（user + project 合并）—— agent 连的是这一份。
    let cfg = crate::core::mcp::load_effective_mcp_config().await?;
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
            // 列出**生效**的配置（user + project 合并），不是只列项目级 ——
            // agent 实际连的就是这份，只显示项目级会让用户以为 server 没配。
            let servers = crate::core::mcp::load_effective_mcp_servers().await?;
            if servers.is_empty() {
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
            for entry in servers {
                let server = entry.server;
                out.push_str(
                    &i18n::t(
                        "cmd.mcp.list.item",
                        "- {name}: {command} ({transport})\n",
                        "- {name}: {command} ({transport})\n",
                    )
                    .replace("{name}", &entry.name)
                    .replace("{command}", server.command.as_deref().unwrap_or("-"))
                    .replace(
                        "{transport}",
                        server.transport_type.as_deref().unwrap_or("-"),
                    ),
                );
                // 标出来源：同名 server 可能同时存在于多个作用域。
                out.push_str(&format!("  (from {})\n", entry.scope.label()));
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
            let env_opt = parse_env_pairs(&env)?;

            let transport_cfg = TransportConfig {
                transport_type: transport_type.clone(),
                command: command.clone(),
                args: args_vec,
                env: env_opt.clone(),
                url: url.clone(),
                headers: None,
            };

            validate_transport(
                &transport_type,
                transport_cfg.command.as_deref(),
                transport_cfg.url.as_deref(),
            )?;

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
            let server_name = name.unwrap_or_else(|| server_name_for_package(&package));

            let args_vec = Some(vec!["-y".to_string(), package]);

            let env_opt = parse_env_pairs(&env)?;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn env(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn env_pairs_are_optional_and_blank_entries_ignored() {
        assert_eq!(parse_env_pairs(&[]).unwrap(), None);
        assert_eq!(parse_env_pairs(&env(&["", "   "])).unwrap(), None);

        let parsed = parse_env_pairs(&env(&[" API_KEY=abc ", "DEBUG=true"]))
            .unwrap()
            .expect("map");
        assert_eq!(parsed.get("API_KEY").map(String::as_str), Some("abc"));
        assert_eq!(parsed.get("DEBUG").map(String::as_str), Some("true"));
    }

    /// 值里可以再出现 `=`（token、base64、URL query 都常见），只按第一个切分。
    #[test]
    fn env_values_may_contain_equals_signs() {
        let parsed = parse_env_pairs(&env(&["URL=https://x/y?a=b"]))
            .unwrap()
            .expect("map");
        assert_eq!(
            parsed.get("URL").map(String::as_str),
            Some("https://x/y?a=b")
        );
    }

    #[test]
    fn malformed_env_entries_are_rejected() {
        let err = parse_env_pairs(&env(&["NO_EQUALS"])).unwrap_err();
        assert!(err.contains("invalid --env value: NO_EQUALS"), "{err}");

        let err = parse_env_pairs(&env(&["=orphan-value"])).unwrap_err();
        assert!(err.contains("invalid --env key: =orphan-value"), "{err}");
    }

    /// `install` 以前对空 key 放行，`add` 拒绝 —— 同一个旗子不能有两种语义。
    #[tokio::test]
    async fn both_add_and_install_share_one_env_parser() {
        let result = execute_mcp_command(McpCommand::Install {
            package: "@scope/server-x".to_string(),
            name: None,
            env: env(&["=v"]),
        })
        .await;
        let err = result.unwrap_err();
        assert!(err.to_string().contains("invalid --env key"), "{err}");
    }

    #[test]
    fn package_names_derive_a_predictable_server_name() {
        assert_eq!(
            server_name_for_package("@modelcontextprotocol/server-memory"),
            "memory"
        );
        assert_eq!(server_name_for_package("mcp-server-fetch"), "fetch");
        assert_eq!(server_name_for_package("plain"), "plain");
        assert_eq!(server_name_for_package("  @a/b  "), "b");
    }

    #[test]
    fn stdio_transport_requires_a_command() {
        assert!(validate_transport("stdio", None, None).is_err());
        assert!(validate_transport("stdio", Some("   "), None).is_err());
        assert!(validate_transport("stdio", Some("npx"), None).is_ok());
    }

    #[test]
    fn url_transports_require_a_url() {
        for transport in ["http", "sse", "streamable_http"] {
            let err = validate_transport(transport, None, None).unwrap_err();
            assert!(err.contains("requires --url"), "{transport}: {err}");
            assert!(validate_transport(transport, None, Some("https://example.com")).is_ok());
        }
    }

    #[test]
    fn unknown_transports_are_not_failed_here() {
        // 兼容层可能有别的名字；真正的握手错误在 initialize 时出现，
        // 这里只拦"必定连不上"的那两种必需品。
        assert!(validate_transport("wasm", None, None).is_ok());
    }

    #[tokio::test]
    async fn adding_a_stdio_server_without_a_command_fails_before_writing() {
        let result = execute_mcp_command(McpCommand::Add {
            name: "bad".to_string(),
            transport: "stdio".to_string(),
            command: None,
            args: None,
            url: None,
            env: Vec::new(),
            disabled: false,
        })
        .await;
        let err = result.unwrap_err();
        assert!(err.to_string().contains("requires --command"), "{err}");
    }
}
