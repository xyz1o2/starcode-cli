//! Voice Mode 模块
//!
//! 对标 Claude Code 的 voice-mode.md：
//! - Push-to-Talk 语音输入
//! - 双后端支持（Anthropic STT / 豆包 ASR）
//! - WebSocket 流式传输

use serde::{Serialize, Deserialize};

/// 语音配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceConfig {
    /// 后端类型
    pub backend: VoiceBackend,
    /// 采样率
    pub sample_rate: u32,
    /// 声道数
    pub channels: u32,
    /// 是否启用 Push-to-Talk
    pub push_to_talk: bool,
    /// PTT 按键
    pub ptt_key: String,
    /// API key
    pub api_key: Option<String>,
    /// API endpoint
    pub api_endpoint: Option<String>,
    /// 语言
    pub language: String,
}

/// 语音后端
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoiceBackend {
    /// Anthropic Speech-to-Text
    Anthropic,
    /// 豆包 ASR (字节跳动)
    Doubao,
    /// 本地 Whisper
    Whisper,
    /// 禁用
    Disabled,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            backend: VoiceBackend::Disabled,
            sample_rate: 16000,
            channels: 1,
            push_to_talk: true,
            ptt_key: "F2".to_string(),
            api_key: None,
            api_endpoint: None,
            language: "zh-CN".to_string(),
        }
    }
}

/// 语音识别结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionResult {
    pub text: String,
    pub confidence: f32,
    pub language: Option<String>,
    pub duration_ms: u64,
    pub is_final: bool,
}

/// 语音合成请求
#[derive(Debug, Clone)]
pub struct TtsRequest {
    pub text: String,
    pub voice: Option<String>,
    pub speed: f32,
}

/// 语音模式管理器
pub struct VoiceManager {
    config: VoiceConfig,
    state: VoiceState,
    audio_buffer: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceState {
    Idle,
    Listening,
    Processing,
    Speaking,
    Error,
}

impl VoiceManager {
    pub fn new(config: VoiceConfig) -> Self {
        Self {
            config,
            state: VoiceState::Idle,
            audio_buffer: Vec::new(),
        }
    }

    /// 开始录音
    pub fn start_recording(&mut self) -> Result<(), String> {
        if self.config.backend == VoiceBackend::Disabled {
            return Err("Voice mode is disabled".to_string());
        }

        self.state = VoiceState::Listening;
        self.audio_buffer.clear();
        Ok(())
    }

    /// 停止录音并获取音频数据
    pub fn stop_recording(&mut self) -> Result<Vec<f32>, String> {
        if self.state != VoiceState::Listening {
            return Err("Not currently recording".to_string());
        }

        self.state = VoiceState::Processing;
        Ok(std::mem::take(&mut self.audio_buffer))
    }

    /// 添加音频数据到缓冲区
    pub fn push_audio(&mut self, samples: &[f32]) {
        if self.state == VoiceState::Listening {
            self.audio_buffer.extend_from_slice(samples);
        }
    }

    /// 识别音频
    pub async fn transcribe(&self, audio: &[f32]) -> Result<TranscriptionResult, String> {
        match self.config.backend {
            VoiceBackend::Anthropic => self.transcribe_anthropic(audio).await,
            VoiceBackend::Doubao => self.transcribe_doubao(audio).await,
            VoiceBackend::Whisper => self.transcribe_whisper(audio).await,
            VoiceBackend::Disabled => Err("Voice mode is disabled".to_string()),
        }
    }

    async fn transcribe_anthropic(&self, _audio: &[f32]) -> Result<TranscriptionResult, String> {
        // Anthropic STT API 调用
        let api_key = self.config.api_key.as_ref()
            .ok_or("Anthropic API key not configured")?;

        // 实际实现需要调用 Anthropic 的语音 API
        Err("Anthropic STT not yet implemented".to_string())
    }

    async fn transcribe_doubao(&self, _audio: &[f32]) -> Result<TranscriptionResult, String> {
        // 豆包 ASR API 调用
        let api_key = self.config.api_key.as_ref()
            .ok_or("Doubao API key not configured")?;

        Err("Doubao ASR not yet implemented".to_string())
    }

    async fn transcribe_whisper(&self, audio: &[f32]) -> Result<TranscriptionResult, String> {
        // 本地 Whisper 模型推理
        Err("Local Whisper not yet implemented".to_string())
    }

    /// 文本转语音
    pub async fn synthesize(&self, request: &TtsRequest) -> Result<Vec<f32>, String> {
        match self.config.backend {
            VoiceBackend::Anthropic => {
                // Anthropic TTS
                Err("Anthropic TTS not yet implemented".to_string())
            }
            _ => Err("TTS not supported for this backend".to_string()),
        }
    }

    /// 获取当前状态
    pub fn state(&self) -> &VoiceState {
        &self.state
    }

    /// 检查是否可用
    pub fn is_available(&self) -> bool {
        self.config.backend != VoiceBackend::Disabled
            && self.config.api_key.is_some()
    }

    /// 获取配置
    pub fn config(&self) -> &VoiceConfig {
        &self.config
    }
}
