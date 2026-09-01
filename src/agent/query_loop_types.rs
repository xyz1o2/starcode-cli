/// 对话继续条件 - 对标claude-code的8种继续条件
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContinueReason {
    /// 正常下一轮
    NextTurn,
    /// Reactive Compact重试
    ReactiveCompactRetry,
    /// Context Collapse排水重试
    CollapseDrainRetry { committed: usize },
    /// Max-output-tokens恢复
    MaxOutputTokensRecovery { attempt: usize },
    /// Max-output-tokens升级（8k → 64k）
    MaxOutputTokensEscalate,
    /// Stop Hook阻止继续
    StopHookBlocking,
    /// Token预算自动继续
    TokenBudgetContinuation,
    /// Media Recovery重试（图片/PDF错误）
    MediaRecoveryRetry,
}

impl std::fmt::Display for ContinueReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContinueReason::NextTurn => write!(f, "next_turn"),
            ContinueReason::ReactiveCompactRetry => write!(f, "reactive_compact_retry"),
            ContinueReason::CollapseDrainRetry { committed } => {
                write!(f, "collapse_drain_retry (committed={})", committed)
            }
            ContinueReason::MaxOutputTokensRecovery { attempt } => {
                write!(f, "max_output_tokens_recovery (attempt={})", attempt)
            }
            ContinueReason::MaxOutputTokensEscalate => write!(f, "max_output_tokens_escalate"),
            ContinueReason::StopHookBlocking => write!(f, "stop_hook_blocking"),
            ContinueReason::TokenBudgetContinuation => write!(f, "token_budget_continuation"),
            ContinueReason::MediaRecoveryRetry => write!(f, "media_recovery_retry"),
        }
    }
}

/// 对话终止原因 - 对标claude-code的11种终止条件
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalReason {
    /// 任务完成
    Completed,
    /// 达到最大轮次
    MaxTurns,
    /// 用户中断
    AbortedStreaming,
    /// 工具执行期间中断
    AbortedTools,
    /// Prompt-too-long
    PromptTooLong,
    /// 模型错误
    ModelError,
    /// 阻塞限制
    BlockingLimit,
    /// Stop Hook阻止
    StopHookPrevented,
    /// 图片错误
    ImageError,
    /// 循环状态停止
    LoopStateStopped,
    /// 最大轮次达到
    MaxTurnsReached,
}

impl std::fmt::Display for TerminalReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TerminalReason::Completed => write!(f, "completed"),
            TerminalReason::MaxTurns => write!(f, "max_turns"),
            TerminalReason::AbortedStreaming => write!(f, "aborted_streaming"),
            TerminalReason::AbortedTools => write!(f, "aborted_tools"),
            TerminalReason::PromptTooLong => write!(f, "prompt_too_long"),
            TerminalReason::ModelError => write!(f, "model_error"),
            TerminalReason::BlockingLimit => write!(f, "blocking_limit"),
            TerminalReason::StopHookPrevented => write!(f, "stop_hook_prevented"),
            TerminalReason::ImageError => write!(f, "image_error"),
            TerminalReason::LoopStateStopped => write!(f, "loop_state_stopped"),
            TerminalReason::MaxTurnsReached => write!(f, "max_turns_reached"),
        }
    }
}

/// 查询循环状态 - 对标claude-code的State类型
#[derive(Debug, Clone)]
pub struct QueryLoopState {
    /// 当前消息
    pub messages: Vec<crate::types::StarMessage>,
    /// 自动压缩跟踪
    pub auto_compact_tracking: Option<AutoCompactTracking>,
    /// Max-output-tokens恢复次数
    pub max_output_tokens_recovery_count: usize,
    /// 是否已尝试Reactive Compact
    pub has_attempted_reactive_compact: bool,
    /// Max-output-tokens覆盖值
    pub max_output_tokens_override: Option<usize>,
    /// Stop Hook是否激活
    pub stop_hook_active: Option<bool>,
    /// 轮次计数
    pub turn_count: usize,
    /// 上一次继续的原因
    pub transition: Option<ContinueReason>,
}

impl QueryLoopState {
    pub fn new(messages: Vec<crate::types::StarMessage>) -> Self {
        Self {
            messages,
            auto_compact_tracking: None,
            max_output_tokens_recovery_count: 0,
            has_attempted_reactive_compact: false,
            max_output_tokens_override: None,
            stop_hook_active: None,
            turn_count: 1,
            transition: None,
        }
    }
}

/// 自动压缩跟踪状态
#[derive(Debug, Clone)]
pub struct AutoCompactTracking {
    /// 是否已压缩
    pub compacted: bool,
    /// 轮次ID
    pub turn_id: String,
    /// 轮次计数器
    pub turn_counter: usize,
    /// 连续失败次数
    pub consecutive_failures: usize,
}

/// Stop Hook结果
#[derive(Debug, Clone)]
pub struct StopHookResult {
    /// 是否阻止继续
    pub prevent_continuation: bool,
    /// 阻止错误消息
    pub blocking_errors: Vec<crate::types::StarMessage>,
}

impl Default for StopHookResult {
    fn default() -> Self {
        Self {
            prevent_continuation: false,
            blocking_errors: Vec::new(),
        }
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

/// 媒体恢复配置
#[derive(Debug, Clone)]
pub struct MediaRecoveryConfig {
    /// 是否启用
    pub enabled: bool,
    /// 最大重试次数
    pub max_retries: usize,
    /// 支持的媒体类型
    pub supported_types: Vec<String>,
}

impl Default for MediaRecoveryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_retries: 1,
            supported_types: vec!["image".to_string(), "pdf".to_string()],
        }
    }
}

impl MediaRecoveryConfig {
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_MEDIA_RECOVERY_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let max_retries = std::env::var("STAR_MEDIA_RECOVERY_MAX_RETRIES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);

        Self {
            enabled,
            max_retries,
            ..Default::default()
        }
    }

    /// 检查是否是媒体错误
    pub fn is_media_error(&self, error: &str) -> bool {
        let error_lower = error.to_lowercase();
        error_lower.contains("image_too_large")
            || error_lower.contains("image_size")
            || error_lower.contains("pdf_too_large")
            || error_lower.contains("pdf_size")
            || error_lower.contains("media_size")
            || error_lower.contains("file_too_large")
    }
}
