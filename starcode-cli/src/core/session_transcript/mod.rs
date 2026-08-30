/// 会话转录系统
/// 
/// 对标claude-code-main的src/services/sessionTranscript/
/// 记录和管理会话转录

pub mod format;
pub mod recorder;
pub mod storage;

pub use format::TranscriptFormat;
pub use recorder::TranscriptRecorder;
pub use storage::TranscriptStorage;

use serde::{Deserialize, Serialize};

/// 转录条目类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EntryType {
    /// 用户消息
    UserMessage,
    /// 助手响应
    AssistantResponse,
    /// 工具调用
    ToolCall,
    /// 工具结果
    ToolResult,
    /// 系统消息
    SystemMessage,
    /// 错误
    Error,
}

/// 转录条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptEntry {
    /// 条目ID
    pub id: String,
    /// 条目类型
    pub entry_type: EntryType,
    /// 时间戳
    pub timestamp: i64,
    /// 内容
    pub content: String,
    /// 元数据
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
}

/// 会话转录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTranscript {
    /// 转录ID
    pub id: String,
    /// 会话ID
    pub session_id: String,
    /// 开始时间
    pub started_at: i64,
    /// 结束时间
    pub ended_at: Option<i64>,
    /// 条目列表
    pub entries: Vec<TranscriptEntry>,
    /// 元数据
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
}

/// 会话转录管理器
pub struct SessionTranscriptManager {
    /// 转录存储
    storage: TranscriptStorage,
    /// 当前转录
    current_transcript: Option<SessionTranscript>,
    /// 是否启用
    enabled: bool,
}

impl SessionTranscriptManager {
    /// 创建新的会话转录管理器
    pub fn new() -> Self {
        Self {
            storage: TranscriptStorage::new(),
            current_transcript: None,
            enabled: true,
        }
    }

    /// 从环境变量创建
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_SESSION_TRANSCRIPT_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        Self {
            storage: TranscriptStorage::new(),
            current_transcript: None,
            enabled,
        }
    }

    /// 开始新转录
    pub fn start_transcript(&mut self, session_id: &str) -> String {
        if !self.enabled {
            return String::new();
        }

        let id = uuid::Uuid::new_v4().to_string();
        let transcript = SessionTranscript {
            id: id.clone(),
            session_id: session_id.to_string(),
            started_at: chrono::Utc::now().timestamp(),
            ended_at: None,
            entries: Vec::new(),
            metadata: std::collections::HashMap::new(),
        };

        self.current_transcript = Some(transcript);
        id
    }

    /// 添加条目
    pub fn add_entry(&mut self, entry_type: EntryType, content: &str) {
        if !self.enabled {
            return;
        }

        if let Some(transcript) = &mut self.current_transcript {
            let entry = TranscriptEntry {
                id: uuid::Uuid::new_v4().to_string(),
                entry_type,
                timestamp: chrono::Utc::now().timestamp(),
                content: content.to_string(),
                metadata: std::collections::HashMap::new(),
            };

            transcript.entries.push(entry);
        }
    }

    /// 结束转录
    pub fn end_transcript(&mut self) -> Option<SessionTranscript> {
        if !self.enabled {
            return None;
        }

        if let Some(mut transcript) = self.current_transcript.take() {
            transcript.ended_at = Some(chrono::Utc::now().timestamp());
            
            // 保存到存储
            self.storage.save(&transcript);
            
            Some(transcript)
        } else {
            None
        }
    }

    /// 获取当前转录
    pub fn current_transcript(&self) -> Option<&SessionTranscript> {
        self.current_transcript.as_ref()
    }

    /// 获取转录历史
    pub fn get_history(&self) -> Vec<&SessionTranscript> {
        self.storage.get_all()
    }

    /// 检查是否启用
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}
