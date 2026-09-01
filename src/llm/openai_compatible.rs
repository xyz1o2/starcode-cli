use crate::llm::{LlmClient, LlmError, LlmEvent, ModelInfo};
use crate::types::{StarMessage, StarResponse, StarTool};
use async_trait::async_trait;
use futures::Stream;
use std::collections::HashMap;
use std::error::Error;
use std::pin::Pin;

/// OpenAI Compatible 客户端 —— 只负责配置（base_url / provider_name / extra_headers 等），
/// 实际 LLM 请求统一委托给 rig-core 框架处理，与 OpenAI / DeepSeek 等内置提供商使用
/// 完全相同的流式与 reasoning 处理逻辑（rig-core PR #1999 / #2112 已修复 thinking 内容）。
#[derive(Debug, Clone)]
pub struct OpenAiCompatibleClient {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub provider_name: String,
    /// 自定义 HTTP 请求头配置（rig-core 暂不支持透传，保留字段以兼容现有 API）
    pub extra_headers: HashMap<String, String>,
    inner: crate::llm::rig_adapter::RigAdapter,
}

impl OpenAiCompatibleClient {
    pub fn new(api_key: String, base_url: String, model: String, provider_name: String) -> Self {
        let inner = crate::llm::rig_adapter::RigAdapter::openai_compatible(
            api_key.clone(),
            model.clone(),
            base_url.clone(),
            provider_name.clone(),
        );
        Self {
            api_key,
            base_url,
            model,
            provider_name,
            extra_headers: HashMap::new(),
            inner,
        }
    }

    /// 配置额外请求头。
    ///
    /// 注意：rig-core 的 HTTP 客户端目前不支持透传自定义请求头，
    /// 此字段保留用于 API 兼容，实际请求头配置请通过 base_url 或服务端处理。
    pub fn set_header(&mut self, key: &str, value: &str) {
        self.extra_headers
            .insert(key.to_string(), value.to_string());
    }
}

#[async_trait]
impl LlmClient for OpenAiCompatibleClient {
    async fn chat_completion(
        &self,
        messages: Vec<StarMessage>,
        tools: Option<Vec<StarTool>>,
    ) -> Result<StarResponse, Box<dyn Error + Send + Sync>> {
        self.inner.chat_completion(messages, tools).await
    }

    async fn chat_stream_events(
        &self,
        messages: Vec<StarMessage>,
        tools: Option<Vec<StarTool>>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + Sync>>, LlmError>
    {
        self.inner.chat_stream_events(messages, tools).await
    }

    fn get_model_info(&self) -> Option<ModelInfo> {
        self.inner.get_model_info()
    }
}
