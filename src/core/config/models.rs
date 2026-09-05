use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProviderConfig {
    pub providers: HashMap<String, ProviderSettings>,
    #[serde(alias = "activeProviderId")]
    pub active_provider_id: Option<String>,
    #[serde(alias = "activeModel")]
    pub active_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ModelConfig {
    pub name: Option<String>,
    #[serde(alias = "contextWindow")]
    pub context_window: Option<usize>,
    pub alias: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProviderSettings {
    #[serde(alias = "apiKey")]
    pub api_key: Option<String>,
    #[serde(alias = "baseUrl", alias = "baseURL")]
    pub base_url: Option<String>,
    #[serde(alias = "selectedModel", alias = "currentModel")]
    pub selected_model: Option<String>,
    pub models: Option<HashMap<String, ModelConfig>>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub r#type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub name: &'static str,
    pub max_tokens: u32,
    pub context_window: u32,
    pub supports_images: bool,
    pub supports_computer_use: bool,
    pub supports_prompt_cache: bool,
    pub input_price: f64,  // Price per million tokens
    pub output_price: f64, // Price per million tokens
    pub cache_write_price: Option<f64>,
    pub cache_read_price: Option<f64>,
    pub description: Option<&'static str>,
}

pub fn is_preview_model(_model: &str) -> bool {
    false
}

fn normalized_model_name(model_name: &str) -> String {
    let name = model_name.to_lowercase();
    if let Some(idx) = name.find('/') {
        name[idx + 1..].to_string()
    } else {
        name
    }
}

pub fn is_deepseek_reasoner_model(model_name: &str) -> bool {
    let clean_name = normalized_model_name(model_name);
    clean_name.starts_with("deepseek-r1") || clean_name.starts_with("deepseek-reasoner")
}

pub fn is_thinking_model(model_name: &str) -> bool {
    let clean_name = normalized_model_name(model_name);

    // DeepSeek models with reasoning capability
    if clean_name.starts_with("deepseek-r1")
        || clean_name.starts_with("deepseek-reasoner")
        || clean_name.starts_with("deepseek-coder-reasoner")
        || clean_name.starts_with("deepseek-v4")
    {
        return true;
    }

    // OpenAI o-series models (o1, o3, o4-mini, etc.)
    if clean_name.starts_with("o1") || clean_name.starts_with("o3") || clean_name.starts_with("o4")
    {
        return true;
    }

    // GPT-5 series with reasoning capability
    if clean_name.starts_with("gpt-5") {
        return true;
    }

    // Qwen reasoning models (QwQ, Qwen-QwQ)
    if clean_name.contains("qwq") || clean_name.contains("qwen-qwq") {
        return true;
    }

    // Kimi thinking models (Moonshot)
    if clean_name.starts_with("kimi") && clean_name.contains("thinking") {
        return true;
    }

    // Gemini thinking models
    if clean_name.starts_with("gemini") && clean_name.contains("thinking") {
        return true;
    }

    // Grok models with reasoning
    if clean_name.starts_with("grok-") {
        return true;
    }

    // Generic patterns — covers other thinking/reasoning models
    clean_name.contains("thinking")
        || clean_name.contains("reasoner")
        || clean_name.contains("reasoning")
}

/// Thinking capability levels for different models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingCapability {
    /// Supports granular thinking levels: Off, Low, Medium, High
    Granular,
    /// Supports only binary thinking: Off, On
    Binary,
    /// Does not support thinking/reasoning
    None,
}

/// Detect the thinking capability of a model based on its name.
pub fn thinking_capability(model_name: &str) -> ThinkingCapability {
    let clean_name = normalized_model_name(model_name);

    // Claude models — full granular thinking support (budget_tokens)
    if clean_name.starts_with("claude-") {
        return ThinkingCapability::Granular;
    }

    // OpenAI o-series — reasoning effort parameter (low/medium/high)
    if clean_name.starts_with("o1") || clean_name.starts_with("o3") || clean_name.starts_with("o4")
    {
        return ThinkingCapability::Granular;
    }

    // GPT-5 — granular reasoning
    if clean_name.starts_with("gpt-5") {
        return ThinkingCapability::Granular;
    }

    // DeepSeek reasoner models — binary thinking
    if clean_name.starts_with("deepseek-r1")
        || clean_name.starts_with("deepseek-reasoner")
        || clean_name.starts_with("deepseek-coder-reasoner")
        || clean_name.starts_with("deepseek-v4")
    {
        return ThinkingCapability::Binary;
    }

    // Qwen QwQ — binary thinking
    if clean_name.contains("qwq") || clean_name.contains("qwen-qwq") {
        return ThinkingCapability::Binary;
    }

    // Gemini thinking — binary
    if clean_name.starts_with("gemini") && clean_name.contains("thinking") {
        return ThinkingCapability::Binary;
    }

    // Kimi thinking — binary
    if clean_name.starts_with("kimi") && clean_name.contains("thinking") {
        return ThinkingCapability::Binary;
    }

    // Grok reasoning — binary
    if clean_name.starts_with("grok-") {
        return ThinkingCapability::Binary;
    }

    // Generic thinking/reasoner/keyword models — binary
    if clean_name.contains("thinking")
        || clean_name.contains("reasoner")
        || clean_name.contains("reasoning")
    {
        return ThinkingCapability::Binary;
    }

    ThinkingCapability::None
}

/// UI 是否该显示思考档位（对标 Claude Code 的 `modelSupportsEffort`）。
///
/// 启动阶段模型名还没解析出来（空串），先当作支持 —— 指示器早一点出现，用户才知道
/// 有这个档位可调；解析完成后这一格会跟着重画。
pub fn supports_thinking_ui(model_name: &str) -> bool {
    model_name.trim().is_empty()
        || !matches!(thinking_capability(model_name), ThinkingCapability::None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unresolved_model_name_still_shows_the_indicator() {
        // 启动阶段模型名是空串 —— 这时候藏起来，用户第一眼就看不到档位在哪
        assert!(supports_thinking_ui(""));
        assert!(supports_thinking_ui("   "));
    }

    #[test]
    fn thinking_models_show_the_indicator() {
        assert!(supports_thinking_ui("claude-opus-5"));
        assert!(supports_thinking_ui("deepseek-reasoner"));
        assert!(supports_thinking_ui("gpt-5"));
    }

    #[test]
    fn a_model_without_thinking_hides_the_indicator() {
        assert!(!supports_thinking_ui("gpt-4o"));
        assert!(!supports_thinking_ui("qwen2.5-coder-32b-instruct"));
    }
}
