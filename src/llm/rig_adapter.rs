//! rig-core adapter — replaces hand-rolled LLM provider code.
//! Uses rig-core's CompletionRequest API for proper message/tool support.

use crate::llm::{LlmClient, LlmError, LlmEvent, ModelInfo};
use crate::types::{StarChoice, StarMessage, StarResponse, StarTool, StarUsage};
use async_stream::stream;
use async_trait::async_trait;
use futures::Stream;
use serde_json::Value;
use std::error::Error;
use std::pin::Pin;

use rig_core::{
    completion::message::{
        AssistantContent, Message, ReasoningContent, Text, ToolCall, ToolFunction, ToolResult,
        ToolResultContent, UserContent,
    },
    completion::{self, CompletionModel, CompletionRequest},
    providers::{anthropic, deepseek, openai},
    OneOrMany,
};

// ── Constants ──────────────────────────────────────────────────────
const ROLE_SYSTEM: &str = "system";
const ROLE_USER: &str = "user";
const ROLE_ASSISTANT: &str = "assistant";
const ROLE_TOOL: &str = "tool";
const CALL_TYPE_FUNCTION: &str = "function";
const FINISH_STOP: &str = "stop";
const FINISH_TOOL_CALLS: &str = "tool_calls";
const EMPTY_JSON: &str = "{}";

/// A unified LLM client wrapping rig-core for native providers.
#[derive(Debug, Clone)]
pub enum RigAdapter {
    OpenAI {
        api_key: String,
        model: String,
        base_url: Option<String>,
    },
    Anthropic {
        api_key: String,
        model: String,
        base_url: Option<String>,
    },
    DeepSeek {
        api_key: String,
        model: String,
        base_url: Option<String>,
    },
    OpenAiCompatible {
        api_key: String,
        model: String,
        base_url: String,
        provider_name: String,
    },
}

impl RigAdapter {
    pub fn openai(api_key: String, model: String) -> Self {
        Self::OpenAI {
            api_key,
            model,
            base_url: None,
        }
    }
    pub fn openai_with_base_url(api_key: String, model: String, base_url: String) -> Self {
        Self::OpenAI {
            api_key,
            model,
            base_url: Some(base_url),
        }
    }
    pub fn anthropic(api_key: String, model: String) -> Self {
        Self::Anthropic {
            api_key,
            model,
            base_url: None,
        }
    }
    pub fn anthropic_with_base_url(api_key: String, model: String, base_url: String) -> Self {
        Self::Anthropic {
            api_key,
            model,
            base_url: Some(base_url),
        }
    }
    pub fn deepseek(api_key: String, model: String) -> Self {
        Self::DeepSeek {
            api_key,
            model,
            base_url: None,
        }
    }
    pub fn deepseek_with_base_url(api_key: String, model: String, base_url: String) -> Self {
        Self::DeepSeek {
            api_key,
            model,
            base_url: Some(base_url),
        }
    }
    pub fn openai_compatible(
        api_key: String,
        model: String,
        base_url: String,
        provider_name: String,
    ) -> Self {
        Self::OpenAiCompatible {
            api_key,
            model,
            base_url,
            provider_name,
        }
    }
}

// ── Message / Tool conversion ────────────────────────────────────
fn convert_messages(msgs: &[StarMessage]) -> Vec<Message> {
    msgs.iter().map(convert_one_message).collect()
}

fn convert_one_message(msg: &StarMessage) -> Message {
    let text = |s: &str| Text {
        text: s.to_string(),
        additional_params: None,
    };
    match msg.role.as_str() {
        ROLE_SYSTEM => Message::System {
            content: msg.content.clone().unwrap_or_default(),
        },
        ROLE_USER => Message::User {
            content: OneOrMany::one(UserContent::Text(text(
                &msg.content.clone().unwrap_or_default(),
            ))),
        },
        ROLE_ASSISTANT => {
            let mut parts: Vec<AssistantContent> = Vec::new();
            // Add reasoning if present. DeepSeek thinking mode requires the
            // `reasoning_content` field to be present (even empty) on every
            // assistant message that carries tool_calls. Dropping empty
            // reasoning causes "The reasoning_content in the thinking mode
            // must be passed back to the API" (400).
            if msg.reasoning_content.is_some() {
                let text = msg.reasoning_content.as_deref().unwrap_or("");
                parts.push(AssistantContent::Reasoning(
                    rig_core::completion::message::Reasoning::new(text),
                ));
            }
            if let Some(tool_calls) = &msg.tool_calls {
                for tc in tool_calls {
                    let args = serde_json::from_str(&tc.function.arguments)
                        .unwrap_or(Value::String(tc.function.arguments.clone()));
                    parts.push(AssistantContent::ToolCall(ToolCall::new(
                        tc.id.clone(),
                        ToolFunction::new(tc.function.name.clone(), args),
                    )));
                }
            }
            if let Some(ref content) = msg.content {
                if !content.is_empty() {
                    parts.push(AssistantContent::Text(text(content)));
                }
            }
            if parts.is_empty() {
                parts.push(AssistantContent::Text(text("")));
            }
            Message::Assistant {
                id: None,
                content: OneOrMany::many(parts)
                    .unwrap_or(OneOrMany::one(AssistantContent::Text(text("")))),
            }
        }
        ROLE_TOOL => Message::User {
            content: OneOrMany::one(UserContent::ToolResult(ToolResult {
                id: msg.tool_call_id.clone().unwrap_or_default(),
                call_id: None,
                content: OneOrMany::one(ToolResultContent::Text(text(
                    &msg.content.clone().unwrap_or_default(),
                ))),
            })),
        },
        _ => Message::User {
            content: OneOrMany::one(UserContent::Text(text(
                &msg.content.clone().unwrap_or_default(),
            ))),
        },
    }
}

fn convert_tools(tools: &Option<Vec<StarTool>>) -> Vec<completion::request::ToolDefinition> {
    let list = match tools {
        Some(v) => v.clone(),
        None => return vec![],
    };
    if list.is_empty() {
        return vec![];
    }
    list.iter()
        .map(|t| completion::request::ToolDefinition {
            name: t.function.name.clone(),
            description: t.function.description.clone(),
            parameters: serde_json::to_value(&t.function.parameters).unwrap_or_default(),
        })
        .collect()
}

fn build_request(
    messages: &[StarMessage],
    tools: &Option<Vec<StarTool>>,
    profile: crate::llm::thinking::ProviderProfile<'_>,
) -> CompletionRequest {
    let mut extra_params = serde_json::Map::new();

    // 这里**故意不再**塞 `cache_control`。
    //
    // 原来的代码把所有带 `cache_control` 的 system 消息收集成一个数组塞进
    // `additional_params["cache_control"]`。rig 的 Anthropic provider 会把这个键
    // 取出来做 `serde_json::from_value::<CacheControl>()` —— 而 `CacheControl` 是
    // `#[serde(tag = "type")]` 的内部标签枚举，只认对象 `{"type":"ephemeral"}`，
    // 数组直接反序列化失败，于是**每一次 Anthropic 请求都在发出之前就报
    // `Invalid Anthropic additional_params.cache_control payload`**。
    //
    // 缓存断点现在交给 rig 自己打（见 `rig_anthropic_complete` /
    // `rig_anthropic_stream` 里的 `with_prompt_caching().with_automatic_caching()`）：
    // system 与 tools 各占一个显式断点，会话历史的移动断点由 Anthropic 服务端
    // 自己往后推。`StarMessage::cache_control` 保留下来只用于 prompt builder 侧
    // 区分静态/动态段落，不再参与请求构造。

    // 思考力度 —— 档位由 UI 侧（Alt+T / `/effort` / 命令面板）经
    // `AgentRequest::SetThinkingEffort` 存进 `llm::thinking` 的会话状态，
    // 这里按 provider 方言翻成它认识的那**一个**字段：`additional_params`
    // 是 `#[serde(flatten)]`，几种字段一起发会让严格的服务端 400。
    let effort = crate::llm::thinking::current_effort();
    let thinking = crate::llm::thinking::thinking_params(&profile, &effort);
    if !thinking.is_empty() {
        crate::utils::logging::append_debug_log_line(&format!(
            "[LLM] thinking effort={} dialect={:?} params={}",
            effort.as_str(),
            thinking.dialect,
            serde_json::Value::Object(thinking.extra.clone())
        ));
    }
    for (key, value) in thinking.extra {
        extra_params.insert(key, value);
    }

    // budget 方言要求 `max_tokens > budget_tokens`，而 rig 只在请求没给
    // `max_tokens` 时才填自己的默认值（老模型上是 2048，比 budget 还小），
    // 所以这里必须显式给出；用户自己配了更大的值就听用户的。
    let max_tokens = thinking.max_tokens.map(|need| {
        std::env::var(crate::llm::ENV_STAR_MAX_TOKENS)
            .ok()
            .and_then(|raw| raw.trim().parse::<u64>().ok())
            .filter(|configured| *configured > need)
            .unwrap_or(need)
    });

    let additional_params = if extra_params.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(extra_params))
    };

    CompletionRequest {
        model: None,
        preamble: None,
        chat_history: OneOrMany::many(convert_messages(messages)).unwrap_or(OneOrMany::one(
            Message::User {
                content: OneOrMany::one(UserContent::Text(Text {
                    text: String::new(),
                    additional_params: None,
                })),
            },
        )),
        documents: vec![],
        tools: convert_tools(tools),
        temperature: None,
        max_tokens,
        tool_choice: None,
        additional_params,
        output_schema: None,
        record_telemetry_content: true,
    }
}

/// Full parsed completion result with text, tool calls, reasoning, and usage.
struct CompletionResult {
    text: String,
    reasoning: Option<String>,
    tool_calls: Vec<crate::types::StarToolCall>,
    usage: Option<StarUsage>,
}

/// Extract all content types from a rig completion response choice.
fn extract_response(contents: &OneOrMany<AssistantContent>) -> CompletionResult {
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut tool_calls: Vec<crate::types::StarToolCall> = Vec::new();

    for content in contents.iter() {
        match content {
            AssistantContent::Text(t) => {
                text.push_str(&t.text);
            }
            AssistantContent::Reasoning(r) => {
                for rc in &r.content {
                    match rc {
                        ReasoningContent::Text { text: t, .. } => reasoning.push_str(t),
                        ReasoningContent::Summary(t) => reasoning.push_str(t),
                        _ => {}
                    }
                }
            }
            AssistantContent::ToolCall(tc) => {
                let args_str = match &tc.function.arguments {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                tool_calls.push(crate::types::StarToolCall {
                    id: tc
                        .id
                        .strip_prefix("call_function_")
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| tc.id.clone()),
                    call_type: CALL_TYPE_FUNCTION.to_string(),
                    function: crate::types::StarToolCallFunction {
                        name: tc.function.name.clone(),
                        arguments: if args_str.is_empty() {
                            EMPTY_JSON.to_string()
                        } else {
                            args_str
                        },
                    },
                });
            }
            _ => {}
        }
    }

    CompletionResult {
        text,
        reasoning: if reasoning.is_empty() {
            None
        } else {
            Some(reasoning)
        },
        tool_calls,
        usage: None,
    }
}

fn build_star_response(result: CompletionResult) -> StarResponse {
    let finish_reason = if result.tool_calls.is_empty() {
        FINISH_STOP
    } else {
        FINISH_TOOL_CALLS
    };
    StarResponse {
        choices: vec![StarChoice {
            message: StarMessage {
                role: ROLE_ASSISTANT.to_string(),
                content: if result.text.is_empty() {
                    None
                } else {
                    Some(result.text)
                },
                tool_calls: if result.tool_calls.is_empty() {
                    None
                } else {
                    Some(result.tool_calls)
                },
                reasoning_content: result.reasoning,
                tool_call_id: None,
                cache_control: None,
            },
            finish_reason: finish_reason.to_string(),
        }],
        usage: result.usage,
    }
}

/// Convert rig Usage to StarUsage.
/// 把 rig 的 `Usage` 翻成 `StarUsage`，**包括缓存计数**
///
/// 之前这里是 `..Default::default()`，于是 `cache_read_tokens` /
/// `cache_creation_tokens` 永远是 0。状态栏的 "Cache N%" 只在
/// `total_cache > 0` 时才画，所以那个指示器从来没出现过 —— 看上去像"没开缓存"，
/// 实际上是"开了但没统计"。两个字段 rig 都给了，照抄即可。
fn convert_usage(usage: &rig_core::completion::Usage) -> StarUsage {
    StarUsage {
        prompt_tokens: usage.input_tokens as u32,
        completion_tokens: usage.output_tokens as u32,
        total_tokens: usage.total_tokens as u32,
        cache_read_tokens: usage.cached_input_tokens as u32,
        cache_creation_tokens: usage.cache_creation_input_tokens as u32,
    }
}

// ── Macro: completion call with optional base_url support
macro_rules! rig_complete_with_url {
    ($mod:ident, $api_key:expr, $model:expr, $base_url:expr, $request:expr) => {{
        use rig_core::client::CompletionClient;
        let mut builder = $mod::Client::builder().api_key($api_key);
        if let Some(url) = $base_url {
            builder = builder.base_url(url);
        }
        let client = builder.build().map_err(|e| {
            LlmError::ProviderError(format!("rig {} client: {e}", stringify!($mod)))
        })?;
        let model = client.completion_model($model.clone());
        model.completion($request).await.map_err(|e| {
            LlmError::ProviderError(format!("rig {} completion: {e}", stringify!($mod)))
        })
    }};
}

/// OpenAI completion using Completions API (not Responses API).
/// The Responses API requires an OpenAI-generated ID for reasoning items,
/// which we don't have for historical messages with reasoning_content.
/// The Completions API silently skips reasoning content, avoiding the error.
async fn rig_openai_complete(
    api_key: &str,
    model: &str,
    base_url: Option<&str>,
    request: CompletionRequest,
) -> Result<CompletionResult, LlmError> {
    use rig_core::client::CompletionClient;
    let mut builder = openai::CompletionsClient::builder().api_key(api_key);
    if let Some(url) = base_url {
        builder = builder.base_url(url);
    }
    let client = builder
        .build()
        .map_err(|e| LlmError::ProviderError(format!("rig openai client: {e}")))?;
    let model_client = client.completion_model(model);
    let r = model_client
        .completion(request)
        .await
        .map_err(|e| LlmError::ProviderError(format!("rig openai completion: {e}")))?;
    let mut result = extract_response(&r.choice);
    result.usage = Some(convert_usage(&r.usage));
    Ok(result)
}

// ── Macro: true streaming with optional base_url support ────────────
// Uses rig-core's `CompletionModel::stream` so reasoning/text deltas are
// forwarded incrementally to the agent layer. The rig stream is consumed on
// a spawned task and forwarded via a channel, which keeps the returned
// stream Send + Sync.
//
// 拆成两层：`rig_stream_with_url!` 负责按 provider 模块建 client + model，
// `rig_stream_from_model!` 负责驱动流。Anthropic 需要在 model 上额外挂
// `with_prompt_caching()` / `with_automatic_caching()`（这两个方法只有 Anthropic
// 的 `CompletionModel` 有），所以它自己建 model 再走下层宏。
macro_rules! rig_stream_with_url {
    ($mod:ident, $api_key:expr, $model:expr, $base_url:expr, $request:expr) => {{
        use rig_core::client::CompletionClient;

        let mut builder = $mod::Client::builder().api_key($api_key);
        if let Some(url) = $base_url {
            builder = builder.base_url(url);
        }
        let client = builder.build().map_err(|e| {
            LlmError::ProviderError(format!("rig {} client: {e}", stringify!($mod)))
        })?;
        let model = client.completion_model($model.clone());
        rig_stream_from_model!(stringify!($mod), model, $request)
    }};
}

macro_rules! rig_stream_from_model {
    ($label:expr, $model:expr, $request:expr) => {{
        use futures::StreamExt;
        use rig_core::completion::request::GetTokenUsage;
        use rig_core::streaming::StreamedAssistantContent;

        let label: &str = $label;
        let stream = $model
            .stream($request)
            .await
            .map_err(|e| LlmError::ProviderError(format!("rig {label} stream: {e}")))?;

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<LlmEvent, LlmError>>(64);
        tokio::spawn(async move {
            let mut stream = stream;
            let mut last_usage: Option<crate::types::StarUsage> = None;
            let mut has_tool_calls = false;
            let mut saved_finish_reason = FINISH_STOP.to_string();

            while let Some(item) = stream.next().await {
                match item {
                    Ok(StreamedAssistantContent::Text(t)) => {
                        if tx.send(Ok(LlmEvent::TextChunk(t.text))).await.is_err() {
                            return;
                        }
                    }
                    Ok(StreamedAssistantContent::Reasoning(r)) => {
                        for rc in r.content {
                            match rc {
                                ReasoningContent::Text { text, .. } => {
                                    if tx.send(Ok(LlmEvent::ReasoningChunk(text))).await.is_err() {
                                        return;
                                    }
                                }
                                ReasoningContent::Summary(s) => {
                                    if tx.send(Ok(LlmEvent::ReasoningChunk(s))).await.is_err() {
                                        return;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    Ok(StreamedAssistantContent::ReasoningDelta { reasoning, .. }) => {
                        if tx
                            .send(Ok(LlmEvent::ReasoningChunk(reasoning)))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    Ok(StreamedAssistantContent::ToolCall { tool_call, .. }) => {
                        has_tool_calls = true;
                        let args = match tool_call.function.arguments {
                            Value::String(s) => s,
                            other => other.to_string(),
                        };
                        if tx
                            .send(Ok(LlmEvent::ToolCall(crate::types::StarToolCall {
                                id: tool_call.id,
                                call_type: CALL_TYPE_FUNCTION.to_string(),
                                function: crate::types::StarToolCallFunction {
                                    name: tool_call.function.name,
                                    arguments: if args.is_empty() {
                                        EMPTY_JSON.to_string()
                                    } else {
                                        args
                                    },
                                },
                            })))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    // rig-core aggregates tool call deltas internally and emits
                    // the complete ToolCall above; nothing to do here.
                    Ok(StreamedAssistantContent::ToolCallDelta { .. }) => {}
                    Ok(StreamedAssistantContent::Final(res)) => {
                        let usage = res.token_usage();
                        // 缓存命中时 `input_tokens` 会很小（大头记在 cached_input_tokens
                        // 上），所以判空必须把缓存计数也算进来，否则高命中率的那一轮
                        // 会被当成"没有 usage"而丢掉。
                        if usage.input_tokens > 0
                            || usage.output_tokens > 0
                            || usage.cached_input_tokens > 0
                            || usage.cache_creation_input_tokens > 0
                        {
                            last_usage = Some(convert_usage(&usage));
                        }
                    }
                    Ok(StreamedAssistantContent::Unknown(_)) => {}
                    Err(e) => {
                        let _ = tx
                            .send(Err(LlmError::ProviderError(format!(
                                "rig {label} stream error: {e}"
                            ))))
                            .await;
                        return;
                    }
                }
            }
            if let Some(usage) = last_usage {
                if tx.send(Ok(LlmEvent::UsageUpdate(usage))).await.is_err() {
                    return;
                }
            }
            saved_finish_reason = if has_tool_calls {
                FINISH_TOOL_CALLS.to_string()
            } else {
                FINISH_STOP.to_string()
            };
            let _ = tx.send(Ok(LlmEvent::Finish(saved_finish_reason))).await;
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }};
}

/// True streaming for OpenAI / OpenAI-compatible providers via
/// rig-core's OpenAI CompletionsClient.
async fn rig_openai_stream(
    api_key: &str,
    model: &str,
    base_url: Option<&str>,
    request: CompletionRequest,
) -> Result<Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + Sync>>, LlmError> {
    use futures::StreamExt;
    use rig_core::client::CompletionClient;
    use rig_core::completion::request::GetTokenUsage;
    use rig_core::streaming::StreamedAssistantContent;

    let mut builder = openai::CompletionsClient::builder().api_key(api_key);
    if let Some(url) = base_url {
        builder = builder.base_url(url);
    }
    let client = builder
        .build()
        .map_err(|e| LlmError::ProviderError(format!("rig openai client: {e}")))?;
    let model_client = client.completion_model(model);
    let stream = model_client
        .stream(request)
        .await
        .map_err(|e| LlmError::ProviderError(format!("rig openai stream: {e}")))?;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<LlmEvent, LlmError>>(64);
    tokio::spawn(async move {
        let mut stream = stream;
        let mut last_usage: Option<crate::types::StarUsage> = None;
        let mut has_tool_calls = false;
        let mut saved_finish_reason = FINISH_STOP.to_string();

        while let Some(item) = stream.next().await {
            match item {
                Ok(StreamedAssistantContent::Text(t)) => {
                    if tx.send(Ok(LlmEvent::TextChunk(t.text))).await.is_err() {
                        return;
                    }
                }
                Ok(StreamedAssistantContent::Reasoning(r)) => {
                    for rc in r.content {
                        match rc {
                            ReasoningContent::Text { text, .. } => {
                                if tx.send(Ok(LlmEvent::ReasoningChunk(text))).await.is_err() {
                                    return;
                                }
                            }
                            ReasoningContent::Summary(s) => {
                                if tx.send(Ok(LlmEvent::ReasoningChunk(s))).await.is_err() {
                                    return;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Ok(StreamedAssistantContent::ReasoningDelta { reasoning, .. }) => {
                    if tx
                        .send(Ok(LlmEvent::ReasoningChunk(reasoning)))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                Ok(StreamedAssistantContent::ToolCall { tool_call, .. }) => {
                    has_tool_calls = true;
                    let args = match tool_call.function.arguments {
                        Value::String(s) => s,
                        other => other.to_string(),
                    };
                    if tx
                        .send(Ok(LlmEvent::ToolCall(crate::types::StarToolCall {
                            id: tool_call.id,
                            call_type: CALL_TYPE_FUNCTION.to_string(),
                            function: crate::types::StarToolCallFunction {
                                name: tool_call.function.name,
                                arguments: if args.is_empty() {
                                    EMPTY_JSON.to_string()
                                } else {
                                    args
                                },
                            },
                        })))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                Ok(StreamedAssistantContent::ToolCallDelta { .. }) => {}
                Ok(StreamedAssistantContent::Final(res)) => {
                    let usage = res.token_usage();
                    // 见 `rig_stream_with_url!` 里的同一段注释：缓存命中的那一轮
                    // `input_tokens` 很小，判空要连缓存计数一起看。
                    if usage.input_tokens > 0
                        || usage.output_tokens > 0
                        || usage.cached_input_tokens > 0
                        || usage.cache_creation_input_tokens > 0
                    {
                        last_usage = Some(convert_usage(&usage));
                    }
                }
                Ok(StreamedAssistantContent::Unknown(_)) => {}
                Err(e) => {
                    let _ = tx
                        .send(Err(LlmError::ProviderError(format!(
                            "rig openai stream error: {e}"
                        ))))
                        .await;
                    return;
                }
            }
        }
        if let Some(usage) = last_usage {
            if tx.send(Ok(LlmEvent::UsageUpdate(usage))).await.is_err() {
                return;
            }
        }
        saved_finish_reason = if has_tool_calls {
            FINISH_TOOL_CALLS.to_string()
        } else {
            FINISH_STOP.to_string()
        };
        let _ = tx.send(Ok(LlmEvent::Finish(saved_finish_reason))).await;
    });

    Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
}

/// 建一个开好 prompt cache 的 Anthropic model
///
/// # 为什么要单独一份而不走通用宏
///
/// `with_prompt_caching()` / `with_automatic_caching()` 只长在 Anthropic 的
/// `CompletionModel` 上，泛型宏（`$mod::Client`）拿不到。
///
/// # 两个开关是叠加的，不是二选一
///
/// rig 的文档写得很清楚：两个一起开时，**顶层自动断点负责会话历史那个会往后移动
/// 的缓存点，rig 仍然在预算允许时给 tools 和 system 各打一个显式断点**。
/// Anthropic 一次请求最多 4 个断点，这里用掉 3 个（顶层 1 + tools 1 + system 1），
/// 正好是 agent 循环想要的形状：
///
/// - system prompt（本项目约 4 万字符）只在首轮付全价；
/// - tool schema（几十个工具的 JSON）同样只付一次；
/// - 会话历史随着轮次增长，断点由服务端自己往后推，每轮只为**增量**付全价。
///
/// 只开 `automatic_caching` 也能省，但 system 和 tools 会跟历史挤在同一个移动断点
/// 之前，命中率会差一截。
fn anthropic_caching_model(
    model: anthropic::completion::CompletionModel,
) -> anthropic::completion::CompletionModel {
    model.with_prompt_caching().with_automatic_caching()
}

/// Anthropic 非流式补全（带 prompt cache）
async fn rig_anthropic_complete(
    api_key: &str,
    model: &str,
    base_url: Option<&str>,
    request: CompletionRequest,
) -> Result<CompletionResult, LlmError> {
    use rig_core::client::CompletionClient;

    let mut builder = anthropic::Client::builder().api_key(api_key);
    if let Some(url) = base_url {
        builder = builder.base_url(url);
    }
    let client = builder
        .build()
        .map_err(|e| LlmError::ProviderError(format!("rig anthropic client: {e}")))?;
    let model_client = anthropic_caching_model(client.completion_model(model));
    let r = model_client
        .completion(request)
        .await
        .map_err(|e| LlmError::ProviderError(format!("rig anthropic completion: {e}")))?;
    let mut result = extract_response(&r.choice);
    result.usage = Some(convert_usage(&r.usage));
    Ok(result)
}

/// Anthropic 流式补全（带 prompt cache）
async fn rig_anthropic_stream(
    api_key: &str,
    model: &str,
    base_url: Option<&str>,
    request: CompletionRequest,
) -> Result<Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + Sync>>, LlmError> {
    use rig_core::client::CompletionClient;

    let mut builder = anthropic::Client::builder().api_key(api_key);
    if let Some(url) = base_url {
        builder = builder.base_url(url);
    }
    let client = builder
        .build()
        .map_err(|e| LlmError::ProviderError(format!("rig anthropic client: {e}")))?;
    let model_client = anthropic_caching_model(client.completion_model(model));
    rig_stream_from_model!("anthropic", model_client, request)
}

#[async_trait]
impl LlmClient for RigAdapter {
    async fn chat_completion(
        &self,
        messages: Vec<StarMessage>,
        tools: Option<Vec<StarTool>>,
    ) -> Result<StarResponse, Box<dyn Error + Send + Sync>> {
        let request = build_request(&messages, &tools, self.thinking_profile());
        let result = self.do_completion(request).await?;
        Ok(build_star_response(result))
    }

    async fn chat_stream_events(
        &self,
        messages: Vec<StarMessage>,
        tools: Option<Vec<StarTool>>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + Sync>>, LlmError>
    {
        let request = build_request(&messages, &tools, self.thinking_profile());
        self.do_stream(request).await
    }

    fn get_model_info(&self) -> Option<ModelInfo> {
        None
    }
}

impl RigAdapter {
    /// 判断思考参数该用哪种方言所需的 provider 画像
    /// （见 `crate::llm::thinking`）。
    fn thinking_profile(&self) -> crate::llm::thinking::ProviderProfile<'_> {
        use crate::llm::thinking::{ProviderKind, ProviderProfile};
        match self {
            Self::OpenAI {
                model, base_url, ..
            } => ProviderProfile {
                kind: ProviderKind::OpenAi,
                model,
                base_url: base_url.as_deref(),
                provider_name: None,
            },
            Self::Anthropic {
                model, base_url, ..
            } => ProviderProfile {
                kind: ProviderKind::Anthropic,
                model,
                base_url: base_url.as_deref(),
                provider_name: None,
            },
            Self::DeepSeek {
                model, base_url, ..
            } => ProviderProfile {
                kind: ProviderKind::DeepSeek,
                model,
                base_url: base_url.as_deref(),
                provider_name: None,
            },
            Self::OpenAiCompatible {
                model,
                base_url,
                provider_name,
                ..
            } => ProviderProfile {
                kind: ProviderKind::Compatible,
                model,
                base_url: Some(base_url),
                provider_name: Some(provider_name),
            },
        }
    }

    async fn do_completion(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResult, LlmError> {
        match self {
            // OpenAI: use CompletionsClient (Chat Completions API) instead of
            // the default Client (Responses API). The Responses API requires an
            // OpenAI-generated ID for reasoning items on historical assistant
            // messages, which we don't have. The Chat Completions API silently
            // skips reasoning content, avoiding the error.
            Self::OpenAI {
                api_key,
                model,
                base_url,
            } => rig_openai_complete(api_key, model, base_url.as_deref(), request).await,
            Self::Anthropic {
                api_key,
                model,
                base_url,
            } => rig_anthropic_complete(api_key, model, base_url.as_deref(), request).await,
            Self::DeepSeek {
                api_key,
                model,
                base_url,
            } => {
                let r =
                    rig_complete_with_url!(deepseek, api_key, model, base_url.as_deref(), request)?;
                let mut result = extract_response(&r.choice);
                result.usage = Some(convert_usage(&r.usage));
                Ok(result)
            }
            // Generic OpenAI Compatible: use rig-core's OpenAI CompletionsClient
            // with custom base_url. This leverages rig-core's proper reasoning
            // content handling (PR #1999, #2112).
            Self::OpenAiCompatible {
                api_key,
                model,
                base_url,
                provider_name: _,
            } => rig_openai_complete(api_key, model, Some(base_url.as_str()), request).await,
        }
    }

    /// True streaming via rig-core's `CompletionModel::stream`. Reasoning and
    /// text deltas are forwarded incrementally so the agent layer sees
    /// thinking content arrive in real time (no fake streaming).
    async fn do_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + Sync>>, LlmError>
    {
        match self {
            Self::OpenAI {
                api_key,
                model,
                base_url,
            } => rig_openai_stream(api_key, model, base_url.as_deref(), request).await,
            Self::OpenAiCompatible {
                api_key,
                model,
                base_url,
                ..
            } => rig_openai_stream(api_key, model, Some(base_url.as_str()), request).await,
            Self::Anthropic {
                api_key,
                model,
                base_url,
            } => rig_anthropic_stream(api_key, model, base_url.as_deref(), request).await,
            Self::DeepSeek {
                api_key,
                model,
                base_url,
            } => {
                rig_stream_with_url!(deepseek, api_key, model, base_url.as_deref(), request)
            }
        }
    }
}
