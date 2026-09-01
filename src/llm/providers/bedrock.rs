/// AWS Bedrock Provider适配器
///
/// 对标claude-code-main的bedrockClient.ts
/// 支持AWS Bedrock服务调用Claude模型
use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::pin::Pin;

use super::{LlmClient, LlmConfig, LlmError, LlmEvent, LlmProvider};
use crate::llm::ModelInfo;
use crate::types::{
    StarChoice, StarMessage, StarResponse, StarTool, StarToolCall, StarToolCallFunction, StarUsage,
};

/// AWS Bedrock配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BedrockConfig {
    /// AWS区域
    pub region: String,
    /// AWS访问密钥ID
    pub access_key_id: String,
    /// AWS秘密访问密钥
    pub secret_access_key: String,
    /// AWS会话令牌（可选）
    pub session_token: Option<String>,
    /// 模型ID
    pub model_id: String,
    /// 是否使用cross-region inference
    pub cross_region: bool,
}

impl BedrockConfig {
    /// 从环境变量创建配置
    pub fn from_env() -> Result<Self, String> {
        let region = std::env::var("AWS_REGION")
            .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
            .unwrap_or_else(|_| "us-east-1".to_string());

        let access_key_id = std::env::var("AWS_ACCESS_KEY_ID")
            .map_err(|_| "AWS_ACCESS_KEY_ID not set".to_string())?;

        let secret_access_key = std::env::var("AWS_SECRET_ACCESS_KEY")
            .map_err(|_| "AWS_SECRET_ACCESS_KEY not set".to_string())?;

        let session_token = std::env::var("AWS_SESSION_TOKEN").ok();

        let model_id = std::env::var("BEDROCK_MODEL_ID")
            .unwrap_or_else(|_| "anthropic.claude-3-5-sonnet-20241022-v2:0".to_string());

        let cross_region = std::env::var("BEDROCK_CROSS_REGION")
            .ok()
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(false);

        Ok(Self {
            region,
            access_key_id,
            secret_access_key,
            session_token,
            model_id,
            cross_region,
        })
    }
}

/// AWS Bedrock Provider
#[derive(Debug)]
pub struct BedrockProvider {
    config: BedrockConfig,
    http_client: reqwest::Client,
}

impl BedrockProvider {
    /// 创建新的Bedrock Provider
    pub fn new(config: BedrockConfig) -> Self {
        let http_client = reqwest::Client::new();
        Self {
            config,
            http_client,
        }
    }

    /// 从环境变量创建
    pub fn from_env() -> Result<Self, String> {
        let config = BedrockConfig::from_env()?;
        Ok(Self::new(config))
    }
}

#[async_trait]
impl LlmClient for BedrockProvider {
    async fn chat_completion(
        &self,
        messages: Vec<StarMessage>,
        tools: Option<Vec<StarTool>>,
    ) -> Result<StarResponse, Box<dyn Error + Send + Sync>> {
        let url = format!(
            "https://bedrock-runtime.{}.amazonaws.com/model/{}/invoke",
            self.config.region, self.config.model_id
        );

        let request_body = serde_json::json!({
            "anthropic_version": "bedrock-2023-05-31",
            "max_tokens": 4096,
            "messages": messages.iter().map(|m| {
                serde_json::json!({
                    "role": m.role,
                    "content": m.content
                })
            }).collect::<Vec<_>>()
        });

        let response = self
            .http_client
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

        let response_body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| LlmError::ParsingError(e.to_string()))?;

        let content = response_body["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string();

        let usage = StarUsage {
            prompt_tokens: response_body["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32,
            completion_tokens: response_body["usage"]["output_tokens"]
                .as_u64()
                .unwrap_or(0) as u32,
            total_tokens: (response_body["usage"]["input_tokens"].as_u64().unwrap_or(0)
                + response_body["usage"]["output_tokens"]
                    .as_u64()
                    .unwrap_or(0)) as u32,
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
