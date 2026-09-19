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

/// 把任意文本压成一个能当配置 key 的 provider id：小写、非 `[a-z0-9_-]` 的字符
/// 折叠成单个 `-`、首尾不留 `-`。输入是空白时返回 `None`。
fn slugify(value: &str) -> Option<String> {
    let mut result = String::new();
    let mut prev_dash = true; // 开头的 `-` 要跳过，所以先当"上一个已经是 -"
    for ch in value.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            result.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            result.push('-');
            prev_dash = true;
        }
    }
    while result.ends_with('-') {
        result.pop();
    }
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// 从 URL 里取出 `host[:port]`，再压成 slug。解析不出 host 就返回 `None`。
fn host_slug(url: &str) -> Option<String> {
    let rest = url
        .trim()
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(url.trim());
    // 去掉 path / query，只留 authority
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    slugify(authority)
}

/// 由名称（优先）或 base URL 的 host 派生 provider id。
///
/// 表单里用户只填一次名称，id 不再单独问。名称留空时退回 URL host，
/// 比如 `http://localhost:1234/v1` → `localhost-1234`。两者都没有就给个兜底。
pub fn derive_provider_id(name: &str, base_url: &str) -> String {
    slugify(name)
        .or_else(|| host_slug(base_url))
        .unwrap_or_else(|| "custom-provider".to_string())
}

/// 当前 Unix 秒，用作新 provider 的 `order`。时钟取不到就退回 0（老条目也是 0，
/// 排序时同级，顺序由 HashMap 迭代决定——只在取不到时间这种极端情况下发生）。
fn now_order() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
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
                    loaded_from_user_settings = true;
                } else {
                    // 两种格式都解析失败：文件存在但内容坏了。不能静默继续——
                    // 否则用户只会看到 "API key required"，不知道配置文件本身
                    // 已经损坏。记录到 agent.log，让 /doctor 与日志能指出来。
                    crate::utils::logging::append_agent_log_line(&format!(
                        "[CONFIG] Failed to parse {}: not valid as user-settings or providers.json. Provider config reset to defaults.",
                        self.global_config_path.display()
                    ));
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
                            order: None,
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
                                order: None,
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
                file_filtering: None,
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
                file_filtering: None,
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
                    order: None,
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
                    order: None,
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
                    order: None,
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
                    order: None,
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
                    order: None,
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

        let builtin_ids: HashSet<&str> = crate::core::config::providers::ALL_PROVIDERS
            .iter()
            .map(|p| p.id)
            .collect();

        let is_configured = |id: &str, settings: Option<&ProviderSettings>| {
            crate::core::config::providers::provider_is_configured(id, settings, active_provider_id)
        };

        // 自定义 provider：order 降序（新的在前，老条目 None 当 0 垫底）。
        // 调用方（provider 面板）把整段自定义 provider 放在内置之前，所以"刚加的"
        // 一定排在列表最上面。
        let mut custom: Vec<(u64, String)> = config
            .providers
            .iter()
            .filter(|(id, _)| !builtin_ids.contains(id.as_str()))
            .filter(|(id, settings)| is_configured(id, Some(settings)))
            .map(|(id, settings)| (settings.order.unwrap_or(0), id.clone()))
            .collect();
        custom.sort_by(|(a, _), (b, _)| b.cmp(a));

        // 内置 provider 按声明顺序排，顺序稳定、可预测
        let builtin: Vec<String> = crate::core::config::providers::ALL_PROVIDERS
            .iter()
            .map(|p| p.id.to_string())
            .filter(|id| is_configured(id, config.providers.get(id.as_str())))
            .collect();

        // 两段按是否内置分过桶，不会重叠
        Ok(custom
            .into_iter()
            .map(|(_, id)| id)
            .chain(builtin)
            .collect())
    }

    /// 解决 id 冲突：和内置 id 或已存在的自定义 key 撞了就追加 `-2` / `-3` ……
    pub async fn resolve_unique_provider_id(&self, base_id: &str) -> String {
        let config = self.load().await.unwrap_or_default();
        resolve_provider_id_conflicts(base_id, &config)
    }

    /// 一次 load+save 写完一个自定义 provider 的全部字段。
    ///
    /// 取代以前散在 UI 代码里的 set_base_url / set_api_key / load-entry-save 三段式：
    /// 那种写法要读写配置三四遍，而且 `order`（新 provider 置顶用的）没法在
    /// 分散的几次调用里原子地塞进去。
    pub async fn add_custom_provider(
        &self,
        provider_id: &str,
        name: Option<&str>,
        provider_type: &str,
        base_url: Option<&str>,
        api_key: Option<&str>,
        selected_model: Option<&str>,
    ) -> Result<(), String> {
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
                    order: None,
                });

        settings.name = normalize_optional_string(name.map(str::to_string));
        settings.r#type = Some(provider_type.to_string());
        settings.base_url = normalize_optional_string(base_url.map(str::to_string));
        settings.api_key = normalize_api_key(api_key.map(str::to_string));
        settings.selected_model = normalize_model_name(selected_model.map(str::to_string));
        // 只有在还没有 order 的情况下打时间戳：重新配置一个已存在的 provider
        // 不该改变它在列表里的位置。
        if settings.order.is_none() {
            settings.order = Some(now_order());
        }

        self.save(&config).await
    }
}

/// `resolve_unique_provider_id` 的纯函数部分，方便单测（不用碰磁盘）。
fn resolve_provider_id_conflicts(base_id: &str, config: &ProviderConfig) -> String {
    let taken = |id: &str| {
        crate::core::config::providers::get_provider_by_id(id).is_some()
            || config.providers.contains_key(id)
    };

    if !taken(base_id) {
        return base_id.to_string();
    }

    for suffix in 2..u64::MAX {
        let candidate = format!("{}-{}", base_id, suffix);
        if !taken(&candidate) {
            return candidate;
        }
    }

    // 几亿个 id 都撞了——实际到不了这里，但函数必须有个终点
    format!("{}-conflict", base_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::models::ProviderConfig;

    #[test]
    fn derive_id_prefers_the_name() {
        assert_eq!(derive_provider_id("My LM Studio", ""), "my-lm-studio");
        assert_eq!(derive_provider_id("work-api", ""), "work-api");
    }

    #[test]
    fn derive_id_falls_back_to_url_host() {
        // 没有名称时从 URL 取 host[:port]，path/query 不要
        assert_eq!(
            derive_provider_id("", "http://localhost:1234/v1"),
            "localhost-1234"
        );
        assert_eq!(
            derive_provider_id("", "https://api.example.com/chat/completions"),
            "api-example-com"
        );
        assert_eq!(
            derive_provider_id("   ", "https://gateway.io/?x=1"),
            "gateway-io"
        );
    }

    #[test]
    fn derive_id_collapses_junk_characters() {
        // 非 [a-z0-9_-] 折叠成单个 -，首尾不留 -
        assert_eq!(derive_provider_id("My!!!Provider###", ""), "my-provider");
        assert_eq!(derive_provider_id("---weird---", ""), "weird");
        assert_eq!(derive_provider_id("provider_2", ""), "provider_2");
    }

    #[test]
    fn derive_id_has_a_last_resort() {
        assert_eq!(derive_provider_id("", ""), "custom-provider");
    }

    #[test]
    fn conflict_with_builtin_gets_a_suffix() {
        // anthropic 是内置 id，得换个名字
        let config = ProviderConfig::default();
        let resolved = resolve_provider_id_conflicts("anthropic", &config);
        assert_eq!(resolved, "anthropic-2");
    }

    #[test]
    fn conflict_with_existing_custom_gets_next_free_suffix() {
        let mut config = ProviderConfig::default();
        config
            .providers
            .insert("work-api".to_string(), ProviderSettings::default());
        config
            .providers
            .insert("work-api-2".to_string(), ProviderSettings::default());

        assert_eq!(
            resolve_provider_id_conflicts("work-api", &config),
            "work-api-3"
        );
        // 没冲突时原样返回
        assert_eq!(
            resolve_provider_id_conflicts("other-api", &config),
            "other-api"
        );
    }

    #[test]
    fn empty_config_resolves_any_fresh_id() {
        let config = ProviderConfig::default();
        assert_eq!(
            resolve_provider_id_conflicts("fresh-id", &config),
            "fresh-id"
        );
    }
}
