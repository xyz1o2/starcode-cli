/// Google Vertex AI Provider适配器
///
/// 对标claude-code-main的vertex.ts
/// 支持Google Cloud Vertex AI服务调用Claude模型
use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::pin::Pin;

use super::{LlmClient, LlmConfig, LlmError, LlmEvent, LlmProvider};
use crate::llm::ModelInfo;
use crate::types::{StarChoice, StarMessage, StarResponse, StarTool, StarToolCall, StarUsage};

/// Vertex AI配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VertexConfig {
    /// GCP项目ID
    pub project_id: String,
    /// GCP区域
    pub region: String,
    /// 模型名称
    pub model: String,
    /// 服务账号密钥JSON（可选，如果使用默认凭据）
    pub service_account_key: Option<String>,
    /// 访问令牌
    pub access_token: Option<String>,
}

impl VertexConfig {
    /// 从环境变量创建配置
    pub fn from_env() -> Result<Self, String> {
        let project_id = std::env::var("VERTEX_PROJECT_ID")
            .or_else(|_| std::env::var("GOOGLE_CLOUD_PROJECT"))
            .map_err(|_| "VERTEX_PROJECT_ID or GOOGLE_CLOUD_PROJECT not set".to_string())?;

        let region = std::env::var("VERTEX_REGION")
            .or_else(|_| std::env::var("GOOGLE_CLOUD_REGION"))
            .unwrap_or_else(|_| "us-east5".to_string());

        let model = std::env::var("VERTEX_MODEL")
            .unwrap_or_else(|_| "claude-3-5-sonnet@20241022".to_string());

        let service_account_key = std::env::var("VERTEX_SERVICE_ACCOUNT_KEY").ok();
        let access_token = std::env::var("VERTEX_ACCESS_TOKEN")
            .or_else(|_| std::env::var("GOOGLE_ACCESS_TOKEN"))
            .ok();

        Ok(Self {
            project_id,
            region,
            model,
            service_account_key,
            access_token,
        })
    }

    /// 获取API端点
    pub fn get_endpoint(&self) -> String {
        format!(
            "https://{}-aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/anthropic/models/{}",
            self.region, self.project_id, self.region, self.model
        )
    }
}

/// Vertex AI Provider
#[derive(Debug)]
pub struct VertexProvider {
    config: VertexConfig,
    http_client: reqwest::Client,
}

impl VertexProvider {
    /// 创建新的Vertex Provider
    pub fn new(config: VertexConfig) -> Self {
        let http_client = reqwest::Client::new();
        Self {
            config,
            http_client,
        }
    }

    /// 从环境变量创建
    pub fn from_env() -> Result<Self, String> {
        let config = VertexConfig::from_env()?;
        Ok(Self::new(config))
    }

    /// 获取访问令牌
    async fn get_access_token(&self) -> Result<String, LlmError> {
        if let Some(token) = &self.config.access_token {
            return Ok(token.clone());
        }

        // 尝试使用gcloud CLI获取令牌
        let output = std::process::Command::new("gcloud")
            .args(["auth", "print-access-token"])
            .output()
            .map_err(|e| LlmError::ProviderError(format!("Failed to get gcloud token: {}", e)))?;

        if output.status.success() {
            let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Ok(token)
        } else {
            Err(LlmError::ProviderError(
                "No access token available. Set VERTEX_ACCESS_TOKEN or configure gcloud."
                    .to_string(),
            ))
        }
    }
}

#[async_trait]
impl LlmClient for VertexProvider {
    async fn chat_completion(
        &self,
        messages: Vec<StarMessage>,
        tools: Option<Vec<StarTool>>,
    ) -> Result<StarResponse, Box<dyn Error + Send + Sync>> {
        let access_token = self.get_access_token().await?;

        let endpoint = format!("{}:rawPredict", self.config.get_endpoint());

        let request_body = serde_json::json!({
            "anthropic_version": "vertex-2023-10-16",
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
            .post(&endpoint)
            .header("Authorization", format!("Bearer {}", access_token))
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
