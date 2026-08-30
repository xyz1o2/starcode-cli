/// 团队记忆同步系统
/// 
/// 对标claude-code-main的src/services/teamMemorySync/
/// 团队协作记忆共享，含密钥扫描和安全守卫

pub mod scanner;
pub mod security;

pub use scanner::SecretScanner;
pub use security::SecurityGuard;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 团队记忆配置
#[derive(Debug, Clone)]
pub struct TeamMemoryConfig {
    /// 是否启用
    pub enabled: bool,
    /// 团队ID
    pub team_id: Option<String>,
    /// 同步端点
    pub sync_endpoint: Option<String>,
    /// 是否启用密钥扫描
    pub secret_scanning: bool,
    /// 是否启用安全守卫
    pub security_guard: bool,
}

impl Default for TeamMemoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            team_id: None,
            sync_endpoint: None,
            secret_scanning: true,
            security_guard: true,
        }
    }
}

impl TeamMemoryConfig {
    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_TEAM_MEMORY_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(false);

        let team_id = std::env::var("STAR_TEAM_ID").ok();
        let sync_endpoint = std::env::var("STAR_TEAM_SYNC_ENDPOINT").ok();

        let secret_scanning = std::env::var("STAR_TEAM_SECRET_SCANNING")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        Self {
            enabled,
            team_id,
            sync_endpoint,
            secret_scanning,
            security_guard: true,
        }
    }
}

/// 团队记忆条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMemoryEntry {
    /// 条目ID
    pub id: String,
    /// 团队ID
    pub team_id: String,
    /// 作者
    pub author: String,
    /// 内容
    pub content: String,
    /// 标签
    pub tags: Vec<String>,
    /// 创建时间
    pub created_at: i64,
    /// 更新时间
    pub updated_at: i64,
    /// 是否包含敏感信息
    pub contains_secrets: bool,
}

/// 团队记忆管理器
pub struct TeamMemoryManager {
    config: TeamMemoryConfig,
    memories: Vec<TeamMemoryEntry>,
    /// 密钥扫描器
    secret_scanner: SecretScanner,
    /// 安全守卫
    security_guard: SecurityGuard,
}

impl TeamMemoryManager {
    pub fn new(config: TeamMemoryConfig) -> Self {
        Self {
            secret_scanner: SecretScanner::new(),
            security_guard: SecurityGuard::new(),
            config,
            memories: Vec::new(),
        }
    }

    /// 添加团队记忆
    pub fn add_memory(&mut self, author: &str, content: &str, tags: Vec<String>) -> Result<String, TeamMemoryError> {
        // 安全检查
        if self.config.security_guard {
            self.security_guard.check_content(content)?;
        }

        // 密钥扫描
        let contains_secrets = if self.config.secret_scanning {
            self.secret_scanner.contains_secrets(content)
        } else {
            false
        };

        // 如果包含密钥，拒绝存储
        if contains_secrets {
            return Err(TeamMemoryError::SecretDetected);
        }

        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();

        let entry = TeamMemoryEntry {
            id: id.clone(),
            team_id: self.config.team_id.clone().unwrap_or_default(),
            author: author.to_string(),
            content: content.to_string(),
            tags,
            created_at: now,
            updated_at: now,
            contains_secrets,
        };

        self.memories.push(entry);
        Ok(id)
    }

    /// 搜索团队记忆
    pub fn search_memories(&self, query: &str) -> Vec<&TeamMemoryEntry> {
        let query_lower = query.to_lowercase();
        self.memories.iter()
            .filter(|m| {
                m.content.to_lowercase().contains(&query_lower) ||
                m.tags.iter().any(|t| t.to_lowercase().contains(&query_lower))
            })
            .collect()
    }

    /// 按标签获取记忆
    pub fn get_memories_by_tag(&self, tag: &str) -> Vec<&TeamMemoryEntry> {
        self.memories.iter()
            .filter(|m| m.tags.contains(&tag.to_string()))
            .collect()
    }

    /// 获取所有记忆
    pub fn get_all_memories(&self) -> &[TeamMemoryEntry] {
        &self.memories
    }

    /// 清理旧记忆
    pub fn cleanup_old_memories(&mut self, max_age_days: i64) {
        let cutoff = chrono::Utc::now().timestamp() - (max_age_days * 86400);
        self.memories.retain(|m| m.created_at > cutoff);
    }
}

/// 团队记忆错误
#[derive(Debug)]
pub enum TeamMemoryError {
    /// 检测到密钥
    SecretDetected,
    /// 安全检查失败
    SecurityCheckFailed(String),
    /// 配置错误
    ConfigError(String),
}

impl std::fmt::Display for TeamMemoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TeamMemoryError::SecretDetected => write!(f, "Secret detected in content"),
            TeamMemoryError::SecurityCheckFailed(e) => write!(f, "Security check failed: {}", e),
            TeamMemoryError::ConfigError(e) => write!(f, "Config error: {}", e),
        }
    }
}

impl std::error::Error for TeamMemoryError {}
