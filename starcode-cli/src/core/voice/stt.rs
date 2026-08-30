/// STT (Speech-to-Text) 提供商
/// 
/// 对标claude-code-main的voiceStreamSTT.ts和doubaoSTT.ts
/// 提供多种STT后端支持

use serde::{Deserialize, Serialize};
use async_trait::async_trait;

/// STT配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttConfig {
    /// 提供商
    pub provider: String,
    /// API密钥
    pub api_key: Option<String>,
    /// API端点
    pub api_endpoint: Option<String>,
    /// 语言
    pub language: String,
    /// 模型
    pub model: Option<String>,
    /// 是否启用流式转录
    pub streaming: bool,
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            provider: "anthropic".to_string(),
            api_key: None,
            api_endpoint: None,
            language: "en".to_string(),
            model: None,
            streaming: false,
        }
    }
}

/// STT转录结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttResult {
    /// 转录文本
    pub text: String,
    /// 置信度
    pub confidence: f64,
    /// 语言
    pub language: Option<String>,
    /// 持续时间（毫秒）
    pub duration_ms: u64,
    /// 是否是最终结果
    pub is_final: bool,
}

/// STT提供商trait
#[async_trait]
pub trait SttProvider: Send + Sync {
    /// 转录音频
    async fn transcribe(&self, audio_data: &[u8], config: &SttConfig) -> Result<SttResult, SttError>;
    
    /// 流式转录
    async fn transcribe_stream(
        &self,
        audio_stream: Box<dyn futures::Stream<Item = Vec<u8>> + Send + Unpin>,
        config: &SttConfig,
    ) -> Result<Box<dyn futures::Stream<Item = Result<SttResult, SttError>> + Send + Unpin>, SttError>;
    
    /// 获取提供商名称
    fn provider_name(&self) -> &str;
}

/// Anthropic STT提供商
pub struct AnthropicSttProvider {
    client: reqwest::Client,
}

impl AnthropicSttProvider {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl SttProvider for AnthropicSttProvider {
    async fn transcribe(&self, audio_data: &[u8], config: &SttConfig) -> Result<SttResult, SttError> {
        let api_key = config.api_key.as_ref()
            .ok_or(SttError::MissingApiKey)?;

        // TODO: 实现Anthropic STT API调用
        // 这里是简化实现
        Ok(SttResult {
            text: "Transcription placeholder".to_string(),
            confidence: 0.9,
            language: Some(config.language.clone()),
            duration_ms: 0,
            is_final: true,
        })
    }

    async fn transcribe_stream(
        &self,
        _audio_stream: Box<dyn futures::Stream<Item = Vec<u8>> + Send + Unpin>,
        _config: &SttConfig,
    ) -> Result<Box<dyn futures::Stream<Item = Result<SttResult, SttError>> + Send + Unpin>, SttError> {
        // TODO: 实现流式转录
        Err(SttError::NotSupported("Streaming transcription not yet implemented".to_string()))
    }

    fn provider_name(&self) -> &str {
        "anthropic"
    }
}

/// 豆包STT提供商
/// 对标claude-code-main的doubaoSTT.ts
pub struct DoubaoSttProvider {
    client: reqwest::Client,
}

impl DoubaoSttProvider {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl SttProvider for DoubaoSttProvider {
    async fn transcribe(&self, audio_data: &[u8], config: &SttConfig) -> Result<SttResult, SttError> {
        let api_key = config.api_key.as_ref()
            .ok_or(SttError::MissingApiKey)?;

        // TODO: 实现豆包STT API调用
        Ok(SttResult {
            text: "豆包转录占位符".to_string(),
            confidence: 0.9,
            language: Some(config.language.clone()),
            duration_ms: 0,
            is_final: true,
        })
    }

    async fn transcribe_stream(
        &self,
        _audio_stream: Box<dyn futures::Stream<Item = Vec<u8>> + Send + Unpin>,
        _config: &SttConfig,
    ) -> Result<Box<dyn futures::Stream<Item = Result<SttResult, SttError>> + Send + Unpin>, SttError> {
        Err(SttError::NotSupported("Streaming transcription not yet implemented".to_string()))
    }

    fn provider_name(&self) -> &str {
        "doubao"
    }
}

/// STT错误
#[derive(Debug)]
pub enum SttError {
    /// 缺少API密钥
    MissingApiKey,
    /// API错误
    ApiError(String),
    /// 网络错误
    NetworkError(String),
    /// 解析错误
    ParseError(String),
    /// 不支持的操作
    NotSupported(String),
    /// 音频格式错误
    AudioFormatError(String),
}

impl std::fmt::Display for SttError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SttError::MissingApiKey => write!(f, "STT API key is missing"),
            SttError::ApiError(e) => write!(f, "STT API error: {}", e),
            SttError::NetworkError(e) => write!(f, "STT network error: {}", e),
            SttError::ParseError(e) => write!(f, "STT parse error: {}", e),
            SttError::NotSupported(e) => write!(f, "STT not supported: {}", e),
            SttError::AudioFormatError(e) => write!(f, "Audio format error: {}", e),
        }
    }
}

impl std::error::Error for SttError {}
