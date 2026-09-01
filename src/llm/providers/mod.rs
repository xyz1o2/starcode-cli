/// 多Provider适配层
///
/// 对标claude-code-main的多Provider支持
/// 支持OpenAI、Anthropic、Bedrock、Vertex、Gemini、Grok等Provider
pub mod bedrock;
pub mod gemini;
pub mod grok;
pub mod vertex;

pub use bedrock::{BedrockConfig, BedrockProvider};
pub use gemini::{GeminiConfig, GeminiProvider};
pub use grok::{GrokConfig, GrokProvider};
pub use vertex::{VertexConfig, VertexProvider};

use crate::llm::{LlmClient, LlmConfig, LlmError, LlmEvent, LlmProvider};

/// Provider工厂
///
/// 根据配置创建相应的Provider
pub struct ProviderFactory;

impl ProviderFactory {
    /// 从环境变量创建Bedrock Provider
    pub fn create_bedrock() -> Result<BedrockProvider, String> {
        BedrockProvider::from_env()
    }

    /// 从环境变量创建Vertex Provider
    pub fn create_vertex() -> Result<VertexProvider, String> {
        VertexProvider::from_env()
    }

    /// 从环境变量创建Gemini Provider
    pub fn create_gemini() -> Result<GeminiProvider, String> {
        GeminiProvider::from_env()
    }

    /// 从环境变量创建Grok Provider
    pub fn create_grok() -> Result<GrokProvider, String> {
        GrokProvider::from_env()
    }

    /// 自动检测并创建Provider
    pub fn auto_detect() -> Result<Box<dyn LlmClient>, String> {
        // 检查环境变量，按优先级尝试创建Provider

        // 1. 检查Bedrock
        if std::env::var("AWS_ACCESS_KEY_ID").is_ok() && std::env::var("BEDROCK_MODEL_ID").is_ok() {
            return Ok(Box::new(Self::create_bedrock()?));
        }

        // 2. 检查Vertex
        if std::env::var("VERTEX_PROJECT_ID").is_ok()
            || std::env::var("GOOGLE_CLOUD_PROJECT").is_ok()
        {
            return Ok(Box::new(Self::create_vertex()?));
        }

        // 3. 检查Gemini
        if std::env::var("GEMINI_API_KEY").is_ok() {
            return Ok(Box::new(Self::create_gemini()?));
        }

        // 4. 检查Grok
        if std::env::var("GROK_API_KEY").is_ok() || std::env::var("XAI_API_KEY").is_ok() {
            return Ok(Box::new(Self::create_grok()?));
        }

        Err(
            "No provider configuration found. Please set the appropriate environment variables."
                .to_string(),
        )
    }
}
