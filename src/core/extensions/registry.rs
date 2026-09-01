/// Extension registry — manages installed extensions.
///
/// The registry is stored at `~/.star/extensions/registry.json` and tracks
/// all installed extensions with their metadata.
use super::types::*;
use std::path::{Path, PathBuf};

pub struct ExtensionRegistryManager {
    registry_path: PathBuf,
    extensions_dir: PathBuf,
}

impl ExtensionRegistryManager {
    pub fn new() -> Self {
        let extensions_dir = Self::global_extensions_dir();
        let registry_path = extensions_dir.join("registry.json");
        Self {
            registry_path,
            extensions_dir,
        }
    }

    /// Get the global extensions directory (~/.star/extensions)
    pub fn global_extensions_dir() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".star")
            .join("extensions")
    }

    /// Get the project extensions directory (.star/extensions)
    pub fn project_extensions_dir() -> PathBuf {
        PathBuf::from(".star").join("extensions")
    }

    /// Get the marketplace directory (~/.star/marketplace)
    pub fn marketplace_dir() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".star")
            .join("marketplace")
    }

    /// Load the registry from disk
    pub fn load(&self) -> ExtensionRegistry {
        if !self.registry_path.exists() {
            return ExtensionRegistry::default();
        }
        match std::fs::read_to_string(&self.registry_path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => ExtensionRegistry::default(),
        }
    }

    /// Save the registry to disk
    pub fn save(&self, registry: &ExtensionRegistry) -> Result<(), String> {
        if let Some(parent) = self.registry_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create directory: {}", e))?;
        }
        let json = serde_json::to_string_pretty(registry)
            .map_err(|e| format!("Failed to serialize registry: {}", e))?;
        std::fs::write(&self.registry_path, json)
            .map_err(|e| format!("Failed to write registry: {}", e))?;
        Ok(())
    }

    /// Register a new extension
    pub fn register(
        &self,
        name: &str,
        extension_type: ExtensionType,
        source: &str,
        version: &str,
    ) -> Result<(), String> {
        let mut registry = self.load();

        // Check if already registered
        if registry.extensions.iter().any(|e| e.name == name) {
            return Err(format!("Extension '{}' is already registered", name));
        }

        let entry = ExtensionRegistryEntry {
            name: name.to_string(),
            extension_type,
            source: source.to_string(),
            installed_at: chrono::Utc::now().timestamp(),
            enabled: true,
            version: version.to_string(),
        };

        registry.extensions.push(entry);
        self.save(&registry)
    }

    /// Unregister an extension
    pub fn unregister(&self, name: &str) -> Result<(), String> {
        let mut registry = self.load();
        let original_len = registry.extensions.len();
        registry.extensions.retain(|e| e.name != name);
        if registry.extensions.len() == original_len {
            return Err(format!("Extension '{}' not found", name));
        }
        self.save(&registry)
    }

    /// Set extension enabled/disabled
    pub fn set_enabled(&self, name: &str, enabled: bool) -> Result<(), String> {
        let mut registry = self.load();
        if let Some(entry) = registry.extensions.iter_mut().find(|e| e.name == name) {
            entry.enabled = enabled;
            self.save(&registry)
        } else {
            Err(format!("Extension '{}' not found", name))
        }
    }

    /// Check if an extension is installed
    pub fn is_installed(&self, name: &str) -> bool {
        let registry = self.load();
        registry.extensions.iter().any(|e| e.name == name)
    }

    /// Get extension info
    pub fn get(&self, name: &str) -> Option<ExtensionRegistryEntry> {
        let registry = self.load();
        registry.extensions.iter().find(|e| e.name == name).cloned()
    }

    /// List all extensions of a given type
    pub fn list_by_type(&self, ext_type: &ExtensionType) -> Vec<ExtensionRegistryEntry> {
        let registry = self.load();
        registry
            .extensions
            .iter()
            .filter(|e| &e.extension_type == ext_type)
            .cloned()
            .collect()
    }

    /// List all installed extensions
    pub fn list_all(&self) -> Vec<ExtensionRegistryEntry> {
        let registry = self.load();
        registry.extensions
    }

    /// Get the extension directory for a given extension
    pub fn extension_dir(&self, name: &str, ext_type: &ExtensionType) -> PathBuf {
        let type_dir = match ext_type {
            ExtensionType::Skill => "skills",
            ExtensionType::Plugin => "plugins",
            ExtensionType::Mcp => "mcp",
        };
        self.extensions_dir.join(type_dir).join(name)
    }

    /// Discover extensions from disk that are not in the registry
    pub fn discover_from_disk(&self) -> Vec<ExtensionManifest> {
        let mut discovered = Vec::new();

        for type_dir in &["skills", "plugins", "mcp"] {
            let dir = self.extensions_dir.join(type_dir);
            if !dir.exists() {
                continue;
            }
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let manifest_path = entry.path().join("manifest.json");
                    if manifest_path.exists() {
                        if let Ok(content) = std::fs::read_to_string(&manifest_path) {
                            if let Ok(manifest) =
                                serde_json::from_str::<ExtensionManifest>(&content)
                            {
                                discovered.push(manifest);
                            }
                        }
                    }
                }
            }
        }

        discovered
    }
}
