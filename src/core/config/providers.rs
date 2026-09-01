use crate::core::config::models::ProviderSettings;
use serde::{Deserialize, Serialize};

pub const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
pub const PLACEHOLDER_API_KEY: &str = "API_KEY_NOT_SET";
pub const PLACEHOLDER_BASE_URL: &str = "BASE_URL_NOT_SET";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProviderCategory {
    Popular,
    Chinese,
    Local,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderMetadata {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub category: ProviderCategory,
    pub default_base_url: Option<&'static str>,
    pub api_key_env_var: Option<&'static str>,
    pub requires_api_key: bool,
    // Models are fetched dynamically from API, no static list
}

pub const ALL_PROVIDERS: &[ProviderMetadata] = &[
    ProviderMetadata {
        id: "anthropic",
        name: "Anthropic",
        description: "Claude Models",
        category: ProviderCategory::Popular,
        default_base_url: Some("https://api.anthropic.com/v1"),
        api_key_env_var: Some("ANTHROPIC_API_KEY"),
        requires_api_key: true,
    },
    ProviderMetadata {
        id: "openai",
        name: "OpenAI",
        description: "GPT Models",
        category: ProviderCategory::Popular,
        default_base_url: Some("https://api.openai.com/v1"),
        api_key_env_var: Some("OPENAI_API_KEY"),
        requires_api_key: true,
    },
    ProviderMetadata {
        id: "deepseek",
        name: "DeepSeek",
        description: "DeepSeek Models",
        category: ProviderCategory::Popular,
        default_base_url: Some("https://api.deepseek.com/v1"),
        api_key_env_var: Some("DEEPSEEK_API_KEY"),
        requires_api_key: true,
    },
    ProviderMetadata {
        id: "minimax",
        name: "MiniMax",
        description: "MiniMax Models",
        category: ProviderCategory::Popular,
        default_base_url: Some("https://api.minimax.chat/v1"),
        api_key_env_var: Some("MINIMAX_API_KEY"),
        requires_api_key: true,
    },
    ProviderMetadata {
        id: "xiaomi",
        name: "Xiaomi (Pay-as-you-go)",
        description: "Xiaomi MiMo Global — pay-as-you-go, mimo-v2-flash etc.",
        category: ProviderCategory::Chinese,
        default_base_url: Some("https://api.xiaomimimo.com/v1"),
        api_key_env_var: Some("XIAOMI_API_KEY"),
        requires_api_key: true,
    },
    ProviderMetadata {
        id: "xiaomi-cn",
        name: "Xiaomi (China  Token Plan)",
        description: "Xiaomi MiMo China — token-plan-cn, mimo-v2.5-pro etc.",
        category: ProviderCategory::Chinese,
        default_base_url: Some("https://token-plan-cn.xiaomimimo.com/v1"),
        api_key_env_var: Some("XIAOMI_API_KEY"),
        requires_api_key: true,
    },
    ProviderMetadata {
        id: "xiaomi-sgp",
        name: "Xiaomi (Singapore  Token Plan)",
        description: "Xiaomi MiMo Asia-Pacific — token-plan-sgp",
        category: ProviderCategory::Chinese,
        default_base_url: Some("https://token-plan-sgp.xiaomimimo.com/v1"),
        api_key_env_var: Some("XIAOMI_API_KEY"),
        requires_api_key: true,
    },
    ProviderMetadata {
        id: "xiaomi-ams",
        name: "Xiaomi (Amsterdam  Token Plan)",
        description: "Xiaomi MiMo Europe — token-plan-ams",
        category: ProviderCategory::Chinese,
        default_base_url: Some("https://token-plan-ams.xiaomimimo.com/v1"),
        api_key_env_var: Some("XIAOMI_API_KEY"),
        requires_api_key: true,
    },
    ProviderMetadata {
        id: "openai-compatible",
        name: "OpenAI Compatible",
        description: "Custom OpenAI-compatible endpoint",
        category: ProviderCategory::Local,
        default_base_url: None,
        api_key_env_var: None,
        requires_api_key: false,
    },
    ProviderMetadata {
        id: "anthropic-compatible",
        name: "Anthropic Compatible",
        description: "Custom Anthropic-compatible endpoint (Claude /v1/messages)",
        category: ProviderCategory::Local,
        default_base_url: None,
        api_key_env_var: None,
        requires_api_key: false,
    },
];

pub fn get_provider_by_id(id: &str) -> Option<&'static ProviderMetadata> {
    ALL_PROVIDERS.iter().find(|p| p.id == id)
}

fn canonical_provider_id(input: &str) -> Option<String> {
    normalize_provider_id(input).or_else(|| {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_lowercase())
        }
    })
}

pub fn provider_requires_manual_base_url(provider_id: &str) -> bool {
    match canonical_provider_id(provider_id).as_deref() {
        Some("openai-compatible" | "anthropic-compatible" | "ollama" | "lmstudio") => true,
        Some(id) if get_provider_by_id(id).is_some() => false,
        Some(_) => true,
        None => true,
    }
}

pub fn provider_forces_openai_compatible(provider_id: &str) -> bool {
    provider_openai_compatible_mode(provider_id).unwrap_or(false)
}

pub fn provider_openai_compatible_mode(provider_id: &str) -> Option<bool> {
    if normalize_provider_id(provider_id).as_deref() == Some("anthropic-compatible") {
        return Some(false);
    }
    normalize_provider_id(provider_id).map(|id| id == "openai-compatible")
}

pub fn resolve_provider_base_url(
    provider_id: &str,
    stored_base_url: Option<String>,
) -> Option<String> {
    stored_base_url
        .and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .or_else(|| {
            canonical_provider_id(provider_id)
                .and_then(|id| get_provider_by_id(&id))
                .and_then(|provider| provider.default_base_url)
                .and_then(|value| {
                    let trimmed = value.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    }
                })
        })
}

const API_KEY_PLACEHOLDERS: &[&str] = &[
    "api-key-not-set",
    "your-api-key",
    "your-api-key-1",
    "your-api-key-2",
    "api-key-here",
    "replace-with-your-api-key",
];

fn option_has_value(value: Option<&str>) -> bool {
    value.is_some_and(|v| !v.trim().is_empty())
}

fn normalize_placeholder_token(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('_', "-")
}

pub fn is_placeholder_api_key(value: &str) -> bool {
    let normalized = normalize_placeholder_token(value);
    API_KEY_PLACEHOLDERS
        .iter()
        .any(|candidate| normalized == *candidate)
}

pub fn normalize_api_key_value(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() || is_placeholder_api_key(trimmed) {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn provider_matches_id(left: &str, right: &str) -> bool {
    match (canonical_provider_id(left), canonical_provider_id(right)) {
        (Some(a), Some(b)) => a == b,
        _ => left.trim().eq_ignore_ascii_case(right.trim()),
    }
}

fn provider_is_configured_with_env_state(
    provider_id: &str,
    settings: Option<&ProviderSettings>,
    active_provider_id: Option<&str>,
    has_env_api_key: bool,
) -> bool {
    let has_saved_api_key = settings
        .and_then(|value| normalize_api_key_value(value.api_key.clone()))
        .is_some();
    let has_saved_base_url = settings
        .and_then(|value| value.base_url.as_deref())
        .is_some_and(|value| !value.trim().is_empty());
    let has_api_key = has_saved_api_key || has_env_api_key;
    let is_active = active_provider_id
        .map(|active_id| provider_matches_id(active_id, provider_id))
        .unwrap_or(false);

    if let Some(metadata) =
        canonical_provider_id(provider_id).and_then(|id| get_provider_by_id(&id))
    {
        if metadata.requires_api_key && !has_api_key {
            return false;
        }

        if provider_requires_manual_base_url(provider_id) {
            if metadata.requires_api_key {
                return has_api_key
                    && (has_saved_base_url || option_has_value(metadata.default_base_url));
            }
            return has_saved_base_url || is_active || has_api_key;
        }

        if metadata.requires_api_key {
            return has_api_key;
        }

        return has_saved_base_url || is_active || has_api_key;
    }

    has_api_key || has_saved_base_url || is_active
}

pub fn provider_is_configured(
    provider_id: &str,
    settings: Option<&ProviderSettings>,
    active_provider_id: Option<&str>,
) -> bool {
    provider_is_configured_with_env_state(
        provider_id,
        settings,
        active_provider_id,
        resolve_api_key_from_env(provider_id).is_some(),
    )
}

/// Normalize provider identifier to a canonical built-in ID when possible.
/// Returns None if the input doesn't match any built-in provider.
pub fn normalize_provider_id(input: &str) -> Option<String> {
    let raw = input.trim();
    if raw.is_empty() {
        return None;
    }

    // Exact ID match (case-insensitive)
    if let Some(p) = ALL_PROVIDERS
        .iter()
        .find(|p| p.id.eq_ignore_ascii_case(raw))
    {
        return Some(p.id.to_string());
    }

    // Display name match (case-insensitive)
    if let Some(p) = ALL_PROVIDERS
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(raw))
    {
        return Some(p.id.to_string());
    }

    // Common aliases and formatting differences
    let compact = raw.to_lowercase().replace([' ', '-', '_'], "");
    match compact.as_str() {
        "lmstudio" => Some("lmstudio".to_string()),
        "openaicompatible" => Some("openai-compatible".to_string()),
        "anthropiccompatible" => Some("anthropic-compatible".to_string()),
        "openrouter" => Some("openrouter".to_string()),
        "kimicode" | "kimi" => Some("kimi-code".to_string()),
        "qwen" | "alibabacloud" | "dashscope" => Some("alibaba".to_string()),
        "doubao" | "bytedance" | "ark" => Some("bytedance".to_string()),
        "glm" | "bigmodel" | "zhipuai" => Some("zhipu".to_string()),
        "opencode" | "opencodezen" | "zen" => Some("opencode".to_string()),
        "xiaomi" | "mimo" | "milm" => Some("xiaomi".to_string()),
        _ => None,
    }
}

fn push_env_var(vars: &mut Vec<&'static str>, value: &'static str) {
    if !vars.iter().any(|v| *v == value) {
        vars.push(value);
    }
}

pub fn api_key_env_candidates(provider_id: &str) -> Vec<&'static str> {
    let mut vars = Vec::new();
    let pid = canonical_provider_id(provider_id).unwrap_or_default();
    if pid == "kimi-code" || pid == "kimi" {
        push_env_var(&mut vars, "KIMI_API_KEY");
    } else if pid == "moonshot" {
        push_env_var(&mut vars, "MOONSHOT_API_KEY");
    }

    if let Some(meta) = get_provider_by_id(&pid) {
        if let Some(env_var) = meta.api_key_env_var {
            push_env_var(&mut vars, env_var);
        }
    }

    // Additional env var aliases for popular CLI tools
    match pid.as_str() {
        "anthropic" => push_env_var(&mut vars, "CLAUDE_API_KEY"),
        "openai" => push_env_var(&mut vars, "CODEX_API_KEY"),
        "google" => push_env_var(&mut vars, "GEMINI_API_KEY"),
        _ => {}
    }

    vars
}

pub fn resolve_api_key_from_env(provider_id: &str) -> Option<String> {
    resolve_api_key_from_env_with_source(provider_id).map(|(value, _)| value)
}

pub fn resolve_api_key_from_env_with_source(provider_id: &str) -> Option<(String, &'static str)> {
    for env_var in api_key_env_candidates(provider_id) {
        if let Ok(key) = std::env::var(env_var) {
            if let Some(normalized) = normalize_api_key_value(Some(key)) {
                return Some((normalized, env_var));
            }
        }
    }
    None
}

pub fn resolve_runtime_api_key_with_source(
    provider_id: Option<&str>,
    configured_api_key: Option<String>,
) -> Option<(String, String)> {
    if let Some(provider_id) = provider_id {
        if let Some((value, env_var)) = resolve_api_key_from_env_with_source(provider_id) {
            return Some((value, format!("provider_env:{env_var}")));
        }
    }

    if let Some(value) = normalize_api_key_value(configured_api_key) {
        return Some((value, "configured".to_string()));
    }

    normalize_api_key_value(std::env::var("STAR_API_KEY").ok())
        .map(|value| (value, "env:STAR_API_KEY".to_string()))
}

pub fn resolve_runtime_api_key(
    provider_id: Option<&str>,
    configured_api_key: Option<String>,
) -> Option<String> {
    resolve_runtime_api_key_with_source(provider_id, configured_api_key).map(|(value, _)| value)
}

pub fn api_key_env_hint(provider_id: &str) -> Option<String> {
    let vars = api_key_env_candidates(provider_id);
    if vars.is_empty() {
        None
    } else {
        Some(vars.join(" or "))
    }
}
