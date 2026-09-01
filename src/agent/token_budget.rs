use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Token预算配置
#[derive(Debug, Clone)]
pub struct TokenBudgetConfig {
    /// 是否启用Token预算
    pub enabled: bool,
    /// 每轮次的Token预算
    pub turn_budget: usize,
    /// 最大自动继续次数
    pub max_continuations: usize,
    /// 触发继续的阈值（百分比）
    pub continue_threshold: f64,
    /// 递减收益检测阈值
    pub diminishing_returns_threshold: f64,
}

impl Default for TokenBudgetConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            turn_budget: 500_000, // 500K tokens
            max_continuations: 5,
            continue_threshold: 0.9,            // 90%
            diminishing_returns_threshold: 0.1, // 10%
        }
    }
}

impl TokenBudgetConfig {
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_TOKEN_BUDGET_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let turn_budget = std::env::var("STAR_TOKEN_BUDGET_TURN")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(500_000);

        let max_continuations = std::env::var("STAR_TOKEN_BUDGET_MAX_CONTINUATIONS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5);

        let continue_threshold = std::env::var("STAR_TOKEN_BUDGET_CONTINUE_THRESHOLD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.9);

        let diminishing_returns_threshold =
            std::env::var("STAR_TOKEN_BUDGET_DIMINISHING_THRESHOLD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.1);

        Self {
            enabled,
            turn_budget,
            max_continuations,
            continue_threshold,
            diminishing_returns_threshold,
        }
    }
}

/// Token使用统计
#[derive(Debug, Clone)]
pub struct TokenUsage {
    /// 输入tokens
    pub input_tokens: usize,
    /// 输出tokens
    pub output_tokens: usize,
    /// 缓存读取tokens
    pub cache_read_tokens: usize,
    /// 缓存写入tokens
    pub cache_write_tokens: usize,
}

impl TokenUsage {
    pub fn total(&self) -> usize {
        self.input_tokens + self.output_tokens + self.cache_read_tokens + self.cache_write_tokens
    }
}

/// Token预算跟踪器
pub struct TokenBudgetTracker {
    config: TokenBudgetConfig,
    /// 当前轮次的输出tokens
    turn_output_tokens: Arc<AtomicUsize>,
    /// 继续次数
    continuation_count: Arc<AtomicUsize>,
    /// 历史输出tokens（用于递减收益检测）
    history_output_tokens: Vec<usize>,
}

impl TokenBudgetTracker {
    pub fn new() -> Self {
        let config = TokenBudgetConfig::from_env();
        Self {
            config,
            turn_output_tokens: Arc::new(AtomicUsize::new(0)),
            continuation_count: Arc::new(AtomicUsize::new(0)),
            history_output_tokens: Vec::new(),
        }
    }

    /// 记录输出tokens
    pub fn record_output_tokens(&self, tokens: usize) {
        self.turn_output_tokens.fetch_add(tokens, Ordering::Relaxed);
    }

    /// 获取当前轮次的输出tokens
    pub fn get_turn_output_tokens(&self) -> usize {
        self.turn_output_tokens.load(Ordering::Relaxed)
    }

    /// 获取继续次数
    pub fn get_continuation_count(&self) -> usize {
        self.continuation_count.load(Ordering::Relaxed)
    }

    /// 重置轮次统计
    pub fn reset_turn(&mut self) {
        let current_output = self.turn_output_tokens.load(Ordering::Relaxed);
        self.history_output_tokens.push(current_output);
        self.turn_output_tokens.store(0, Ordering::Relaxed);
        self.continuation_count.store(0, Ordering::Relaxed);
    }

    /// 检查是否应该继续
    pub fn should_continue(&self) -> TokenBudgetDecision {
        if !self.config.enabled {
            return TokenBudgetDecision::Stop {
                reason: "Token budget disabled".to_string(),
            };
        }

        let current_output = self.get_turn_output_tokens();
        let continuation_count = self.get_continuation_count();

        // 检查是否超过最大继续次数
        if continuation_count >= self.config.max_continuations {
            return TokenBudgetDecision::Stop {
                reason: format!(
                    "Max continuations reached ({}/{})",
                    continuation_count, self.config.max_continuations
                ),
            };
        }

        // 检查是否超过预算阈值
        let budget_usage = current_output as f64 / self.config.turn_budget as f64;
        if budget_usage < self.config.continue_threshold {
            return TokenBudgetDecision::Stop {
                reason: format!(
                    "Budget usage {:.1}% below threshold {:.1}%",
                    budget_usage * 100.0,
                    self.config.continue_threshold * 100.0
                ),
            };
        }

        // 检查递减收益
        if self.has_diminishing_returns() {
            return TokenBudgetDecision::Stop {
                reason: "Diminishing returns detected".to_string(),
            };
        }

        // 应该继续
        let pct = (budget_usage * 100.0) as u32;
        TokenBudgetDecision::Continue {
            nudge_message: format!(
                "Token budget {}% used ({} / {} tokens). Continue working on the task.",
                pct, current_output, self.config.turn_budget
            ),
            continuation_count: continuation_count + 1,
            pct,
            turn_tokens: current_output,
            budget: self.config.turn_budget,
        }
    }

    /// 检测递减收益
    fn has_diminishing_returns(&self) -> bool {
        if self.history_output_tokens.len() < 2 {
            return false;
        }

        let recent = &self.history_output_tokens;
        let len = recent.len();

        // 比较最近两次的输出tokens
        let last = recent[len - 1] as f64;
        let prev = recent[len - 2] as f64;

        if prev == 0.0 {
            return false;
        }

        let change_ratio = (last - prev).abs() / prev;
        change_ratio < self.config.diminishing_returns_threshold
    }

    /// 增加继续次数
    pub fn increment_continuation(&self) {
        self.continuation_count.fetch_add(1, Ordering::Relaxed);
    }
}

/// Token预算决策
#[derive(Debug, Clone)]
pub enum TokenBudgetDecision {
    /// 继续执行
    Continue {
        nudge_message: String,
        continuation_count: usize,
        pct: u32,
        turn_tokens: usize,
        budget: usize,
    },
    /// 停止执行
    Stop { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_budget_decision() {
        let tracker = TokenBudgetTracker {
            config: TokenBudgetConfig {
                enabled: true,
                turn_budget: 100_000,
                max_continuations: 3,
                continue_threshold: 0.9,
                diminishing_returns_threshold: 0.1,
            },
            turn_output_tokens: Arc::new(AtomicUsize::new(95_000)),
            continuation_count: Arc::new(AtomicUsize::new(0)),
            history_output_tokens: Vec::new(),
        };

        let decision = tracker.should_continue();
        match decision {
            TokenBudgetDecision::Continue { pct, .. } => {
                assert_eq!(pct, 95);
            }
            _ => panic!("Expected Continue decision"),
        }
    }

    #[test]
    fn test_max_continuations() {
        let tracker = TokenBudgetTracker {
            config: TokenBudgetConfig {
                enabled: true,
                turn_budget: 100_000,
                max_continuations: 3,
                continue_threshold: 0.9,
                diminishing_returns_threshold: 0.1,
            },
            turn_output_tokens: Arc::new(AtomicUsize::new(95_000)),
            continuation_count: Arc::new(AtomicUsize::new(3)),
            history_output_tokens: Vec::new(),
        };

        let decision = tracker.should_continue();
        match decision {
            TokenBudgetDecision::Stop { reason } => {
                assert!(reason.contains("Max continuations"));
            }
            _ => panic!("Expected Stop decision"),
        }
    }
}
