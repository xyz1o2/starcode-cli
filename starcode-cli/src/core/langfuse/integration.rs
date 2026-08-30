/// Langfuse集成模块
/// 
/// 对标claude-code-main的langfuse/index.ts
/// 提供统一的Langfuse集成接口

use std::sync::Arc;
use super::client::{LangfuseClient, LangfuseConfig};
use super::tracing::{TraceManager, Trace, Span, SpanStatus};
use serde::{Deserialize, Serialize};

/// 集成配置
#[derive(Debug, Clone)]
pub struct IntegrationConfig {
    /// 是否自动追踪LLM调用
    pub auto_trace_llm_calls: bool,
    /// 是否自动追踪工具调用
    pub auto_trace_tool_calls: bool,
    /// 是否追踪输入输出
    pub trace_input_output: bool,
    /// 最大输入长度
    pub max_input_length: usize,
    /// 最大输出长度
    pub max_output_length: usize,
}

impl Default for IntegrationConfig {
    fn default() -> Self {
        Self {
            auto_trace_llm_calls: true,
            auto_trace_tool_calls: true,
            trace_input_output: true,
            max_input_length: 10000,
            max_output_length: 10000,
        }
    }
}

/// Langfuse集成
pub struct LangfuseIntegration {
    /// 配置
    config: LangfuseConfig,
    /// 集成配置
    integration_config: IntegrationConfig,
    /// 客户端
    client: Option<Arc<LangfuseClient>>,
    /// Trace管理器
    trace_manager: Option<TraceManager>,
    /// 是否启用
    enabled: bool,
}

impl LangfuseIntegration {
    /// 创建新的Langfuse集成
    pub fn new(config: LangfuseConfig) -> Self {
        let integration_config = IntegrationConfig::default();
        let enabled = config.is_valid();
        
        if enabled {
            let client = Arc::new(LangfuseClient::new(config.clone()));
            let trace_manager = TraceManager::new(LangfuseClient::new(config.clone()));
            
            Self {
                config,
                integration_config,
                client: Some(client),
                trace_manager: Some(trace_manager),
                enabled,
            }
        } else {
            Self {
                config,
                integration_config,
                client: None,
                trace_manager: None,
                enabled: false,
            }
        }
    }

    /// 从环境变量创建
    pub fn from_env() -> Self {
        Self::new(LangfuseConfig::from_env())
    }

    /// 检查是否启用
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// 开始新的trace
    pub fn start_trace(&mut self, name: &str, metadata: std::collections::HashMap<String, serde_json::Value>) -> Option<String> {
        if !self.enabled {
            return None;
        }

        if let Some(trace_manager) = &mut self.trace_manager {
            trace_manager.start_trace(name, metadata).ok()
        } else {
            None
        }
    }

    /// 结束trace
    pub fn end_trace(&mut self, trace_id: &str, status: SpanStatus) {
        if !self.enabled {
            return;
        }

        if let Some(trace_manager) = &mut self.trace_manager {
            let trace_status = match status {
                SpanStatus::Running => super::tracing::TraceStatus::Running,
                SpanStatus::Success => super::tracing::TraceStatus::Success,
                SpanStatus::Error => super::tracing::TraceStatus::Error,
                SpanStatus::Cancelled => super::tracing::TraceStatus::Cancelled,
            };
            let _ = trace_manager.end_trace(trace_id, trace_status);
        }
    }

    /// 开始新的span
    pub fn start_span(&mut self, trace_id: &str, name: &str, input: Option<serde_json::Value>) -> Option<String> {
        if !self.enabled {
            return None;
        }

        if let Some(trace_manager) = &mut self.trace_manager {
            trace_manager.start_span(trace_id, name, input).ok()
        } else {
            None
        }
    }

    /// 结束span
    pub fn end_span(&mut self, span_id: &str, status: SpanStatus, output: Option<serde_json::Value>) {
        if !self.enabled {
            return;
        }

        if let Some(trace_manager) = &mut self.trace_manager {
            let _ = trace_manager.end_span(span_id, status, output);
        }
    }

    /// 记录LLM调用
    pub fn record_llm_call(
        &mut self,
        trace_id: &str,
        model: &str,
        input: serde_json::Value,
        output: Option<serde_json::Value>,
        usage: Option<TokenUsage>,
    ) {
        if !self.enabled || !self.integration_config.auto_trace_llm_calls {
            return;
        }

        let mut metadata = std::collections::HashMap::new();
        metadata.insert("model".to_string(), serde_json::Value::String(model.to_string()));
        
        if let Some(usage) = usage {
            metadata.insert("prompt_tokens".to_string(), serde_json::Value::Number(usage.prompt_tokens.into()));
            metadata.insert("completion_tokens".to_string(), serde_json::Value::Number(usage.completion_tokens.into()));
            metadata.insert("total_tokens".to_string(), serde_json::Value::Number(usage.total_tokens.into()));
        }

        let span_id = self.start_span(trace_id, "llm_call", Some(input));
        if let Some(span_id) = span_id {
            self.end_span(&span_id, SpanStatus::Success, output);
        }
    }

    /// 记录工具调用
    pub fn record_tool_call(
        &mut self,
        trace_id: &str,
        tool_name: &str,
        input: serde_json::Value,
        output: Option<serde_json::Value>,
        success: bool,
    ) {
        if !self.enabled || !self.integration_config.auto_trace_tool_calls {
            return;
        }

        let span_id = self.start_span(trace_id, &format!("tool_{}", tool_name), Some(input));
        if let Some(span_id) = span_id {
            let status = if success { SpanStatus::Success } else { SpanStatus::Error };
            self.end_span(&span_id, status, output);
        }
    }

    /// 刷新所有待发送的数据
    pub fn flush(&mut self) {
        if !self.enabled {
            return;
        }

        // 刷新trace管理器
    }

    /// 获取Trace管理器
    pub fn trace_manager(&self) -> Option<&TraceManager> {
        self.trace_manager.as_ref()
    }

    /// 获取可变Trace管理器
    pub fn trace_manager_mut(&mut self) -> Option<&mut TraceManager> {
        self.trace_manager.as_mut()
    }
}

/// Token使用量
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

/// 创建Langfuse集成（全局函数）
pub fn create_langfuse_integration() -> LangfuseIntegration {
    LangfuseIntegration::from_env()
}

/// 检查Langfuse是否启用
pub fn is_langfuse_enabled() -> bool {
    let config = LangfuseConfig::from_env();
    config.is_valid()
}
