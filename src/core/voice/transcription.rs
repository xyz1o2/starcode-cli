/// 转录引擎
/// 
/// 整合音频捕获和STT提供商，提供统一的转录接口

use serde::{Deserialize, Serialize};
use super::capture::{AudioCapture, CaptureConfig, CaptureError};
use super::stt::{SttProvider, SttConfig, SttError, SttResult};

/// 转录请求
#[derive(Debug, Clone)]
pub struct TranscriptionRequest {
    /// 音频数据
    pub audio_data: Vec<u8>,
    /// 配置
    pub config: SttConfig,
}

/// 转录响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionResponse {
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

/// 转录引擎
pub struct TranscriptionEngine {
    /// STT提供商
    provider: Box<dyn SttProvider>,
    /// 配置
    config: SttConfig,
}

impl TranscriptionEngine {
    /// 创建新的转录引擎
    pub fn new(provider: Box<dyn SttProvider>, config: SttConfig) -> Self {
        Self { provider, config }
    }

    /// 转录音频数据
    pub async fn transcribe(&self, audio_data: &[u8]) -> Result<TranscriptionResponse, TranscriptionError> {
        let result = self.provider.transcribe(audio_data, &self.config).await
            .map_err(|e| TranscriptionError::SttError(e.to_string()))?;

        Ok(TranscriptionResponse {
            text: result.text,
            confidence: result.confidence,
            language: result.language,
            duration_ms: result.duration_ms,
            is_final: result.is_final,
        })
    }

    /// 获取提供商名称
    pub fn provider_name(&self) -> &str {
        self.provider.provider_name()
    }

    /// 获取配置
    pub fn config(&self) -> &SttConfig {
        &self.config
    }
}

/// 转录错误
#[derive(Debug)]
pub enum TranscriptionError {
    /// STT错误
    SttError(String),
    /// 捕获错误
    CaptureError(String),
    /// 配置错误
    ConfigError(String),
}

impl std::fmt::Display for TranscriptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TranscriptionError::SttError(e) => write!(f, "Transcription STT error: {}", e),
            TranscriptionError::CaptureError(e) => write!(f, "Transcription capture error: {}", e),
            TranscriptionError::ConfigError(e) => write!(f, "Transcription config error: {}", e),
        }
    }
}

impl std::error::Error for TranscriptionError {}
