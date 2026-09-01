//! P1/P2: 程序化 Agent 执行器
//!
//! 提供在不启动 TUI 的情况下运行 agent 并收集完整工具调用历史的能力。
//! 供 eval harness 的 P1 (行为评估) 和 P2 (E2E 评估) 使用。

use crate::agent::eval_behaviors::{
    compute_efficiency, detect_finish_signal, FinishSignal, ToolCallRecord,
};
use crate::agent::messaging::AgentEvent;
use crate::agent::Agent;
use crate::core::config::Config;
use crate::llm::client::StarClient;
use crate::types::StarMessage;
use futures::StreamExt;
use serde::Serialize;
use std::sync::Arc;

/// Agent 运行配置
#[derive(Debug, Clone)]
pub struct AgentRunConfig {
    /// 最大 LLM 调用轮次
    pub max_turns: usize,
    /// 超时秒数 (暂留，P2 worktree 隔离时用)
    pub timeout_secs: u64,
    /// 是否收集详细 trace
    pub trace_enabled: bool,
    /// 系统提示词覆盖 (e.g. 用于 Evaluator agent)
    pub system_prompt_override: Option<String>,
    /// 可用工具白名单 (None = 全部工具)
    pub allowed_tools: Option<Vec<String>>,
    /// 禁止工具列表
    pub forbidden_tools: Option<Vec<String>>,
}

impl Default for AgentRunConfig {
    fn default() -> Self {
        Self {
            max_turns: 30,
            timeout_secs: 300,
            trace_enabled: true,
            system_prompt_override: None,
            allowed_tools: None,
            forbidden_tools: None,
        }
    }
}

/// Agent 运行结果
#[derive(Debug, Serialize, Clone)]
pub struct AgentRunResult {
    /// 最终 assistant 文本
    pub final_message: String,
    /// 完整工具调用历史
    pub tool_calls: Vec<ToolCallRecord>,
    /// 总 LLM 轮次
    pub total_turns: usize,
    /// 完成信号
    pub finish_signal: FinishSignal,
    /// 总耗时 ms
    pub duration_ms: u64,
    /// 是否成功 (无工具错误)
    pub success: bool,
    /// 错误信息 (如有)
    pub error: Option<String>,
    /// 累计 token 用量（跨轮次累加，用于成本/效率评估）
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
}

/// 运行 agent 并收集完整结果。
///
/// 这是 P1/P2 的核心执行函数。它通过 `run_stream` API 驱动 agent，
/// 收集所有 `ToolStarted`/`ToolFinished` 事件，最终返回结构化的运行结果。
pub async fn run_agent_with_trace(
    agent: &mut Agent,
    prompt: &str,
    config: &AgentRunConfig,
) -> AgentRunResult {
    let started = std::time::Instant::now();
    let mut tool_calls: Vec<ToolCallRecord> = Vec::new();
    let mut final_message = String::new();
    let mut total_turns = 0usize;
    let mut error: Option<String> = None;
    let mut prompt_tokens: u64 = 0;
    let mut completion_tokens: u64 = 0;

    // 如果有系统提示词覆盖，注入为第一条消息
    if let Some(ref sys_prompt) = config.system_prompt_override {
        agent.push_message(StarMessage::system(sys_prompt.clone()));
    }

    let mut stream = agent.run_stream(prompt.to_string());

    while let Some(event) = stream.next().await {
        match event {
            Ok(AgentEvent::ToolStarted { tool_call }) => {
                // 仅记录，等待 ToolFinished 获取结果
                if config.trace_enabled {
                    let _ = &tool_call; // trace 已通过 ToolFinished 完整记录
                }
            }
            Ok(AgentEvent::ToolFinished { tool_call, result }) => {
                let output_summary = summarize_output(result.output.as_deref().unwrap_or(""), 200);
                let args_value = serde_json::from_str(&tool_call.function.arguments).unwrap_or(
                    serde_json::Value::String(tool_call.function.arguments.clone()),
                );
                tool_calls.push(ToolCallRecord {
                    tool_name: tool_call.function.name.clone(),
                    arguments: args_value,
                    success: result.success,
                    output_summary,
                });
            }
            Ok(AgentEvent::Message(content)) => {
                final_message = content;
                total_turns += 1;
            }
            Ok(AgentEvent::TextDelta(_)) | Ok(AgentEvent::ReasoningDelta(_)) => {
                // 流式增量文本，不计入最终结果
            }
            Ok(AgentEvent::TurnFinished) => {
                // 一轮结束
            }
            Ok(AgentEvent::Done) => {
                break;
            }
            Ok(AgentEvent::Error(err)) => {
                error = Some(format!("Agent error: {err}"));
                break;
            }
            Ok(AgentEvent::ToolProgress { .. }) => {
                // 进度事件，略过
            }
            Ok(AgentEvent::StatsUpdate { token_usage }) => {
                if let Some(usage) = token_usage {
                    prompt_tokens += usage.prompt_tokens as u64;
                    completion_tokens += usage.completion_tokens as u64;
                }
            }
            Ok(AgentEvent::Trace { .. }) => {
                // 诊断 trace
            }
            Err(e) => {
                error = Some(format!("Stream error: {e}"));
                break;
            }
        }
    }

    let duration_ms = started.elapsed().as_millis() as u64;
    let _efficiency = compute_efficiency(&tool_calls, total_turns);
    let finish_signal = detect_finish_signal(
        &tool_calls,
        if final_message.is_empty() {
            None
        } else {
            Some(&final_message)
        },
        config.max_turns,
        total_turns,
    );

    let success = error.is_none()
        && tool_calls.iter().all(|tc| tc.success)
        && !matches!(
            finish_signal,
            FinishSignal::FalseFinish { .. } | FinishSignal::ToolError { .. }
        );

    AgentRunResult {
        final_message,
        tool_calls,
        total_turns,
        finish_signal,
        duration_ms,
        success,
        error,
        total_tokens: prompt_tokens + completion_tokens,
        prompt_tokens,
        completion_tokens,
    }
}

/// 快速运行 agent (不走 stream，更快但信息更少)。
/// 适用于仅需最终回复的场景。
pub async fn run_agent_quick(
    agent: &mut Agent,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    agent.run(prompt).await.map_err(|e| e)
}

/// 创建 Evaluator agent。
///
/// Evaluator 是**只读**的 agent，上下文仅包含：
/// - 原始 prompt
/// - Builder 的 diff
/// - 验收标准
///
/// 绝不允许 Write/Edit 工具。
///
/// 注意：此函数返回一个 `Agent` 结构体，调用方负责传入正确的
/// `StarClient` 和 `Config`（已配置只读工具）。
pub fn make_evaluator_agent(client: StarClient, config: Arc<Config>) -> Agent {
    Agent::new(client, config)
}

/// 创建 Builder agent (全权限，用于执行 E2E 任务)。
pub fn make_builder_agent(client: StarClient, config: Arc<Config>) -> Agent {
    Agent::new(client, config)
}

/// 截断工具输出为摘要
fn summarize_output(output: &str, max_len: usize) -> String {
    let trimmed = output.trim();
    if trimmed.len() <= max_len {
        trimmed.to_string()
    } else {
        format!("{}...({} chars)", &trimmed[..max_len], trimmed.len())
    }
}
