use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH, Instant};
use serde::{Deserialize, Serialize};
use super::client::LangfuseClient;
use super::convert::{EventConverter, LlmCall, ToolCall, ErrorEvent};
use super::sanitize::DataSanitizer;

/// Trace状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TraceStatus {
    /// 进行中
    Running,
    /// 成功完成
    Success,
    /// 失败
    Error,
    /// 取消
    Cancelled,
}

/// Span状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SpanStatus {
    /// 进行中
    Running,
    /// 成功完成
    Success,
    /// 失败
    Error,
    /// 取消
    Cancelled,
}

/// Trace
#[derive(Debug, Clone)]
pub struct Trace {
    /// Trace ID
    pub id: String,
    /// Trace名称
    pub name: String,
    /// 开始时间
    pub start_time: Instant,
    /// 状态
    pub status: TraceStatus,
    /// 元数据
    pub metadata: HashMap<String, serde_json::Value>,
    /// 标签
    pub tags: Vec<String>,
}

/// Span
#[derive(Debug, Clone)]
pub struct Span {
    /// Span ID
    pub id: String,
    /// 父Trace ID
    pub trace_id: String,
    /// Span名称
    pub name: String,
    /// 开始时间
    pub start_time: Instant,
    /// 状态
    pub status: SpanStatus,
    /// 输入
    pub input: Option<serde_json::Value>,
    /// 输出
    pub output: Option<serde_json::Value>,
    /// 元数据
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Trace管理器
/// 
/// 管理Langfuse的trace和span
pub struct TraceManager {
    /// Langfuse客户端
    client: LangfuseClient,
    /// 事件转换器
    converter: EventConverter,
    /// 数据清理器
    sanitizer: DataSanitizer,
    /// 活跃的traces
    active_traces: HashMap<String, Trace>,
    /// 活跃的spans
    active_spans: HashMap<String, Span>,
    /// 是否启用
    enabled: bool,
}

impl TraceManager {
    /// 创建新的Trace管理器
    pub fn new(client: LangfuseClient) -> Self {
        Self {
            client,
            converter: EventConverter::new(),
            sanitizer: DataSanitizer::new(),
            active_traces: HashMap::new(),
            active_spans: HashMap::new(),
            enabled: true,
        }
    }

    /// 从环境变量创建
    pub fn from_env() -> Self {
        Self::new(LangfuseClient::from_env())
    }

    /// 开始新的trace
    pub fn start_trace(&mut self, name: &str, metadata: HashMap<String, serde_json::Value>) -> Result<String, TraceError> {
        if !self.enabled || !self.client.config().enabled {
            return Ok("disabled".to_string());
        }

        let trace_id = uuid::Uuid::new_v4().to_string();
        let trace = Trace {
            id: trace_id.clone(),
            name: name.to_string(),
            start_time: Instant::now(),
            status: TraceStatus::Running,
            metadata: metadata.clone(),
            tags: Vec::new(),
        };

        self.active_traces.insert(trace_id.clone(), trace);

        // 发送到Langfuse
        self.client.create_trace(&trace_id, name, metadata)
            .map_err(|e| TraceError::ClientError(e.to_string()))?;

        Ok(trace_id)
    }

    /// 结束trace
    pub fn end_trace(&mut self, trace_id: &str, status: TraceStatus) -> Result<(), TraceError> {
        if !self.enabled || !self.client.config().enabled {
            return Ok(());
        }

        if let Some(mut trace) = self.active_traces.remove(trace_id) {
            trace.status = status.clone();

            // 发送更新到Langfuse
            let status_str = match status {
                TraceStatus::Running => "running",
                TraceStatus::Success => "success",
                TraceStatus::Error => "error",
                TraceStatus::Cancelled => "cancelled",
            };

            // 这里应该调用client的更新方法
            // 简化实现：直接打印
            println!("[Langfuse] Trace {} ended with status: {}", trace_id, status_str);
        }

        Ok(())
    }

    /// 开始新的span
    pub fn start_span(&mut self, trace_id: &str, name: &str, input: Option<serde_json::Value>) -> Result<String, TraceError> {
        if !self.enabled || !self.client.config().enabled {
            return Ok("disabled".to_string());
        }

        let span_id = uuid::Uuid::new_v4().to_string();
        let span = Span {
            id: span_id.clone(),
            trace_id: trace_id.to_string(),
            name: name.to_string(),
            start_time: Instant::now(),
            status: SpanStatus::Running,
            input: input.clone(),
            output: None,
            metadata: HashMap::new(),
        };

        self.active_spans.insert(span_id.clone(), span);

        // 发送到Langfuse
        let mut metadata = HashMap::new();
        if let Some(input_value) = input {
            metadata.insert("input".to_string(), input_value);
        }

        self.client.create_span(trace_id, &span_id, name, metadata)
            .map_err(|e| TraceError::ClientError(e.to_string()))?;

        Ok(span_id)
    }

    /// 结束span
    pub fn end_span(&mut self, span_id: &str, status: SpanStatus, output: Option<serde_json::Value>) -> Result<(), TraceError> {
        if !self.enabled || !self.client.config().enabled {
            return Ok(());
        }

        if let Some(mut span) = self.active_spans.remove(span_id) {
            span.status = status.clone();
            span.output = output.clone();

            // 发送更新到Langfuse
            let status_str = match status {
                SpanStatus::Running => "running",
                SpanStatus::Success => "success",
                SpanStatus::Error => "error",
                SpanStatus::Cancelled => "cancelled",
            };

            self.client.update_span(span_id, status_str, output)
                .map_err(|e| TraceError::ClientError(e.to_string()))?;
        }

        Ok(())
    }

    /// 记录LLM调用
    pub fn record_llm_call(&mut self, trace_id: &str, call: &LlmCall) -> Result<(), TraceError> {
        if !self.enabled || !self.client.config().enabled {
            return Ok(());
        }

        let converted = self.converter.convert_llm_call(call);
        let generation_id = uuid::Uuid::new_v4().to_string();

        // 清理输入数据
        let sanitized_input = self.sanitizer.sanitize_value(converted.input);
        let sanitized_output = converted.output.map(|o| self.sanitizer.sanitize_value(o));

        self.client.create_generation(
            trace_id,
            &generation_id,
            &call.model,
            sanitized_input,
            sanitized_output,
            converted.metadata,
        ).map_err(|e| TraceError::ClientError(e.to_string()))?;

        Ok(())
    }

    /// 记录工具调用
    pub fn record_tool_call(&mut self, trace_id: &str, call: &ToolCall) -> Result<(), TraceError> {
        if !self.enabled || !self.client.config().enabled {
            return Ok(());
        }

        let converted = self.converter.convert_tool_call(call);
        let span_id = uuid::Uuid::new_v4().to_string();

        // 清理输入数据
        let sanitized_input = self.sanitizer.sanitize_value(converted.input);
        let sanitized_output = converted.output.map(|o| self.sanitizer.sanitize_value(o));

        self.client.create_span(
            trace_id,
            &span_id,
            &converted.name,
            converted.metadata,
        ).map_err(|e| TraceError::ClientError(e.to_string()))?;

        // 结束span
        let status = if converted.level == "ERROR" {
            SpanStatus::Error
        } else {
            SpanStatus::Success
        };

        self.end_span(&span_id, status, sanitized_output)?;

        Ok(())
    }

    /// 记录错误
    pub fn record_error(&mut self, trace_id: &str, error: &ErrorEvent) -> Result<(), TraceError> {
        if !self.enabled || !self.client.config().enabled {
            return Ok(());
        }

        let converted = self.converter.convert_error(error);
        let span_id = uuid::Uuid::new_v4().to_string();

        self.client.create_span(
            trace_id,
            &span_id,
            &converted.name,
            converted.metadata,
        ).map_err(|e| TraceError::ClientError(e.to_string()))?;

        // 结束span（错误状态）
        self.end_span(&span_id, SpanStatus::Error, converted.output)?;

        Ok(())
    }

    /// 刷新所有待发送的数据
    pub fn flush(&mut self) -> Result<(), TraceError> {
        if !self.enabled || !self.client.config().enabled {
            return Ok(());
        }

        self.client.flush()
            .map_err(|e| TraceError::ClientError(e.to_string()))?;

        Ok(())
    }

    /// 检查是否需要刷新
    pub fn should_flush(&self) -> bool {
        self.client.should_flush()
    }

    /// 获取活跃trace数量
    pub fn active_trace_count(&self) -> usize {
        self.active_traces.len()
    }

    /// 获取活跃span数量
    pub fn active_span_count(&self) -> usize {
        self.active_spans.len()
    }

    /// 启用或禁用trace管理器
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

/// Trace错误
#[derive(Debug)]
pub enum TraceError {
    /// 客户端错误
    ClientError(String),
    /// 配置错误
    ConfigError(String),
    /// 超时错误
    TimeoutError,
}

impl std::fmt::Display for TraceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TraceError::ClientError(msg) => write!(f, "Trace client error: {}", msg),
            TraceError::ConfigError(msg) => write!(f, "Trace config error: {}", msg),
            TraceError::TimeoutError => write!(f, "Trace timeout error"),
        }
    }
}

impl std::error::Error for TraceError {}
