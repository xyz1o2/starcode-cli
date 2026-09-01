use std::collections::HashSet;
use std::path::PathBuf;
use tokio::fs;

use crate::core::config::json_with_comments::parse_json_with_comments;
use crate::core::config::models::{ProviderConfig, ProviderSettings};
use crate::core::config::settings_manager::UserSettings;
use crate::core::config::storage::Storage;
use crate::core::utils::paths::find_project_file_upwards;

pub struct ProviderStore {
    global_config_path: PathBuf,
    project_config_path: Option<PathBuf>,
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn normalize_api_key(value: Option<String>) -> Option<String> {
    crate::core::config::providers::normalize_api_key_value(value)
}

fn normalize_model_name(value: Option<String>) -> Option<String> {
    normalize_optional_string(value)
}

impl ProviderStore {
    pub fn new() -> Self {
        // Unified Config: Use user-settings.json instead of providers.json
        let global_config_path = Storage::global_star_dir().join("user-settings.json");

        // Prefer the nearest project-level overrides, similar to opencode's find-up behavior.
        let project_config_path = std::env::current_dir().ok().and_then(|cwd| {
            find_project_file_upwards(
                &cwd,
                &[
                    ".star/provider.jsonc",
                    ".star/provider.json",
                    ".starcode/config.json",
                ],
            )
        });

        Self {
            global_config_path,
            project_config_path,
        }
    }

    pub async fn load(&self) -> Result<ProviderConfig, String> {
        let mut config = ProviderConfig::default();
        let legacy_path = Storage::global_star_dir().join("providers.json");
        let mut loaded_from_user_settings = false;

        // 1. Load Global Config (from user-settings.json)
        if self.global_config_path.exists() {
            let content = fs::read_to_string(&self.global_config_path)
                .await
                .map_err(|e| format!("Failed to read global config: {}", e))?;

            // Try to parse as UserSettings (Unified Config)
            if let Ok(user_settings) = parse_json_with_comments::<UserSettings>(&content) {
                if let Some(providers) = user_settings.providers {
                    config.providers = providers;
                    loaded_from_user_settings = true;
                }
                config.active_provider_id = user_settings.active_provider_id;
                config.active_model = user_settings.active_model;
            } else {
                // Fallback: Try to parse as old ProviderConfig (providers.json format)
                if let Ok(old_config) = parse_json_with_comments::<ProviderConfig>(&content) {
                    config = old_config;
                }
            }
        }

        // 2. Migration: If no providers loaded from user-settings, try legacy providers.json
        if !loaded_from_user_settings && legacy_path.exists() {
            if let Ok(content) = fs::read_to_string(&legacy_path).await {
                if let Ok(legacy_config) = parse_json_with_comments::<ProviderConfig>(&content) {
                    // Merge legacy config
                    config.providers = legacy_config.providers;
                    if config.active_provider_id.is_none() {
                        config.active_provider_id = legacy_config.active_provider_id;
                    }
                    if config.active_model.is_none() {
                        config.active_model = legacy_config.active_model;
                    }

                    // Auto-save to user-settings.json to complete migration
                    let _ = self.save(&config).await;

                    // Rename legacy file to avoid confusion/re-reading
                    let _ = fs::rename(&legacy_path, legacy_path.with_extension("json.bak")).await;
                }
            }
        }

        // 3. Load and Merge Project Config (Overrides Global)
        if let Some(path) = &self.project_config_path {
            if path.exists() {
                let content = fs::read_to_string(path)
                    .await
                    .map_err(|e| format!("Failed to read project config: {}", e))?;

                if let Ok(project_config) = parse_json_with_comments::<ProviderConfig>(&content) {
                    // Merge active_provider_id
                    if let Some(id) = project_config.active_provider_id {
                        config.active_provider_id = Some(id);
                    }

                    // Merge active_model
                    if let Some(model) = project_config.active_model {
                        config.active_model = Some(model);
                    }

                    // Merge providers
                    for (id, settings) in project_config.providers {
                        let entry = config.providers.entry(id).or_insert(ProviderSettings {
                            api_key: None,
                            base_url: None,
                            selected_model: None,
                            models: None,
                            name: None,
                            description: None,
                            r#type: None,
                        });

                        if let Some(key) = settings.api_key {
                            entry.api_key = Some(key);
                        }
                        if let Some(url) = settings.base_url {
                            entry.base_url = Some(url);
                        }
                        if let Some(model) = settings.selected_model {
                            entry.selected_model = Some(model);
                        }
                        if let Some(models) = settings.models {
                            entry.models = Some(models);
                        }
                        if let Some(name) = settings.name {
                            entry.name = Some(name);
                        }
                        if let Some(desc) = settings.description {
                            entry.description = Some(desc);
                        }
                        if let Some(t) = settings.r#type {
                            entry.r#type = Some(t);
                        }
                    }
                }
            }
        }

        // Normalize active_provider_id if it doesn't match any known provider key.
        let mut needs_save = false;
        if let Some(active) = config.active_provider_id.clone() {
            let has_custom = config.providers.contains_key(&active);
            let has_builtin = crate::core::config::providers::get_provider_by_id(&active).is_some();
            if !has_custom && !has_builtin {
                if let Some(normalized) =
                    crate::core::config::providers::normalize_provider_id(&active)
                {
                    config.active_provider_id = Some(normalized);
                    needs_save = true;
                } else if let Some(key) = config
                    .providers
                    .keys()
                    .find(|k| k.eq_ignore_ascii_case(&active))
                {
                    config.active_provider_id = Some(key.clone());
                    needs_save = true;
                }
            }
        }
        if needs_save {
            let _ = self.save(&config).await;
        }

        if let Some(active_provider_id) = config.active_provider_id.clone() {
            let should_migrate_active_model = config
                .providers
                .get(&active_provider_id)
                .and_then(|settings| normalize_model_name(settings.selected_model.clone()))
                .is_none();
            if should_migrate_active_model {
                if let Some(active_model) = normalize_model_name(config.active_model.clone()) {
                    let settings =
                        config
                            .providers
                            .entry(active_provider_id)
                            .or_insert(ProviderSettings {
                                api_key: None,
                                base_url: None,
                                selected_model: None,
                                models: None,
                                name: None,
                                description: None,
                                r#type: None,
                            });
                    settings.selected_model = Some(active_model);
                    let _ = self.save(&config).await;
                }
            }
        }

        Ok(config)
    }

    pub async fn save(&self, config: &ProviderConfig) -> Result<(), String> {
        // We only save to global config (user-settings.json)

        // 1. Read existing UserSettings to preserve other fields
        let mut user_settings = if self.global_config_path.exists() {
            let content = fs::read_to_string(&self.global_config_path)
                .await
                .map_err(|e| format!("Failed to read global config: {}", e))?;
            parse_json_with_comments::<UserSettings>(&content).unwrap_or_else(|_| UserSettings {
                api_key: None,
                base_url: None,
                default_model: None,
                models: None,
                settings_version: Some(2),
                is_openai_compatible: Some(true),
                providers: None,
                active_provider_id: None,
                active_model: None,
                sandbox: None,
                ui_language: None,
                thinking_effort: None,
                output_style: None,
                context_window: None,
            })
        } else {
            // Default if not exists
            UserSettings {
                api_key: None,
                base_url: None,
                default_model: None,
                models: None,
                settings_version: Some(2),
                is_openai_compatible: Some(true),
                providers: None,
                active_provider_id: None,
                active_model: None,
                sandbox: None,
                ui_language: None,
                thinking_effort: None,
                output_style: None,
                context_window: None,
            }
        };

        // 2. Update fields
        user_settings.providers = Some(config.providers.clone());
        user_settings.active_provider_id = config.active_provider_id.clone();
        user_settings.active_model = config.active_model.clone();

        // 3. Save
        let content = serde_json::to_string_pretty(&user_settings)
            .map_err(|e| format!("Failed to serialize user settings: {}", e))?;

        if let Some(parent) = self.global_config_path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create config directory: {}", e))?;
        }

        fs::write(&self.global_config_path, content)
            .await
            .map_err(|e| format!("Failed to write user settings: {}", e))
    }

    pub async fn get_api_key(&self, provider_id: &str) -> Result<Option<String>, String> {
        let config = self.load().await?;
        Ok(normalize_api_key(
            config
                .providers
                .get(provider_id)
                .and_then(|p| p.api_key.clone()),
        ))
    }

    pub async fn get_base_url(&self, provider_id: &str) -> Result<Option<String>, String> {
        let config = self.load().await?;
        Ok(normalize_optional_string(
            config
                .providers
                .get(provider_id)
                .and_then(|p| p.base_url.clone()),
        ))
    }

    pub async fn set_api_key(&self, provider_id: &str, api_key: &str) -> Result<(), String> {
        let mut config = self.load().await?;
        let settings =
            config
                .providers
                .entry(provider_id.to_string())
                .or_insert(ProviderSettings {
                    api_key: None,
                    base_url: None,
                    selected_model: None,
                    models: None,
                    name: None,
                    description: None,
                    r#type: None,
                });
        settings.api_key = normalize_api_key(Some(api_key.to_string()));
        self.save(&config).await
    }

    pub async fn set_base_url(&self, provider_id: &str, base_url: &str) -> Result<(), String> {
        let mut config = self.load().await?;
        let settings =
            config
                .providers
                .entry(provider_id.to_string())
                .or_insert(ProviderSettings {
                    api_key: None,
                    base_url: None,
                    selected_model: None,
                    models: None,
                    name: None,
                    description: None,
                    r#type: None,
                });
        settings.base_url = normalize_optional_string(Some(base_url.to_string()));
        self.save(&config).await
    }

    pub async fn get_selected_model(&self, provider_id: &str) -> Result<Option<String>, String> {
        let config = self.load().await?;
        Ok(normalize_model_name(
            config
                .providers
                .get(provider_id)
                .and_then(|p| p.selected_model.clone()),
        ))
    }

    pub async fn set_selected_model(&self, provider_id: &str, model: &str) -> Result<(), String> {
        let mut config = self.load().await?;
        let settings =
            config
                .providers
                .entry(provider_id.to_string())
                .or_insert(ProviderSettings {
                    api_key: None,
                    base_url: None,
                    selected_model: None,
                    models: None,
                    name: None,
                    description: None,
                    r#type: None,
                });
        let normalized_model = normalize_model_name(Some(model.to_string()));
        settings.selected_model = normalized_model.clone();
        if config.active_provider_id.as_deref() == Some(provider_id) {
            config.active_model = normalized_model;
        }
        self.save(&config).await
    }

    pub async fn set_active_provider(&self, provider_id: &str) -> Result<(), String> {
        let mut config = self.load().await?;
        config.active_provider_id = Some(provider_id.to_string());
        config.active_model = config
            .providers
            .get(provider_id)
            .and_then(|provider| normalize_model_name(provider.selected_model.clone()));
        self.save(&config).await
    }

    pub async fn set_active_model(&self, model: &str) -> Result<(), String> {
        let mut config = self.load().await?;
        config.active_model = Some(model.to_string());
        if let Some(active_provider_id) = config.active_provider_id.clone() {
            let settings = config
                .providers
                .entry(active_provider_id)
                .or_insert(ProviderSettings {
                    api_key: None,
                    base_url: None,
                    selected_model: None,
                    models: None,
                    name: None,
                    description: None,
                    r#type: None,
                });
            settings.selected_model = normalize_model_name(Some(model.to_string()));
        }
        self.save(&config).await
    }

    /// Combined: set both active provider and model in a single load+save cycle
    pub async fn set_active_provider_and_model(
        &self,
        provider_id: &str,
        model: &str,
    ) -> Result<(), String> {
        let mut config = self.load().await?;
        config.active_provider_id = Some(provider_id.to_string());
        config.active_model = Some(model.to_string());
        let settings =
            config
                .providers
                .entry(provider_id.to_string())
                .or_insert(ProviderSettings {
                    api_key: None,
                    base_url: None,
                    selected_model: None,
                    models: None,
                    name: None,
                    description: None,
                    r#type: None,
                });
        settings.selected_model = normalize_model_name(Some(model.to_string()));
        self.save(&config).await
    }

    pub async fn get_active_model(&self) -> Result<Option<String>, String> {
        let config = self.load().await?;
        if let Some(active_provider_id) = config.active_provider_id.as_deref() {
            return Ok(config
                .providers
                .get(active_provider_id)
                .and_then(|provider| normalize_model_name(provider.selected_model.clone())));
        }
        Ok(normalize_model_name(config.active_model))
    }

    pub async fn configured_provider_ids(&self) -> Result<Vec<String>, String> {
        let config = self.load().await?;
        let active_provider_id = config.active_provider_id.as_deref();
        let mut ids = HashSet::new();

        for provider in crate::core::config::providers::ALL_PROVIDERS {
            let settings = config.providers.get(provider.id);
            if crate::core::config::providers::provider_is_configured(
                provider.id,
                settings,
                active_provider_id,
            ) {
                ids.insert(provider.id.to_string());
            }
        }

        for (provider_id, settings) in &config.providers {
            if crate::core::config::providers::provider_is_configured(
                provider_id,
                Some(settings),
                active_provider_id,
            ) {
                ids.insert(provider_id.clone());
            }
        }

        let mut ids: Vec<_> = ids.into_iter().collect();
        ids.sort();
        Ok(ids)
    }
}
