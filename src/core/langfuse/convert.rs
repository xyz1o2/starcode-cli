use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 事件转换器
///
/// 将内部事件转换为Langfuse格式
pub struct EventConverter {
    /// 是否包含详细信息
    include_details: bool,
    /// 最大输入长度
    max_input_length: usize,
    /// 最大输出长度
    max_output_length: usize,
}

impl EventConverter {
    /// 创建新的事件转换器
    pub fn new() -> Self {
        Self {
            include_details: true,
            max_input_length: 10000,
            max_output_length: 10000,
        }
    }

    /// 设置是否包含详细信息
    pub fn with_include_details(mut self, include: bool) -> Self {
        self.include_details = include;
        self
    }

    /// 设置最大输入长度
    pub fn with_max_input_length(mut self, length: usize) -> Self {
        self.max_input_length = length;
        self
    }

    /// 设置最大输出长度
    pub fn with_max_output_length(mut self, length: usize) -> Self {
        self.max_output_length = length;
        self
    }

    /// 转换LLM调用事件
    pub fn convert_llm_call(&self, call: &LlmCall) -> ConvertedEvent {
        let mut metadata = HashMap::new();
        metadata.insert(
            "model".to_string(),
            serde_json::Value::String(call.model.clone()),
        );
        metadata.insert(
            "provider".to_string(),
            serde_json::Value::String(call.provider.clone()),
        );

        if let Some(tokens) = call.total_tokens {
            metadata.insert(
                "total_tokens".to_string(),
                serde_json::Value::Number(tokens.into()),
            );
        }

        if let Some(duration) = call.duration_ms {
            metadata.insert(
                "duration_ms".to_string(),
                serde_json::Value::Number(duration.into()),
            );
        }

        let input = if self.include_details {
            serde_json::json!({
                "messages": call.messages,
                "tools": call.tools,
                "parameters": call.parameters
            })
        } else {
            serde_json::json!({
                "message_count": call.messages.len(),
                "tool_count": call.tools.len()
            })
        };

        let output = call.response.as_ref().map(|r| {
            serde_json::json!({
                "content": r.content,
                "usage": r.usage,
                "model": r.model
            })
        });

        ConvertedEvent {
            event_type: "generation".to_string(),
            name: format!("llm_call_{}", call.model),
            input: self.truncate_json(input, self.max_input_length),
            output: output.map(|o| self.truncate_json(o, self.max_output_length)),
            metadata,
            level: "DEFAULT".to_string(),
            status_message: None,
        }
    }

    /// 转换工具调用事件
    pub fn convert_tool_call(&self, call: &ToolCall) -> ConvertedEvent {
        let mut metadata = HashMap::new();
        metadata.insert(
            "tool_name".to_string(),
            serde_json::Value::String(call.tool_name.clone()),
        );
        metadata.insert(
            "arguments".to_string(),
            serde_json::Value::String(call.arguments.clone()),
        );

        if let Some(duration) = call.duration_ms {
            metadata.insert(
                "duration_ms".to_string(),
                serde_json::Value::Number(duration.into()),
            );
        }

        let input = serde_json::json!({
            "tool": call.tool_name,
            "arguments": call.arguments
        });

        let output = call.result.as_ref().map(|r| {
            serde_json::json!({
                "success": r.success,
                "output": r.output,
                "error": r.error
            })
        });

        let level = if call.result.as_ref().map_or(false, |r| !r.success) {
            "ERROR"
        } else {
            "DEFAULT"
        };

        ConvertedEvent {
            event_type: "span".to_string(),
            name: format!("tool_{}", call.tool_name),
            input,
            output,
            metadata,
            level: level.to_string(),
            status_message: call.result.as_ref().and_then(|r| r.error.clone()),
        }
    }

    /// 转换错误事件
    pub fn convert_error(&self, error: &ErrorEvent) -> ConvertedEvent {
        let mut metadata = HashMap::new();
        metadata.insert(
            "error_type".to_string(),
            serde_json::Value::String(error.error_type.clone()),
        );
        metadata.insert(
            "message".to_string(),
            serde_json::Value::String(error.message.clone()),
        );

        if let Some(stack_trace) = &error.stack_trace {
            metadata.insert(
                "stack_trace".to_string(),
                serde_json::Value::String(stack_trace.clone()),
            );
        }

        ConvertedEvent {
            event_type: "span".to_string(),
            name: "error".to_string(),
            input: serde_json::json!({}),
            output: Some(serde_json::json!({
                "error": error.message,
                "type": error.error_type
            })),
            metadata,
            level: "ERROR".to_string(),
            status_message: Some(error.message.clone()),
        }
    }

    /// 截断JSON字符串
    fn truncate_json(&self, value: serde_json::Value, max_length: usize) -> serde_json::Value {
        let string_value = serde_json::to_string(&value).unwrap_or_default();

        if string_value.len() <= max_length {
            value
        } else {
            let truncated = &string_value[..max_length];
            serde_json::Value::String(format!("{}...", truncated))
        }
    }
}

/// 转换后的事件
#[derive(Debug, Clone)]
pub struct ConvertedEvent {
    pub event_type: String,
    pub name: String,
    pub input: serde_json::Value,
    pub output: Option<serde_json::Value>,
    pub metadata: HashMap<String, serde_json::Value>,
    pub level: String,
    pub status_message: Option<String>,
}

/// LLM调用
#[derive(Debug, Clone)]
pub struct LlmCall {
    pub model: String,
    pub provider: String,
    pub messages: Vec<serde_json::Value>,
    pub tools: Vec<serde_json::Value>,
    pub parameters: serde_json::Value,
    pub response: Option<LlmResponse>,
    pub total_tokens: Option<u32>,
    pub duration_ms: Option<u64>,
}

/// LLM响应
#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: String,
    pub usage: serde_json::Value,
    pub model: String,
}

/// 工具调用
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub tool_name: String,
    pub arguments: String,
    pub result: Option<ToolResult>,
    pub duration_ms: Option<u64>,
}

/// 工具结果
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<String>,
}

/// 错误事件
#[derive(Debug, Clone)]
pub struct ErrorEvent {
    pub error_type: String,
    pub message: String,
    pub stack_trace: Option<String>,
}
