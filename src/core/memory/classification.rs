//! 记忆四类型分类模块
//!
//! 对标 Claude Code 的 project-memory.mdx：
//! - user: 用户偏好和反馈
//! - feedback: 代码审查反馈
//! - project: 项目特定知识
//! - reference: 参考资料和最佳实践

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// 记忆类型
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MemoryType {
    /// 用户偏好
    User,
    /// 反馈
    Feedback,
    /// 项目知识
    Project,
    /// 参考资料
    Reference,
}

impl std::fmt::Display for MemoryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryType::User => write!(f, "user"),
            MemoryType::Feedback => write!(f, "feedback"),
            MemoryType::Project => write!(f, "project"),
            MemoryType::Reference => write!(f, "reference"),
        }
    }
}

impl MemoryType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "user" => Some(MemoryType::User),
            "feedback" => Some(MemoryType::Feedback),
            "project" => Some(MemoryType::Project),
            "reference" => Some(MemoryType::Reference),
            _ => None,
        }
    }

    /// 获取存储子目录
    pub fn subdirectory(&self) -> &str {
        match self {
            MemoryType::User => "user",
            MemoryType::Feedback => "feedback",
            MemoryType::Project => "project",
            MemoryType::Reference => "reference",
        }
    }
}

/// 分类记忆条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifiedMemory {
    pub id: String,
    pub memory_type: MemoryType,
    pub title: String,
    pub content: String,
    pub tags: Vec<String>,
    pub relevance_score: f32,
    pub created_at: u64,
    pub accessed_at: u64,
    pub access_count: u32,
    pub source: Option<String>,
}

/// MEMORY.md 索引条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryIndexEntry {
    pub id: String,
    pub memory_type: MemoryType,
    pub title: String,
    pub file_path: String,
    pub tags: Vec<String>,
    pub relevance_hint: f32,
}

/// 四类型记忆管理器
pub struct ClassifiedMemoryManager {
    base_dir: PathBuf,
    index: Vec<MemoryIndexEntry>,
}

impl ClassifiedMemoryManager {
    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            base_dir,
            index: Vec::new(),
        }
    }

    /// 初始化目录结构
    pub fn initialize(&self) -> Result<(), String> {
        for mem_type in [
            MemoryType::User,
            MemoryType::Feedback,
            MemoryType::Project,
            MemoryType::Reference,
        ] {
            let dir = self.base_dir.join(mem_type.subdirectory());
            std::fs::create_dir_all(&dir)
                .map_err(|e| format!("Failed to create {}: {}", dir.display(), e))?;
        }
        Ok(())
    }

    /// 存储记忆
    pub fn store(&mut self, memory: ClassifiedMemory) -> Result<(), String> {
        let dir = self.base_dir.join(memory.memory_type.subdirectory());
        std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create dir: {}", e))?;

        let file_path = dir.join(format!("{}.md", memory.id));
        let content = format!(
            "---\nid: {}\ntype: {}\ntitle: {}\ntags: [{}]\ncreated_at: {}\n---\n\n{}",
            memory.id,
            memory.memory_type,
            memory.title,
            memory.tags.join(", "),
            memory.created_at,
            memory.content
        );

        std::fs::write(&file_path, content)
            .map_err(|e| format!("Failed to write memory: {}", e))?;

        // 更新索引
        self.index.push(MemoryIndexEntry {
            id: memory.id.clone(),
            memory_type: memory.memory_type,
            title: memory.title,
            file_path: file_path.to_string_lossy().to_string(),
            tags: memory.tags,
            relevance_hint: memory.relevance_score,
        });

        Ok(())
    }

    /// 智能召回（按相关性筛选）
    pub fn recall(&self, query: &str, limit: usize) -> Vec<&MemoryIndexEntry> {
        let query_lower = query.to_lowercase();
        let query_words: Vec<&str> = query_lower.split_whitespace().collect();

        let mut scored: Vec<(f32, &MemoryIndexEntry)> = self
            .index
            .iter()
            .filter_map(|entry| {
                let score = self.calculate_recall_score(entry, &query_lower, &query_words);
                if score > 0.0 {
                    Some((score, entry))
                } else {
                    None
                }
            })
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored
            .into_iter()
            .take(limit)
            .map(|(_, entry)| entry)
            .collect()
    }

    fn calculate_recall_score(
        &self,
        entry: &MemoryIndexEntry,
        query: &str,
        query_words: &[&str],
    ) -> f32 {
        let mut score = entry.relevance_hint;

        let title_lower = entry.title.to_lowercase();
        let tags_lower: Vec<String> = entry.tags.iter().map(|t| t.to_lowercase()).collect();

        // 标题匹配
        if title_lower.contains(query) {
            score += 5.0;
        }

        for word in query_words {
            if title_lower.contains(word) {
                score += 2.0;
            }
            for tag in &tags_lower {
                if tag.contains(word) {
                    score += 1.5;
                }
            }
        }

        // 记忆类型权重
        score += match entry.memory_type {
            MemoryType::Project => 1.0, // 项目记忆优先
            MemoryType::User => 0.8,
            MemoryType::Feedback => 0.6,
            MemoryType::Reference => 0.4,
        };

        score
    }

    /// 按类型获取记忆
    pub fn by_type(&self, memory_type: &MemoryType) -> Vec<&MemoryIndexEntry> {
        self.index
            .iter()
            .filter(|e| &e.memory_type == memory_type)
            .collect()
    }

    /// 加载索引
    pub fn load_index(&mut self) -> Result<(), String> {
        let index_path = self.base_dir.join("MEMORY.md");
        if !index_path.exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(&index_path)
            .map_err(|e| format!("Failed to read MEMORY.md: {}", e))?;

        // 简单解析 MEMORY.md 中的索引
        for line in content.lines() {
            if line.starts_with("- ") {
                // 格式: - [type] title (id)
                let parts: Vec<&str> = line[2..].splitn(3, ' ').collect();
                if parts.len() >= 3 {
                    let mem_type =
                        MemoryType::from_str(parts[0].trim_matches(|c| c == '[' || c == ']'))
                            .unwrap_or(MemoryType::Project);
                    self.index.push(MemoryIndexEntry {
                        id: parts[2].trim_matches(|c| c == '(' || c == ')').to_string(),
                        memory_type: mem_type,
                        title: parts[1].to_string(),
                        file_path: String::new(),
                        tags: Vec::new(),
                        relevance_hint: 0.5,
                    });
                }
            }
        }

        Ok(())
    }

    /// 保存索引到 MEMORY.md
    pub fn save_index(&self) -> Result<(), String> {
        let index_path = self.base_dir.join("MEMORY.md");
        let mut content = String::from("# Memory Index\n\n");

        for entry in &self.index {
            content.push_str(&format!(
                "- [{}] {} ({})\n",
                entry.memory_type, entry.title, entry.id
            ));
        }

        std::fs::write(&index_path, content)
            .map_err(|e| format!("Failed to write MEMORY.md: {}", e))?;

        Ok(())
    }

    /// 验证记忆是否仍然有效（记忆漂移防御）
    pub fn validate_memory(&self, memory: &ClassifiedMemory) -> bool {
        // 检查引用的文件是否仍然存在
        if let Some(source) = &memory.source {
            if source.contains('/') || source.contains('\\') {
                return std::path::Path::new(source).exists();
            }
        }
        true
    }

    /// 获取统计信息
    pub fn stats(&self) -> HashMap<String, usize> {
        let mut stats = HashMap::new();
        for entry in &self.index {
            *stats.entry(entry.memory_type.to_string()).or_insert(0) += 1;
        }
        stats.insert("total".to_string(), self.index.len());
        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_type_parsing() {
        assert_eq!(MemoryType::from_str("user"), Some(MemoryType::User));
        assert_eq!(MemoryType::from_str("FEEDBACK"), Some(MemoryType::Feedback));
        assert_eq!(MemoryType::from_str("project"), Some(MemoryType::Project));
        assert_eq!(
            MemoryType::from_str("reference"),
            Some(MemoryType::Reference)
        );
        assert_eq!(MemoryType::from_str("unknown"), None);
    }

    #[test]
    fn test_recall_scoring() {
        let mut mgr = ClassifiedMemoryManager::new(PathBuf::from("/tmp/test_memory"));

        mgr.index.push(MemoryIndexEntry {
            id: "1".to_string(),
            memory_type: MemoryType::Project,
            title: "Rust error handling patterns".to_string(),
            file_path: "/tmp/test".to_string(),
            tags: vec!["rust".to_string(), "error".to_string()],
            relevance_hint: 0.8,
        });

        mgr.index.push(MemoryIndexEntry {
            id: "2".to_string(),
            memory_type: MemoryType::User,
            title: "Prefer using anyhow".to_string(),
            file_path: "/tmp/test".to_string(),
            tags: vec!["rust".to_string(), "preference".to_string()],
            relevance_hint: 0.5,
        });

        let results = mgr.recall("rust error handling", 5);
        assert!(!results.is_empty());
        // Project memory should rank higher
        assert_eq!(results[0].id, "1");
    }
}
