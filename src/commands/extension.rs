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
use std::path::Path;

pub async fn execute_extension_command(args: &[&str]) -> Result<String, String> {
    let registry = ExtensionRegistryManager::new();
    let marketplace = Marketplace::new();
    execute_extension_command_with(args, &registry, &marketplace).await
}

/// 真正的实现。两个管理对象由调用方给 —— 单测拿它们指向临时目录，
/// 否则 `/extension install` 会往开发者自己的 `~/.star/extensions` 里装东西。
pub(crate) async fn execute_extension_command_with(
    args: &[&str],
    registry: &ExtensionRegistryManager,
    marketplace: &Marketplace,
) -> Result<String, String> {
    if args.is_empty() {
        return Ok(extension_help());
    }

    match args[0] {
        "list" => cmd_list(registry),
        "market" | "marketplace" => cmd_market(marketplace),
        "install" | "add" => {
            if args.len() < 2 {
                return Err("Usage: /extension install <name>".to_string());
            }
            cmd_install(marketplace, args[1]).await
        }
        "uninstall" | "remove" | "rm" => {
            if args.len() < 2 {
                return Err("Usage: /extension uninstall <name>".to_string());
            }
            cmd_uninstall(marketplace, args[1])
        }
        "enable" => {
            if args.len() < 2 {
                return Err("Usage: /extension enable <name>".to_string());
            }
            cmd_enable(registry, args[1])
        }
        "disable" => {
            if args.len() < 2 {
                return Err("Usage: /extension disable <name>".to_string());
            }
            cmd_disable(registry, args[1])
        }
        // 帮助文案写的是 `search`，分支名却曾经是 `Grep` —— 照着提示敲的命令
        // 会落到 Unknown subcommand。`search` 是正名，其余保留为别名。
        "search" | "Grep" | "find" => {
            if args.len() < 2 {
                return Err("Usage: /extension search <query>".to_string());
            }
            cmd_search(marketplace, &args[1..].join(" "))
        }
        "info" | "show" => {
            if args.len() < 2 {
                return Err("Usage: /extension info <name>".to_string());
            }
            cmd_info(registry, args[1])
        }
        "skills" => cmd_list_by_type(registry, "skill"),
        "plugins" => cmd_list_by_type(registry, "plugin"),
        "mcp" => cmd_list_by_type(registry, "mcp"),
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

fn cmd_list(registry: &ExtensionRegistryManager) -> Result<String, String> {
    let entries = registry.list_all();

    if entries.is_empty() {
        return Ok(
            "No extensions installed. Use '/extension market' to browse available extensions."
                .to_string(),
        );
    }

    let mut output = String::from("Installed Extensions:\n\n");

    for entry in &entries {
        let status = if entry.enabled { "✓" } else { "✗" };
        output.push_str(&format!(
            "  {} {:<20} {:<8} v{}\n",
            status, entry.name, entry.extension_type, entry.version
        ));
    }

    output.push_str(&format!("\nTotal: {} extensions", entries.len()));
    Ok(output)
}

fn cmd_list_by_type(
    registry: &ExtensionRegistryManager,
    type_name: &str,
) -> Result<String, String> {
    let ext_type = match type_name {
        "skill" => ExtensionType::Skill,
        "plugin" => ExtensionType::Plugin,
        "mcp" => ExtensionType::Mcp,
        _ => return Err(format!("Unknown type: {}", type_name)),
    };

    let entries = registry.list_by_type(&ext_type);

    if entries.is_empty() {
        return Ok(format!("No {} extensions installed.", type_name));
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

fn cmd_market(marketplace: &Marketplace) -> Result<String, String> {
    let featured = marketplace.list_featured();

    let mut output = String::from("🏪 Extension Marketplace\n\n");
    output.push_str("Featured Extensions:\n\n");

    for entry in &featured {
        output.push_str(&format!(
            "  {:<20} {:<8} v{} - {}\n",
            entry.name, entry.extension_type, entry.version, entry.description
        ));
    }

    output.push_str("\nBrowse by category:\n");
    output.push_str("  /extension skills     — Browse skills\n");
    output.push_str("  /extension plugins    — Browse plugins\n");
    output.push_str("  /extension mcp        — Browse MCP servers\n");
    output.push_str("\nInstall: /extension install <name>");

    Ok(output)
}

async fn cmd_install(marketplace: &Marketplace, name: &str) -> Result<String, String> {
    let result = marketplace.install(name).await?;
    Ok(result.message)
}

fn cmd_uninstall(marketplace: &Marketplace, name: &str) -> Result<String, String> {
    let result = marketplace.uninstall(name)?;
    Ok(result.message)
}

fn cmd_enable(registry: &ExtensionRegistryManager, name: &str) -> Result<String, String> {
    registry.set_enabled(name, true)?;
    Ok(format!("Extension '{}' enabled", name))
}

fn cmd_disable(registry: &ExtensionRegistryManager, name: &str) -> Result<String, String> {
    registry.set_enabled(name, false)?;
    Ok(format!("Extension '{}' disabled", name))
}

fn cmd_search(marketplace: &Marketplace, query: &str) -> Result<String, String> {
    let results = marketplace.search(query);

    if results.is_empty() {
        return Ok(format!("No extensions found for '{}'", query));
    }

    let mut output = format!("Search results for '{}':\n\n", query);

    for entry in &results {
        output.push_str(&format!(
            "  {:<20} {:<8} v{} - {}\n",
            entry.name, entry.extension_type, entry.version, entry.description
        ));
    }

    output.push_str(&format!("\nFound {} extensions", results.len()));
    Ok(output)
}

fn cmd_info(registry: &ExtensionRegistryManager, name: &str) -> Result<String, String> {
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

/// 单测用的隔离环境：安装落点与市场缓存都在临时目录里。
#[cfg(test)]
struct Sandbox {
    _dir: tempfile::TempDir,
    registry: ExtensionRegistryManager,
    marketplace: Marketplace,
}

#[cfg(test)]
impl Sandbox {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let extensions = dir.path().join("extensions");
        let market = dir.path().join("marketplace");
        Self {
            registry: ExtensionRegistryManager::with_root(&extensions),
            marketplace: Marketplace::with_roots(&extensions, &market),
            _dir: dir,
        }
    }

    fn root(&self) -> &Path {
        self._dir.path()
    }

    async fn run(&self, args: &[&str]) -> Result<String, String> {
        execute_extension_command_with(args, &self.registry, &self.marketplace).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn no_arguments_prints_the_help() {
        let sb = Sandbox::new();
        let out = sb.run(&[]).await.unwrap();
        assert!(out.contains("/extension install <name>"), "{out}");
        assert!(out.contains("Aliases: /ext, /extension"), "{out}");
    }

    #[tokio::test]
    async fn an_unknown_subcommand_names_itself_and_points_at_help() {
        let sb = Sandbox::new();
        let err = sb.run(&["bogus"]).await.unwrap_err();
        assert!(err.contains("Unknown subcommand: 'bogus'"), "{err}");
        assert!(err.contains("/extension help"), "{err}");
    }

    /// 每个需要参数的子命令都必须先报用法，而不是拿空名字去查注册表。
    #[tokio::test]
    async fn subcommands_that_need_a_name_report_their_usage() {
        let sb = Sandbox::new();
        for (sub, usage) in [
            ("install", "Usage: /extension install <name>"),
            ("uninstall", "Usage: /extension uninstall <name>"),
            ("enable", "Usage: /extension enable <name>"),
            ("disable", "Usage: /extension disable <name>"),
            ("search", "Usage: /extension search <query>"),
            ("info", "Usage: /extension info <name>"),
        ] {
            let err = sb.run(&[sub]).await.unwrap_err();
            assert_eq!(err, usage, "subcommand {sub}");
        }
    }

    /// 帮助文案里的 `search` 曾经走不通（分支名是 `Grep`）。三种写法都要能
    /// 命中同一段逻辑，否则照提示敲的命令会报 Unknown subcommand。
    #[tokio::test]
    async fn search_is_reachable_through_every_documented_spelling() {
        let sb = Sandbox::new();
        for sub in ["search", "Grep", "find"] {
            let out = sb.run(&[sub, "definitely-nothing"]).await.unwrap();
            assert!(
                out.contains("No extensions found for 'definitely-nothing'"),
                "{sub}: {out}"
            );
        }
    }

    #[tokio::test]
    async fn search_joins_multi_word_queries_and_reports_counts() {
        let sb = Sandbox::new();
        let out = sb.run(&["search", "search", "expert"]).await.unwrap();
        assert!(out.contains("Search results for 'search expert'"), "{out}");
        assert!(out.contains("Found "), "{out}");
    }

    #[tokio::test]
    async fn market_lists_featured_entries_and_the_browse_hints() {
        let sb = Sandbox::new();
        let out = sb.run(&["market"]).await.unwrap();
        assert!(out.starts_with("🏪 Extension Marketplace"), "{out}");
        assert!(out.contains("Featured Extensions:"), "{out}");
        // 内置技能里的搜索专家必须以自己的真名出现（`search`，不是工具名 `Grep`）
        assert!(out.contains("search"), "{out}");
        assert!(out.contains("/extension install <name>"), "{out}");
    }

    #[tokio::test]
    async fn an_empty_registry_says_how_to_browse() {
        let sb = Sandbox::new();
        let out = sb.run(&["list"]).await.unwrap();
        assert!(out.contains("No extensions installed"), "{out}");
        assert!(out.contains("/extension market"), "{out}");
    }

    /// 安装 → 列表 → 详情 → 禁用 → 卸载，全链路走一遍；每一步都要在
    /// 临时目录里留下可验证的痕迹。
    #[tokio::test]
    async fn install_list_info_disable_uninstall_round_trip() {
        let sb = Sandbox::new();

        let installed = sb.run(&["install", "search"]).await.unwrap();
        assert!(installed.contains("installed successfully"), "{installed}");
        assert!(
            sb.root().join("extensions/skills/search/SKILL.md").exists(),
            "skill body must be written"
        );
        assert!(sb
            .root()
            .join("extensions/skills/search/manifest.json")
            .exists());

        let listed = sb.run(&["list"]).await.unwrap();
        assert!(listed.contains("search"), "{listed}");
        assert!(listed.contains("skill"), "{listed}");
        assert!(listed.contains("Total: 1 extensions"), "{listed}");

        let skills = sb.run(&["skills"]).await.unwrap();
        assert!(skills.contains("Installed skills:"), "{skills}");
        assert!(skills.contains("search"), "{skills}");

        let info = sb.run(&["info", "search"]).await.unwrap();
        assert!(info.contains("Extension: search"), "{info}");
        assert!(info.contains("Type:      Skill"), "{info}");
        assert!(info.contains("Enabled:   yes"), "{info}");

        let disabled = sb.run(&["disable", "search"]).await.unwrap();
        assert!(
            disabled.contains("Extension 'search' disabled"),
            "{disabled}"
        );
        let info = sb.run(&["info", "search"]).await.unwrap();
        assert!(info.contains("Enabled:   no"), "{info}");

        let enabled = sb.run(&["enable", "search"]).await.unwrap();
        assert!(enabled.contains("Extension 'search' enabled"), "{enabled}");

        let removed = sb.run(&["uninstall", "search"]).await.unwrap();
        assert!(removed.contains("uninstalled"), "{removed}");
        assert!(sb
            .run(&["list"])
            .await
            .unwrap()
            .contains("No extensions installed"));
    }

    #[tokio::test]
    async fn installing_an_unknown_extension_fails_loudly() {
        let sb = Sandbox::new();
        let err = sb.run(&["install", "no-such-extension"]).await.unwrap_err();
        assert!(err.contains("not found in marketplace"), "{err}");
    }

    #[tokio::test]
    async fn enabling_or_describing_a_missing_extension_is_an_error() {
        let sb = Sandbox::new();
        assert!(sb
            .run(&["enable", "ghost"])
            .await
            .unwrap_err()
            .contains("not found"));
        assert!(sb
            .run(&["disable", "ghost"])
            .await
            .unwrap_err()
            .contains("not found"));
        assert!(sb
            .run(&["info", "ghost"])
            .await
            .unwrap_err()
            .contains("not found"));
    }

    #[tokio::test]
    async fn type_filters_only_show_their_own_kind() {
        let sb = Sandbox::new();
        sb.run(&["install", "search"]).await.unwrap();

        let plugins = sb.run(&["plugins"]).await.unwrap();
        assert!(
            plugins.contains("No plugin extensions installed"),
            "{plugins}"
        );

        let mcp = sb.run(&["mcp"]).await.unwrap();
        assert!(mcp.contains("No mcp extensions installed"), "{mcp}");
    }
}
