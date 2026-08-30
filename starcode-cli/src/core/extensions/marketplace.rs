/// Extension marketplace — browse and install extensions from a registry.
///
/// # Marketplace Flow
///
/// 1. User opens marketplace (via `/extension market` or Ctrl+P palette)
/// 2. Fetch index from remote or use cached version
/// 3. User browses/searches extensions
/// 4. User selects an extension to install
/// 5. Download and extract to `~/.star/extensions/<type>/<name>/`
/// 6. Register in `~/.star/extensions/registry.json`
///
/// # Index Format
///
/// The marketplace index is a JSON file that lists all available extensions:
///
/// ```json
/// {
///   "version": 1,
///   "updated_at": "2024-01-01T00:00:00Z",
///   "entries": [
///     {
///       "name": "analyzer",
///       "version": "1.0.0",
///       "description": "Code analysis skill",
///       "type": "skill",
///       "source": "https://github.com/starcode-ai/skills/analyzer",
///       "tags": ["code-analysis"],
///       "downloads": 1000,
///       "stars": 50
///     }
///   ]
/// }
/// ```
use super::registry::ExtensionRegistryManager;
use super::types::*;
use std::path::{Path, PathBuf};

/// Default marketplace index URL
/// Points to the official Claude Code plugins marketplace (anthropics/claude-plugins-official).
/// Contains 160+ plugins from Anthropic, AWS, Google, Microsoft, Shopify, and more.
/// Format: Claude Code `.claude-plugin/marketplace.json` — may need adaptation for StarCode's index format.
const DEFAULT_INDEX_URL: &str =
    "https://raw.githubusercontent.com/anthropics/claude-plugins-official/main/.claude-plugin/marketplace.json";

/// Built-in skills that ship with the extension system
const BUILTIN_SKILLS: &[(&str, &str, &str)] = &[
    ("analyzer", "Code analysis expert", "skills/analyzer"),
    ("editor", "Batch editing expert", "skills/editor"),
    ("Grep", "Search expert", "skills/search"),
    ("navigator", "Recursive context navigator", "skills/navigator"),
    ("auto_fix", "Auto-fix with test-driven approach", "skills/auto_fix"),
    ("verify", "Build/test/lint verification", "skills/verify"),
];

pub struct Marketplace {
    registry: ExtensionRegistryManager,
    index_path: PathBuf,
    packages_dir: PathBuf,
}

impl Marketplace {
    pub fn new() -> Self {
        let marketplace_dir = ExtensionRegistryManager::marketplace_dir();
        let index_path = marketplace_dir.join("index.json");
        let packages_dir = marketplace_dir.join("packages");
        Self {
            registry: ExtensionRegistryManager::new(),
            index_path,
            packages_dir,
        }
    }

    /// Load the marketplace index (from cache or generate default)
    pub fn load_index(&self) -> MarketplaceIndex {
        // Try to load from cache
        if self.index_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&self.index_path) {
                if let Ok(index) = serde_json::from_str::<MarketplaceIndex>(&content) {
                    return index;
                }
            }
        }

        // Generate default index with built-in skills
        self.generate_default_index()
    }

    /// Generate a default marketplace index with built-in skills
    fn generate_default_index(&self) -> MarketplaceIndex {
        let mut entries = Vec::new();

        for (name, desc, _path) in BUILTIN_SKILLS {
            entries.push(MarketplaceEntry {
                name: name.to_string(),
                version: "1.0.0".to_string(),
                description: desc.to_string(),
                author: "starcode".to_string(),
                extension_type: ExtensionType::Skill,
                source: format!("builtin:{}", name),
                tags: vec!["builtin".to_string(), "skill".to_string()],
                downloads: 0,
                stars: 0,
                featured: true,
            });
        }

        // Add some common MCP servers
        let common_mcp = vec![
            ("filesystem", "Filesystem operations MCP server", "@modelcontextprotocol/server-filesystem"),
            ("github", "GitHub API MCP server", "@modelcontextprotocol/server-github"),
            ("memory", "In-memory knowledge graph", "@modelcontextprotocol/server-memory"),
            ("brave-search", "Brave Search MCP server", "@anthropic/brave-search-mcp"),
            ("puppeteer", "Browser automation MCP server", "@anthropic/puppeteer-mcp"),
        ];

        for (name, desc, pkg) in common_mcp {
            entries.push(MarketplaceEntry {
                name: name.to_string(),
                version: "latest".to_string(),
                description: desc.to_string(),
                author: "anthropic".to_string(),
                extension_type: ExtensionType::Mcp,
                source: format!("npm:{}", pkg),
                tags: vec!["mcp".to_string(), "server".to_string()],
                downloads: 0,
                stars: 0,
                featured: false,
            });
        }

        MarketplaceIndex {
            version: 1,
            updated_at: chrono::Utc::now().to_rfc3339(),
            entries,
        }
    }

    /// Save the marketplace index to cache
    pub fn save_index(&self, index: &MarketplaceIndex) -> Result<(), String> {
        if let Some(parent) = self.index_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create directory: {}", e))?;
        }
        let json = serde_json::to_string_pretty(index)
            .map_err(|e| format!("Failed to serialize index: {}", e))?;
        std::fs::write(&self.index_path, json)
            .map_err(|e| format!("Failed to write index: {}", e))?;
        Ok(())
    }

    /// Search extensions by query
    pub fn search(&self, query: &str) -> Vec<MarketplaceEntry> {
        let index = self.load_index();
        let query_lower = query.to_lowercase();

        index
            .entries
            .iter()
            .filter(|e| {
                e.name.to_lowercase().contains(&query_lower)
                    || e.description.to_lowercase().contains(&query_lower)
                    || e.tags.iter().any(|t| t.to_lowercase().contains(&query_lower))
            })
            .cloned()
            .collect()
    }

    /// List all available extensions
    pub fn list_all(&self) -> Vec<MarketplaceEntry> {
        self.load_index().entries
    }

    /// List extensions by type
    pub fn list_by_type(&self, ext_type: &ExtensionType) -> Vec<MarketplaceEntry> {
        self.load_index()
            .entries
            .iter()
            .filter(|e| &e.extension_type == ext_type)
            .cloned()
            .collect()
    }

    /// List featured extensions
    pub fn list_featured(&self) -> Vec<MarketplaceEntry> {
        self.load_index()
            .entries
            .iter()
            .filter(|e| e.featured)
            .cloned()
            .collect()
    }

    /// Install an extension from the marketplace
    pub async fn install(&self, name: &str) -> Result<InstallResult, String> {
        let index = self.load_index();
        let entry = index
            .entries
            .iter()
            .find(|e| e.name == name)
            .ok_or_else(|| format!("Extension '{}' not found in marketplace", name))?;

        match entry.extension_type {
            ExtensionType::Skill => self.install_builtin_skill(entry).await,
            ExtensionType::Plugin => self.install_plugin(entry).await,
            ExtensionType::Mcp => self.install_mcp(entry).await,
        }
    }

    /// Install a built-in skill
    async fn install_builtin_skill(&self, entry: &MarketplaceEntry) -> Result<InstallResult, String> {
        let extensions_dir = ExtensionRegistryManager::global_extensions_dir();
        let skills_dir = extensions_dir.join("skills").join(&entry.name);

        // Check if already installed
        if self.registry.is_installed(&entry.name) {
            return Ok(InstallResult {
                name: entry.name.clone(),
                extension_type: ExtensionType::Skill,
                success: true,
                message: format!("Skill '{}' is already installed", entry.name),
            });
        }

        // For built-in skills, copy from the embedded resources
        if entry.source.starts_with("builtin:") {
            let skill_content = self.get_builtin_skill_content(&entry.name)?;
            std::fs::create_dir_all(&skills_dir)
                .map_err(|e| format!("Failed to create skill directory: {}", e))?;

            // Write SKILL.md
            std::fs::write(skills_dir.join("SKILL.md"), &skill_content)
                .map_err(|e| format!("Failed to write SKILL.md: {}", e))?;

            // Write manifest
            let manifest = ExtensionManifest {
                name: entry.name.clone(),
                version: entry.version.clone(),
                description: entry.description.clone(),
                author: entry.author.clone(),
                extension_type: ExtensionType::Skill,
                source: entry.source.clone(),
                installed_at: chrono::Utc::now().timestamp(),
                enabled: true,
                tags: entry.tags.clone(),
                dependencies: Vec::new(),
            };
            let manifest_json = serde_json::to_string_pretty(&manifest)
                .map_err(|e| format!("Failed to serialize manifest: {}", e))?;
            std::fs::write(skills_dir.join("manifest.json"), manifest_json)
                .map_err(|e| format!("Failed to write manifest: {}", e))?;

            // Register
            self.registry.register(
                &entry.name,
                ExtensionType::Skill,
                &entry.source,
                &entry.version,
            )?;

            return Ok(InstallResult {
                name: entry.name.clone(),
                extension_type: ExtensionType::Skill,
                success: true,
                message: format!("Skill '{}' installed successfully", entry.name),
            });
        }

        // For git-based skills, clone the repository
        self.install_from_git(entry, &skills_dir).await
    }

    /// Install a plugin
    async fn install_plugin(&self, entry: &MarketplaceEntry) -> Result<InstallResult, String> {
        let extensions_dir = ExtensionRegistryManager::global_extensions_dir();
        let plugin_dir = extensions_dir.join("plugins").join(&entry.name);

        if self.registry.is_installed(&entry.name) {
            return Ok(InstallResult {
                name: entry.name.clone(),
                extension_type: ExtensionType::Plugin,
                success: true,
                message: format!("Plugin '{}' is already installed", entry.name),
            });
        }

        self.install_from_git(entry, &plugin_dir).await
    }

    /// Install an MCP server
    async fn install_mcp(&self, entry: &MarketplaceEntry) -> Result<InstallResult, String> {
        if self.registry.is_installed(&entry.name) {
            return Ok(InstallResult {
                name: entry.name.clone(),
                extension_type: ExtensionType::Mcp,
                success: true,
                message: format!("MCP server '{}' is already installed", entry.name),
            });
        }

        // For npm-based MCP servers, add to mcp.json
        if entry.source.starts_with("npm:") {
            let pkg = entry.source.strip_prefix("npm:").unwrap_or(&entry.source);
            return self.install_mcp_npm(&entry.name, pkg).await;
        }

        // For git-based MCP servers
        let extensions_dir = ExtensionRegistryManager::global_extensions_dir();
        let mcp_dir = extensions_dir.join("mcp").join(&entry.name);
        self.install_from_git(entry, &mcp_dir).await
    }

    /// Install from git repository
    async fn install_from_git(
        &self,
        entry: &MarketplaceEntry,
        target_dir: &Path,
    ) -> Result<InstallResult, String> {
        let source = if entry.source.starts_with("builtin:") {
            return Err("Cannot install built-in from git".to_string());
        } else {
            &entry.source
        };

        // Clone the repository
        let output = tokio::process::Command::new("git")
            .args(&["clone", "--depth", "1", source, &target_dir.to_string_lossy()])
            .output()
            .await
            .map_err(|e| format!("Failed to run git: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Git clone failed: {}", stderr));
        }

        // Write manifest
        let manifest = ExtensionManifest {
            name: entry.name.clone(),
            version: entry.version.clone(),
            description: entry.description.clone(),
            author: entry.author.clone(),
            extension_type: entry.extension_type.clone(),
            source: entry.source.clone(),
            installed_at: chrono::Utc::now().timestamp(),
            enabled: true,
            tags: entry.tags.clone(),
            dependencies: Vec::new(),
        };
        let manifest_json = serde_json::to_string_pretty(&manifest)
            .map_err(|e| format!("Failed to serialize manifest: {}", e))?;
        std::fs::write(target_dir.join("manifest.json"), manifest_json)
            .map_err(|e| format!("Failed to write manifest: {}", e))?;

        // Register
        self.registry.register(
            &entry.name,
            entry.extension_type.clone(),
            &entry.source,
            &entry.version,
        )?;

        Ok(InstallResult {
            name: entry.name.clone(),
            extension_type: entry.extension_type.clone(),
            success: true,
            message: format!("{} '{}' installed successfully", entry.extension_type, entry.name),
        })
    }

    /// Install MCP server via npm
    async fn install_mcp_npm(&self, name: &str, package: &str) -> Result<InstallResult, String> {
        // Add to .star/mcp.json
        let mcp_config_path = PathBuf::from(".star").join("mcp.json");
        let mut config: serde_json::Value = if mcp_config_path.exists() {
            let content = std::fs::read_to_string(&mcp_config_path)
                .map_err(|e| format!("Failed to read mcp.json: {}", e))?;
            serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
        } else {
            serde_json::json!({})
        };

        // Ensure mcpServers object exists
        if config.get("mcpServers").is_none() {
            config["mcpServers"] = serde_json::json!({});
        }

        // Add the server
        config["mcpServers"][name] = serde_json::json!({
            "command": "npx",
            "args": ["-y", package],
            "disabled": false
        });

        // Write back
        let json = serde_json::to_string_pretty(&config)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;
        std::fs::create_dir_all(mcp_config_path.parent().unwrap_or(&PathBuf::from(".")))
            .map_err(|e| format!("Failed to create directory: {}", e))?;
        std::fs::write(&mcp_config_path, json)
            .map_err(|e| format!("Failed to write mcp.json: {}", e))?;

        // Register
        self.registry.register(
            name,
            ExtensionType::Mcp,
            &format!("npm:{}", package),
            "latest",
        )?;

        Ok(InstallResult {
            name: name.to_string(),
            extension_type: ExtensionType::Mcp,
            success: true,
            message: format!(
                "MCP server '{}' installed (package: {}). Restart to activate.",
                name, package
            ),
        })
    }

    /// Uninstall an extension
    pub fn uninstall(&self, name: &str) -> Result<InstallResult, String> {
        let entry = self
            .registry
            .get(name)
            .ok_or_else(|| format!("Extension '{}' not found", name))?;

        let ext_dir = self.registry.extension_dir(name, &entry.extension_type);

        // Remove directory
        if ext_dir.exists() {
            std::fs::remove_dir_all(&ext_dir)
                .map_err(|e| format!("Failed to remove extension directory: {}", e))?;
        }

        // For MCP, also remove from mcp.json
        if entry.extension_type == ExtensionType::Mcp {
            self.remove_mcp_config(name)?;
        }

        // Unregister
        self.registry.unregister(name)?;

        Ok(InstallResult {
            name: name.to_string(),
            extension_type: entry.extension_type,
            success: true,
            message: format!("Extension '{}' uninstalled", name),
        })
    }

    /// Remove MCP server from mcp.json
    fn remove_mcp_config(&self, name: &str) -> Result<(), String> {
        let mcp_config_path = PathBuf::from(".star").join("mcp.json");
        if !mcp_config_path.exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(&mcp_config_path)
            .map_err(|e| format!("Failed to read mcp.json: {}", e))?;
        let mut config: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse mcp.json: {}", e))?;

        if let Some(servers) = config.get_mut("mcpServers") {
            if let Some(obj) = servers.as_object_mut() {
                obj.remove(name);
            }
        }

        let json = serde_json::to_string_pretty(&config)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;
        std::fs::write(&mcp_config_path, json)
            .map_err(|e| format!("Failed to write mcp.json: {}", e))?;

        Ok(())
    }

    /// Get built-in skill content
    fn get_builtin_skill_content(&self, name: &str) -> Result<String, String> {
        let skills: std::collections::HashMap<&str, &str> = vec![
            ("analyzer", include_str!("../../agent/skills/resources/analyzer_skill.md")),
            ("editor", include_str!("../../agent/skills/resources/editor_skill.md")),
            ("Grep", include_str!("../../agent/skills/resources/search_skill.md")),
            ("navigator", include_str!("../../agent/skills/resources/navigator_skill.md")),
            ("auto_fix", include_str!("../../agent/skills/resources/auto_fix_skill.md")),
            ("verify", include_str!("../../agent/skills/resources/verify_skill.md")),
        ]
        .into_iter()
        .collect();

        skills
            .get(name)
            .map(|s| s.to_string())
            .ok_or_else(|| format!("Unknown built-in skill: {}", name))
    }
}
