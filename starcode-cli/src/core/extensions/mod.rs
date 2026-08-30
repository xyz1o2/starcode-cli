/// Extension System — Unified extension management for skills, plugins, and MCP servers.
///
/// # Architecture
///
/// ```text
/// ~/.star/extensions/                # Extension resource directory
/// ├── registry.json                 # Installed extensions registry
/// ├── skills/                       # Skill extensions
/// │   └── <skill-name>/
/// │       ├── SKILL.md             # Skill definition
/// │       └── manifest.json        # Extension manifest
/// ├── plugins/                      # Plugin extensions
/// │   └── <plugin-name>/
/// │       ├── .star-plugin/
/// │       │   └── plugin.json
/// │       └── manifest.json
/// └── mcp/                          # MCP server packages
///     └── <server-name>/
///         ├── manifest.json
///         └── ...
///
/// ~/.star/marketplace/               # Marketplace cache
/// ├── index.json                    # Cached marketplace index
/// └── packages/                     # Downloaded packages
/// ```
///
/// # Extension Types
///
/// - **Skill**: SKILL.md based, loaded into agent as SubAgent
/// - **Plugin**: plugin.json based, provides tools/hooks/commands
/// - **MCP**: MCP server package, provides tools via JSON-RPC
///
pub mod marketplace;
pub mod registry;
pub mod types;

pub use marketplace::Marketplace;
pub use registry::ExtensionRegistryManager;
pub use types::*;
