use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 穷鬼模式配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetModeConfig {
    /// 是否启用穷鬼模式
    pub enabled: bool,
    /// 跳过记忆提取
    pub skip_memory_extraction: bool,
    /// 跳过提示建议
    pub skip_prompt_suggestion: bool,
    /// 跳过验证代理
    pub skip_verification_agent: bool,
    /// 减少上下文窗口大小
    pub reduced_context_window: Option<usize>,
    /// 禁用自动压缩
    pub disable_auto_compact: bool,
    /// 简化系统提示
    pub simplified_system_prompt: bool,
    /// 最大token限制
    pub max_tokens_per_request: Option<usize>,
    /// 禁用工具搜索
    pub disable_tool_search: bool,
    /// 禁用技能发现
    pub disable_skill_discovery: bool,
}

impl Default for BudgetModeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            skip_memory_extraction: true,
            skip_prompt_suggestion: true,
            skip_verification_agent: true,
            reduced_context_window: Some(50000),
            disable_auto_compact: false,
            simplified_system_prompt: true,
            max_tokens_per_request: Some(4096),
            disable_tool_search: true,
            disable_skill_discovery: true,
        }
    }
}

/// 穷鬼模式统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetModeStats {
    /// 总请求数
    pub total_requests: u64,
    /// 节省的token数
    pub tokens_saved: u64,
    /// 跳过的记忆提取次数
    pub memory_extractions_skipped: u64,
    /// 跳过的提示建议次数
    pub prompt_suggestions_skipped: u64,
    /// 跳过的验证代理次数
    pub verification_agents_skipped: u64,
    /// 上次启用时间
    pub last_enabled_at: Option<chrono::DateTime<chrono::Utc>>,
    /// 总启用时长（秒）
    pub total_enabled_seconds: u64,
}

impl Default for BudgetModeStats {
    fn default() -> Self {
        Self {
            total_requests: 0,
            tokens_saved: 0,
            memory_extractions_skipped: 0,
            prompt_suggestions_skipped: 0,
            verification_agents_skipped: 0,
            last_enabled_at: None,
            total_enabled_seconds: 0,
        }
    }
}

/// 穷鬼模式管理器
pub struct BudgetModeManager {
    config: Arc<RwLock<BudgetModeConfig>>,
    stats: Arc<RwLock<BudgetModeStats>>,
    config_path: Option<PathBuf>,
    enabled_since: Arc<RwLock<Option<chrono::DateTime<chrono::Utc>>>>,
}

impl BudgetModeManager {
    pub fn new() -> Self {
        Self {
            config: Arc::new(RwLock::new(BudgetModeConfig::default())),
            stats: Arc::new(RwLock::new(BudgetModeStats::default())),
            config_path: None,
            enabled_since: Arc::new(RwLock::new(None)),
        }
    }

    /// 创建带配置路径的管理器
    pub fn with_config_path(config_path: PathBuf) -> Self {
        let manager = Self {
            config: Arc::new(RwLock::new(BudgetModeConfig::default())),
            stats: Arc::new(RwLock::new(BudgetModeStats::default())),
            config_path: Some(config_path.clone()),
            enabled_since: Arc::new(RwLock::new(None)),
        };

        // 尝试加载配置
        if config_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&config_path) {
                if let Ok(config) = serde_json::from_str::<BudgetModeConfig>(&content) {
                    let mut config_lock = manager.config.blocking_write();
                    *config_lock = config;
                }
            }
        }

        manager
    }

    /// 保存配置
    async fn save_config(&self) -> Result<(), String> {
        if let Some(path) = &self.config_path {
            let config = self.config.read().await;
            let content = serde_json::to_string_pretty(&*config)
                .map_err(|e| format!("Failed to serialize config: {}", e))?;

            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create config directory: {}", e))?;
            }

            std::fs::write(path, content).map_err(|e| format!("Failed to write config: {}", e))?;
        }
        Ok(())
    }

    /// 启用穷鬼模式
    pub async fn enable(&self) {
        {
            let mut config = self.config.write().await;
            config.enabled = true;
        }

        {
            let mut enabled_since = self.enabled_since.write().await;
            *enabled_since = Some(chrono::Utc::now());
        }

        {
            let mut stats = self.stats.write().await;
            stats.last_enabled_at = Some(chrono::Utc::now());
        }

        let _ = self.save_config().await;
        tracing::info!("Budget mode enabled");
    }

    /// 禁用穷鬼模式
    pub async fn disable(&self) {
        {
            let mut config = self.config.write().await;
            config.enabled = false;
        }

        // 计算启用时长
        let enabled_duration = {
            let enabled_since = self.enabled_since.read().await;
            if let Some(since) = *enabled_since {
                chrono::Utc::now()
                    .signed_duration_since(since)
                    .num_seconds() as u64
            } else {
                0
            }
        };

        {
            let mut stats = self.stats.write().await;
            stats.total_enabled_seconds += enabled_duration;
        }

        {
            let mut enabled_since = self.enabled_since.write().await;
            *enabled_since = None;
        }

        let _ = self.save_config().await;
        tracing::info!("Budget mode disabled");
    }

    /// 切换穷鬼模式
    pub async fn toggle(&self) -> bool {
        let is_enabled = self.is_enabled().await;
        if is_enabled {
            self.disable().await;
        } else {
            self.enable().await;
        }
        !is_enabled
    }

    /// 检查是否启用
    pub async fn is_enabled(&self) -> bool {
        let config = self.config.read().await;
        config.enabled
    }

    /// 获取配置
    pub async fn get_config(&self) -> BudgetModeConfig {
        let config = self.config.read().await;
        config.clone()
    }

    /// 更新配置
    pub async fn update_config(&self, new_config: BudgetModeConfig) {
        {
            let mut config = self.config.write().await;
            *config = new_config;
        }
        let _ = self.save_config().await;
        tracing::info!("Budget mode config updated");
    }

    /// 获取统计信息
    pub async fn get_stats(&self) -> BudgetModeStats {
        let stats = self.stats.read().await;
        stats.clone()
    }

    /// 记录请求
    pub async fn record_request(&self, tokens_saved: u64) {
        let mut stats = self.stats.write().await;
        stats.total_requests += 1;
        stats.tokens_saved += tokens_saved;
    }

    /// 记录跳过的记忆提取
    pub async fn record_memory_extraction_skipped(&self) {
        let mut stats = self.stats.write().await;
        stats.memory_extractions_skipped += 1;
    }

    /// 记录跳过的提示建议
    pub async fn record_prompt_suggestion_skipped(&self) {
        let mut stats = self.stats.write().await;
        stats.prompt_suggestions_skipped += 1;
    }

    /// 记录跳过的验证代理
    pub async fn record_verification_agent_skipped(&self) {
        let mut stats = self.stats.write().await;
        stats.verification_agents_skipped += 1;
    }

    /// 检查是否应该跳过记忆提取
    pub async fn should_skip_memory_extraction(&self) -> bool {
        let config = self.config.read().await;
        if config.enabled && config.skip_memory_extraction {
            drop(config);
            self.record_memory_extraction_skipped().await;
            true
        } else {
            false
        }
    }

    /// 检查是否应该跳过提示建议
    pub async fn should_skip_prompt_suggestion(&self) -> bool {
        let config = self.config.read().await;
        if config.enabled && config.skip_prompt_suggestion {
            drop(config);
            self.record_prompt_suggestion_skipped().await;
            true
        } else {
            false
        }
    }

    /// 检查是否应该跳过验证代理
    pub async fn should_skip_verification_agent(&self) -> bool {
        let config = self.config.read().await;
        if config.enabled && config.skip_verification_agent {
            drop(config);
            self.record_verification_agent_skipped().await;
            true
        } else {
            false
        }
    }

    /// 获取上下文窗口大小
    pub async fn get_context_window_size(&self, default_size: usize) -> usize {
        let config = self.config.read().await;
        if config.enabled {
            config.reduced_context_window.unwrap_or(default_size)
        } else {
            default_size
        }
    }

    /// 获取最大token限制
    pub async fn get_max_tokens(&self, default_max: usize) -> usize {
        let config = self.config.read().await;
        if config.enabled {
            config.max_tokens_per_request.unwrap_or(default_max)
        } else {
            default_max
        }
    }

    /// 检查是否应该禁用自动压缩
    pub async fn should_disable_auto_compact(&self) -> bool {
        let config = self.config.read().await;
        config.enabled && config.disable_auto_compact
    }

    /// 检查是否应该使用简化系统提示
    pub async fn should_use_simplified_system_prompt(&self) -> bool {
        let config = self.config.read().await;
        config.enabled && config.simplified_system_prompt
    }

    /// 检查是否应该禁用工具搜索
    pub async fn should_disable_tool_search(&self) -> bool {
        let config = self.config.read().await;
        config.enabled && config.disable_tool_search
    }

    /// 检查是否应该禁用技能发现
    pub async fn should_disable_skill_discovery(&self) -> bool {
        let config = self.config.read().await;
        config.enabled && config.disable_skill_discovery
    }

    /// 重置统计信息
    pub async fn reset_stats(&self) {
        let mut stats = self.stats.write().await;
        *stats = BudgetModeStats::default();
    }
}

impl Clone for BudgetModeManager {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            stats: self.stats.clone(),
            config_path: self.config_path.clone(),
            enabled_since: self.enabled_since.clone(),
        }
    }
}

/// 穷鬼模式状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetModeStatus {
    pub enabled: bool,
    pub config: BudgetModeConfig,
    pub stats: BudgetModeStats,
    pub enabled_since: Option<chrono::DateTime<chrono::Utc>>,
}

impl BudgetModeStatus {
    pub fn new(
        manager: &BudgetModeManager,
        config: BudgetModeConfig,
        stats: BudgetModeStats,
    ) -> Self {
        Self {
            enabled: config.enabled,
            config,
            stats,
            enabled_since: None,
        }
    }
}
