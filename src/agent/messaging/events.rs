use crate::types::{StarToolCall, ToolProgressStatus, ToolResult};
use serde::Serialize;

/// agent 运行期间发出的事件。
///
/// # 为什么加 `Serialize`
///
/// 这套枚举原本是**纯进程内**的 Rust 类型（`tokio::sync::mpsc` 上跑），外部进程
/// 看不到任何一个变体。headless 的 `--output-format jsonl` 也只输出最终的
/// `ChatEntry` —— 工具调用轨迹、token 用量、错误分类全都丢了。
///
/// 有了 `Serialize`，headless 的 `stream-json` 就能把事件流原样吐给外部
/// harness（CI 脚本、编排器、IDE 插件）。`type` 字段是**对外契约**：它用
/// 小写下划线的稳定名（`text_delta` / `tool_started` / …），不随 Rust 变体
/// 改名而变。
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Streaming text delta from LLM
    TextDelta(String),
    /// Streaming reasoning/thinking delta from LLM (for models with reasoning_content)
    ReasoningDelta(String),
    /// Structured trace event for transcript / harness consumers
    Trace {
        event: String,
        payload: serde_json::Value,
    },
    /// Agent decided to call a tool
    ToolStarted { tool_call: StarToolCall },
    /// Tool execution finished
    ToolFinished {
        tool_call: StarToolCall,
        result: ToolResult,
    },
    /// Tool execution emitted progress output
    ToolProgress {
        tool_name: String,
        tool_call_id: Option<String>,
        status: ToolProgressStatus,
        message: String,
        current: Option<u32>,
        total: Option<u32>,
    },
    /// Agent produced a final text message for the user
    Message(String),
    /// An error occurred
    Error(String),
    /// Execution finished for the current turn
    TurnFinished,
    /// Execution completely finished (e.g. max turns reached or task done)
    Done,
    /// Stats update (Token usage, AU2 state)
    StatsUpdate {
        token_usage: Option<crate::types::StarUsage>,
    },
}

/// 手写序列化，而不是 `#[serde(tag = "type")]`。
///
/// serde 的 internally-tagged 枚举**不支持 newtype 变体**（只接受 struct 变体），
/// 而这里一半的变体都是 newtype（`TextDelta(String)`、`Message(String)`…）。
/// 改成 struct 变体会改动所有调用点的构造语法 —— 那是内部 API 的无谓破坏。
///
/// 所以序列化时手工把 newtype 变体包成 `{"type": …, "content": …}`，
/// 对外形状与 struct 变体保持一致。
impl Serialize for AgentEvent {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        match self {
            Self::TextDelta(content) => {
                map.serialize_entry("type", "text_delta")?;
                map.serialize_entry("content", content)?;
            }
            Self::ReasoningDelta(content) => {
                map.serialize_entry("type", "reasoning_delta")?;
                map.serialize_entry("content", content)?;
            }
            Self::Trace { event, payload } => {
                map.serialize_entry("type", "trace")?;
                map.serialize_entry("event", event)?;
                map.serialize_entry("payload", payload)?;
            }
            Self::ToolStarted { tool_call } => {
                map.serialize_entry("type", "tool_started")?;
                map.serialize_entry("tool_call", tool_call)?;
            }
            Self::ToolFinished { tool_call, result } => {
                map.serialize_entry("type", "tool_finished")?;
                map.serialize_entry("tool_call", tool_call)?;
                map.serialize_entry("result", result)?;
            }
            Self::ToolProgress {
                tool_name,
                tool_call_id,
                status,
                message,
                current,
                total,
            } => {
                map.serialize_entry("type", "tool_progress")?;
                map.serialize_entry("tool_name", tool_name)?;
                map.serialize_entry("tool_call_id", tool_call_id)?;
                map.serialize_entry("status", status)?;
                map.serialize_entry("message", message)?;
                map.serialize_entry("current", current)?;
                map.serialize_entry("total", total)?;
            }
            Self::Message(content) => {
                map.serialize_entry("type", "message")?;
                map.serialize_entry("content", content)?;
            }
            Self::Error(err) => {
                map.serialize_entry("type", "error")?;
                map.serialize_entry("content", err)?;
            }
            Self::TurnFinished => {
                map.serialize_entry("type", "turn_finished")?;
            }
            Self::Done => {
                map.serialize_entry("type", "done")?;
            }
            Self::StatsUpdate { token_usage } => {
                map.serialize_entry("type", "stats_update")?;
                map.serialize_entry("token_usage", token_usage)?;
            }
        }
        map.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_tool_call() -> crate::types::StarToolCall {
        crate::types::StarToolCall {
            id: "call-1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::StarToolCallFunction {
                name: "Read".to_string(),
                arguments: "{}".to_string(),
            },
        }
    }

    /// 事件类型名是对外契约 —— 改名会让已经在用 `stream-json` 的 harness 静默失效。
    #[test]
    fn event_type_names_are_stable() {
        let cases: Vec<(AgentEvent, &str)> = vec![
            (AgentEvent::TextDelta(String::new()), "text_delta"),
            (AgentEvent::ReasoningDelta(String::new()), "reasoning_delta"),
            (
                AgentEvent::Trace {
                    event: "x".into(),
                    payload: serde_json::Value::Null,
                },
                "trace",
            ),
            (
                AgentEvent::ToolStarted {
                    tool_call: test_tool_call(),
                },
                "tool_started",
            ),
            (
                AgentEvent::ToolFinished {
                    tool_call: test_tool_call(),
                    result: ToolResult {
                        success: true,
                        output: None,
                        error: None,
                        data: None,
                        llm_content: None,
                    },
                },
                "tool_finished",
            ),
            (
                AgentEvent::ToolProgress {
                    tool_name: "t".into(),
                    tool_call_id: None,
                    status: ToolProgressStatus::Running,
                    message: String::new(),
                    current: None,
                    total: None,
                },
                "tool_progress",
            ),
            (AgentEvent::Message(String::new()), "message"),
            (AgentEvent::Error(String::new()), "error"),
            (AgentEvent::TurnFinished, "turn_finished"),
            (AgentEvent::Done, "done"),
            (
                AgentEvent::StatsUpdate { token_usage: None },
                "stats_update",
            ),
        ];

        for (event, expected) in cases {
            let json = serde_json::to_value(&event).expect("event serializes");
            assert_eq!(
                json.get("type").and_then(|v| v.as_str()),
                Some(expected),
                "事件类型名不能变"
            );
        }
    }

    #[test]
    fn events_round_trip_through_json() {
        let event = AgentEvent::Message("hello".into());
        let json = serde_json::to_string(&event).expect("serialize");
        assert_eq!(json, r#"{"type":"message","content":"hello"}"#);
    }
}
