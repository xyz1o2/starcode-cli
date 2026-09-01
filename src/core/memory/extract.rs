/// 记忆提取系统
///
/// 对标claude-code-main的src/services/extractMemories/
/// 自动从对话中提取有价值的记忆信息
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 记忆类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum MemoryType {
    /// 代码偏好
    CodePreference,
    /// 项目知识
    ProjectKnowledge,
    /// 用户习惯
    UserHabit,
    /// 错误解决方案
    ErrorSolution,
    /// 工作流程
    Workflow,
    /// 架构决策
    ArchitectureDecision,
}

/// 提取的记忆
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedMemory {
    /// 记忆ID
    pub id: String,
    /// 记忆类型
    pub memory_type: MemoryType,
    /// 记忆内容
    pub content: String,
    /// 相关上下文
    pub context: String,
    /// 置信度（0.0-1.0）
    pub confidence: f64,
    /// 提取时间
    pub extracted_at: i64,
    /// 来源（对话ID、文件路径等）
    pub source: String,
    /// 标签
    pub tags: Vec<String>,
}

/// 记忆提取器配置
#[derive(Debug, Clone)]
pub struct MemoryExtractorConfig {
    /// 是否启用自动提取
    pub enabled: bool,
    /// 最小置信度阈值
    pub min_confidence: f64,
    /// 最大记忆数量
    pub max_memories: usize,
    /// 提取的语言（用于提示词）
    pub language: String,
}

impl Default for MemoryExtractorConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_confidence: 0.7,
            max_memories: 1000,
            language: "en".to_string(),
        }
    }
}

impl MemoryExtractorConfig {
    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_MEMORY_EXTRACTION_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let min_confidence = std::env::var("STAR_MEMORY_MIN_CONFIDENCE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.7);

        let max_memories = std::env::var("STAR_MEMORY_MAX_COUNT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1000);

        let language = std::env::var("STAR_MEMORY_LANGUAGE").unwrap_or_else(|_| "en".to_string());

        Self {
            enabled,
            min_confidence,
            max_memories,
            language,
        }
    }
}

/// 记忆提取器
///
/// 从对话历史中自动提取有价值的记忆
pub struct MemoryExtractor {
    config: MemoryExtractorConfig,
    /// 已提取的记忆
    memories: Vec<ExtractedMemory>,
    /// 提取统计
    stats: ExtractionStats,
}

/// 提取统计
#[derive(Debug, Default)]
pub struct ExtractionStats {
    pub total_extractions: u64,
    pub successful_extractions: u64,
    pub failed_extractions: u64,
    pub memories_by_type: HashMap<MemoryType, u64>,
}

impl MemoryExtractor {
    /// 创建新的记忆提取器
    pub fn new(config: MemoryExtractorConfig) -> Self {
        Self {
            config,
            memories: Vec::new(),
            stats: ExtractionStats::default(),
        }
    }

    /// 从环境变量创建
    pub fn from_env() -> Self {
        Self::new(MemoryExtractorConfig::from_env())
    }

    /// 分析对话并提取记忆
    pub fn extract_from_conversation(
        &mut self,
        messages: &[crate::types::StarMessage],
        session_id: &str,
    ) -> Vec<ExtractedMemory> {
        if !self.config.enabled {
            return Vec::new();
        }

        let mut extracted = Vec::new();

        // 分析用户消息中的偏好
        for msg in messages.iter().filter(|m| m.role == "user") {
            if let Some(content) = &msg.content {
                if let Some(memory) = self.extract_preference(content, session_id) {
                    extracted.push(memory);
                }
            }
        }

        // 分析助手消息中的项目知识
        for msg in messages.iter().filter(|m| m.role == "assistant") {
            if let Some(content) = &msg.content {
                if let Some(memory) = self.extract_project_knowledge(content, session_id) {
                    extracted.push(memory);
                }
            }
        }

        // 分析工具调用中的工作流程
        for msg in messages.iter().filter(|m| m.role == "assistant") {
            if let Some(tool_calls) = &msg.tool_calls {
                for tc in tool_calls {
                    if let Some(memory) = self.extract_workflow(&tc.function.name, session_id) {
                        extracted.push(memory);
                    }
                }
            }
        }

        // 过滤低置信度记忆
        extracted.retain(|m| m.confidence >= self.config.min_confidence);

        // 添加到记忆列表
        for memory in &extracted {
            self.memories.push(memory.clone());
            *self
                .stats
                .memories_by_type
                .entry(memory.memory_type.clone())
                .or_insert(0) += 1;
        }

        self.stats.total_extractions += 1;
        self.stats.successful_extractions += extracted.len() as u64;

        extracted
    }

    /// 提取用户偏好
    fn extract_preference(&self, content: &str, session_id: &str) -> Option<ExtractedMemory> {
        let content_lower = content.to_lowercase();

        // 检测偏好关键词
        let preference_indicators = [
            "i prefer",
            "i like",
            "i want",
            "please use",
            "always use",
            "don't use",
            "never use",
            "my preference",
            "i usually",
        ];

        for indicator in &preference_indicators {
            if content_lower.contains(indicator) {
                return Some(ExtractedMemory {
                    id: uuid::Uuid::new_v4().to_string(),
                    memory_type: MemoryType::CodePreference,
                    content: content.to_string(),
                    context: format!("User preference detected in session {}", session_id),
                    confidence: 0.8,
                    extracted_at: chrono::Utc::now().timestamp(),
                    source: session_id.to_string(),
                    tags: vec!["preference".to_string()],
                });
            }
        }

        None
    }

    /// 提取项目知识
    fn extract_project_knowledge(
        &self,
        content: &str,
        session_id: &str,
    ) -> Option<ExtractedMemory> {
        let content_lower = content.to_lowercase();

        // 检测架构决策
        let architecture_indicators = [
            "the architecture",
            "we use",
            "the project uses",
            "this is because",
            "the reason for",
            "we decided",
            "the design",
        ];

        for indicator in &architecture_indicators {
            if content_lower.contains(indicator) {
                return Some(ExtractedMemory {
                    id: uuid::Uuid::new_v4().to_string(),
                    memory_type: MemoryType::ArchitectureDecision,
                    content: content.to_string(),
                    context: format!("Architecture knowledge from session {}", session_id),
                    confidence: 0.75,
                    extracted_at: chrono::Utc::now().timestamp(),
                    source: session_id.to_string(),
                    tags: vec!["architecture".to_string()],
                });
            }
        }

        None
    }

    /// 提取工作流程
    fn extract_workflow(&self, tool_name: &str, session_id: &str) -> Option<ExtractedMemory> {
        // 记录常用的工作流程模式
        let workflow_patterns = [
            ("Bash", "git commit", "Git commit workflow"),
            ("Bash", "cargo test", "Rust testing workflow"),
            ("Bash", "npm run", "Node.js script workflow"),
        ];

        for (tool, pattern, description) in &workflow_patterns {
            if tool_name == *tool {
                return Some(ExtractedMemory {
                    id: uuid::Uuid::new_v4().to_string(),
                    memory_type: MemoryType::Workflow,
                    content: description.to_string(),
                    context: format!("Workflow detected: {} in session {}", tool_name, session_id),
                    confidence: 0.7,
                    extracted_at: chrono::Utc::now().timestamp(),
                    source: session_id.to_string(),
                    tags: vec!["workflow".to_string(), tool_name.to_lowercase()],
                });
            }
        }

        None
    }

    /// 获取所有记忆
    pub fn memories(&self) -> &[ExtractedMemory] {
        &self.memories
    }

    /// 按类型获取记忆
    pub fn memories_by_type(&self, memory_type: &MemoryType) -> Vec<&ExtractedMemory> {
        self.memories
            .iter()
            .filter(|m| m.memory_type == *memory_type)
            .collect()
    }

    /// 获取统计信息
    pub fn stats(&self) -> &ExtractionStats {
        &self.stats
    }

    /// 清理旧记忆
    pub fn cleanup_old_memories(&mut self, max_age_days: i64) {
        let cutoff = chrono::Utc::now().timestamp() - (max_age_days * 86400);
        self.memories.retain(|m| m.extracted_at > cutoff);
    }

    /// 导出记忆为JSON
    pub fn export_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(&self.memories)
    }

    /// 从JSON导入记忆
    pub fn import_json(&mut self, json: &str) -> Result<(), serde_json::Error> {
        let imported: Vec<ExtractedMemory> = serde_json::from_str(json)?;
        self.memories.extend(imported);
        Ok(())
    }
}
