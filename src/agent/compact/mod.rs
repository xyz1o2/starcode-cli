use crate::types::StarMessage;
use async_trait::async_trait;

pub mod auto_compact;
pub mod cached_micro_compact;
pub mod compact_warning_hook;
pub mod context_collapse;
pub mod grouping;
pub mod micro_compact;
pub mod post_compact_cleanup;
pub mod reactive_compact;
pub mod session_memory_compact;
pub mod snip_compact;
pub mod time_based_config;
pub mod token_counter;
pub mod tool_output_compact;

/// 压缩策略配置
#[derive(Debug, Clone)]
pub struct CompactConfig {
    /// 触发压缩的最大 token 数
    pub max_tokens: usize,
    /// 压缩目标 token 数
    pub target_tokens: usize,
    /// 是否启用自动压缩
    pub auto_compact_enabled: bool,
    /// 微压缩触发阈值（行数）
    pub micro_compact_threshold: usize,
    /// 是否启用语义相关性评分
    pub semantic_relevance_enabled: bool,
    /// 最近消息的权重（0.0-1.0）
    pub recency_weight: f64,
    /// 相关性权重（0.0-1.0）
    pub relevance_weight: f64,
    /// 工具结果的权重（0.0-1.0）
    pub tool_result_weight: f64,
}

impl Default for CompactConfig {
    fn default() -> Self {
        Self {
            max_tokens: 150_000,
            target_tokens: 100_000,
            auto_compact_enabled: true,
            micro_compact_threshold: 100,
            semantic_relevance_enabled: true,
            recency_weight: 0.4,
            relevance_weight: 0.4,
            tool_result_weight: 0.2,
        }
    }
}

impl CompactConfig {
    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        let max_tokens = std::env::var("STAR_COMPACT_MAX_TOKENS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(150_000);

        let target_tokens = std::env::var("STAR_COMPACT_TARGET_TOKENS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100_000);

        let auto_compact_enabled = std::env::var("STAR_AUTO_COMPACT_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let micro_compact_threshold = std::env::var("STAR_MICRO_COMPACT_THRESHOLD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100);

        let semantic_relevance_enabled = std::env::var("STAR_SEMANTIC_RELEVANCE_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let recency_weight = std::env::var("STAR_COMPACT_RECENCY_WEIGHT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.4);

        let relevance_weight = std::env::var("STAR_COMPACT_RELEVANCE_WEIGHT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.4);

        let tool_result_weight = std::env::var("STAR_COMPACT_TOOL_RESULT_WEIGHT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.2);

        Self {
            max_tokens,
            target_tokens,
            auto_compact_enabled,
            micro_compact_threshold,
            semantic_relevance_enabled,
            recency_weight,
            relevance_weight,
            tool_result_weight,
        }
    }
}

/// 压缩策略 trait
#[async_trait]
pub trait CompactStrategy: Send + Sync {
    /// 策略名称
    fn name(&self) -> &str;

    /// 检查是否可以应用此策略
    fn can_apply(&self, messages: &[StarMessage], token_count: usize) -> bool;

    /// 应用压缩策略
    fn apply(&self, messages: &[StarMessage]) -> Vec<StarMessage>;

    /// 策略优先级（数值越小优先级越高）
    fn priority(&self) -> u32;
}

/// 压缩结果
#[derive(Debug, Clone)]
pub struct CompactResult {
    /// 压缩后的消息
    pub messages: Vec<StarMessage>,
    /// 是否执行了压缩
    pub was_compacted: bool,
    /// 原始 token 数量
    pub original_token_count: usize,
    /// 压缩后 token 数量
    pub new_token_count: usize,
    /// 使用的策略名称
    pub strategy_name: String,
    /// 压缩详情
    pub details: String,
}

/// 消息相关性评分
#[derive(Debug, Clone)]
pub struct MessageRelevanceScore {
    /// 消息索引
    pub index: usize,
    /// 时间相关性分数（0.0-1.0）
    pub recency_score: f64,
    /// 语义相关性分数（0.0-1.0）
    pub relevance_score: f64,
    /// 工具结果重要性分数（0.0-1.0）
    pub tool_result_score: f64,
    /// 综合分数（0.0-1.0）
    pub combined_score: f64,
}

/// 语义相关性分析器
pub struct RelevanceAnalyzer {
    /// 当前用户查询
    current_query: String,
    /// 当前上下文关键词
    context_keywords: Vec<String>,
    /// 权重配置
    weights: RelevanceWeights,
}

/// 相关性权重配置
#[derive(Debug, Clone)]
pub struct RelevanceWeights {
    pub recency: f64,
    pub relevance: f64,
    pub tool_result: f64,
}

impl RelevanceWeights {
    pub fn from_config(config: &CompactConfig) -> Self {
        Self {
            recency: config.recency_weight,
            relevance: config.relevance_weight,
            tool_result: config.tool_result_weight,
        }
    }
}

impl RelevanceAnalyzer {
    /// 创建新的相关性分析器
    pub fn new(current_query: &str, config: &CompactConfig) -> Self {
        let context_keywords = extract_keywords(current_query);
        Self {
            current_query: current_query.to_string(),
            context_keywords,
            weights: RelevanceWeights::from_config(config),
        }
    }

    /// 计算消息列表中每条消息的相关性分数
    pub fn score_messages(&self, messages: &[StarMessage]) -> Vec<MessageRelevanceScore> {
        let total_messages = messages.len();
        if total_messages == 0 {
            return Vec::new();
        }

        let mut scores = Vec::with_capacity(total_messages);

        for (index, message) in messages.iter().enumerate() {
            let recency_score = self.calculate_recency_score(index, total_messages);
            let relevance_score = self.calculate_relevance_score(message);
            let tool_result_score = self.calculate_tool_result_score(message);

            let combined_score = (recency_score * self.weights.recency)
                + (relevance_score * self.weights.relevance)
                + (tool_result_score * self.weights.tool_result);

            scores.push(MessageRelevanceScore {
                index,
                recency_score,
                relevance_score,
                tool_result_score,
                combined_score,
            });
        }

        scores
    }

    /// 计算时间相关性分数（越新越高）
    fn calculate_recency_score(&self, index: usize, total: usize) -> f64 {
        if total <= 1 {
            return 1.0;
        }
        // 线性衰减：最新的消息得分最高
        index as f64 / (total - 1) as f64
    }

    /// 计算语义相关性分数
    fn calculate_relevance_score(&self, message: &StarMessage) -> f64 {
        let content = match &message.content {
            Some(c) => c.to_lowercase(),
            None => return 0.0,
        };

        if self.context_keywords.is_empty() {
            return 0.5; // 默认中等相关性
        }

        // 计算关键词匹配比例
        let matches = self
            .context_keywords
            .iter()
            .filter(|keyword| content.contains(keyword.as_str()))
            .count();

        if self.context_keywords.is_empty() {
            return 0.5;
        }

        let match_ratio = matches as f64 / self.context_keywords.len() as f64;

        // 检查是否包含当前查询的直接引用
        let query_lower = self.current_query.to_lowercase();
        let has_direct_reference = content.contains(&query_lower);

        let base_score = match_ratio * 0.7;
        if has_direct_reference {
            (base_score + 0.3).min(1.0)
        } else {
            base_score
        }
    }

    /// 计算工具结果重要性分数
    fn calculate_tool_result_score(&self, message: &StarMessage) -> f64 {
        // 系统消息通常重要
        if message.role == "system" {
            return 0.9;
        }

        // 工具消息
        if message.role == "tool" {
            let content = match &message.content {
                Some(c) => c.to_lowercase(),
                None => return 0.3,
            };

            // 错误信息通常重要
            if content.contains("error")
                || content.contains("failed")
                || content.contains("failure")
            {
                return 0.8;
            }

            // 文件内容通常重要
            if content.contains("file") || content.contains("path") {
                return 0.6;
            }

            // 搜索结果可能重要
            if content.contains("found") || content.contains("match") || content.contains("result")
            {
                return 0.5;
            }

            return 0.4;
        }

        // 用户消息
        if message.role == "user" {
            return 0.7;
        }

        // 助手消息
        if message.role == "assistant" {
            return 0.5;
        }

        0.3
    }

    /// 根据相关性分数过滤和排序消息
    pub fn filter_messages_by_relevance(
        &self,
        messages: &[StarMessage],
        min_score: f64,
        max_messages: usize,
    ) -> Vec<StarMessage> {
        let scores = self.score_messages(messages);

        // 按综合分数排序
        let mut indexed_scores: Vec<(usize, f64)> =
            scores.iter().map(|s| (s.index, s.combined_score)).collect();

        indexed_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // 过滤低分消息并限制数量
        let mut result = Vec::new();
        for (index, score) in indexed_scores {
            if score >= min_score && result.len() < max_messages {
                result.push(messages[index].clone());
            }
        }

        // 按原始顺序排序
        result.sort_by(|a, b| {
            let a_idx = messages
                .iter()
                .position(|m| std::ptr::eq(m, a))
                .unwrap_or(0);
            let b_idx = messages
                .iter()
                .position(|m| std::ptr::eq(m, b))
                .unwrap_or(0);
            a_idx.cmp(&b_idx)
        });

        result
    }
}

/// 从文本中提取关键词
fn extract_keywords(text: &str) -> Vec<String> {
    let stop_words: std::collections::HashSet<&str> = [
        "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
        "do", "does", "did", "will", "would", "could", "should", "may", "might", "can", "shall",
        "to", "of", "in", "for", "on", "with", "at", "by", "from", "as", "into", "through",
        "during", "before", "after", "above", "below", "between", "out", "off", "over", "under",
        "again", "further", "then", "once", "here", "there", "when", "where", "why", "how", "all",
        "both", "each", "few", "more", "most", "other", "some", "such", "no", "nor", "not", "only",
        "own", "same", "so", "than", "too", "very", "just", "because", "but", "and", "or", "if",
        "while", "this", "that", "these", "those", "it", "its",
    ]
    .iter()
    .cloned()
    .collect();

    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|word| word.len() >= 3 && !stop_words.contains(word))
        .map(|word| word.to_string())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect()
}

/// 预测性压缩配置
#[derive(Debug, Clone)]
pub struct PredictiveCompactConfig {
    /// 是否启用预测性压缩
    pub enabled: bool,
    /// 预估每轮次工具结果增长的token数（保守估计）
    pub estimated_tool_result_growth: usize,
    /// 预估每轮次模型输出的token数
    pub estimated_model_output: usize,
    /// 预测性压缩的安全边际（0.0-1.0）
    pub safety_margin: f64,
}

impl Default for PredictiveCompactConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            estimated_tool_result_growth: 15_000, // 保守估计15K tokens
            estimated_model_output: 8_000,        // 估计8K tokens
            safety_margin: 0.1,                   // 10%安全边际
        }
    }
}

impl PredictiveCompactConfig {
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_PREDICTIVE_COMPACT_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let estimated_tool_result_growth = std::env::var("STAR_PREDICTED_TOOL_GROWTH")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(15_000);

        let estimated_model_output = std::env::var("STAR_PREDICTED_MODEL_OUTPUT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8_000);

        let safety_margin = std::env::var("STAR_PREDICTIVE_SAFETY_MARGIN")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.1);

        Self {
            enabled,
            estimated_tool_result_growth,
            estimated_model_output,
            safety_margin,
        }
    }

    /// 估算当前轮次的总增长
    pub fn estimate_turn_growth(&self) -> usize {
        let base_growth = self.estimated_tool_result_growth + self.estimated_model_output;
        let with_margin = (base_growth as f64 * (1.0 + self.safety_margin)) as usize;
        with_margin
    }
}

/// 压缩管理器
///
/// 管理多个压缩策略，按优先级顺序尝试应用
pub struct CompactManager {
    strategies: Vec<Box<dyn CompactStrategy>>,
    config: CompactConfig,
    predictive_config: PredictiveCompactConfig,
}

impl CompactManager {
    /// 创建新的压缩管理器
    pub fn new(config: CompactConfig) -> Self {
        let mut strategies: Vec<Box<dyn CompactStrategy>> = Vec::new();

        // 添加工具输出压缩策略（优先级最高）
        strategies.push(Box::new(
            tool_output_compact::ToolOutputCompactStrategy::new(),
        ));

        // 添加微压缩策略
        strategies.push(Box::new(
            micro_compact::MicroCompactStrategy::new()
                .with_tool_output_threshold(config.micro_compact_threshold),
        ));

        // 添加自动压缩策略
        strategies.push(Box::new(auto_compact::AutoCompactStrategy::new(
            config.clone(),
        )));

        // 添加激进压缩策略（优先级最低）
        strategies.push(Box::new(snip_compact::SnipCompactStrategy::new()));

        // 按优先级排序
        strategies.sort_by_key(|s| s.priority());

        let predictive_config = PredictiveCompactConfig::from_env();

        Self {
            strategies,
            config,
            predictive_config,
        }
    }

    /// 从环境变量创建压缩管理器
    pub fn from_env() -> Self {
        Self::new(CompactConfig::from_env())
    }

    /// 预测性压缩检查
    ///
    /// 估算当前轮次增长是否会超过上下文窗口，如果是则提前压缩
    pub fn predictive_compact(&self, messages: &[StarMessage]) -> Option<CompactResult> {
        if !self.predictive_config.enabled {
            return None;
        }

        let current_tokens = token_counter::count_tokens(messages);
        let estimated_growth = self.predictive_config.estimate_turn_growth();
        let projected_tokens = current_tokens + estimated_growth;

        // 如果预测的token数超过阈值，触发压缩
        if projected_tokens > self.config.max_tokens {
            crate::utils::logging::append_debug_log_line(
                &format!(
                    "[COMPACT] Predictive compaction triggered: current={}, estimated_growth={}, projected={}, threshold={}",
                    current_tokens, estimated_growth, projected_tokens, self.config.max_tokens
                )
            );

            // 尝试压缩到目标token数以下
            let target_tokens = self.config.target_tokens;
            let compact_result = self.compact(messages);

            if compact_result.was_compacted {
                return Some(compact_result);
            }
        }

        None
    }

    /// 获取预测性压缩配置
    pub fn predictive_config(&self) -> &PredictiveCompactConfig {
        &self.predictive_config
    }

    /// 获取配置
    pub fn config(&self) -> &CompactConfig {
        &self.config
    }

    /// 获取所有策略名称
    pub fn strategy_names(&self) -> Vec<&str> {
        self.strategies.iter().map(|s| s.name()).collect()
    }

    /// 检查是否需要压缩
    pub fn needs_compaction(&self, token_count: usize) -> bool {
        self.config.auto_compact_enabled && token_count > self.config.max_tokens
    }

    /// 执行压缩
    ///
    /// Preserves the system message prefix to maintain cache hit rates.
    /// Only compresses non-system messages (user/assistant/tool).
    pub fn compact(&self, messages: &[StarMessage]) -> CompactResult {
        let original_token_count = token_counter::count_tokens(messages);

        // Split messages into system prefix and non-system suffix.
        // The system prefix is preserved unchanged for cache stability.
        // Note: cached_system messages have role="system" with cache_control set.
        let mut system_prefix: Vec<StarMessage> = Vec::new();
        let mut non_system_start = 0usize;
        for (i, msg) in messages.iter().enumerate() {
            if msg.role == "system" {
                system_prefix.push(msg.clone());
                non_system_start = i + 1;
            } else {
                break;
            }
        }

        let non_system_msgs = &messages[non_system_start..];

        // If there are no non-system messages, nothing to compress
        if non_system_msgs.is_empty() {
            return CompactResult {
                messages: messages.to_vec(),
                was_compacted: false,
                original_token_count,
                new_token_count: original_token_count,
                strategy_name: "none".to_string(),
                details: "No non-system messages to compress".to_string(),
            };
        }

        // 按优先级顺序尝试策略（只压缩非系统消息）
        for strategy in &self.strategies {
            if strategy.can_apply(non_system_msgs, original_token_count) {
                let compressed_non_system = strategy.apply(non_system_msgs);
                let new_token_count = token_counter::count_tokens(&compressed_non_system)
                    + token_counter::count_tokens(&system_prefix);

                // 检查压缩是否有效
                if new_token_count < original_token_count {
                    // Reassemble: system prefix + compressed non-system messages
                    let mut new_messages = system_prefix.clone();
                    new_messages.extend(compressed_non_system);

                    return CompactResult {
                        messages: new_messages,
                        was_compacted: true,
                        original_token_count,
                        new_token_count,
                        strategy_name: strategy.name().to_string(),
                        details: format!(
                            "Applied {} compression: {} -> {} tokens (preserved {} system messages)",
                            strategy.name(),
                            original_token_count,
                            new_token_count,
                            system_prefix.len()
                        ),
                    };
                }
            }
        }

        // 没有策略被应用
        CompactResult {
            messages: messages.to_vec(),
            was_compacted: false,
            original_token_count,
            new_token_count: original_token_count,
            strategy_name: "none".to_string(),
            details: "No compression needed".to_string(),
        }
    }

    /// 强制执行压缩（使用指定策略）
    pub fn compact_with_strategy(
        &self,
        messages: &[StarMessage],
        strategy_name: &str,
    ) -> Option<CompactResult> {
        let original_token_count = token_counter::count_tokens(messages);

        for strategy in &self.strategies {
            if strategy.name() == strategy_name {
                let new_messages = strategy.apply(messages);
                let new_token_count = token_counter::count_tokens(&new_messages);

                return Some(CompactResult {
                    messages: new_messages,
                    was_compacted: true,
                    original_token_count,
                    new_token_count,
                    strategy_name: strategy.name().to_string(),
                    details: format!(
                        "Applied {} compression: {} -> {} tokens",
                        strategy.name(),
                        original_token_count,
                        new_token_count
                    ),
                });
            }
        }

        None
    }

    /// 添加自定义策略
    pub fn add_strategy(&mut self, strategy: Box<dyn CompactStrategy>) {
        self.strategies.push(strategy);
        // 重新排序
        self.strategies.sort_by_key(|s| s.priority());
    }
}
