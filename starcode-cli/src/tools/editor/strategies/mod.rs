// ============================================================================
// 编辑策略模块
// ============================================================================
//
// 定义编辑策略的统一接口，所有具体策略实现此接口
//
// 策略列表：
// 1. ExactMatchStrategy - 精确字符串匹配（最快）
// 2. FlexibleIndentStrategy - 弹性缩进匹配（容忍空白差异）
// 3. RegexFuzzyStrategy - 正则模糊匹配（最灵活）
// 4. LlmFixStrategy - LLM 处理策略（最后手段）

use super::{EditContext, EditResult};
use async_trait::async_trait;

pub mod exact;
pub mod flexible;
pub mod llm_fix;
pub mod regex_fuzzy;

// 重新导出策略
pub use exact::ExactMatchStrategy;
pub use flexible::FlexibleIndentStrategy;
pub use llm_fix::LlmFixStrategy;
pub use regex_fuzzy::RegexFuzzyStrategy;

/// 编辑策略接口
///
/// 所有编辑策略必须实现此 trait
#[async_trait]
pub trait EditStrategy: Send + Sync {
    /// 策略名称（用于日志和遥测）
    fn name(&self) -> &'static str;

    /// 尝试执行编辑
    ///
    /// # Arguments
    /// * `context` - 编辑上下文（文件路径、内容、old/new字符串）
    ///
    /// # Returns
    /// * `Ok(Some(result))` - 成功执行编辑
    /// * `Ok(None)` - 此策略不适用，应尝试下一个策略
    /// * `Err(e)` - 执行失败
    async fn try_edit(
        &self,
        context: &EditContext,
    ) -> Result<Option<EditResult>, Box<dyn std::error::Error + Send + Sync>>;

    /// 策略优先级（数字越小优先级越高）
    ///
    /// 默认优先级：
    /// - ExactMatch: 0 (最快)
    /// - FlexibleIndent: 10
    /// - RegexFuzzy: 20
    /// - LlmFix: 100 (最慢，最后尝试)
    fn priority(&self) -> u32 {
        50
    }

    /// 是否应该启用此策略
    ///
    /// 可以根据环境变量或配置动态启用/禁用策略
    fn is_enabled(&self) -> bool {
        true
    }
}

/// 策略工厂
pub struct StrategyFactory;

impl StrategyFactory {
    /// 创建所有可用策略（按优先级排序）
    pub fn create_all_strategies(
        llm_client: Option<crate::llm::client::StarClient>,
    ) -> Vec<Box<dyn EditStrategy>> {
        let mut strategies: Vec<Box<dyn EditStrategy>> = vec![
            Box::new(ExactMatchStrategy),
            Box::new(FlexibleIndentStrategy),
            Box::new(RegexFuzzyStrategy),
        ];

        // 如果有 LLM 客户端，添加 LLM 处理策略
        if let Some(client) = llm_client {
            strategies.push(Box::new(LlmFixStrategy::new(client)));
        }

        // 按优先级排序
        strategies.sort_by_key(|s| s.priority());

        // 过滤禁用的策略
        strategies.into_iter().filter(|s| s.is_enabled()).collect()
    }

    /// 创建基础策略（不包含 LLM）
    pub fn create_basic_strategies() -> Vec<Box<dyn EditStrategy>> {
        vec![
            Box::new(ExactMatchStrategy),
            Box::new(FlexibleIndentStrategy),
            Box::new(RegexFuzzyStrategy),
        ]
    }
}
