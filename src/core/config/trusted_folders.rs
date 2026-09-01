use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

pub const TRUSTED_FOLDERS_FILENAME: &str = "trustedFolders.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TrustLevel {
    #[serde(rename = "TRUST_FOLDER")]
    TrustFolder,
    #[serde(rename = "TRUST_PARENT")]
    TrustParent,
    #[serde(rename = "DO_NOT_TRUST")]
    DoNotTrust,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TrustedFoldersConfig {
    pub config: HashMap<String, TrustLevel>,
}

#[derive(Clone)]
pub struct TrustedFolders {
    config: Arc<RwLock<TrustedFoldersConfig>>,
    file_path: PathBuf,
}

impl TrustedFolders {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let home_dir = dirs::home_dir().ok_or("Could not find home directory")?;
        let file_path = home_dir.join(".star").join(TRUSTED_FOLDERS_FILENAME);

        let config = if file_path.exists() {
            match std::fs::read_to_string(&file_path) {
                Ok(content) => {
                    // Handle potential comments if JSON allows it (standard JSON doesn't, but star-cli stripped comments)
                    // For now, assume standard JSON
                    serde_json::from_str(&content).unwrap_or_default()
                }
                Err(_) => TrustedFoldersConfig::default(),
            }
        } else {
            TrustedFoldersConfig::default()
        };

        Ok(Self {
            config: Arc::new(RwLock::new(config)),
            file_path,
        })
    }

    pub fn is_path_trusted(&self, location: &Path) -> Option<bool> {
        let config = self.config.read().ok()?;
        let mut trusted_paths = Vec::new();
        let mut untrusted_paths = Vec::new();

        for (path_str, trust_level) in &config.config {
            let path = PathBuf::from(path_str);
            match trust_level {
                TrustLevel::TrustFolder => trusted_paths.push(path),
                TrustLevel::TrustParent => {
                    if let Some(parent) = path.parent() {
                        trusted_paths.push(parent.to_path_buf());
                    }
                }
                TrustLevel::DoNotTrust => untrusted_paths.push(path),
            }
        }

        let location_abs = if location.is_absolute() {
            location.to_path_buf()
        } else {
            std::env::current_dir().ok()?.join(location)
        };

        let normalized_location = Self::normalize_path(&location_abs);

        // Check trusted paths
        for trusted_path in &trusted_paths {
            if self.is_within_root(&normalized_location, trusted_path) {
                return Some(true);
            }
        }

        // Check untrusted paths
        for untrusted_path in &untrusted_paths {
            if normalized_location == *untrusted_path {
                return Some(false);
            }
        }

        None
    }

    fn normalize_path(path: &Path) -> PathBuf {
        let mut result = PathBuf::new();
        for component in path.components() {
            match component {
                std::path::Component::Prefix(..) | std::path::Component::RootDir => {
                    result.push(component);
                }
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    result.pop();
                }
                std::path::Component::Normal(c) => {
                    result.push(c);
                }
            }
        }
        result
    }

    fn is_within_root(&self, path: &Path, root: &Path) -> bool {
        path.starts_with(root)
    }

    pub fn set_trust_level(
        &self,
        path: &Path,
        trust_level: TrustLevel,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let path_str = path.to_string_lossy().to_string();

        {
            let mut config = self
                .config
                .write()
                .map_err(|_| "Failed to acquire write lock")?;
            config.config.insert(path_str, trust_level);
        }

        self.save()?;
        Ok(())
    }

    fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let config = self
            .config
            .read()
            .map_err(|_| "Failed to acquire read lock")?;
        let content = serde_json::to_string_pretty(&*config)?;

        if let Some(parent) = self.file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(&self.file_path, content)?;
        Ok(())
    }
}
