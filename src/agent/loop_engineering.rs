use std::collections::{HashMap, VecDeque};
use std::time::Duration;
use serde::{Serialize, Deserialize};

/// Structured error feedback - includes error + relevant code context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredError {
    /// Error message
    pub error: String,
    /// Error category
    pub category: ErrorCategory,
    /// Relevant code context (file, line, surrounding code)
    pub code_context: Option<CodeContext>,
    /// Suggested fix approach
    pub suggestion: Option<String>,
    /// Whether this error is recoverable
    pub recoverable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ErrorCategory {
    Compilation,
    Runtime,
    TestFailure,
    ToolExecution,
    Permission,
    NotFound,
    Timeout,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeContext {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub surrounding_code: String,
    pub error_line: String,
}

impl StructuredError {
    /// Parse error from raw output and enrich with context
    pub fn from_tool_output(tool_name: &str, output: &str, error: &str) -> Self {
        let category = Self::categorize_error(tool_name, error);
        let recoverable = Self::is_recoverable(&category);
        let suggestion = Self::generate_suggestion(&category, error);
        let code_context = Self::extract_code_context(output, error);

        StructuredError {
            error: error.to_string(),
            category,
            code_context,
            suggestion,
            recoverable,
        }
    }

    fn categorize_error(tool_name: &str, error: &str) -> ErrorCategory {
        let error_lower = error.to_lowercase();
        
        if error_lower.contains("compilation") || error_lower.contains("compile") 
            || error_lower.contains("syntax error") || error_lower.contains("cannot find") {
            ErrorCategory::Compilation
        } else if error_lower.contains("test") && (error_lower.contains("fail") || error_lower.contains("assert")) {
            ErrorCategory::TestFailure
        } else if error_lower.contains("permission") || error_lower.contains("access denied") {
            ErrorCategory::Permission
        } else if error_lower.contains("not found") || error_lower.contains("no such file") {
            ErrorCategory::NotFound
        } else if error_lower.contains("timeout") || error_lower.contains("timed out") {
            ErrorCategory::Timeout
        } else if tool_name == "Bash" || tool_name == "powershell" {
            ErrorCategory::Runtime
        } else {
            ErrorCategory::ToolExecution
        }
    }

    fn is_recoverable(category: &ErrorCategory) -> bool {
        match category {
            ErrorCategory::Compilation => true,
            ErrorCategory::Runtime => true,
            ErrorCategory::TestFailure => true,
            ErrorCategory::NotFound => true,
            ErrorCategory::Permission => false,
            ErrorCategory::Timeout => true,
            ErrorCategory::ToolExecution => true,
            ErrorCategory::Unknown => true,
        }
    }

    fn generate_suggestion(category: &ErrorCategory, _error: &str) -> Option<String> {
        match category {
            ErrorCategory::Compilation => {
                Some("Read the error message carefully. Check syntax, imports, and type annotations. Use `get_diagnostics` to see all issues.".to_string())
            }
            ErrorCategory::TestFailure => {
                Some("Read the test output to understand what failed. Check if your changes broke existing behavior. Run the specific failing test to isolate the issue.".to_string())
            }
            ErrorCategory::NotFound => {
                Some("Verify the file path exists. Use `glob` or `search` to find the correct path.".to_string())
            }
            ErrorCategory::Timeout => {
                Some("The operation timed out. Try breaking it into smaller steps or increasing the timeout.".to_string())
            }
            ErrorCategory::Permission => {
                Some("Permission denied. This operation requires elevated privileges.".to_string())
            }
            _ => None,
        }
    }

    fn extract_code_context(output: &str, error: &str) -> Option<CodeContext> {
        let patterns = [
            r#"at (\S+):(\d+)"#,
            r#"(\S+):(\d+):\d+"#,
            r#"in (\S+) line (\d+)"#,
            r#"(\S+)\((\d+),"#,
        ];

        for pattern in &patterns {
            if let Ok(re) = regex::Regex::new(pattern) {
                if let Some(cap) = re.captures(error) {
                    let file = cap.get(1)?.as_str().to_string();
                    let line = cap.get(2)?.as_str().parse().unwrap_or(0);
                    
                    return Some(CodeContext {
                        file,
                        line,
                        column: 0,
                        surrounding_code: String::new(),
                        error_line: error.lines().next().unwrap_or("").to_string(),
                    });
                }
            }
        }

        // Try to extract from output as well
        for pattern in &patterns {
            if let Ok(re) = regex::Regex::new(pattern) {
                if let Some(cap) = re.captures(output) {
                    let file = cap.get(1)?.as_str().to_string();
                    let line = cap.get(2)?.as_str().parse().unwrap_or(0);
                    
                    return Some(CodeContext {
                        file,
                        line,
                        column: 0,
                        surrounding_code: String::new(),
                        error_line: output.lines().next().unwrap_or("").to_string(),
                    });
                }
            }
        }

        None
    }

    /// Format for display to user
    pub fn format_display(&self) -> String {
        let mut parts = Vec::new();
        
        parts.push(format!("[{:?}] {}", self.category, self.error));
        
        if let Some(ctx) = &self.code_context {
            parts.push(format!("  at {}:{}", ctx.file, ctx.line));
            if !ctx.surrounding_code.is_empty() {
                parts.push(format!("  {}", ctx.surrounding_code));
            }
        }
        
        if let Some(suggestion) = &self.suggestion {
            parts.push(format!("  → {}", suggestion));
        }
        
        parts.join("\n")
    }

    /// Convert to JSON value for embedding in tool results
    pub fn to_json_value(&self) -> serde_json::Value {
        serde_json::json!({
            "structured_error": {
                "error": self.error,
                "category": format!("{:?}", self.category),
                "code_context": self.code_context,
                "suggestion": self.suggestion,
                "recoverable": self.recoverable,
            }
        })
    }
}

/// Attempt history - tracks what was tried and what happened
#[derive(Debug, Clone)]
pub struct AttemptHistory {
    pub attempts: VecDeque<Attempt>,
    pub max_history: usize,
}

#[derive(Debug, Clone)]
pub struct Attempt {
    pub tool: String,
    pub action: String,
    pub result: AttemptResult,
    pub timestamp: i64,
}

#[derive(Debug, Clone)]
pub enum AttemptResult {
    Success,
    Failure(StructuredError),
    Partial(String),
}

impl AttemptHistory {
    pub fn new(max_history: usize) -> Self {
        Self {
            attempts: VecDeque::new(),
            max_history,
        }
    }

    pub fn record(&mut self, tool: &str, action: &str, result: AttemptResult) {
        self.attempts.push_back(Attempt {
            tool: tool.to_string(),
            action: action.to_string(),
            result,
            timestamp: chrono::Utc::now().timestamp(),
        });
        
        while self.attempts.len() > self.max_history {
            self.attempts.pop_front();
        }
    }

    /// Generate a compact summary of recent attempts
    pub fn summarize(&self) -> String {
        if self.attempts.is_empty() {
            return "No previous attempts.".to_string();
        }

        let mut summary = String::from("Recent attempts:\n");
        for (i, attempt) in self.attempts.iter().rev().take(5).enumerate() {
            let status = match &attempt.result {
                AttemptResult::Success => "✓",
                AttemptResult::Failure(_) => "✗",
                AttemptResult::Partial(_) => "~",
            };
            summary.push_str(&format!(
                "  {}. {} {} {} - {}\n",
                i + 1,
                status,
                attempt.tool,
                attempt.action,
                match &attempt.result {
                    AttemptResult::Success => "succeeded".to_string(),
                    AttemptResult::Failure(e) => format!("failed: {}", e.error),
                    AttemptResult::Partial(msg) => format!("partial: {}", msg),
                }
            ));
        }

        // Detect repeated failures
        let recent_failures: Vec<&Attempt> = self.attempts.iter()
            .rev()
            .take(3)
            .filter(|a| matches!(a.result, AttemptResult::Failure(_)))
            .collect();

        if recent_failures.len() >= 3 {
            summary.push_str("\n⚠️ Multiple consecutive failures detected. Try a different approach.\n");
        }

        summary
    }

    /// Check if a specific error pattern is repeating
    pub fn is_repeating_failure(&self) -> bool {
        if self.attempts.len() < 5 {  // Increased from 3 to 5
            return false;
        }

        let recent: Vec<&str> = self.attempts.iter()
            .rev()
            .take(5)  // Check last 5 instead of 3
            .filter_map(|a| match &a.result {
                AttemptResult::Failure(e) => Some(e.error.as_str()),
                _ => None,
            })
            .collect();

        if recent.len() < 4 {  // Need at least 4 out of 5 failures
            return false;
        }

        // Check if all recent failures have similar error messages
        let first = recent[0];
        recent.iter().all(|e| {
            e.len() >= 50 && first.len() >= 50 && e[..50] == first[..50]
        })
    }

}

/// Tool call budget - limits tool calls per iteration
#[derive(Debug, Clone)]
pub struct ToolCallBudget {
    pub max_calls_per_turn: usize,
    pub max_calls_per_tool: usize,
    pub calls_this_turn: usize,
    pub tool_calls: std::collections::HashMap<String, usize>,
    pub total_cost_estimate: f64,
    pub max_cost_per_turn: f64,
}

impl ToolCallBudget {
    pub fn new(max_calls_per_turn: usize, max_cost_per_turn: f64) -> Self {
        Self {
            max_calls_per_turn,
            max_calls_per_tool: 15,  // Increased from 5 - SWE-bench needs more tool calls per tool
            calls_this_turn: 0,
            tool_calls: std::collections::HashMap::new(),
            total_cost_estimate: 0.0,
            max_cost_per_turn,
        }
    }

    /// Reset for new turn
    pub fn reset_turn(&mut self) {
        self.calls_this_turn = 0;
        self.tool_calls.clear();
        self.total_cost_estimate = 0.0;
    }

    /// Get budget summary
    pub fn summary(&self) -> String {
        format!(
            "Budget: {}/{} calls, ${:.2}/${:.2} cost",
            self.calls_this_turn, self.max_calls_per_turn,
            self.total_cost_estimate, self.max_cost_per_turn
        )
    }
}

/// Loop state - tracks the current state of the agent loop
///
/// # SWE-bench Completeness
///
/// This struct implements the loop engineering patterns needed for SWE-bench:
/// - **Turn tracking**: Limits total iterations to prevent infinite loops
/// - **Failure detection**: Tracks consecutive failures and adjusts strategy
/// - **Budget management**: Limits tool calls per turn to control costs
/// - **Attempt history**: Records what was tried to avoid repeating failures
/// - **Convergence detection**: Detects when the agent is stuck in a loop
/// - **Strategy adaptation**: Changes approach based on failure patterns
#[derive(Debug, Clone)]
pub struct LoopState {
    pub turn: usize,
    pub max_turns: usize,
    pub consecutive_failures: usize,
    pub consecutive_recoverable_errors: usize,
    pub max_consecutive_failures: usize,
    pub strategy: LoopStrategy,
    pub attempt_history: AttemptHistory,
    pub budget: ToolCallBudget,
    /// Tracks unique actions to detect loops (e.g. same file edit 3 times)
    pub recent_actions: VecDeque<String>,
    /// Maximum unique actions to track for loop detection
    pub max_recent_actions: usize,
}

#[derive(Debug, Clone)]
pub enum LoopStrategy {
    Normal,
    RetryWithDifferentArgs,
    FallbackToSimplerTool,
    BreakAndReport,
}

impl LoopState {
    pub fn new(max_turns: usize) -> Self {
        Self {
            turn: 0,
            max_turns,
            consecutive_failures: 0,
            consecutive_recoverable_errors: 0,
            max_consecutive_failures: 8,  // Increased from 3 - SWE-bench tasks need more tolerance
            strategy: LoopStrategy::Normal,
            attempt_history: AttemptHistory::new(30),  // Increased from 20
            budget: ToolCallBudget::new(50, 50.0),  // Increased from (20, 2.0) - SWE-bench needs more budget
            recent_actions: VecDeque::new(),
            max_recent_actions: 15,  // Increased from 10
        }
    }

    /// Advance to next turn
    pub fn next_turn(&mut self) {
        self.turn += 1;
        self.budget.reset_turn();
    }

    /// Check if we should continue
    pub fn should_continue(&self) -> bool {
        self.turn < self.max_turns && self.consecutive_failures < self.max_consecutive_failures
    }

    /// Record a success
    pub fn record_success(&mut self, tool: &str, action: &str) {
        self.consecutive_failures = 0;
        self.consecutive_recoverable_errors = 0;
        self.strategy = LoopStrategy::Normal;
        self.attempt_history.record(tool, action, AttemptResult::Success);
        self.track_action(tool, action);
    }

    /// Record a failure
    pub fn record_failure(&mut self, tool: &str, action: &str, error: StructuredError) {
        // Track recoverable errors separately — they don't count as hard failures
        // but if they repeat too many times, we should still adjust strategy
        let is_recoverable = matches!(
            error.category,
            ErrorCategory::NotFound | ErrorCategory::Timeout | ErrorCategory::Compilation
        );

        if is_recoverable {
            self.consecutive_recoverable_errors += 1;
        } else {
            self.consecutive_failures += 1;
            self.consecutive_recoverable_errors = 0; // Reset on hard failure
        }
        self.attempt_history.record(tool, action, AttemptResult::Failure(error));
        self.track_action(tool, action);
        
        // Adjust strategy based on failure pattern - more tolerant for SWE-bench
        let effective_failures = self.consecutive_failures + 
            (self.consecutive_recoverable_errors / 3); // 3 recoverable = 1 hard failure
        
        if self.is_action_loop_detected() && effective_failures >= 5 {
            self.strategy = LoopStrategy::BreakAndReport;
        } else if self.attempt_history.is_repeating_failure() && effective_failures >= 4 {
            self.strategy = LoopStrategy::FallbackToSimplerTool;
        } else if effective_failures >= self.max_consecutive_failures {
            self.strategy = LoopStrategy::BreakAndReport;
        } else if effective_failures >= 3 {
            self.strategy = LoopStrategy::RetryWithDifferentArgs;
        }
    }

    /// Track an action for loop detection
    fn track_action(&mut self, tool: &str, action: &str) {
        let key = format!("{}:{}", tool, action);
        self.recent_actions.push_back(key);
        while self.recent_actions.len() > self.max_recent_actions {
            self.recent_actions.pop_front();
        }
    }

    /// Detect if the agent is stuck in a loop (same action repeated 5+ times)
    pub fn is_action_loop_detected(&self) -> bool {
        if self.recent_actions.len() < 5 {  // Increased from 3 to 5
            return false;
        }
        let last = self.recent_actions.back().unwrap();
        let count = self.recent_actions.iter().filter(|a| *a == last).count();
        count >= 5  // Increased from 3 to 5
    }

    /// Format loop state for display
    pub fn format_status(&self) -> String {
        format!(
            "Turn {}/{}, {} consecutive failures, strategy: {:?}, {}",
            self.turn, self.max_turns,
            self.consecutive_failures,
            self.strategy,
            self.budget.summary()
        )
    }

    /// Generate context summary for LLM
    pub fn generate_context_summary(&self) -> String {
        let mut summary = String::new();
        
        summary.push_str(&format!("{}\n", self.format_status()));
        summary.push_str(&self.attempt_history.summarize());
        
        match &self.strategy {
            LoopStrategy::Normal => {},
            LoopStrategy::RetryWithDifferentArgs => {
                summary.push_str("\n💡 Strategy: Try a different approach or arguments.\n");
            },
            LoopStrategy::FallbackToSimplerTool => {
                summary.push_str("\n💡 Strategy: Fall back to a simpler tool.\n");
            },
            LoopStrategy::BreakAndReport => {
                summary.push_str("\n⚠️ Strategy: Break loop and report issue to user.\n");
            },
        }
        
        summary
    }
}
pub struct RecoveryManager {
    pub retry_counts: HashMap<String, u32>,
    pub max_retries: u32,
    pub fallback_providers: Vec<String>,
    pub current_provider_index: usize,
    /// 记录失败的恢复策略，避免重复尝试
    pub failed_strategies: HashMap<String, Vec<String>>,
}

pub enum RecoveryAction {
    Continue,
    CompactAndRetry,
    SwitchProviderAndRetry,
    EscalateOutputTokens,
    InjectRecoveryMessage(String),
    StopWithError(String),
    CircuitBreakerCooldown(Duration),
    /// 回退到更简单的工具
    FallbackToSimplerTool { original_tool: String, fallback_tool: String, reason: String },
    /// 用不同的参数重试
    RetryWithDifferentArgs { suggestion: String },
    /// 跳过当前步骤，继续下一步
    SkipAndContinue { reason: String },
}

impl RecoveryManager {
    pub fn new(fallback_providers: Vec<String>) -> Self {
        Self {
            retry_counts: HashMap::new(),
            max_retries: 3,
            fallback_providers,
            current_provider_index: 0,
            failed_strategies: HashMap::new(),
        }
    }

    pub fn handle_error(&mut self, error: &AgentError, context: &RecoveryContext) -> RecoveryAction {
        match error {
            AgentError::PromptTooLong => {
                self.handle_prompt_too_long(context)
            }

            AgentError::MaxOutputTokens => {
                self.handle_max_output_tokens(context)
            }

            AgentError::RateLimit => {
                self.handle_rate_limit(context)
            }

            AgentError::StreamingError => {
                self.handle_streaming_error(context)
            }

            AgentError::ToolLoopDetected(signatures) => {
                self.handle_tool_loop(signatures, context)
            }

            AgentError::ToolExecutionFailed { tool_name, error_msg } => {
                self.handle_tool_failure(tool_name, error_msg, context)
            }

            AgentError::CompilationError { error_msg } => {
                self.handle_compilation_error(error_msg, context)
            }

            AgentError::TestFailure { error_msg } => {
                self.handle_test_failure(error_msg, context)
            }
        }
    }

    fn handle_prompt_too_long(&mut self, _context: &RecoveryContext) -> RecoveryAction {
        let count = self.retry_counts.get("prompt_too_long").copied().unwrap_or(0);
        
        // 智能学习：如果某种策略已失败 2+ 次，跳过它
        let compact_failed = self.is_strategy_known_failed("prompt_too_long", "compact");
        
        if count == 0 {
            self.retry_counts.insert("prompt_too_long".to_string(), 1);
            return RecoveryAction::CompactAndRetry;
        }
        if count == 1 {
            self.retry_counts.insert("prompt_too_long".to_string(), 2);
            if compact_failed {
                // 压缩已无效，直接停止
                return RecoveryAction::StopWithError(
                    "Context too long — compaction previously failed. Use /compact manually.".to_string()
                );
            }
            return RecoveryAction::CompactAndRetry;
        }
        
        self.record_failed_strategy("prompt_too_long", "multiple_compact_attempts");
        RecoveryAction::StopWithError("Context too long after compression".to_string())
    }

    fn handle_max_output_tokens(&mut self, _context: &RecoveryContext) -> RecoveryAction {
        let count = self.retry_counts.get("max_output_tokens").copied().unwrap_or(0);
        
        if count < 3 {
            self.retry_counts.insert("max_output_tokens".to_string(), count + 1);
            
            if count == 0 {
                return RecoveryAction::EscalateOutputTokens;
            }
            
            // 后续重试使用更具体的指导
            let messages = [
                "Continue where you left off. Focus on completing the current step only.",
                "Break the remaining work into smaller pieces. Complete just one piece now.",
                "Provide a minimal working solution. You can improve it later.",
            ];
            
            let index = count as usize;
            return RecoveryAction::InjectRecoveryMessage(
                messages[index.min(messages.len() - 1)].to_string()
            );
        }
        
        self.record_failed_strategy("max_output_tokens", "escalation_exhausted");
        RecoveryAction::StopWithError("Max output tokens exceeded".to_string())
    }

    fn handle_rate_limit(&mut self, _context: &RecoveryContext) -> RecoveryAction {
        let count = self.retry_counts.get("rate_limit").copied().unwrap_or(0);

        // 学习驱动：如果之前 provider 切换失败过，跳过直接 cooldown
        let switch_failed = self.is_strategy_known_failed("rate_limit", "provider_switch");

        if !switch_failed && self.current_provider_index < self.fallback_providers.len() {
            let _provider = self.fallback_providers[self.current_provider_index].clone();
            self.current_provider_index += 1;
            self.retry_counts.insert("rate_limit".to_string(), count + 1);
            return RecoveryAction::SwitchProviderAndRetry;
        }

        // 最多 3 次 cooldown 后停止，避免无限等待
        if count >= 3 {
            return RecoveryAction::StopWithError(
                "Rate limit persists after multiple cooldowns — try again later".to_string()
            );
        }

        self.retry_counts.insert("rate_limit".to_string(), count + 1);
        // Exponential backoff instead of a fixed 60s: 30s → 60s → 90s.
        // A fixed 60s makes the UI appear frozen with zero feedback.
        let backoff_secs = 30 * (count + 1) as u64;
        RecoveryAction::CircuitBreakerCooldown(Duration::from_secs(backoff_secs))
    }

    fn handle_streaming_error(&mut self, _context: &RecoveryContext) -> RecoveryAction {
        let count = self.retry_counts.get("streaming").copied().unwrap_or(0);
        
        if count == 0 {
            self.retry_counts.insert("streaming".to_string(), 1);
            return RecoveryAction::CompactAndRetry;
        }
        
        if count == 1 {
            self.retry_counts.insert("streaming".to_string(), 2);
            // 尝试非流式模式
            return RecoveryAction::InjectRecoveryMessage(
                "Switching to non-streaming mode for reliability.".to_string()
            );
        }
        
        RecoveryAction::StopWithError("Streaming failed after retry".to_string())
    }

    fn handle_tool_loop(&mut self, signatures: &[String], _context: &RecoveryContext) -> RecoveryAction {
        // 分析循环模式
        let loop_analysis = analyze_tool_loop(signatures);
        
        // 尝试打破循环
        if signatures.len() >= 2 {
            let last_tool = &signatures[signatures.len() - 1];
            let tool_name = extract_tool_name(last_tool);
            
            // 建议使用不同的工具
            if let Some(fallback) = suggest_alternative_tool(&tool_name) {
                return RecoveryAction::FallbackToSimplerTool {
                    original_tool: tool_name,
                    fallback_tool: fallback,
                    reason: format!("Breaking tool loop: {}", loop_analysis),
                };
            }
        }
        
        RecoveryAction::StopWithError(format!(
            "Tool loop detected: {}",
            loop_analysis
        ))
    }

    fn handle_tool_failure(
        &mut self,
        tool_name: &str,
        error_msg: &str,
        _context: &RecoveryContext,
    ) -> RecoveryAction {
        let key = format!("tool_failure:{}", tool_name);
        let count = self.retry_counts.get(&key).copied().unwrap_or(0);
        
        // 检查是否已知此工具的失败模式
        if let Some(failures) = self.failed_strategies.get(tool_name) {
            if failures.contains(&error_msg.to_string()) {
                // 已知失败，建议替代方案
                if let Some(alternative) = suggest_alternative_tool(tool_name) {
                    return RecoveryAction::FallbackToSimplerTool {
                        original_tool: tool_name.to_string(),
                        fallback_tool: alternative,
                        reason: format!("Known failure: {}", error_msg),
                    };
                }
            }
        }
        
        if count < 2 {
            self.retry_counts.insert(key, count + 1);
            
            // 分析错误类型并提供建议
            let suggestion = analyze_tool_error(tool_name, error_msg);
            return RecoveryAction::RetryWithDifferentArgs { suggestion };
        }
        
        // 记录失败
        self.record_failed_strategy(tool_name, error_msg);
        
        // 尝试回退到更简单的工具
        if let Some(alternative) = suggest_alternative_tool(tool_name) {
            return RecoveryAction::FallbackToSimplerTool {
                original_tool: tool_name.to_string(),
                fallback_tool: alternative,
                reason: format!("Tool failed after retries: {}", error_msg),
            };
        }
        
        RecoveryAction::StopWithError(format!("Tool '{}' failed: {}", tool_name, error_msg))
    }

    fn handle_compilation_error(&mut self, error_msg: &str, _context: &RecoveryContext) -> RecoveryAction {
        let count = self.retry_counts.get("compilation_error").copied().unwrap_or(0);
        
        if count < 3 {
            self.retry_counts.insert("compilation_error".to_string(), count + 1);
            
            // 提供具体的修复建议
            let suggestion = analyze_compilation_error(error_msg);
            return RecoveryAction::RetryWithDifferentArgs { suggestion };
        }
        
        RecoveryAction::StopWithError(format!(
            "Compilation error persists after {} attempts: {}",
            count, error_msg
        ))
    }

    fn handle_test_failure(&mut self, error_msg: &str, _context: &RecoveryContext) -> RecoveryAction {
        let count = self.retry_counts.get("test_failure").copied().unwrap_or(0);
        
        if count < 2 {
            self.retry_counts.insert("test_failure".to_string(), count + 1);
            
            // 分析测试失败原因
            let suggestion = analyze_test_failure(error_msg);
            return RecoveryAction::RetryWithDifferentArgs { suggestion };
        }
        
        RecoveryAction::StopWithError(format!(
            "Test failure persists after {} attempts: {}",
            count, error_msg
        ))
    }

    /// 记录失败的恢复策略
    fn record_failed_strategy(&mut self, context: &str, error: &str) {
        self.failed_strategies
            .entry(context.to_string())
            .or_insert_with(Vec::new)
            .push(error.to_string());
    }

    /// 检查策略是否已被证明失败（避免重复尝试无效策略）
    fn is_strategy_known_failed(&self, context: &str, strategy: &str) -> bool {
        self.failed_strategies
            .get(context)
            .map(|failures| failures.iter().any(|f| f == strategy))
            .unwrap_or(false)
    }
}

pub struct RecoveryContext {
    pub current_tokens: usize,
    pub max_tokens: usize,
    pub current_output_tokens: usize,
    pub max_output_tokens: usize,
    pub turn_count: u32,
    pub messages_since_last_compact: u32,
    pub last_tool_used: Option<String>,
    pub last_tool_error: Option<String>,
}

pub enum AgentError {
    PromptTooLong,
    MaxOutputTokens,
    RateLimit,
    StreamingError,
    ToolLoopDetected(Vec<String>),
    ToolExecutionFailed {
        tool_name: String,
        error_msg: String,
    },
    CompilationError {
        error_msg: String,
    },
    TestFailure {
        error_msg: String,
    },
}

/// 分析工具循环模式
fn analyze_tool_loop(signatures: &[String]) -> String {
    if signatures.is_empty() {
        return "unknown loop pattern".to_string();
    }
    
    let unique_tools: std::collections::HashSet<_> = signatures
        .iter()
        .map(|s| extract_tool_name(s))
        .collect();
    
    if unique_tools.len() == 1 {
        format!("repeated call to same tool with same arguments")
    } else if unique_tools.len() == 2 {
        let tools: Vec<_> = unique_tools.into_iter().collect();
        format!("alternating between {} and {}", tools[0], tools[1])
    } else {
        format!("circular pattern involving {} tools", unique_tools.len())
    }
}

/// 从签名中提取工具名
fn extract_tool_name(signature: &str) -> String {
    signature
        .split('(')
        .next()
        .unwrap_or(signature)
        .trim()
        .to_string()
}

/// 建议替代工具
fn suggest_alternative_tool(tool_name: &str) -> Option<String> {
    match tool_name {
        "Bash" | "run_shell_command" => Some("Grep".to_string()),
        "Write" | "create_file" => Some("Edit".to_string()),
        "Read" | "view_file" => Some("Grep".to_string()),
        "Grep" => Some("Glob".to_string()),
        "SemanticSearch" => Some("Grep".to_string()),
        "multi_edit" => Some("Edit".to_string()),
        _ => None,
    }
}

/// 分析工具错误并提供建议
fn analyze_tool_error(tool_name: &str, error_msg: &str) -> String {
    let error_lower = error_msg.to_lowercase();
    
    if error_lower.contains("permission denied") || error_lower.contains("access denied") {
        return "Check file permissions or try a different path".to_string();
    }
    
    if error_lower.contains("not found") || error_lower.contains("no such file") {
        return "Verify the file path exists; use 'glob' to find the correct path".to_string();
    }
    
    if error_lower.contains("timeout") {
        return "Operation timed out; try a simpler approach or break into smaller steps".to_string();
    }
    
    if error_lower.contains("syntax error") || error_lower.contains("parse error") {
        return "Check the syntax of your command or file content".to_string();
    }
    
    match tool_name {
        "Bash" | "run_shell_command" => {
            "Try a simpler command or break into multiple steps".to_string()
        }
        "Write" | "create_file" => {
            "Ensure directory exists and you have write permissions".to_string()
        }
        "Edit" | "multi_edit" => {
            "Verify the exact text to replace exists in the file".to_string()
        }
        _ => format!("Try a different approach for '{}'", tool_name),
    }
}

/// 分析编译错误并提供建议
fn analyze_compilation_error(error_msg: &str) -> String {
    let error_lower = error_msg.to_lowercase();
    
    if error_lower.contains("unresolved import") || error_lower.contains("module not found") {
        return "Check import paths and ensure dependencies are installed".to_string();
    }
    
    if error_lower.contains("type mismatch") || error_lower.contains("expected") {
        return "Verify variable types match the expected signatures".to_string();
    }
    
    if error_lower.contains("borrow") || error_lower.contains("lifetime") {
        return "Review Rust ownership and borrowing rules; consider using references".to_string();
    }
    
    if error_lower.contains("undefined") || error_lower.contains("not defined") {
        return "Ensure all variables and functions are properly defined before use".to_string();
    }
    
    "Review the error location and fix the specific issue mentioned".to_string()
}

/// 分析测试失败并提供建议
fn analyze_test_failure(error_msg: &str) -> String {
    let error_lower = error_msg.to_lowercase();
    
    if error_lower.contains("assertion") {
        return "Check expected values vs actual values in the assertion".to_string();
    }
    
    if error_lower.contains("timeout") {
        return "Test timed out; check for infinite loops or slow operations".to_string();
    }
    
    if error_lower.contains("panic") {
        return "Test panicked; check for unwrap() calls or index out of bounds".to_string();
    }
    
    "Review the test failure details and fix the underlying issue".to_string()
}

