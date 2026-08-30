/// Google Gemini Provider适配器
/// 
/// 对标claude-code-main的gemini/
/// 支持Google Gemini API

use async_trait::async_trait;
use futures::Stream;
use std::error::Error;
use std::pin::Pin;
use serde::{Deserialize, Serialize};

use super::{LlmClient, LlmConfig, LlmError, LlmEvent, LlmProvider};
use crate::llm::ModelInfo;
use crate::types::{StarMessage, StarResponse, StarChoice, StarTool, StarToolCall, StarUsage};

/// Gemini配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiConfig {
    /// API密钥
    pub api_key: String,
    /// 模型名称
    pub model: String,
    /// API端点
    pub api_endpoint: String,
    /// 最大输出token数
    pub max_output_tokens: u32,
    /// 温度
    pub temperature: f32,
}

impl GeminiConfig {
    /// 从环境变量创建配置
    pub fn from_env() -> Result<Self, String> {
        let api_key = std::env::var("GEMINI_API_KEY")
            .map_err(|_| "GEMINI_API_KEY not set".to_string())?;

        let model = std::env::var("GEMINI_MODEL")
            .or_else(|_| std::env::var("GEMINI_DEFAULT_SONNET_MODEL"))
            .unwrap_or_else(|_| "gemini-1.5-pro".to_string());

        let api_endpoint = std::env::var("GEMINI_API_ENDPOINT")
            .unwrap_or_else(|_| "https://generativelanguage.googleapis.com".to_string());

        let max_output_tokens = std::env::var("GEMINI_MAX_OUTPUT_TOKENS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4096);

        let temperature = std::env::var("GEMINI_TEMPERATURE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.7);

        Ok(Self {
            api_key,
            model,
            api_endpoint,
            max_output_tokens,
            temperature,
        })
    }
}

/// Gemini Provider
#[derive(Debug)]
pub struct GeminiProvider {
    config: GeminiConfig,
    http_client: reqwest::Client,
}

impl GeminiProvider {
    /// 创建新的Gemini Provider
    pub fn new(config: GeminiConfig) -> Self {
        let http_client = reqwest::Client::new();
        Self { config, http_client }
    }

    /// 从环境变量创建
    pub fn from_env() -> Result<Self, String> {
        let config = GeminiConfig::from_env()?;
        Ok(Self::new(config))
    }

    /// 构建API URL
    fn get_api_url(&self, method: &str) -> String {
        format!(
            "{}/v1beta/models/{}:{}?key={}",
            self.config.api_endpoint, self.config.model, method, self.config.api_key
        )
    }
}

#[async_trait]
impl LlmClient for GeminiProvider {
    async fn chat_completion(
        &self,
        messages: Vec<StarMessage>,
        tools: Option<Vec<StarTool>>,
    ) -> Result<StarResponse, Box<dyn Error + Send + Sync>> {
        let url = self.get_api_url("generateContent");

        let contents: Vec<serde_json::Value> = messages.iter().map(|m| {
            let role = if m.role == "assistant" { "model" } else { "user" };
            serde_json::json!({
                "role": role,
                "parts": [{
                    "text": m.content
                }]
            })
        }).collect();

        let mut request_body = serde_json::json!({
            "contents": contents,
            "generationConfig": {
                "maxOutputTokens": self.config.max_output_tokens,
                "temperature": self.config.temperature
            }
        });

        let response = self.http_client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| LlmError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(Box::new(LlmError::ProviderError(error_text)));
        }

        let response_body: serde_json::Value = response.json().await
            .map_err(|e| LlmError::ParsingError(e.to_string()))?;

        let content = response_body["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string();

        let usage = StarUsage {
            prompt_tokens: response_body["usageMetadata"]["promptTokenCount"].as_u64().unwrap_or(0) as u32,
            completion_tokens: response_body["usageMetadata"]["candidatesTokenCount"].as_u64().unwrap_or(0) as u32,
            total_tokens: response_body["usageMetadata"]["totalTokenCount"].as_u64().unwrap_or(0) as u32,
            ..Default::default()
        };

        Ok(StarResponse {
            choices: vec![StarChoice {
                message: StarMessage::assistant(&content),
                finish_reason: "stop".to_string(),
            }],
            usage: Some(usage),
        })
    }

    fn get_model_info(&self) -> Option<ModelInfo> {
        None
    }
}
