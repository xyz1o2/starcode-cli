/// 转录存储

use super::SessionTranscript;
use std::collections::HashMap;

/// 转录存储
pub struct TranscriptStorage {
    /// 转录映射
    transcripts: HashMap<String, SessionTranscript>,
    /// 存储路径
    storage_path: Option<String>,
}

impl TranscriptStorage {
    /// 创建新的转录存储
    pub fn new() -> Self {
        Self {
            transcripts: HashMap::new(),
            storage_path: None,
        }
    }

    /// 保存转录
    pub fn save(&mut self, transcript: &SessionTranscript) {
        self.transcripts.insert(transcript.id.clone(), transcript.clone());

        // 限制存储数量
        if self.transcripts.len() > 100 {
            // 删除最旧的
            if let Some(oldest_id) = self.transcripts.keys().next().cloned() {
                self.transcripts.remove(&oldest_id);
            }
        }
    }

    /// 获取转录
    pub fn get(&self, id: &str) -> Option<&SessionTranscript> {
        self.transcripts.get(id)
    }

    /// 获取所有转录
    pub fn get_all(&self) -> Vec<&SessionTranscript> {
        self.transcripts.values().collect()
    }

    /// 删除转录
    pub fn delete(&mut self, id: &str) {
        self.transcripts.remove(id);
    }

    /// 获取转录数量
    pub fn count(&self) -> usize {
        self.transcripts.len()
    }
}
