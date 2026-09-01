/// 语音系统
///
/// 对标claude-code-main的src/services/voice.ts和voiceStreamSTT.ts
/// 提供语音输入、STT转录和语音模式管理
pub mod capture;
pub mod config;
pub mod enhanced;
pub mod stt;
pub mod transcription;

pub use capture::{AudioCapture, CaptureConfig, CaptureError};
pub use config::VoiceConfig;
pub use enhanced::{TranscriptionResult, VoiceBackend, VoiceManager, VoiceState};
pub use stt::{SttConfig, SttError, SttProvider};
pub use transcription::{TranscriptionEngine, TranscriptionRequest, TranscriptionResponse};

use serde::{Deserialize, Serialize};

/// 语音模式
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum VoiceMode {
    /// 禁用
    Disabled,
    /// 按键说话
    PushToTalk,
    /// 语音活动检测
    VoiceActivityDetection,
    /// 连续监听
    Continuous,
}

/// 语音状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum VoiceStatus {
    /// 空闲
    Idle,
    /// 录制中
    Recording,
    /// 处理中
    Processing,
    /// 错误
    Error(String),
}

/// 语音管理器配置
#[derive(Debug, Clone)]
pub struct VoiceManagerConfig {
    /// 语音模式
    pub mode: VoiceMode,
    /// 采样率
    pub sample_rate: u32,
    /// 声道数
    pub channels: u16,
    /// 位深度
    pub bits_per_sample: u16,
    /// STT提供商
    pub stt_provider: String,
    /// STT API密钥
    pub stt_api_key: Option<String>,
    /// STT API端点
    pub stt_api_endpoint: Option<String>,
    /// 语言
    pub language: String,
    /// 是否启用关键词检测
    pub keyword_detection: bool,
    /// 关键词列表
    pub keywords: Vec<String>,
}

impl Default for VoiceManagerConfig {
    fn default() -> Self {
        Self {
            mode: VoiceMode::Disabled,
            sample_rate: 16000,
            channels: 1,
            bits_per_sample: 16,
            stt_provider: "anthropic".to_string(),
            stt_api_key: None,
            stt_api_endpoint: None,
            language: "en".to_string(),
            keyword_detection: false,
            keywords: Vec::new(),
        }
    }
}

impl VoiceManagerConfig {
    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        let mode = std::env::var("STAR_VOICE_MODE")
            .ok()
            .map(|v| match v.to_lowercase().as_str() {
                "push_to_talk" | "ptt" => VoiceMode::PushToTalk,
                "vad" | "voice_activity" => VoiceMode::VoiceActivityDetection,
                "continuous" => VoiceMode::Continuous,
                _ => VoiceMode::Disabled,
            })
            .unwrap_or(VoiceMode::Disabled);

        let sample_rate = std::env::var("STAR_VOICE_SAMPLE_RATE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(16000);

        let stt_provider =
            std::env::var("STAR_VOICE_STT_PROVIDER").unwrap_or_else(|_| "anthropic".to_string());

        let stt_api_key = std::env::var("STAR_VOICE_STT_API_KEY").ok();
        let stt_api_endpoint = std::env::var("STAR_VOICE_STT_API_ENDPOINT").ok();

        let language = std::env::var("STAR_VOICE_LANGUAGE").unwrap_or_else(|_| "en".to_string());

        Self {
            mode,
            sample_rate,
            channels: 1,
            bits_per_sample: 16,
            stt_provider,
            stt_api_key,
            stt_api_endpoint,
            language,
            keyword_detection: false,
            keywords: Vec::new(),
        }
    }
}
