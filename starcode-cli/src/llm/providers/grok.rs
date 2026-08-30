/// xAI Grok Provider适配器
/// 
/// 对标claude-code-main的grok/
/// 支持xAI Grok API

use async_trait::async_trait;
use futures::Stream;
use std::error::Error;
use std::pin::Pin;
use serde::{Deserialize, Serialize};

use super::{LlmClient, LlmConfig, LlmError, LlmEvent, LlmProvider};
use crate::llm::ModelInfo;
use crate::types::{StarMessage, StarResponse, StarChoice, StarTool, StarToolCall, StarToolCallFunction, StarUsage};

/// Grok配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokConfig {
    /// API密钥
    pub api_key: String,
    /// 模型名称
    pub model: String,
    /// API端点
    pub api_endpoint: String,
    /// 最大输出token数
    pub max_tokens: u32,
    /// 温度
    pub temperature: f32,
}

impl GrokConfig {
    /// 从环境变量创建配置
    pub fn from_env() -> Result<Self, String> {
        let api_key = std::env::var("GROK_API_KEY")
            .or_else(|_| std::env::var("XAI_API_KEY"))
            .map_err(|_| "GROK_API_KEY or XAI_API_KEY not set".to_string())?;

        let model = std::env::var("GROK_MODEL")
            .unwrap_or_else(|_| "grok-2".to_string());

        let api_endpoint = std::env::var("GROK_API_ENDPOINT")
            .unwrap_or_else(|_| "https://api.x.ai".to_string());

        let max_tokens = std::env::var("GROK_MAX_TOKENS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4096);

        let temperature = std::env::var("GROK_TEMPERATURE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.7);

        Ok(Self {
            api_key,
            model,
            api_endpoint,
            max_tokens,
            temperature,
        })
    }
}

/// Grok Provider
#[derive(Debug)]
pub struct GrokProvider {
    config: GrokConfig,
    http_client: reqwest::Client,
}

impl GrokProvider {
    /// 创建新的Grok Provider
    pub fn new(config: GrokConfig) -> Self {
        let http_client = reqwest::Client::new();
        Self { config, http_client }
    }

    /// 从环境变量创建
    pub fn from_env() -> Result<Self, String> {
        let config = GrokConfig::from_env()?;
        Ok(Self::new(config))
    }
}

#[async_trait]
impl LlmClient for GrokProvider {
    async fn chat_completion(
        &self,
        messages: Vec<StarMessage>,
        tools: Option<Vec<StarTool>>,
    ) -> Result<StarResponse, Box<dyn Error + Send + Sync>> {
        let url = format!("{}/v1/chat/completions", self.config.api_endpoint);

        let messages_json: Vec<serde_json::Value> = messages.iter().map(|m| {
            serde_json::json!({
                "role": m.role,
                "content": m.content
            })
        }).collect();

        let mut request_body = serde_json::json!({
            "model": self.config.model,
            "messages": messages_json,
            "max_tokens": self.config.max_tokens,
            "temperature": self.config.temperature
        });

        let response = self.http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
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

        let content = response_body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        let usage = StarUsage {
            prompt_tokens: response_body["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            completion_tokens: response_body["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32,
            total_tokens: response_body["usage"]["total_tokens"].as_u64().unwrap_or(0) as u32,
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
