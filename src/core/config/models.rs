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
    if clean_name.starts_with("o1")
        || clean_name.starts_with("o3")
        || clean_name.starts_with("o4")
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
    if clean_name.starts_with("o1") || clean_name.starts_with("o3") || clean_name.starts_with("o4") {
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
    if clean_name.contains("thinking") || clean_name.contains("reasoner") || clean_name.contains("reasoning") {
        return ThinkingCapability::Binary;
    }

    ThinkingCapability::None
}
 