use crate::core::policy::types::*;
use std::fs;
use std::path::Path;

pub struct TomlLoader;

impl TomlLoader {
    pub fn load_policy_engine_config(
        path: &Path,
    ) -> Result<PolicyEngineConfig, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(path)?;
        let config: PolicyEngineConfig = toml::from_str(&content)?;
        Ok(config)
    }

    pub fn load_policy_settings(path: &Path) -> Result<PolicySettings, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(path)?;
        let settings: PolicySettings = toml::from_str(&content)?;
        Ok(settings)
    }

    pub fn save_policy_settings(
        path: &Path,
        settings: &PolicySettings,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let content = toml::to_string_pretty(settings)?;
        fs::write(path, content)?;
        Ok(())
    }
}
