/// Auto Dream后台记忆整合系统
/// 
/// 对标claude-code-main的src/services/autoDream/
/// 在后台自动整合多个会话的记忆

pub mod config;
pub mod consolidation;
pub mod prompt;

pub use config::AutoDreamConfig;
pub use consolidation::ConsolidationEngine;
pub use prompt::ConsolidationPrompt;

use serde::{Deserialize, Serialize};

/// 整合状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ConsolidationState {
    /// 空闲
    Idle,
    /// 整合中
    Consolidating,
    /// 完成
    Completed,
    /// 错误
    Error(String),
}

/// 整合结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationResult {
    /// 结果ID
    pub id: String,
    /// 整合的记忆数
    pub memories_consolidated: u32,
    /// 生成的洞察数
    pub insights_generated: u32,
    /// 整合时间
    pub consolidated_at: i64,
    /// 摘要
    pub summary: String,
    /// 洞察
    pub insights: Vec<String>,
}

/// Auto Dream管理器
pub struct AutoDreamManager {
    /// 配置
    config: AutoDreamConfig,
    /// 整合引擎
    engine: ConsolidationEngine,
    /// 整合状态
    state: ConsolidationState,
    /// 整合历史
    history: Vec<ConsolidationResult>,
}

impl AutoDreamManager {
    /// 创建新的Auto Dream管理器
    pub fn new(config: AutoDreamConfig) -> Self {
        Self {
            config,
            engine: ConsolidationEngine::new(),
            state: ConsolidationState::Idle,
            history: Vec::new(),
        }
    }

    /// 从环境变量创建
    pub fn from_env() -> Self {
        Self::new(AutoDreamConfig::from_env())
    }

    /// 触发整合
    pub fn consolidate(&mut self, memories: &[String]) -> Result<ConsolidationResult, DreamError> {
        if !self.config.enabled {
            return Err(DreamError::NotEnabled);
        }

        self.state = ConsolidationState::Consolidating;

        // 执行整合
        let result = self.engine.consolidate(memories)?;

        // 保存结果
        self.history.push(result.clone());
        self.state = ConsolidationState::Completed;

        // 限制历史记录大小
        if self.history.len() > 100 {
            self.history.remove(0);
        }

        Ok(result)
    }

    /// 获取整合历史
    pub fn get_history(&self) -> &[ConsolidationResult] {
        &self.history
    }

    /// 获取状态
    pub fn state(&self) -> &ConsolidationState {
        &self.state
    }

    /// 检查是否启用
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }
}

/// Dream错误
#[derive(Debug)]
pub enum DreamError {
    /// 未启用
    NotEnabled,
    /// 整合错误
    ConsolidationError(String),
    /// 配置错误
    ConfigError(String),
}

impl std::fmt::Display for DreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DreamError::NotEnabled => write!(f, "Auto Dream is not enabled"),
            DreamError::ConsolidationError(e) => write!(f, "Consolidation error: {}", e),
            DreamError::ConfigError(e) => write!(f, "Config error: {}", e),
        }
    }
}

impl std::error::Error for DreamError {}
