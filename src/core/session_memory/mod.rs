/// 会话记忆系统
///
/// 对标claude-code-main的src/services/SessionMemory/
/// 管理跨会话的记忆和上下文
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 会话记忆条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMemoryEntry {
    /// 条目ID
    pub id: String,
    /// 会话ID
    pub session_id: String,
    /// 记忆类型
    pub memory_type: String,
    /// 内容
    pub content: String,
    /// 创建时间
    pub created_at: i64,
    /// 元数据
    pub metadata: HashMap<String, serde_json::Value>,
}

/// 会话记忆管理器
pub struct SessionMemoryManager {
    /// 记忆存储
    memories: Vec<SessionMemoryEntry>,
    /// 最大记忆数
    max_memories: usize,
}

impl SessionMemoryManager {
    pub fn new(max_memories: usize) -> Self {
        Self {
            memories: Vec::new(),
            max_memories,
        }
    }

    /// 添加记忆
    pub fn add_memory(&mut self, session_id: &str, memory_type: &str, content: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();

        let entry = SessionMemoryEntry {
            id: id.clone(),
            session_id: session_id.to_string(),
            memory_type: memory_type.to_string(),
            content: content.to_string(),
            created_at: chrono::Utc::now().timestamp(),
            metadata: HashMap::new(),
        };

        self.memories.push(entry);

        // 限制记忆数量
        if self.memories.len() > self.max_memories {
            self.memories.remove(0);
        }

        id
    }

    /// 获取会话记忆
    pub fn get_session_memories(&self, session_id: &str) -> Vec<&SessionMemoryEntry> {
        self.memories
            .iter()
            .filter(|m| m.session_id == session_id)
            .collect()
    }

    /// 搜索记忆
    pub fn search_memories(&self, query: &str) -> Vec<&SessionMemoryEntry> {
        let query_lower = query.to_lowercase();
        self.memories
            .iter()
            .filter(|m| m.content.to_lowercase().contains(&query_lower))
            .collect()
    }

    /// 清理旧记忆
    pub fn cleanup_old_memories(&mut self, max_age_seconds: i64) {
        let cutoff = chrono::Utc::now().timestamp() - max_age_seconds;
        self.memories.retain(|m| m.created_at > cutoff);
    }
}
