/// 转录记录器

use super::{SessionTranscript, TranscriptEntry, EntryType};

/// 转录记录器
pub struct TranscriptRecorder {
    /// 是否正在记录
    recording: bool,
    /// 条目缓冲区
    buffer: Vec<TranscriptEntry>,
}

impl TranscriptRecorder {
    /// 创建新的转录记录器
    pub fn new() -> Self {
        Self {
            recording: false,
            buffer: Vec::new(),
        }
    }

    /// 开始记录
    pub fn start(&mut self) {
        self.recording = true;
        self.buffer.clear();
    }

    /// 停止记录
    pub fn stop(&mut self) -> Vec<TranscriptEntry> {
        self.recording = false;
        std::mem::take(&mut self.buffer)
    }

    /// 记录条目
    pub fn record(&mut self, entry_type: EntryType, content: &str) {
        if !self.recording {
            return;
        }

        let entry = TranscriptEntry {
            id: uuid::Uuid::new_v4().to_string(),
            entry_type,
            timestamp: chrono::Utc::now().timestamp(),
            content: content.to_string(),
            metadata: std::collections::HashMap::new(),
        };

        self.buffer.push(entry);
    }

    /// 检查是否正在记录
    pub fn is_recording(&self) -> bool {
        self.recording
    }

    /// 获取缓冲区大小
    pub fn buffer_size(&self) -> usize {
        self.buffer.len()
    }
}
