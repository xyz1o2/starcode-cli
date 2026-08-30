/// 记忆目录系统
/// 
/// 对标claude-code-main的src/memdir/
/// 结构化记忆管理

pub mod memory_age;
pub mod memory_scan;
pub mod memory_types;
pub mod paths;

pub use memory_age::MemoryAgeManager;
pub use memory_scan::MemoryScanner;
pub use memory_types::*;
pub use paths::MemoryPaths;

use serde::{Deserialize, Serialize};

/// 记忆条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    /// 条目ID
    pub id: String,
    /// 记忆类型
    pub memory_type: MemoryType,
    /// 内容
    pub content: String,
    /// 创建时间
    pub created_at: i64,
    /// 最后访问时间
    pub last_accessed_at: i64,
    /// 访问次数
    pub access_count: u32,
    /// 标签
    pub tags: Vec<String>,
    /// 元数据
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
}

/// 记忆目录管理器
pub struct MemdirManager {
    /// 记忆存储
    memories: std::collections::HashMap<String, MemoryEntry>,
    /// 路径管理器
    paths: MemoryPaths,
    /// 老化管理器
    age_manager: MemoryAgeManager,
    /// 扫描器
    scanner: MemoryScanner,
}

impl MemdirManager {
    /// 创建新的记忆目录管理器
    pub fn new() -> Self {
        Self {
            memories: std::collections::HashMap::new(),
            paths: MemoryPaths::new(),
            age_manager: MemoryAgeManager::new(),
            scanner: MemoryScanner::new(),
        }
    }

    /// 添加记忆
    pub fn add_memory(&mut self, memory_type: MemoryType, content: &str, tags: Vec<String>) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();

        let entry = MemoryEntry {
            id: id.clone(),
            memory_type,
            content: content.to_string(),
            created_at: now,
            last_accessed_at: now,
            access_count: 0,
            tags,
            metadata: std::collections::HashMap::new(),
        };

        self.memories.insert(id.clone(), entry);
        id
    }

    /// 获取记忆
    pub fn get_memory(&self, id: &str) -> Option<&MemoryEntry> {
        self.memories.get(id)
    }

    /// 搜索记忆
    pub fn search_memories(&self, query: &str) -> Vec<&MemoryEntry> {
        let query_lower = query.to_lowercase();
        self.memories.values()
            .filter(|m| {
                m.content.to_lowercase().contains(&query_lower) ||
                m.tags.iter().any(|t| t.to_lowercase().contains(&query_lower))
            })
            .collect()
    }

    /// 按类型获取记忆
    pub fn get_memories_by_type(&self, memory_type: &MemoryType) -> Vec<&MemoryEntry> {
        self.memories.values()
            .filter(|m| m.memory_type == *memory_type)
            .collect()
    }

    /// 清理旧记忆
    pub fn cleanup_old_memories(&mut self, max_age_days: i64) {
        let cutoff = chrono::Utc::now().timestamp() - (max_age_days * 86400);
        self.memories.retain(|_, m| m.created_at > cutoff);
    }

    /// 获取记忆统计
    pub fn get_statistics(&self) -> MemoryStatistics {
        MemoryStatistics {
            total_memories: self.memories.len() as u64,
            by_type: self.count_by_type(),
            oldest_memory: self.oldest_memory_timestamp(),
            newest_memory: self.newest_memory_timestamp(),
        }
    }

    fn count_by_type(&self) -> std::collections::HashMap<MemoryType, u64> {
        let mut counts = std::collections::HashMap::new();
        for memory in self.memories.values() {
            *counts.entry(memory.memory_type.clone()).or_insert(0) += 1;
        }
        counts
    }

    fn oldest_memory_timestamp(&self) -> Option<i64> {
        self.memories.values().map(|m| m.created_at).min()
    }

    fn newest_memory_timestamp(&self) -> Option<i64> {
        self.memories.values().map(|m| m.created_at).max()
    }
}

/// 记忆统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStatistics {
    pub total_memories: u64,
    pub by_type: std::collections::HashMap<MemoryType, u64>,
    pub oldest_memory: Option<i64>,
    pub newest_memory: Option<i64>,
}
