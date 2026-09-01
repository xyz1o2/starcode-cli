/// Extension command — manage extensions (skills, plugins, MCP servers).
///
/// # Commands
///
/// - `/extension list` — List installed extensions
/// - `/extension market` — Open marketplace
/// - `/extension install <name>` — Install an extension
/// - `/extension uninstall <name>` — Uninstall an extension
/// - `/extension enable <name>` — Enable an extension
/// - `/extension disable <name>` — Disable an extension
/// - `/extension search <query>` — Search marketplace
/// - `/extension info <name>` — Show extension details
use crate::core::extensions::marketplace::Marketplace;
use crate::core::extensions::registry::ExtensionRegistryManager;
use crate::core::extensions::types::*;

pub async fn execute_extension_command(args: &[&str]) -> Result<String, String> {
    if args.is_empty() {
        return Ok(extension_help());
    }

    match args[0] {
        "list" => cmd_list(),
        "market" | "marketplace" => cmd_market(),
        "install" | "add" => {
            if args.len() < 2 {
                return Err("Usage: /extension install <name>".to_string());
            }
            cmd_install(args[1]).await
        }
        "uninstall" | "remove" | "rm" => {
            if args.len() < 2 {
                return Err("Usage: /extension uninstall <name>".to_string());
            }
            cmd_uninstall(args[1])
        }
        "enable" => {
            if args.len() < 2 {
                return Err("Usage: /extension enable <name>".to_string());
            }
            cmd_enable(args[1])
        }
        "disable" => {
            if args.len() < 2 {
                return Err("Usage: /extension disable <name>".to_string());
            }
            cmd_disable(args[1])
        }
        "Grep" | "find" => {
            if args.len() < 2 {
                return Err("Usage: /extension search <query>".to_string());
            }
            cmd_search(&args[1..].join(" "))
        }
        "info" | "show" => {
            if args.len() < 2 {
                return Err("Usage: /extension info <name>".to_string());
            }
            cmd_info(args[1])
        }
        "skills" => cmd_list_by_type("skill"),
        "plugins" => cmd_list_by_type("plugin"),
        "mcp" => cmd_list_by_type("mcp"),
        "help" => Ok(extension_help()),
        _ => Err(format!(
            "Unknown subcommand: '{}'. Run '/extension help' for usage.",
            args[0]
        )),
    }
}

fn extension_help() -> String {
    r#"Extension Management Commands:

  /extension list                List all installed extensions
  /extension market              Browse the marketplace
  /extension install <name>      Install an extension
  /extension uninstall <name>    Uninstall an extension
  /extension enable <name>       Enable an extension
  /extension disable <name>      Disable an extension
  /extension search <query>      Search marketplace
  /extension info <name>         Show extension details

  /extension skills              List installed skills
  /extension plugins             List installed plugins
  /extension mcp                 List installed MCP servers

Aliases: /ext, /extension
"#
    .to_string()
}

fn cmd_list() -> Result<String, String> {
    let registry = ExtensionRegistryManager::new();
    let entries = registry.list_all();

    if entries.is_empty() {
        return Ok("No extensions installed. Use '/extension market' to browse available extensions.".to_string());
    }

    let mut output = String::from("Installed Extensions:\n\n");

    for entry in &entries {
        let status = if entry.enabled { "✓" } else { "✗" };
        let type_label = match entry.extension_type {
            ExtensionType::Skill => "skill",
            ExtensionType::Plugin => "plugin",
            ExtensionType::Mcp => "mcp",
        };
        output.push_str(&format!(
            "  {} {:<20} {:<8} v{}\n",
            status, entry.name, type_label, entry.version
        ));
    }

    output.push_str(&format!("\nTotal: {} extensions", entries.len()));
    Ok(output)
}

fn cmd_list_by_type(type_name: &str) -> Result<String, String> {
    let ext_type = match type_name {
        "skill" => ExtensionType::Skill,
        "plugin" => ExtensionType::Plugin,
        "mcp" => ExtensionType::Mcp,
        _ => return Err(format!("Unknown type: {}", type_name)),
    };

    let registry = ExtensionRegistryManager::new();
    let entries = registry.list_by_type(&ext_type);

    if entries.is_empty() {
        return Ok(format!(
            "No {} extensions installed.",
            type_name
        ));
    }

    let mut output = format!("Installed {}s:\n\n", type_name);

    for entry in &entries {
        let status = if entry.enabled { "✓" } else { "✗" };
        output.push_str(&format!(
            "  {} {:<20} v{}\n",
            status, entry.name, entry.version
        ));
    }

    Ok(output)
}

fn cmd_market() -> Result<String, String> {
    let marketplace = Marketplace::new();
    let featured = marketplace.list_featured();

    let mut output = String::from("🏪 Extension Marketplace\n\n");
    output.push_str("Featured Extensions:\n\n");

    for entry in &featured {
        let type_label = match entry.extension_type {
            ExtensionType::Skill => "skill",
            ExtensionType::Plugin => "plugin",
            ExtensionType::Mcp => "mcp",
        };
        output.push_str(&format!(
            "  {:<20} {:<8} v{} - {}\n",
            entry.name, type_label, entry.version, entry.description
        ));
    }

    output.push_str("\nBrowse by category:\n");
    output.push_str("  /extension skills     — Browse skills\n");
    output.push_str("  /extension plugins    — Browse plugins\n");
    output.push_str("  /extension mcp        — Browse MCP servers\n");
    output.push_str("\nInstall: /extension install <name>");

    Ok(output)
}

async fn cmd_install(name: &str) -> Result<String, String> {
    let marketplace = Marketplace::new();
    let result = marketplace.install(name).await?;
    Ok(result.message)
}

fn cmd_uninstall(name: &str) -> Result<String, String> {
    let marketplace = Marketplace::new();
    let result = marketplace.uninstall(name)?;
    Ok(result.message)
}

fn cmd_enable(name: &str) -> Result<String, String> {
    let registry = ExtensionRegistryManager::new();
    registry.set_enabled(name, true)?;
    Ok(format!("Extension '{}' enabled", name))
}

fn cmd_disable(name: &str) -> Result<String, String> {
    let registry = ExtensionRegistryManager::new();
    registry.set_enabled(name, false)?;
    Ok(format!("Extension '{}' disabled", name))
}

fn cmd_search(query: &str) -> Result<String, String> {
    let marketplace = Marketplace::new();
    let results = marketplace.search(query);

    if results.is_empty() {
        return Ok(format!("No extensions found for '{}'", query));
    }

    let mut output = format!("Search results for '{}':\n\n", query);

    for entry in &results {
        let type_label = match entry.extension_type {
            ExtensionType::Skill => "skill",
            ExtensionType::Plugin => "plugin",
            ExtensionType::Mcp => "mcp",
        };
        output.push_str(&format!(
            "  {:<20} {:<8} v{} - {}\n",
            entry.name, type_label, entry.version, entry.description
        ));
    }

    output.push_str(&format!("\nFound {} extensions", results.len()));
    Ok(output)
}

fn cmd_info(name: &str) -> Result<String, String> {
    let registry = ExtensionRegistryManager::new();
    let entry = registry
        .get(name)
        .ok_or_else(|| format!("Extension '{}' not found", name))?;

    let mut output = format!("Extension: {}\n\n", entry.name);
    output.push_str(&format!("  Type:      {:?}\n", entry.extension_type));
    output.push_str(&format!("  Version:   {}\n", entry.version));
    output.push_str(&format!("  Source:    {}\n", entry.source));
    output.push_str(&format!(
        "  Enabled:   {}\n",
        if entry.enabled { "yes" } else { "no" }
    ));
    output.push_str(&format!(
        "  Installed: {}\n",
        chrono::DateTime::from_timestamp(entry.installed_at, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "unknown".to_string())
    ));

    Ok(output)
}
