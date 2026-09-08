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
        Self::from_file_path(home_dir.join(".star").join(TRUSTED_FOLDERS_FILENAME))
    }

    fn from_file_path(file_path: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
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

    /// 允许工具测试使用隔离的 trust 配置，避免触碰用户的 `~/.star`。
    #[cfg(test)]
    pub(crate) fn new_for_test(file_path: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        Self::from_file_path(file_path)
    }

    #[cfg(test)]
    pub(crate) fn config_for_test(&self) -> TrustedFoldersConfig {
        self.config
            .read()
            .expect("trusted folders test lock poisoned")
            .clone()
    }

    pub fn is_path_trusted(&self, location: &Path) -> Option<bool> {
        let config = self.config.read().ok()?;
        let mut trusted_paths = Vec::new();
        let mut untrusted_paths = Vec::new();

        for (path_str, trust_level) in &config.config {
            let path = Self::normalize_absolute_path(&PathBuf::from(path_str)).ok()?;
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

        let normalized_location = Self::normalize_absolute_path(location).ok()?;

        for trusted_path in &trusted_paths {
            if normalized_location.starts_with(trusted_path) {
                return Some(true);
            }
        }

        for untrusted_path in &untrusted_paths {
            if normalized_location == *untrusted_path {
                return Some(false);
            }
        }

        None
    }

    fn normalize_absolute_path(path: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        Ok(crate::core::utils::paths::normalize_path(&absolute))
    }

    pub fn set_trust_level(
        &self,
        path: &Path,
        trust_level: TrustLevel,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let normalized_path = Self::normalize_absolute_path(path)?;
        let path_str = normalized_path.to_string_lossy().to_string();

        let mut config = self
            .config
            .write()
            .map_err(|_| "Failed to acquire write lock")?;
        let mut updated_config = config.clone();
        updated_config.config.insert(path_str, trust_level);
        self.save_config(&updated_config)?;
        *config = updated_config;
        Ok(())
    }

    fn save_config(&self, config: &TrustedFoldersConfig) -> Result<(), Box<dyn std::error::Error>> {
        let content = serde_json::to_string_pretty(config)?;

        if let Some(parent) = self.file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(&self.file_path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_roots_and_lookup_paths_share_a_normalized_identity() {
        let temp = tempfile::tempdir().expect("创建临时目录失败");
        let trusted_folders = TrustedFolders::new_for_test(temp.path().join("trusted.json"))
            .expect("创建隔离 trust 配置失败");
        let stored_root = temp.path().join("workspace/nested/..");
        let location = temp.path().join("workspace/src/../src/file.rs");

        trusted_folders
            .set_trust_level(&stored_root, TrustLevel::TrustFolder)
            .expect("保存 trust 配置失败");

        assert_eq!(trusted_folders.is_path_trusted(&location), Some(true));
        let expected_root =
            crate::core::utils::paths::normalize_path(&temp.path().join("workspace"));
        assert!(trusted_folders
            .config_for_test()
            .config
            .contains_key(expected_root.to_string_lossy().as_ref()));
    }

    #[test]
    fn failed_save_does_not_grant_in_memory_trust() {
        let temp = tempfile::tempdir().expect("创建临时目录失败");
        let blocked_parent = temp.path().join("not-a-directory");
        std::fs::write(&blocked_parent, "file").expect("创建阻塞文件失败");
        let trusted_folders = TrustedFolders::new_for_test(blocked_parent.join("trusted.json"))
            .expect("创建隔离 trust 配置失败");
        let target = temp.path().join("workspace/file.rs");

        assert!(trusted_folders
            .set_trust_level(&temp.path().join("workspace"), TrustLevel::TrustFolder)
            .is_err());
        assert_ne!(trusted_folders.is_path_trusted(&target), Some(true));
        assert!(trusted_folders.config_for_test().config.is_empty());
    }
}
