use reqwest;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio;

use crate::core::config::json_with_comments::parse_json_with_comments;
use crate::core::config::models::ProviderSettings;
use crate::core::config::providers::DEFAULT_OPENAI_BASE_URL;
use crate::core::utils::paths::find_project_file_upwards;

/// Current settings version - increment this when adding new models or changing settings structure
/// This triggers automatic migration for existing users
const SETTINGS_VERSION: u32 = 2;

/// 设置源类型
/// 
/// 对标claude-code-main的SettingSource
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SettingSource {
    /// 用户设置（全局）
    UserSettings,
    /// 项目设置（共享）
    ProjectSettings,
    /// 本地设置（git忽略）
    LocalSettings,
    /// 标志设置（CLI参数）
    FlagSettings,
    /// 策略设置（托管设置）
    PolicySettings,
}

impl SettingSource {
    /// 获取源名称
    pub fn name(&self) -> &str {
        match self {
            SettingSource::UserSettings => "user",
            SettingSource::ProjectSettings => "project",
            SettingSource::LocalSettings => "local",
            SettingSource::FlagSettings => "flag",
            SettingSource::PolicySettings => "policy",
        }
    }

    /// 获取显示名称
    pub fn display_name(&self) -> &str {
        match self {
            SettingSource::UserSettings => "User Settings",
            SettingSource::ProjectSettings => "Project Settings",
            SettingSource::LocalSettings => "Local Settings",
            SettingSource::FlagSettings => "CLI Flag",
            SettingSource::PolicySettings => "Managed Settings",
        }
    }
}

/// 设置变更事件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsChangeEvent {
    /// 变更的源
    pub source: SettingSource,
    /// 变更的键
    pub key: String,
    /// 旧值
    pub old_value: Option<serde_json::Value>,
    /// 新值
    pub new_value: Option<serde_json::Value>,
    /// 时间戳
    pub timestamp: i64,
}

/// 设置缓存
/// 
/// 对标claude-code-main的settingsCache.ts
pub struct SettingsCache {
    /// 缓存的设置
    cache: HashMap<String, CachedSettings>,
    /// 缓存TTL（秒）
    ttl_secs: u64,
}

/// 缓存的设置
#[derive(Debug, Clone)]
struct CachedSettings {
    /// 设置数据
    data: serde_json::Value,
    /// 缓存时间
    cached_at: i64,
    /// 源
    source: SettingSource,
}

impl SettingsCache {
    /// 创建新的设置缓存
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            cache: HashMap::new(),
            ttl_secs,
        }
    }

    /// 获取缓存的设置
    pub fn get(&self, key: &str) -> Option<&CachedSettings> {
        self.cache.get(key).and_then(|cached| {
            let now = chrono::Utc::now().timestamp();
            if now - cached.cached_at < self.ttl_secs as i64 {
                Some(cached)
            } else {
                None
            }
        })
    }

    /// 设置缓存
    pub fn set(&mut self, key: String, data: serde_json::Value, source: SettingSource) {
        let cached = CachedSettings {
            data,
            cached_at: chrono::Utc::now().timestamp(),
            source,
        };
        self.cache.insert(key, cached);
    }

    /// 清除缓存
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// 清除过期缓存
    pub fn clear_expired(&mut self) {
        let now = chrono::Utc::now().timestamp();
        self.cache.retain(|_, cached| {
            now - cached.cached_at < self.ttl_secs as i64
        });
    }
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct UserSettings {
    #[serde(alias = "apiKey")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(alias = "baseUrl", alias = "baseURL")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(alias = "defaultModel")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<String>>,
    #[serde(alias = "settingsVersion")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings_version: Option<u32>,
    #[serde(
        alias = "isOpenAICompatible",
        alias = "is_openai_compatible",
        alias = "openaiCompatible"
    )]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_openai_compatible: Option<bool>,

    // UI language (e.g. "en-US", "zh-CN", or "auto")
    #[serde(alias = "uiLanguage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui_language: Option<String>,

    // Provider configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub providers: Option<HashMap<String, ProviderSettings>>,
    #[serde(alias = "activeProviderId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_provider_id: Option<String>,
    #[serde(alias = "activeModel")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_model: Option<String>,

    // Sandbox configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<SandboxSettings>,

    // Thinking/reasoning effort level (off, low, medium, high)
    #[serde(alias = "thinkingEffort")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_effort: Option<String>,

    // Output style preference (default, concise, verbose)
    #[serde(alias = "outputStyle")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_style: Option<String>,

    // Context window override (tokens, e.g. 1000000)
    #[serde(alias = "contextWindow")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
}

/// Sandbox settings for user-level configuration
#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct SandboxSettings {
    /// Enable sandbox mode
    #[serde(default)]
    pub enabled: bool,
    /// Sandbox mode (bubblewrap, seatbelt, docker, opensandbox)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Network configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<SandboxNetworkSettings>,
    /// Filesystem rules
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filesystem: Option<SandboxFilesystemSettings>,
    /// OpenSandbox specific settings
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opensandbox: Option<OpenSandboxSettings>,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct SandboxNetworkSettings {
    /// Default action for unmatched connections (true = allow, false = deny)
    #[serde(default = "default_network_action")]
    pub default_action: bool,
    /// Allowed domains
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    /// Denied domains
    #[serde(default)]
    pub denied_domains: Vec<String>,
    /// Allow localhost connections
    #[serde(default = "default_true")]
    pub allow_localhost: bool,
}

fn default_network_action() -> bool {
    false
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct SandboxFilesystemSettings {
    /// Paths to allow read access
    #[serde(default)]
    pub allow_read: Vec<String>,
    /// Paths to allow write access
    #[serde(default)]
    pub allow_write: Vec<String>,
    /// Paths to deny access
    #[serde(default)]
    pub deny: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct OpenSandboxSettings {
    /// Server URL
    pub server_url: String,
    /// API key
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Container image
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct ProjectSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(alias = "mcpServers", alias = "mcp_servers")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_servers: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct SystemSettings {
    #[serde(alias = "defaultModel")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct ForceSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

pub struct SettingsManager {
    user_settings_path: PathBuf,
    project_settings_path: PathBuf,
    system_settings_path: PathBuf,
    force_settings_path: PathBuf,
    /// 设置缓存
    cache: SettingsCache,
    /// 变更监听器
    change_listeners: Vec<Box<dyn Fn(&SettingsChangeEvent) + Send + Sync>>,
}

impl SettingsManager {
    pub fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let home_dir = dirs::home_dir().ok_or("Could not find home directory")?;
        let user_settings_path = home_dir.join(".star").join("user-settings.json");
        let cwd = std::env::current_dir()?;
        let project_settings_path =
            find_project_file_upwards(&cwd, &[".star/settings.json", ".star/settings.jsonc"])
                .unwrap_or_else(|| cwd.join(".star").join("settings.json"));

        let system_settings_path = std::env::var("STAR_CLI_SYSTEM_SETTINGS_PATH")
            .map(PathBuf::from)
            .unwrap_or_default();
        let force_settings_path = std::env::var("STAR_CLI_FORCE_SETTINGS_PATH")
            .map(PathBuf::from)
            .unwrap_or_default();

        // Create .star directory in home if it doesn't exist
        if let Some(parent) = user_settings_path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }

        Ok(SettingsManager {
            user_settings_path,
            project_settings_path,
            system_settings_path,
            force_settings_path,
            cache: SettingsCache::new(300), // 5分钟缓存
            change_listeners: Vec::new(),
        })
    }

    pub async fn load_system_settings(
        &self,
    ) -> Result<SystemSettings, Box<dyn std::error::Error + Send + Sync>> {
        if self.system_settings_path.exists() {
            let content = tokio::fs::read_to_string(&self.system_settings_path).await?;
            let settings: SystemSettings = parse_json_with_comments(&content)?;
            Ok(settings)
        } else {
            Ok(SystemSettings {
                default_model: None,
            })
        }
    }

    pub async fn load_force_settings(
        &self,
    ) -> Result<ForceSettings, Box<dyn std::error::Error + Send + Sync>> {
        if self.force_settings_path.exists() {
            let content = tokio::fs::read_to_string(&self.force_settings_path).await?;
            let settings: ForceSettings = parse_json_with_comments(&content)?;
            Ok(settings)
        } else {
            Ok(ForceSettings { model: None })
        }
    }

    pub async fn load_user_settings(
        &self,
    ) -> Result<UserSettings, Box<dyn std::error::Error + Send + Sync>> {
        if self.user_settings_path.exists() {
            let content = tokio::fs::read_to_string(&self.user_settings_path).await?;
            if content.is_empty() {
                return Ok(self.create_default_user_settings());
            }
            let mut settings: UserSettings = parse_json_with_comments(&content)?;

            // Check if migration is needed
            let current_version = settings.settings_version.unwrap_or(1);
            if current_version < SETTINGS_VERSION {
                settings = self.migrate_settings(settings, current_version).await?;
                self.save_user_settings(&settings).await?;
            }

            Ok(settings)
        } else {
            // Create default settings
            let default_settings = self.create_default_user_settings();
            self.save_user_settings(&default_settings).await?;
            Ok(default_settings)
        }
    }

    pub async fn migrate_settings(
        &self,
        settings: UserSettings,
        from_version: u32,
    ) -> Result<UserSettings, Box<dyn std::error::Error + Send + Sync>> {
        let mut migrated = settings;

        // Migration from version 1 to 2: Add new Star 4.1 and Star 4 Fast models
        if from_version < 2 {
            let default_models = self.get_default_models();
            let existing_models: std::collections::HashSet<String> = migrated
                .models
                .clone()
                .unwrap_or_default()
                .into_iter()
                .collect();

            // Add any new models that don't exist in user's current list
            let new_models: Vec<String> = default_models
                .iter()
                .filter(|model| !existing_models.contains(*model))
                .cloned()
                .collect();

            // Prepend new models to the list (newest models first)
            let mut updated_models = new_models;
            updated_models.extend(migrated.models.unwrap_or_default());

            migrated.models = Some(updated_models);
        }

        migrated.settings_version = Some(SETTINGS_VERSION);
        Ok(migrated)
    }

    fn create_default_user_settings(&self) -> UserSettings {
        UserSettings {
            api_key: None,
            base_url: None,
            default_model: None,
            models: Some(self.get_default_models()),
            settings_version: Some(SETTINGS_VERSION),
            is_openai_compatible: Some(false),
            ui_language: None,
            providers: None,
            active_provider_id: None,
            active_model: None,
            sandbox: None,
            thinking_effort: None,
            output_style: None,
            context_window: None,
        }
    }

    fn get_default_models(&self) -> Vec<String> {
        Vec::new()
    }

    pub async fn save_user_settings(
        &self,
        settings: &UserSettings,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(parent) = self.user_settings_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let content = serde_json::to_string_pretty(settings)?;
        tokio::fs::write(&self.user_settings_path, content).await?;
        Ok(())
    }

    pub async fn load_project_settings(
        &self,
    ) -> Result<ProjectSettings, Box<dyn std::error::Error + Send + Sync>> {
        if self.project_settings_path.exists() {
            let content = tokio::fs::read_to_string(&self.project_settings_path).await?;
            let settings: ProjectSettings = parse_json_with_comments(&content)?;
            Ok(settings)
        } else {
            Ok(ProjectSettings::default())
        }
    }

    pub async fn save_project_settings(
        &self,
        settings: &ProjectSettings,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Create .star directory if it doesn't exist
        if let Some(parent) = self.project_settings_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let content = serde_json::to_string_pretty(settings)?;
        tokio::fs::write(&self.project_settings_path, content).await?;
        Ok(())
    }

    pub async fn get_api_key(&self) -> Option<String> {
        // First check environment variable
        if let Ok(api_key) = std::env::var("STAR_API_KEY") {
            return Some(api_key);
        }

        // Then check user settings
        match self.load_user_settings().await {
            Ok(settings) => settings.api_key,
            Err(_) => None,
        }
    }

    pub async fn get_base_url(&self) -> String {
        // First check environment variable
        if let Ok(base_url) = std::env::var("STAR_BASE_URL") {
            return base_url;
        }

        // Then check user settings
        match self.load_user_settings().await {
            Ok(settings) => settings
                .base_url
                .unwrap_or_else(|| DEFAULT_OPENAI_BASE_URL.to_string()),
            Err(_) => DEFAULT_OPENAI_BASE_URL.to_string(),
        }
    }

    pub async fn get_current_model(&self) -> String {
        // 1. First check environment variable
        if let Ok(model) = std::env::var("STAR_MODEL") {
            if !model.trim().is_empty() {
                return model;
            }
        }

        // 2. Check force settings
        if let Ok(force_settings) = self.load_force_settings().await {
            if let Some(model) = force_settings.model {
                return model;
            }
        }

        // 3. Check project-specific model setting
        if let Ok(project_settings) = self.load_project_settings().await {
            if let Some(model) = project_settings.model {
                return model;
            }
        }

        // 4. Then check user's default model
        if let Ok(user_settings) = self.load_user_settings().await {
            if let Some(model) = user_settings.default_model {
                return model;
            }
        }

        // 5. Check system settings
        if let Ok(system_settings) = self.load_system_settings().await {
            if let Some(model) = system_settings.default_model {
                return model;
            }
        }

        // 6. No default model - return empty string
        String::new()
    }

    pub async fn fetch_remote_models(&self) -> Result<Vec<String>, String> {
        let base_url = self.get_base_url().await;
        let client = reqwest::Client::new();
        let url = format!("{}/models", base_url);

        // Try with bearer if available, else unauthenticated
        let api_key_opt = self.get_api_key().await;
        let mut last_err = None;

        for attempt in 0..2 {
            let use_auth = api_key_opt.is_some() && attempt == 0;
            let mut req = client.get(&url).header("Content-Type", "application/json");
            if use_auth {
                req = req.header(
                    "Authorization",
                    format!("Bearer {}", api_key_opt.clone().unwrap()),
                );
            }

            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    last_err = Some(format!("request error (auth={}): {}", use_auth, e));
                    continue;
                }
            };

            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();

            if !status.is_success() {
                last_err = Some(format!(
                    "status {} body {} (auth={})",
                    status, body_text, use_auth
                ));
                // If first attempt with auth failed, retry without auth
                if use_auth {
                    continue;
                } else {
                    continue;
                }
            }

            let json: serde_json::Value = match serde_json::from_str(&body_text) {
                Ok(v) => v,
                Err(e) => {
                    last_err = Some(format!(
                        "parse error {} (auth={}): {}",
                        body_text, use_auth, e
                    ));
                    continue;
                }
            };

            let models = match json.get("data").and_then(|d| d.as_array()) {
                Some(arr) => arr,
                None => {
                    last_err = Some(format!("no data array in response (auth={})", use_auth));
                    continue;
                }
            };

            let ids: Vec<String> = models
                .iter()
                .filter_map(|m| {
                    m.get("id")
                        .and_then(|id| id.as_str())
                        .map(|s| s.to_string())
                })
                .collect();

            if ids.is_empty() {
                last_err = Some(format!("data array empty (auth={})", use_auth));
                continue;
            }

            return Ok(ids);
        }

        Err(last_err.unwrap_or_else(|| "unknown error".to_string()))
    }

    pub async fn update_user_setting<K>(
        &self,
        key: &str,
        value: K,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    where
        K: serde::Serialize,
    {
        let mut settings = self
            .load_user_settings()
            .await
            .unwrap_or_else(|_| self.create_default_user_settings());

        match key {
            "apiKey" => {
                if let Ok(api_key_val) = serde_json::to_value(value) {
                    settings.api_key = api_key_val.as_str().and_then(|s| {
                        if s.is_empty() {
                            None
                        } else {
                            Some(s.to_string())
                        }
                    });
                }
            }
            "baseURL" => {
                if let Ok(base_url_val) = serde_json::to_value(value) {
                    settings.base_url = base_url_val.as_str().and_then(|s| {
                        if s.is_empty() {
                            None
                        } else {
                            Some(s.to_string())
                        }
                    });
                }
            }
            "defaultModel" => {
                if let Ok(default_model_val) = serde_json::to_value(value) {
                    settings.default_model = default_model_val.as_str().map(|s| s.to_string());
                }
            }
            "uiLanguage" | "ui_language" => {
                if let Ok(lang_val) = serde_json::to_value(value) {
                    settings.ui_language = lang_val.as_str().and_then(|s| {
                        if s.trim().is_empty() {
                            None
                        } else {
                            Some(s.to_string())
                        }
                    });
                }
            }
            _ => {}
        }

        self.save_user_settings(&settings).await?;
        Ok(())
    }
}

pub async fn get_settings_manager(
) -> Result<SettingsManager, Box<dyn std::error::Error + Send + Sync>> {
    SettingsManager::new()
}

 