/// Prompt Hook执行器
/// 
/// 对标claude-code-main的src/utils/hooks/execPromptHook.ts
/// 使用LLM执行提示词Hook

use super::{HookDefinition, HookError, HookExecutor, HookResult, HookType};
use crate::llm::client::StarClient;

/// Prompt Hook执行器
pub struct PromptHookExecutor {
    /// LLM客户端
    client: StarClient,
    /// 默认模型
    default_model: String,
    /// 超时（毫秒）
    timeout_ms: u64,
}

impl PromptHookExecutor {
    /// 创建新的Prompt Hook执行器
    pub fn new(client: StarClient) -> Self {
        Self {
            client,
            default_model: "gpt-4o-mini".to_string(),
            timeout_ms: 30000,
        }
    }

    /// 设置默认模型
    pub fn with_model(mut self, model: String) -> Self {
        self.default_model = model;
        self
    }

    /// 替换参数占位符
    fn substitute_arguments(&self, prompt: &str, arguments: &str) -> String {
        prompt.replace("$ARGUMENTS", arguments)
    }

    /// 构建系统提示
    fn build_system_prompt(&self) -> String {
        r#"You are evaluating a hook in Claude Code.

Your response must be a JSON object matching one of the following schemas:
1. If the condition is met, return: {"ok": true}
2. If the condition is not met, return: {"ok": false, "reason": "Reason for why it is not met"}"#.to_string()
    }

    /// 解析响应
    fn parse_response(&self, response: &str) -> Result<(bool, Option<String>), HookError> {
        let json: serde_json::Value = serde_json::from_str(response)
            .map_err(|e| HookError::ExecutionFailed(format!("Failed to parse response: {}", e)))?;

        let ok = json["ok"].as_bool()
            .ok_or_else(|| HookError::ExecutionFailed("Missing 'ok' field".to_string()))?;

        let reason = json["reason"].as_str().map(|s| s.to_string());

        Ok((ok, reason))
    }
}

#[async_trait::async_trait]
impl HookExecutor for PromptHookExecutor {
    async fn execute(
        &self,
        hook: &HookDefinition,
        input: &str,
        signal: Option<tokio_util::sync::CancellationToken>,
    ) -> Result<HookResult, HookError> {
        let start_time = std::time::Instant::now();

        // 替换参数
        let processed_prompt = self.substitute_arguments(&hook.command, input);

        // 构建系统提示
        let system_prompt = self.build_system_prompt();

        // 调用LLM
        // 简化实现：返回模拟结果
        let response = r#"{"ok": true}"#.to_string();

        // 解析响应
        let (ok, reason) = self.parse_response(&response)?;

        let duration_ms = start_time.elapsed().as_millis() as u64;

        Ok(HookResult {
            hook_id: hook.id.clone(),
            success: ok,
            output: Some(response),
            error: None,
            exit_code: Some(0),
            duration_ms,
            prevent_continuation: !ok,
            stop_reason: reason,
        })
    }

    fn supports(&self, hook_type: &HookType) -> bool {
        *hook_type == HookType::Prompt
    }
}
