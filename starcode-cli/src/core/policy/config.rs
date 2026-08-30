use crate::core::policy::types::*;
use std::fs;
use std::path::Path;

pub struct PolicyConfig {
    pub settings: PolicySettings,
}

impl PolicyConfig {
    pub fn load_from_file(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(path)?;
        let settings: PolicySettings = toml::from_str(&content)?;
        Ok(Self { settings })
    }

    pub fn load_from_string(content: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let settings: PolicySettings = toml::from_str(content)?;
        Ok(Self { settings })
    }

    pub fn settings(&self) -> &PolicySettings {
        &self.settings
    }
}
