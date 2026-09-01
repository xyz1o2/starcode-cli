/// Extension system types.
///
/// # Extension Manifest Format
///
/// Each extension has a `manifest.json` that describes it:
///
/// ```json
/// {
///   "name": "my-extension",
///   "version": "1.0.0",
///   "description": "A useful extension",
///   "author": "someone",
///   "type": "skill",
///   "source": "https://github.com/owner/repo",
///   "installed_at": 1234567890,
///   "enabled": true,
///   "tags": ["code-analysis", "testing"]
/// }
/// ```
use serde::{Deserialize, Serialize};

/// Extension type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionType {
    Skill,
    Plugin,
    Mcp,
}

impl std::fmt::Display for ExtensionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtensionType::Skill => write!(f, "skill"),
            ExtensionType::Plugin => write!(f, "plugin"),
            ExtensionType::Mcp => write!(f, "mcp"),
        }
    }
}

/// Extension manifest — stored in each extension's directory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(rename = "type")]
    pub extension_type: ExtensionType,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub installed_at: i64,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
}

fn default_true() -> bool {
    true
}

/// Extension registry — tracks all installed extensions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionRegistry {
    pub version: u32,
    pub extensions: Vec<ExtensionRegistryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionRegistryEntry {
    pub name: String,
    #[serde(rename = "type")]
    pub extension_type: ExtensionType,
    pub source: String,
    pub installed_at: i64,
    pub enabled: bool,
    pub version: String,
}

impl Default for ExtensionRegistry {
    fn default() -> Self {
        Self {
            version: 1,
            extensions: Vec::new(),
        }
    }
}

/// Marketplace index entry — describes an available extension
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceEntry {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    #[serde(rename = "type")]
    pub extension_type: ExtensionType,
    pub source: String,
    pub tags: Vec<String>,
    pub downloads: u64,
    pub stars: u64,
    #[serde(default)]
    pub featured: bool,
}

/// Marketplace index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceIndex {
    pub version: u32,
    pub updated_at: String,
    pub entries: Vec<MarketplaceEntry>,
}

/// Install result
#[derive(Debug)]
pub struct InstallResult {
    pub name: String,
    pub extension_type: ExtensionType,
    pub success: bool,
    pub message: String,
}
