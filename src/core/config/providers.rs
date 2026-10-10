use crate::core::config::models::ProviderSettings;
use serde::{Deserialize, Serialize};

pub const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
pub const PLACEHOLDER_API_KEY: &str = "API_KEY_NOT_SET";
pub const PLACEHOLDER_BASE_URL: &str = "BASE_URL_NOT_SET";

/// OpenCode Zen 网关根地址。Zen 在同一个 base_url 下按模型名分流到不同协议端点，
/// 因此这里不追加任何协议后缀 —— 具体端点由 `zen_protocol_for_model` 决定。
pub const OPENCODE_ZEN_BASE_URL: &str = "https://opencode.ai/zen/v1";
/// base_url 里出现这个片段即视为 Zen 网关（用于用户手填自定义 base_url 的场景）。
pub const OPENCODE_ZEN_URL_MARKER: &str = "opencode.ai/zen";

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
        id: "opencode",
        name: "OpenCode Zen",
        description:
            "OpenCode Zen — curated coding models. Key optional: free models work without one.",
        category: ProviderCategory::Popular,
        default_base_url: Some(OPENCODE_ZEN_BASE_URL),
        api_key_env_var: Some("OPENCODE_API_KEY"),
        // Zen 的 `space-bunny-free` 完全不需要 key（实测无鉴权头即可 200，
        // 且 tool calling 正常）。key 只在用付费模型时才必需，
        // 所以这里不能强制要求，否则免费模型根本进不到发请求那一步。
        requires_api_key: false,
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

/// OpenCode Zen 上游协议。
///
/// Zen 是多协议网关：同一个 base_url 下，不同模型前缀挂在不同端点上，
/// 所以这里不能只靠 provider id 推断协议，必须结合模型名。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZenProtocol {
    /// `POST {base}/chat/completions` — OpenAI Chat Completions
    ChatCompletions,
    /// `POST {base}/messages` — Anthropic Messages
    AnthropicMessages,
    /// `POST {base}/responses` — OpenAI Responses（本项目尚未接入）
    OpenAiResponses,
    /// `POST {base}/models/{id}` — Gemini 原生（本项目尚未接入）
    GeminiNative,
    /// `POST {base}/systemone` — Jev，返回概率评分而非文本，不能用于对话
    SystemOne,
}

/// 判断某个 provider id / base_url 是否指向 OpenCode Zen 网关。
pub fn is_opencode_zen(provider_id: Option<&str>, base_url: &str) -> bool {
    if let Some(pid) = provider_id {
        if normalize_provider_id(pid).as_deref() == Some("opencode") {
            return true;
        }
    }
    base_url
        .to_ascii_lowercase()
        .contains(OPENCODE_ZEN_URL_MARKER)
}

/// 按模型名决定 Zen 上该模型实际使用的协议端点。
///
/// 依据 Zen 官方模型表（`/zen/v1/models` 与文档端点列）按前缀匹配：
/// - `claude-*` 及 `qwen3.5-plus` / `qwen3.6-plus` / `qwen3.8-flash` → Anthropic Messages
/// - `gemini-*` → Gemini 原生
/// - `jev-*` → SystemOne
/// - `gpt-*` / `grok-*` / `muse-spark-*` → OpenAI Responses
/// - 其余（deepseek / glm / kimi / minimax / mistral / qwen3.8-max / 各 `*-free`）→ Chat Completions
///
/// 未识别的模型名保守回落到 Chat Completions —— 这是覆盖面最广的默认协议。
pub fn zen_protocol_for_model(model: &str) -> ZenProtocol {
    let m = model.trim().to_ascii_lowercase();
    // 允许用户带 `opencode/` 前缀（与 opencode 自身 config 的写法一致）。
    let m = m.strip_prefix("opencode/").unwrap_or(&m).trim();

    if m.starts_with("jev-") {
        return ZenProtocol::SystemOne;
    }
    if m.starts_with("gemini-") {
        return ZenProtocol::GeminiNative;
    }
    if m.starts_with("claude-") {
        return ZenProtocol::AnthropicMessages;
    }
    // qwen 走 Chat Completions 的例外只有 qwen3.8-max；其余 qwen 系走 Anthropic Messages。
    if m.starts_with("qwen") && !m.starts_with("qwen3.8-max") {
        return ZenProtocol::AnthropicMessages;
    }
    if m.starts_with("gpt-") || m.starts_with("grok-") || m.starts_with("muse-spark-") {
        return ZenProtocol::OpenAiResponses;
    }
    ZenProtocol::ChatCompletions
}

/// 该协议在当前版本里是否已经能真正跑通请求。
///
/// 未接入的协议仍然允许出现在模型列表里（用户能看到 Zen 有什么），
/// 但真正发请求前会给出明确提示，而不是发出一个注定失败的请求。
pub fn zen_protocol_supported(protocol: ZenProtocol) -> bool {
    matches!(
        protocol,
        ZenProtocol::ChatCompletions | ZenProtocol::AnthropicMessages
    )
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
        // Zen 的 base_url 由网关固定（多协议共用一个根地址），不需要用户手填。
        Some("opencode") => false,
        Some(id) if get_provider_by_id(id).is_some() => false,
        Some(_) => true,
        None => true,
    }
}

pub fn provider_forces_openai_compatible(provider_id: &str) -> bool {
    provider_openai_compatible_mode(provider_id).unwrap_or(false)
}

/// 该 provider 是否强制走 OpenAI Chat Completions 协议。
///
/// 返回 `None` 表示"不由 provider 决定"，调用方应另行推断（例如 Zen 需要按模型名分流）。
pub fn provider_openai_compatible_mode(provider_id: &str) -> Option<bool> {
    if normalize_provider_id(provider_id).as_deref() == Some("anthropic-compatible") {
        return Some(false);
    }
    // Zen 一个 provider 下混着多种协议，必须交给 infer_provider 按模型名判断。
    if normalize_provider_id(provider_id).as_deref() == Some("opencode") {
        return None;
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
        // Zen 文档里也写作 OPENCODE_ZEN_API_KEY，两个都收。
        "opencode" => push_env_var(&mut vars, "OPENCODE_ZEN_API_KEY"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencode_aliases_resolve_to_a_real_registered_provider() {
        // 曾经的 bug：别名映射到 "opencode"，但 ALL_PROVIDERS 里没有这条记录，
        // 规范化成功却拿到一个查不到元数据的幽灵 id。
        for alias in [
            "opencode",
            "opencodezen",
            "zen",
            "OpenCode Zen",
            "open-code",
        ] {
            let id = normalize_provider_id(alias)
                .unwrap_or_else(|| panic!("alias {alias} should normalize"));
            assert_eq!(id, "opencode", "alias {alias}");
            assert!(
                get_provider_by_id(&id).is_some(),
                "normalized id {id} must exist in ALL_PROVIDERS"
            );
        }
    }

    #[test]
    fn opencode_provider_has_zen_base_url_and_env_var() {
        let meta = get_provider_by_id("opencode").expect("opencode provider registered");
        assert_eq!(meta.default_base_url, Some(OPENCODE_ZEN_BASE_URL));
        assert_eq!(meta.api_key_env_var, Some("OPENCODE_API_KEY"));
        // 免费模型（space-bunny-free）实测无需 key 即可 200，强制要求 key
        // 会让它们连请求都发不出去。
        assert!(
            !meta.requires_api_key,
            "Zen must stay usable without an API key so free models can run"
        );
    }

    #[test]
    fn opencode_zen_is_detected_by_id_or_by_base_url() {
        assert!(is_opencode_zen(Some("opencode"), "https://example.com/v1"));
        assert!(is_opencode_zen(Some("zen"), "https://example.com/v1"));
        assert!(is_opencode_zen(None, OPENCODE_ZEN_BASE_URL));
        assert!(is_opencode_zen(None, "https://OpenCode.AI/zen/v1"));
        assert!(!is_opencode_zen(None, "https://api.anthropic.com/v1"));
        assert!(!is_opencode_zen(None, "https://opencode.ai/other"));
        assert!(!is_opencode_zen(
            Some("anthropic"),
            "https://api.anthropic.com/v1"
        ));
        assert!(!is_opencode_zen(None, ""));

        // base_url 是权威信号：即使 provider_id 写的是 anthropic，只要实际
        // 端点指向 Zen 网关，请求就会发往 Zen，需要的是 Zen 的 key。
        assert!(is_opencode_zen(Some("anthropic"), OPENCODE_ZEN_BASE_URL));
    }

    #[test]
    fn zen_routes_anthropic_models_to_messages_endpoint() {
        for m in [
            "claude-opus-4-8",
            "claude-sonnet-5-5",
            "claude-haiku-4-5",
            "qwen3.5-plus",
            "qwen3.6-plus",
            "qwen3.7-max",
            "qwen3.8-flash",
        ] {
            assert_eq!(
                zen_protocol_for_model(m),
                ZenProtocol::AnthropicMessages,
                "model {m}"
            );
        }
    }

    #[test]
    fn zen_routes_chat_completions_models() {
        for m in [
            "deepseek-v4-pro",
            "glm-5.3",
            "kimi-k3",
            "minimax-m3",
            "mistral-large-4",
            "qwen3.8-max",
            "big-pickle",
            "space-bunny-free",
            "exo-free",
            "mimo-v2.6-flash-free",
        ] {
            assert_eq!(
                zen_protocol_for_model(m),
                ZenProtocol::ChatCompletions,
                "model {m}"
            );
        }
    }

    #[test]
    fn zen_flags_protocols_that_are_not_wired_up_yet() {
        for m in ["gpt-5.5", "gpt-6-astra", "grok-4.7", "muse-spark-1.3"] {
            let p = zen_protocol_for_model(m);
            assert_eq!(p, ZenProtocol::OpenAiResponses, "model {m}");
            assert!(
                !zen_protocol_supported(p),
                "model {m} must not be supported yet"
            );
        }
        for m in ["gemini-3.1-pro", "gemini-3.8-flash"] {
            let p = zen_protocol_for_model(m);
            assert_eq!(p, ZenProtocol::GeminiNative, "model {m}");
            assert!(!zen_protocol_supported(p));
        }
        let jev = zen_protocol_for_model("jev-1.13");
        assert_eq!(jev, ZenProtocol::SystemOne);
        assert!(!zen_protocol_supported(jev));
    }

    #[test]
    fn zen_protocol_ignores_prefix_case_and_opencode_namespace() {
        assert_eq!(
            zen_protocol_for_model("opencode/claude-sonnet-5"),
            ZenProtocol::AnthropicMessages
        );
        assert_eq!(
            zen_protocol_for_model("  CLAUDE-OPUS-5  "),
            ZenProtocol::AnthropicMessages
        );
        assert_eq!(
            zen_protocol_for_model("OpenCode/DeepSeek-V4-Pro"),
            ZenProtocol::ChatCompletions
        );
    }

    #[test]
    fn unknown_zen_model_falls_back_to_chat_completions() {
        // 保守默认：覆盖面最广的协议。
        assert_eq!(
            zen_protocol_for_model("some-model-nobody-has-heard-of"),
            ZenProtocol::ChatCompletions
        );
        assert!(zen_protocol_supported(zen_protocol_for_model(
            "some-model-nobody-has-heard-of"
        )));
    }

    #[test]
    fn zen_defers_protocol_choice_instead_of_forcing_openai_compatible() {
        // 关键：Zen 不能被 provider 级开关锁死成某一种协议，
        // 否则 claude-* 会被发到 /chat/completions。
        assert_eq!(provider_openai_compatible_mode("opencode"), None);
        assert!(!provider_forces_openai_compatible("opencode"));
        // 但 base_url 不该要求用户手填。
        assert!(!provider_requires_manual_base_url("opencode"));
        // 其它 provider 的行为不受影响。
        assert_eq!(
            provider_openai_compatible_mode("openai-compatible"),
            Some(true)
        );
        assert_eq!(
            provider_openai_compatible_mode("anthropic-compatible"),
            Some(false)
        );
        assert!(provider_requires_manual_base_url("openai-compatible"));
    }

    #[test]
    fn opencode_reads_both_zen_api_key_env_names() {
        let vars = api_key_env_candidates("opencode");
        assert!(vars.contains(&"OPENCODE_API_KEY"));
        assert!(vars.contains(&"OPENCODE_ZEN_API_KEY"));
    }

    /// 拿 Zen 线上真实的模型 ID 快照跑一遍路由。
    ///
    /// 快照取自 `GET https://opencode.ai/zen/v1/models`（该端点无需鉴权）。
    /// 目的是让「模型名 → 协议」的映射在真实数据上被验证过：Zen 新增模型时，
    /// 这个测试不会自动失败（它只是快照），但它能保证当前这批模型的映射
    /// 没有被后续改动破坏，且新增的 `*-free` / `-contributor-free` 系列
    /// 会落到 Chat Completions 而不是未知分支。
    #[test]
    fn zen_live_model_snapshot_routes_to_expected_protocols() {
        let snapshot: &[(&str, ZenProtocol)] = &[
            // Anthropic Messages
            ("claude-fable-5", ZenProtocol::AnthropicMessages),
            ("claude-opus-5-5", ZenProtocol::AnthropicMessages),
            ("claude-sonnet-5-5", ZenProtocol::AnthropicMessages),
            ("claude-haiku-4-5", ZenProtocol::AnthropicMessages),
            ("qwen3.5-plus", ZenProtocol::AnthropicMessages),
            ("qwen3.6-plus", ZenProtocol::AnthropicMessages),
            ("qwen3.8-flash", ZenProtocol::AnthropicMessages),
            // Chat Completions（含 qwen3.8-max 这个唯一的 qwen 例外）
            ("qwen3.8-max", ZenProtocol::ChatCompletions),
            ("deepseek-v4-pro", ZenProtocol::ChatCompletions),
            ("deepseek-v4.1-flash", ZenProtocol::ChatCompletions),
            ("glm-5.3", ZenProtocol::ChatCompletions),
            ("glm-5.3-flash", ZenProtocol::ChatCompletions),
            ("kimi-k3", ZenProtocol::ChatCompletions),
            ("kimi-k2.7-code", ZenProtocol::ChatCompletions),
            ("minimax-m3", ZenProtocol::ChatCompletions),
            ("mistral-large-4", ZenProtocol::ChatCompletions),
            ("big-pickle", ZenProtocol::ChatCompletions),
            // 各类 free / contributor 系列
            ("space-bunny-free", ZenProtocol::ChatCompletions),
            ("longcat-2.5-preview-free", ZenProtocol::ChatCompletions),
            ("step-5-preview-free", ZenProtocol::ChatCompletions),
            ("exo-free", ZenProtocol::ChatCompletions),
            ("mimo-v2.6-flash-free", ZenProtocol::ChatCompletions),
            ("mimo-v2.5-free", ZenProtocol::ChatCompletions),
            ("ling-3.1-flash-free", ZenProtocol::ChatCompletions),
            ("ling-3.0-flash-fin-free", ZenProtocol::ChatCompletions),
            ("nemotron-3-ultra-free", ZenProtocol::ChatCompletions),
            ("nemotron-3.5-lightning-free", ZenProtocol::ChatCompletions),
            // 未接入的协议
            ("gpt-6-astra", ZenProtocol::OpenAiResponses),
            ("gpt-5.5-pro", ZenProtocol::OpenAiResponses),
            ("grok-4.7", ZenProtocol::OpenAiResponses),
            ("grok-build-0.1", ZenProtocol::OpenAiResponses),
            ("muse-spark-1.3", ZenProtocol::OpenAiResponses),
            (
                "muse-spark-1.3-contributor-free",
                ZenProtocol::OpenAiResponses,
            ),
            ("gemini-3.1-pro", ZenProtocol::GeminiNative),
            ("gemini-3.8-flash", ZenProtocol::GeminiNative),
            ("jev-1.13", ZenProtocol::SystemOne),
            ("jev-1.13-free", ZenProtocol::SystemOne),
        ];

        for (model, expected) in snapshot {
            assert_eq!(zen_protocol_for_model(model), *expected, "model {model}");
        }

        // 快照覆盖的协议必须是当前真实支持集合的子集 ——
        // 一旦有人把某个协议标记为已支持，这里就会提醒同步更新说明文案。
        for (model, protocol) in snapshot {
            assert_eq!(
                zen_protocol_supported(*protocol),
                matches!(
                    *protocol,
                    ZenProtocol::ChatCompletions | ZenProtocol::AnthropicMessages
                ),
                "support flag for {model} must match the implemented protocols"
            );
        }
    }
}
