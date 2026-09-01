/// 记忆老化管理

use super::MemoryEntry;

/// 记忆老化管理器
pub struct MemoryAgeManager {
    /// 最大年龄（天）
    max_age_days: i64,
}

impl MemoryAgeManager {
    /// 创建新的记忆老化管理器
    pub fn new() -> Self {
        Self {
            max_age_days: 30,
        }
    }

    /// 检查记忆是否过期
    pub fn is_expired(&self, memory: &MemoryEntry) -> bool {
        let now = chrono::Utc::now().timestamp();
        let age_days = (now - memory.created_at) / 86400;
        age_days > self.max_age_days
    }

    /// 获取记忆年龄（天）
    pub fn get_age_days(&self, memory: &MemoryEntry) -> i64 {
        let now = chrono::Utc::now().timestamp();
        (now - memory.created_at) / 86400
    }
}
