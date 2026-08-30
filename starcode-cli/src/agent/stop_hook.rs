use crate::types::StarMessage;

/// Stop Hook配置
#[derive(Debug, Clone)]
pub struct StopHookConfig {
    /// 是否启用Stop Hook
    pub enabled: bool,
    /// 最大连续空响应次数
    pub max_empty_responses: usize,
    /// 最大连续相同工具调用次数
    pub max_same_tool_calls: usize,
    /// 最大连续失败次数
    pub max_consecutive_failures: usize,
}

impl Default for StopHookConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_empty_responses: 3,
            max_same_tool_calls: 5,
            max_consecutive_failures: 3,
        }
    }
}

impl StopHookConfig {
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_STOP_HOOK_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let max_empty_responses = std::env::var("STAR_STOP_HOOK_MAX_EMPTY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);

        let max_same_tool_calls = std::env::var("STAR_STOP_HOOK_MAX_SAME_TOOL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5);

        let max_consecutive_failures = std::env::var("STAR_STOP_HOOK_MAX_FAILURES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);

        Self {
            enabled,
            max_empty_responses,
            max_same_tool_calls,
            max_consecutive_failures,
        }
    }
}

/// Stop Hook管理器
pub struct StopHookManager {
    config: StopHookConfig,
    /// 连续空响应计数
    empty_response_count: usize,
    /// 连续相同工具调用计数
    same_tool_call_count: usize,
    /// 上一次工具调用名称
    last_tool_name: Option<String>,
    /// 连续失败计数
    consecutive_failure_count: usize,
    /// 是否已阻止
    is_blocked: bool,
    /// 阻止原因
    block_reason: Option<String>,
}

impl StopHookManager {
    pub fn new() -> Self {
        let config = StopHookConfig::from_env();
        Self {
            config,
            empty_response_count: 0,
            same_tool_call_count: 0,
            last_tool_name: None,
            consecutive_failure_count: 0,
            is_blocked: false,
            block_reason: None,
        }
    }

    /// 记录响应
    pub fn record_response(&mut self, content: &Option<String>, tool_calls: &[String]) {
        if !self.config.enabled {
            return;
        }

        // 检查是否是空响应
        let is_empty = content.as_ref().map_or(true, |c| c.trim().is_empty());
        if is_empty && tool_calls.is_empty() {
            self.empty_response_count += 1;
            if self.empty_response_count >= self.config.max_empty_responses {
                self.is_blocked = true;
                self.block_reason = Some(format!(
                    "Too many consecutive empty responses ({}/{})",
                    self.empty_response_count, self.config.max_empty_responses
                ));
            }
        } else {
            self.empty_response_count = 0;
        }

        // 检查是否是连续相同工具调用
        if let Some(tool_name) = tool_calls.first() {
            if self.last_tool_name.as_ref() == Some(tool_name) {
                self.same_tool_call_count += 1;
                if self.same_tool_call_count >= self.config.max_same_tool_calls {
                    self.is_blocked = true;
                    self.block_reason = Some(format!(
                        "Too many consecutive same tool calls: {} ({}/{})",
                        tool_name, self.same_tool_call_count, self.config.max_same_tool_calls
                    ));
                }
            } else {
                self.same_tool_call_count = 1;
                self.last_tool_name = Some(tool_name.clone());
            }
        }
    }

    /// 记录工具执行结果
    pub fn record_tool_result(&mut self, success: bool) {
        if !self.config.enabled {
            return;
        }

        if !success {
            self.consecutive_failure_count += 1;
            if self.consecutive_failure_count >= self.config.max_consecutive_failures {
                self.is_blocked = true;
                self.block_reason = Some(format!(
                    "Too many consecutive tool failures ({}/{})",
                    self.consecutive_failure_count, self.config.max_consecutive_failures
                ));
            }
        } else {
            self.consecutive_failure_count = 0;
        }
    }

    /// 检查是否应该阻止继续
    pub fn should_prevent_continuation(&self) -> StopHookResult {
        if !self.config.enabled || !self.is_blocked {
            return StopHookResult {
                prevent_continuation: false,
                blocking_errors: Vec::new(),
                reason: None,
            };
        }

        let reason = self.block_reason.clone().unwrap_or_default();
        let error_message = StarMessage::system(format!(
            "[STOP_HOOK] Conversation stopped: {}",
            reason
        ));

        StopHookResult {
            prevent_continuation: true,
            blocking_errors: vec![error_message],
            reason: Some(reason),
        }
    }

    /// 重置状态
    pub fn reset(&mut self) {
        self.empty_response_count = 0;
        self.same_tool_call_count = 0;
        self.last_tool_name = None;
        self.consecutive_failure_count = 0;
        self.is_blocked = false;
        self.block_reason = None;
    }

    /// 获取状态
    pub fn get_status(&self) -> StopHookStatus {
        StopHookStatus {
            enabled: self.config.enabled,
            is_blocked: self.is_blocked,
            block_reason: self.block_reason.clone(),
            empty_response_count: self.empty_response_count,
            same_tool_call_count: self.same_tool_call_count,
            consecutive_failure_count: self.consecutive_failure_count,
        }
    }
}

/// Stop Hook结果
#[derive(Debug, Clone)]
pub struct StopHookResult {
    /// 是否阻止继续
    pub prevent_continuation: bool,
    /// 阻止错误消息
    pub blocking_errors: Vec<StarMessage>,
    /// 阻止原因
    pub reason: Option<String>,
}

/// Stop Hook状态
#[derive(Debug, Clone)]
pub struct StopHookStatus {
    pub enabled: bool,
    pub is_blocked: bool,
    pub block_reason: Option<String>,
    pub empty_response_count: usize,
    pub same_tool_call_count: usize,
    pub consecutive_failure_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_response_detection() {
        let mut manager = StopHookManager::new();
        
        // 连续空响应
        manager.record_response(&None, &[]);
        manager.record_response(&None, &[]);
        assert!(!manager.should_prevent_continuation().prevent_continuation);
        
        manager.record_response(&None, &[]);
        assert!(manager.should_prevent_continuation().prevent_continuation);
    }

    #[test]
    fn test_same_tool_call_detection() {
        let mut manager = StopHookManager::new();
        
        // 连续相同工具调用
        for _ in 0..4 {
            manager.record_response(&None, &["Bash".to_string()]);
        }
        assert!(!manager.should_prevent_continuation().prevent_continuation);
        
        manager.record_response(&None, &["Bash".to_string()]);
        assert!(manager.should_prevent_continuation().prevent_continuation);
    }

    #[test]
    fn test_consecutive_failure_detection() {
        let mut manager = StopHookManager::new();
        
        // 连续失败
        manager.record_tool_result(false);
        manager.record_tool_result(false);
        assert!(!manager.should_prevent_continuation().prevent_continuation);
        
        manager.record_tool_result(false);
        assert!(manager.should_prevent_continuation().prevent_continuation);
    }
}
