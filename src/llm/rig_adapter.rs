//! rig-core adapter — replaces hand-rolled LLM provider code.
//! Uses rig-core 0.46's wire architecture (`Model<Wire, Transport>`)
//! for proper message/tool support across OpenAI-compatible, Anthropic
//! and DeepSeek endpoints.
//!
//! # rig 0.46 迁移说明（2026-11）
//!
//! 0.42 的 `CompletionsClient` / `CompletionClient::completion_model` /
//! `StreamedAssistantContent` 在 0.46 里全部消失，换成 wire 架构：
//!
//! - **wire**（`Chat` / `Messages`）：纯数据端点，`encode` 请求、`decoder`
//!   解回复。`OpenAIConfig::chat(model)` / `AnthropicConfig::completion(model)`
//!   构造。
//! - **client**（`openai::OpenAI` / `anthropic::Anthropic`）：配置 + transport，
//!   `.chat(...)` / `.completion(...)` 产出 `Model<Wire>`。
//! - **transport**：`HttpClientExt` 的任意实现经 `Model::new(wire, transport)`
//!   装上。项目自己的 `build_rig_http_client()`（统一超时/代理）包装成
//!   `rig_reqwest::ReqwestClient` 后注入，因此 0.42 时代的自定义 HTTP client
//!   配置**完整保留**。
//! - **流式**：`Model::stream` 返回 `Streamed<Completion>`，事件驱动
//!   （`Item<StreamEvent>`：`Start` / `Text` / `Reasoning` / `Arguments` /
//!   `End`），流末尾 `finish()` 折叠成 `CompletionResponse`（usage 在里面）。
//! - **usage 解码**：0.42 的强类型 `Usage`（缺 `prompt_tokens` 字段即整帧
//!   `Corrupt` → `JsonError` 杀整条流）在 0.46 全部改成
//!   `Option<u64>` + `#[serde(default)]`，OpenAI 线格式走 lenient
//!   `normalized_usage`（"so no usage shape fails a reply"）。这条迁移
//!   正是本次升级的直接动机。
//!
//! # Anthropic prompt cache
//!
//! 0.42 的 `CompletionModel::with_prompt_caching()/.with_automatic_caching()`
//! 变成 wire 上的字段开关：`Messages::with_prompt_caching()`（0.46 把
//! automatic caching 并进了同一个开关 + 请求的 `GenerationOptions::cache`）。
//! `Model.wire` 是 pub 字段，构建后直接改写。

use crate::llm::{LlmClient, LlmError, LlmEvent, ModelInfo};
use crate::types::{StarChoice, StarMessage, StarResponse, StarTool, StarUsage};
use async_stream::stream;
use async_trait::async_trait;
use futures::Stream;
use serde_json::Value;
use std::error::Error;
use std::pin::Pin;
use std::sync::Arc;

use rig_core::completion::request::{CompletionRequest, ToolDefinition};
use rig_core::message::{
    AssistantContent, CallId, Message, Reasoning, Text, ToolCall, ToolFunction, ToolName,
    ToolResult, ToolResultContent, UserContent,
};
use rig_core::providers::anthropic;
use rig_core::providers::openai;
use rig_core::streaming::{Item, StreamEvent};

// ── Constants ──────────────────────────────────────────────────────
const ROLE_SYSTEM: &str = "system";
const ROLE_USER: &str = "user";
const ROLE_ASSISTANT: &str = "assistant";
const ROLE_TOOL: &str = "tool";
const CALL_TYPE_FUNCTION: &str = "function";
const FINISH_STOP: &str = "stop";
const FINISH_TOOL_CALLS: &str = "tool_calls";
const EMPTY_JSON: &str = "{}";

/// Anthropic 协议要求显式 `max_tokens`，rig 不代填时的兜底值。
/// 只在 `/models` 与 `STAR_MAX_TOKENS` 都没给值时才用到。
const DEFAULT_ANTHROPIC_MAX_TOKENS: u64 = 8192;

/// A unified LLM client wrapping rig-core for native providers.
///
/// `capability_provider_id` 是 provider store 里的 id（`local` /
/// `opencode`…）。`/models` 声明的能力（max_output_tokens、reasoning
/// 档位）按 provider + model 入库，请求构造时凭这个键查回声明值；缺省
/// None 时落回 env / 硬编码兜底。
#[derive(Debug, Clone)]
pub enum RigAdapter {
    OpenAI {
        api_key: String,
        model: String,
        base_url: Option<String>,
        capability_provider_id: Option<String>,
    },
    Anthropic {
        api_key: String,
        model: String,
        base_url: Option<String>,
        capability_provider_id: Option<String>,
    },
    DeepSeek {
        api_key: String,
        model: String,
        base_url: Option<String>,
        capability_provider_id: Option<String>,
    },
    OpenAiCompatible {
        api_key: String,
        model: String,
        base_url: String,
        provider_name: String,
        capability_provider_id: Option<String>,
    },
}

impl RigAdapter {
    pub fn openai(api_key: String, model: String) -> Self {
        Self::OpenAI {
            api_key,
            model,
            base_url: None,
            capability_provider_id: None,
        }
    }
    pub fn openai_with_base_url(api_key: String, model: String, base_url: String) -> Self {
        Self::OpenAI {
            api_key,
            model,
            base_url: Some(base_url),
            capability_provider_id: None,
        }
    }
    pub fn anthropic(api_key: String, model: String) -> Self {
        Self::Anthropic {
            api_key,
            model,
            base_url: None,
            capability_provider_id: None,
        }
    }
    pub fn anthropic_with_base_url(api_key: String, model: String, base_url: String) -> Self {
        Self::Anthropic {
            api_key,
            model,
            base_url: Some(base_url),
            capability_provider_id: None,
        }
    }
    pub fn deepseek(api_key: String, model: String) -> Self {
        Self::DeepSeek {
            api_key,
            model,
            base_url: None,
            capability_provider_id: None,
        }
    }
    pub fn deepseek_with_base_url(api_key: String, model: String, base_url: String) -> Self {
        Self::DeepSeek {
            api_key,
            model,
            base_url: Some(base_url),
            capability_provider_id: None,
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
            capability_provider_id: None,
        }
    }
    /// 装上能力缓存的 provider 键。所有变体共用，便于 `create_client`
    /// 在提供 `config.provider_id` 时统一补挂。
    pub fn with_provider_id(mut self, provider_id: Option<String>) -> Self {
        match &mut self {
            Self::OpenAI {
                capability_provider_id,
                ..
            }
            | Self::Anthropic {
                capability_provider_id,
                ..
            }
            | Self::DeepSeek {
                capability_provider_id,
                ..
            }
            | Self::OpenAiCompatible {
                capability_provider_id,
                ..
            } => {
                *capability_provider_id = provider_id;
            }
        }
        self
    }

    /// 项目的 HTTP 客户端（超时/代理统一配置）包成 rig 0.46 的
    /// `DynHttpClient` transport。所有 provider 共用这一份。
    fn transport() -> rig_core::http_client::DynHttpClient {
        rig_core::http_client::DynHttpClient::new(rig_reqwest::ReqwestClient::from(
            crate::llm::build_rig_http_client(),
        ))
    }
}

// ── Message / Tool conversion ────────────────────────────────────
fn convert_messages(msgs: &[StarMessage]) -> Vec<Message> {
    msgs.iter().map(convert_one_message).collect()
}

fn convert_one_message(msg: &StarMessage) -> Message {
    match msg.role.as_str() {
        ROLE_SYSTEM => Message::System {
            content: msg.content.clone().unwrap_or_default(),
        },
        ROLE_USER => Message::User {
            content: vec![UserContent::Text(Text::new(
                msg.content.clone().unwrap_or_default(),
            ))],
        },
        ROLE_ASSISTANT => {
            let mut parts: Vec<AssistantContent> = Vec::new();
            // DeepSeek thinking mode requires the `reasoning_content` field to
            // be present (even empty) on every assistant message that carries
            // tool_calls. Dropping empty reasoning causes "The
            // reasoning_content in the thinking mode must be passed back to
            // the API" (400).
            if msg.reasoning_content.is_some() {
                let text = msg.reasoning_content.as_deref().unwrap_or("");
                parts.push(AssistantContent::Reasoning(Reasoning::new(text)));
            }
            if let Some(tool_calls) = &msg.tool_calls {
                for tc in tool_calls {
                    // 0.46 的 ToolFunction::parse 是官方入口：对象原样保留，
                    // 字符串形式的对象再解一层，截断/非对象参数存进
                    // invalid_arguments 保留原文。手搓 from_str::<Map> 会丢掉
                    // 截断对象里已经写出的字段。
                    let name = ToolName::new(tc.function.name.clone())
                        .unwrap_or_else(|_| ToolName::new("unknown_tool").expect("non-empty"));
                    let function = ToolFunction::parse(name, &tc.function.arguments);
                    let call_id = CallId::from_wire(tc.id.clone());
                    parts.push(AssistantContent::ToolCall(ToolCall::new(call_id, function)));
                }
            }
            if let Some(ref content) = msg.content {
                if !content.is_empty() {
                    parts.push(AssistantContent::Text(Text::new(content)));
                }
            }
            if parts.is_empty() {
                parts.push(AssistantContent::Text(Text::new("")));
            }
            Message::Assistant(rig_core::message::AssistantMessage::new(parts))
        }
        ROLE_TOOL => {
            // 0.46 的 ToolResult 要求工具名（provider replay 需要独立于 call
            // id）。历史消息里只有 tool_call_id，工具名不存在 —— 但 OpenAI
            // 线格式只发 `tool_call_id` + `content`（chat.rs 的
            // `tool_message` 不发 name），所以空名只影响Anthropic replay。
            // 用占位名过类型检查，不影响 OpenAI 路径。
            let tool_name = ToolName::new("tool").expect("non-empty");
            Message::User {
                content: vec![UserContent::ToolResult(ToolResult {
                    call: CallId::from_wire(msg.tool_call_id.clone().unwrap_or_default()),
                    name: tool_name,
                    content: vec![ToolResultContent::Text(Text::new(
                        msg.content.clone().unwrap_or_default(),
                    ))],
                    is_error: false,
                })],
            }
        }
        _ => Message::User {
            content: vec![UserContent::Text(Text::new(
                msg.content.clone().unwrap_or_default(),
            ))],
        },
    }
}

fn convert_tools(tools: &Option<Vec<StarTool>>) -> Vec<Arc<ToolDefinition>> {
    let list = match tools {
        Some(v) => v.clone(),
        None => return vec![],
    };
    if list.is_empty() {
        return vec![];
    }
    list.iter()
        .filter_map(|t| {
            Some(Arc::new(ToolDefinition {
                name: ToolName::new(t.function.name.clone()).ok()?,
                description: t.function.description.clone(),
                parameters: serde_json::to_value(&t.function.parameters).unwrap_or_default(),
                annotations: Default::default(),
            }))
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
    // 断点由两条路径各自负责，都不经过这里：
    //
    // - **Anthropic**：rig 自己打（`Messages::with_prompt_caching()`，0.46 把
    //   automatic caching 并进了这个开关 + `GenerationOptions::cache`），
    //   system 与 tools 各占一个显式断点，会话历史的移动断点由
    //   Anthropic 服务端自己往后推。
    // - **OpenAI 兼容**：`prompt_cache_ext::PatchTransport` 在 transport 层对
    //   已编码的 body 打点（tools 末项 + system 末块）。之所以不能走
    //   `additional_params`：0.46 的 raw_layer 把它 `deep_merge` 到顶层，
    //   往里塞 `messages` 会整体覆盖整个消息数组。`cache_points` 字段
    //   Chat wire 也不消费（只有 Anthropic Messages 和 OpenAI Responses 实现了）。
    //
    // `StarMessage::cache_control` 保留下来只用于 prompt builder 侧区分静态/动态
    // 段落；它由上面的两条路径各自消费，不参与请求构造。

    // 思考力度 —— 档位由 UI 侧（Alt+T / `/effort` / 命令面板）经
    // `AgentRequest::SetThinkingEffort` 存进 `llm::thinking` 的会话状态，
    // 这里按 provider 方言翻成它认识的那**一个**字段：`additional_params`
    // 是 deep-merge 到顶层的，几种字段一起发会让严格的服务端 400。
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

    // Anthropic 协议要求请求必须带 `max_tokens`，所以即使不在 budget 方言下也要
    // 显式给出。budget 方言要求 `max_tokens > budget_tokens`，而 rig 只在请求没给
    // `max_tokens` 时才按模型档案填默认值，所以同样必须显式给出；用户自己配了更大
    // 的值就听用户的。
    //
    // 兜底阶梯：`/models` 声明的模型输出上限 > `STAR_MAX_TOKENS` > 硬编码兜底。
    // 曾经只有硬编码 8192：`/models` 明明报了 max_output_tokens（实测 glowp
    // 网关 131072），请求却按 8192 发 —— 长回答被无声截断，模型永远到不了
    // "finish_reason=length" 的自然边界。
    let env_max_tokens = std::env::var(crate::llm::ENV_STAR_MAX_TOKENS)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok());
    let declared_max_tokens = crate::agent::model_catalog::get_cached_max_output_tokens(
        profile.provider_id,
        profile.model,
    )
    .map(|tokens| tokens as u64);
    let max_tokens = match thinking.max_tokens {
        Some(need) => {
            // budget 方言：必须 > budget_tokens，用户配置和声明值里挑能用的。
            let raise = env_max_tokens
                .filter(|configured| *configured > need)
                .or(declared_max_tokens.filter(|declared| *declared > need));
            Some(raise.unwrap_or(need))
        }
        None => {
            if matches!(profile.kind, crate::llm::thinking::ProviderKind::Anthropic) {
                Some(
                    env_max_tokens
                        .or(declared_max_tokens)
                        .unwrap_or(DEFAULT_ANTHROPIC_MAX_TOKENS),
                )
            } else {
                declared_max_tokens.map(Some).unwrap_or(None)
            }
        }
    };

    let additional_params = if extra_params.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(extra_params))
    };

    let mut history = convert_messages(messages);
    let mut request = CompletionRequest::new(if history.is_empty() {
        Message::User {
            content: vec![UserContent::Text(Text::new(String::new()))],
        }
    } else {
        // `new` 把 prompt 放最后一条；这里整个历史就是 prompt，
        // 之后用 `messages` 语义不对 —— 直接填 chat_history 字段。
        history.remove(0)
    });
    for message in history {
        request.chat_history.push(Arc::new(message));
    }
    request.tools = convert_tools(tools);
    request.max_tokens = max_tokens;
    request.additional_params = additional_params;
    request.record_telemetry_content = true;
    request
}

/// Full parsed completion result with text, tool calls, reasoning, and usage.
struct CompletionResult {
    text: String,
    reasoning: Option<String>,
    tool_calls: Vec<crate::types::StarToolCall>,
    usage: Option<StarUsage>,
}

/// Extract all content types from a rig completion response choice.
fn extract_response(contents: &[AssistantContent]) -> CompletionResult {
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut tool_calls: Vec<crate::types::StarToolCall> = Vec::new();

    for content in contents.iter() {
        match content {
            AssistantContent::Text(t) => {
                text.push_str(&t.text);
            }
            AssistantContent::Reasoning(r) => {
                reasoning.push_str(&r.text);
            }
            AssistantContent::ToolCall(tc) => {
                // 0.46：arguments 是 serde_json::Map（已解析），裸字符串留在
                // invalid_arguments。序列化成 JSON 字符串交给上层（工具执行器
                // 仍按 JSON 字符串解析）。
                let args_str = if let Some(raw) = &tc.function.invalid_arguments {
                    raw.clone()
                } else {
                    Value::Object(tc.function.arguments.clone()).to_string()
                };
                tool_calls.push(crate::types::StarToolCall {
                    id: tc.id.wire().to_string(),
                    call_type: CALL_TYPE_FUNCTION.to_string(),
                    function: crate::types::StarToolCallFunction {
                        name: tc.function.name.to_string(),
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
///
/// 0.46 的 Usage 全是 `Option<u64>`：缺字段就是 None，与显式 0 无法区分。
/// 缓存遥测只在服务端真的报了非零值时标记 provenance。
fn convert_usage(usage: &rig_core::completion::request::Usage) -> StarUsage {
    let u64_or_0 = |v: Option<u64>| v.unwrap_or(0);
    let cached = u64_or_0(usage.cached_input_tokens);
    let cache_creation = u64_or_0(usage.cache_creation_input_tokens);
    StarUsage {
        prompt_tokens: u64_or_0(usage.input_tokens) as u32,
        completion_tokens: u64_or_0(usage.output_tokens) as u32,
        total_tokens: u64_or_0(usage.total_tokens) as u32,
        cache_read_tokens: cached as u32,
        cache_creation_tokens: cache_creation as u32,
        cache_telemetry_reported: cached > 0 || cache_creation > 0,
    }
}

/// 缓存命中时 `input_tokens` 会很小（大头记在 cached_input_tokens 上），所以
/// 判「服务端没报 usage」必须把缓存计数也算进来，否则高命中率的那一轮会被
/// 当成"没有 usage"而丢掉。
fn usage_reported(usage: &rig_core::completion::request::Usage) -> bool {
    usage.is_reported()
}

/// 驱动一条 rig 0.46 流并转成我们的 LlmEvent 流。
///
/// 0.46 的流是事件驱动的：`Start` 开 part，`Text` / `Reasoning` /
/// `Arguments` 增量，`End` 带定稿内容（完整 ToolCall 在这里出来）。
/// usage 不再是流内事件 —— `Streamed::finish()` 折叠出的
/// `CompletionResponse` 携带它，所以在流耗尽后发送 `UsageUpdate`。
async fn drive_stream(
    label: &'static str,
    mut streamed: rig_core::streaming::Streamed<rig_core::operation::Completion>,
    tx: tokio::sync::mpsc::Sender<Result<LlmEvent, LlmError>>,
) {
    use futures::StreamExt;

    let mut has_tool_calls = false;
    let mut saw_usage_carrier = false;
    let mut last_usage: Option<rig_core::completion::request::Usage> = None;

    while let Some(item) = streamed.next().await {
        match item {
            // Start 开 part、Arguments 是 tool call 的参数增量：都由
            // rig 内部聚合，End 事件才携带定稿内容，这里无事可做。
            Ok(Item::Event(StreamEvent::Start { .. }))
            | Ok(Item::Event(StreamEvent::Arguments { .. })) => {}
            Ok(Item::Event(StreamEvent::Text { text, .. })) => {
                if tx.send(Ok(LlmEvent::TextChunk(text))).await.is_err() {
                    return;
                }
            }
            Ok(Item::Event(StreamEvent::Reasoning { text, .. })) => {
                if tx.send(Ok(LlmEvent::ReasoningChunk(text))).await.is_err() {
                    return;
                }
            }
            Ok(Item::Event(StreamEvent::End { content, .. })) => {
                match content {
                    AssistantContent::ToolCall(tc) => {
                        has_tool_calls = true;
                        // 0.46：arguments 是 Map（增量 JSON 已由 rig 聚合），
                        // 裸字符串（非对象）留在 invalid_arguments。
                        let args = tc.function.invalid_arguments.clone().unwrap_or_else(|| {
                            Value::Object(tc.function.arguments.clone()).to_string()
                        });
                        if tx
                            .send(Ok(LlmEvent::ToolCall(crate::types::StarToolCall {
                                id: tc.id.wire().to_string(),
                                call_type: CALL_TYPE_FUNCTION.to_string(),
                                function: crate::types::StarToolCallFunction {
                                    name: tc.function.name.to_string(),
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
                    AssistantContent::Reasoning(r) if !r.text.is_empty() => {
                        // 完整 reasoning 块已作为增量流出过；只对没发过增量的
                        // 场景补发，避免重复。
                    }
                    _ => {}
                }
            }
            Ok(Item::Unknown(_)) => {}
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

    // 流结束：折叠出 usage。0.46 的 usage 在 CompletionResponse 里，
    // 中途出错时 finish() 也返回 Err —— 那条错误已经在上面以流内 Err 形式
    // 发过了，这里不再重复。
    if let Ok(response) = streamed.finish().await {
        saw_usage_carrier = usage_reported(&response.usage);
        if saw_usage_carrier {
            last_usage = Some(response.usage.clone());
        }
    }
    if let Some(usage) = last_usage {
        if tx
            .send(Ok(LlmEvent::UsageUpdate(convert_usage(&usage))))
            .await
            .is_err()
        {
            return;
        }
    }
    let finish = if has_tool_calls {
        FINISH_TOOL_CALLS
    } else {
        FINISH_STOP
    };
    let _ = tx.send(Ok(LlmEvent::Finish(finish.to_string()))).await;
}

/// 起一条流：`Model::stream` 拿到 `Streamed`，丢进 spawned task 驱动，
/// 返回 `ReceiverStream`（保持 Send + Sync 契约，与 0.42 相同）。
async fn start_stream<T>(
    label: &'static str,
    model: rig_core::Model<rig_core::providers::openai::wire::Chat, T>,
    request: CompletionRequest,
) -> Result<Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + Sync>>, LlmError>
where
    T: rig_core::driver::Transport<rig_core::providers::openai::wire::Chat>,
{
    let streamed = model
        .stream(request)
        .map_err(|e| LlmError::ProviderError(format!("rig {label} stream: {e}")))?;
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<LlmEvent, LlmError>>(64);
    tokio::spawn(drive_stream(label, streamed, tx));
    Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
}

/// OpenAI Chat Completions 线格式（不是 Responses API）。
///
/// The Responses API requires an OpenAI-generated ID for reasoning items,
/// which we don't have for historical messages with reasoning_content.
/// The Completions API silently skips reasoning content, avoiding the error.
/// 非流式 OpenAI 兼容补全。
///
/// `prompt_cache` 由调用方按 provider 类型给出:`Self::OpenAI` 恒传 `false`
/// （官方 `api.openai.com` 严格校验 content part，会拒绝规范外的 `cache_control`），
/// 只有 `Self::OpenAiCompatible` 才可能打开。
async fn rig_openai_complete(
    api_key: &str,
    model: &str,
    base_url: Option<&str>,
    request: CompletionRequest,
    prompt_cache: bool,
) -> Result<CompletionResult, LlmError> {
    let mut config = openai_config(api_key);
    if let Some(url) = base_url {
        config.base_url = url.to_string();
    }
    let chat = openai::wire::Chat::new(config, model);
    let response = if prompt_cache {
        // 断点注入 transport：在 rig 编码之后、发送之前给 body 打点。
        let transport = crate::llm::prompt_cache_ext::patched_transport(
            crate::llm::build_rig_http_client(),
            true,
        );
        let client = rig_core::Model::new(chat, transport);
        client
            .call(request)
            .await
            .map_err(|e| LlmError::ProviderError(format!("rig openai completion: {e}")))?
    } else {
        let client = rig_core::Model::new(chat, RigAdapter::transport());
        client
            .call(request)
            .await
            .map_err(|e| LlmError::ProviderError(format!("rig openai completion: {e}")))?
    };
    let mut result = extract_response(&response.choice);
    result.usage = Some(convert_usage(&response.usage));
    Ok(result)
}

/// OpenAI 配置：官方 dialect（已注册，可序列化），base_url 覆盖走运行时字段。
///
/// `OpenAIConfig::new` 是官方 `OPENAI` 方言 + Bearer 认证 —— 与 0.42
/// `CompletionsClient` 的行为等价；`from_env` 会读 `OPENAI_API_KEY` /
/// `OPENAI_BASE_URL`，这里显式传 key 不走 env，避免与 `STAR_*` 语义混淆。
fn openai_config(api_key: &str) -> openai::wire::OpenAIConfig {
    openai::wire::OpenAIConfig::new(api_key)
}

/// DeepSeek 配置：DEEPSEEK 方言（`reasoning_field: "reasoning_content"` 已内置，
/// 0.42 时代我们手工回填 reasoning_content 的兼容行为由 rig 接管）。
fn deepseek_config(api_key: &str) -> openai::wire::OpenAIConfig {
    openai::wire::OpenAIConfig::with_key(&openai::wire::DEEPSEEK, api_key)
}

/// 通用 OpenAI 兼容客户端（流式）。
///
/// `prompt_cache` 的语义与 [`rig_openai_complete`] 相同：官方 OpenAI 恒为 `false`，
/// 断点只对明确支持的聚合网关打开。transport 注入点对两条路径一致，
/// 所以流式与非流式打出的断点完全一致。
async fn rig_openai_stream(
    api_key: &str,
    model: &str,
    base_url: Option<&str>,
    request: CompletionRequest,
    prompt_cache: bool,
) -> Result<Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + Sync>>, LlmError> {
    let mut config = openai_config(api_key);
    if let Some(url) = base_url {
        config.base_url = url.to_string();
    }
    let chat = openai::wire::Chat::new(config, model);
    if prompt_cache {
        let transport = crate::llm::prompt_cache_ext::patched_transport(
            crate::llm::build_rig_http_client(),
            true,
        );
        let client = rig_core::Model::new(chat, transport);
        start_stream("openai", client, request).await
    } else {
        let client = rig_core::Model::new(chat, RigAdapter::transport());
        start_stream("openai", client, request).await
    }
}

/// DeepSeek 非流式补全（复用 OpenAI Chat wire + DEEPSEEK 方言）。
async fn rig_deepseek_complete(
    api_key: &str,
    model: &str,
    base_url: Option<&str>,
    request: CompletionRequest,
) -> Result<CompletionResult, LlmError> {
    let mut config = deepseek_config(api_key);
    if let Some(url) = base_url {
        config.base_url = url.to_string();
    }
    let chat = openai::wire::Chat::new(config, model);
    let client = rig_core::Model::new(
        chat,
        rig_core::http_client::DynHttpClient::new(rig_reqwest::ReqwestClient::from(
            crate::llm::build_rig_http_client(),
        )),
    );
    let response = client
        .call(request)
        .await
        .map_err(|e| LlmError::ProviderError(format!("rig deepseek completion: {e}")))?;
    let mut result = extract_response(&response.choice);
    result.usage = Some(convert_usage(&response.usage));
    Ok(result)
}

/// DeepSeek 流式补全。
async fn rig_deepseek_stream(
    api_key: &str,
    model: &str,
    base_url: Option<&str>,
    request: CompletionRequest,
) -> Result<Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + Sync>>, LlmError> {
    let mut config = deepseek_config(api_key);
    if let Some(url) = base_url {
        config.base_url = url.to_string();
    }
    let chat = openai::wire::Chat::new(config, model);
    let client = rig_core::Model::new(
        chat,
        rig_core::http_client::DynHttpClient::new(rig_reqwest::ReqwestClient::from(
            crate::llm::build_rig_http_client(),
        )),
    );
    start_stream("deepseek", client, request).await
}

/// 建一个开好 prompt cache 的 Anthropic Messages wire
///
/// # 为什么要单独一份而不走通用路径
///
/// `with_prompt_caching()` 只长在 Anthropic 的 `Messages` wire 上。
///
/// # 缓存形状
///
/// 0.46 把 0.42 的 `with_prompt_caching()` + `with_automatic_caching()` 合并：
/// `Messages::with_prompt_caching()` 开启 system / tools / 最后一条消息的
/// 显式断点。Anthropic 一次请求最多 4 个断点，这里用掉 3 个
/// （tools 1 + system 1 + 会话末尾移动断点 1），正好是 agent 循环想要的形状：
///
/// - system prompt（本项目约 4 万字符）只在首轮付全价；
/// - tool schema（几十个工具的 JSON）同样只付一次；
/// - 会话历史随着轮次增长，断点由服务端自己往后推，每轮只为**增量**付全价。
///
/// 会话末尾的移动断点由 rig 的 manual 模式补打（`completion.rs` 里
/// `manual && top.is_none()` 分支），不需要请求方设置任何字段。
///
/// # 已知现状（2026-11-12 两轮探针实测）
///
/// magpie 网关（`anthropic-compatible` 类型）**不支持** `cache_control`：
/// 标记被原样接受但 `cache_creation_input_tokens` 恒为 0，实测命中
/// （`cache_read_input_tokens`）来自网关/上游的隐式前缀匹配，与本开关无关。
/// 断点代码本身正确（见
/// `tests::anthropic_caching_wire_marks_breakpoints_in_the_real_request`），
/// 对官方 Anthropic API 或支持标记的网关仍然有效，故保留。
fn anthropic_caching_wire(mut wire: anthropic::wire::Messages) -> anthropic::wire::Messages {
    wire.prompt_caching = true;
    wire
}

/// Anthropic 配置（ANTHROPIC 方言）。`AnthropicConfig.base_url` 是 API root
/// **不带** `/v1`；调用方给的 URL 若带 `/v1` 尾缀需剥掉（0.42 时代 rig 的
/// anthropic client 同样这样约定，行为不变）。
fn anthropic_config(api_key: &str, base_url: Option<&str>) -> anthropic::AnthropicConfig {
    let mut config = anthropic::AnthropicConfig::new(api_key);
    if let Some(url) = base_url {
        config.base_url = url
            .trim_end_matches('/')
            .trim_end_matches("/v1")
            .to_string();
    }
    config
}

/// Anthropic 非流式补全（带 prompt cache）
async fn rig_anthropic_complete(
    api_key: &str,
    model: &str,
    base_url: Option<&str>,
    request: CompletionRequest,
) -> Result<CompletionResult, LlmError> {
    let wire = anthropic_caching_wire(anthropic_config(api_key, base_url).completion(model));
    let client = rig_core::Model::new(
        wire,
        rig_core::http_client::DynHttpClient::new(rig_reqwest::ReqwestClient::from(
            crate::llm::build_rig_http_client(),
        )),
    );
    let response = client
        .call(request)
        .await
        .map_err(|e| LlmError::ProviderError(format!("rig anthropic completion: {e}")))?;
    let mut result = extract_response(&response.choice);
    result.usage = Some(convert_usage(&response.usage));
    Ok(result)
}

/// Anthropic 流式补全（带 prompt cache）
async fn rig_anthropic_stream(
    api_key: &str,
    model: &str,
    base_url: Option<&str>,
    request: CompletionRequest,
) -> Result<Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + Sync>>, LlmError> {
    let wire = anthropic_caching_wire(anthropic_config(api_key, base_url).completion(model));
    let client = rig_core::Model::new(
        wire,
        rig_core::http_client::DynHttpClient::new(rig_reqwest::ReqwestClient::from(
            crate::llm::build_rig_http_client(),
        )),
    );
    // Anthropic 的流式 wire 类型不同，单独起流（复用同一个驱动循环）。
    let streamed = client
        .stream(request)
        .map_err(|e| LlmError::ProviderError(format!("rig anthropic stream: {e}")))?;
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<LlmEvent, LlmError>>(64);
    tokio::spawn(drive_stream("anthropic", streamed, tx));
    Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
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
                model,
                base_url,
                capability_provider_id,
                ..
            } => ProviderProfile {
                kind: ProviderKind::OpenAi,
                model,
                base_url: base_url.as_deref(),
                provider_name: None,
                provider_id: capability_provider_id.as_deref(),
            },
            Self::Anthropic {
                model,
                base_url,
                capability_provider_id,
                ..
            } => ProviderProfile {
                kind: ProviderKind::Anthropic,
                model,
                base_url: base_url.as_deref(),
                provider_name: None,
                provider_id: capability_provider_id.as_deref(),
            },
            Self::DeepSeek {
                model,
                base_url,
                capability_provider_id,
                ..
            } => ProviderProfile {
                kind: ProviderKind::DeepSeek,
                model,
                base_url: base_url.as_deref(),
                provider_name: None,
                provider_id: capability_provider_id.as_deref(),
            },
            Self::OpenAiCompatible {
                model,
                base_url,
                provider_name,
                capability_provider_id,
                ..
            } => ProviderProfile {
                kind: ProviderKind::Compatible,
                model,
                base_url: Some(base_url),
                provider_name: Some(provider_name),
                provider_id: capability_provider_id.as_deref(),
            },
        }
    }

    async fn do_completion(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResult, LlmError> {
        match self {
            // OpenAI: use Chat Completions wire (not Responses API). The
            // Responses API requires an OpenAI-generated ID for reasoning
            // items on historical assistant messages, which we don't have.
            // The Chat Completions API silently skips reasoning content.
            Self::OpenAI {
                api_key,
                model,
                base_url,
                ..
            } => {
                // 官方 OpenAI 严格校验 content part，`cache_control` 是规范外字段，
                // 这里恒定关掉——它只在明确支持的聚合网关上打开。
                rig_openai_complete(api_key, model, base_url.as_deref(), request, false).await
            }
            Self::Anthropic {
                api_key,
                model,
                base_url,
                ..
            } => rig_anthropic_complete(api_key, model, base_url.as_deref(), request).await,
            Self::DeepSeek {
                api_key,
                model,
                base_url,
                ..
            } => rig_deepseek_complete(api_key, model, base_url.as_deref(), request).await,
            // Generic OpenAI Compatible: rig-core's OpenAI Chat wire with
            // custom base_url. This leverages rig-core's proper reasoning
            // content handling (PR #1999, #2112).
            Self::OpenAiCompatible {
                api_key,
                model,
                base_url,
                ..
            } => {
                rig_openai_complete(
                    api_key,
                    model,
                    Some(base_url.as_str()),
                    request,
                    crate::llm::prompt_cache_ext::enabled_from_env(),
                )
                .await
            }
        }
    }

    /// True streaming via rig-core's `Model::stream`. Reasoning and
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
                ..
            } => rig_openai_stream(api_key, model, base_url.as_deref(), request, false).await,
            Self::OpenAiCompatible {
                api_key,
                model,
                base_url,
                ..
            } => {
                rig_openai_stream(
                    api_key,
                    model,
                    Some(base_url.as_str()),
                    request,
                    crate::llm::prompt_cache_ext::enabled_from_env(),
                )
                .await
            }
            Self::Anthropic {
                api_key,
                model,
                base_url,
                ..
            } => rig_anthropic_stream(api_key, model, base_url.as_deref(), request).await,
            Self::DeepSeek {
                api_key,
                model,
                base_url,
                ..
            } => rig_deepseek_stream(api_key, model, base_url.as_deref(), request).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig_core::providers::anthropic;
    use rig_core::wire::Wire as _;

    /// 端到端锁死 Anthropic 缓存断点：`anthropic_caching_wire` 开关 + rig 的
    /// 标记逻辑，必须把 `cache_control` 真落进**发上线的请求体**里——
    /// system 末块、tools 末项、最后一条 user 消息各一个。2026-11-12 排查
    /// magpie 冒烟 `cache_creation=0` 时补上：wire 开关是通的，缺的是
    /// 请求里的验证。
    #[test]
    fn anthropic_caching_wire_marks_breakpoints_in_the_real_request() {
        let mut request = CompletionRequest::new(Message::user("hello"));
        request
            .chat_history
            .insert(0, Arc::new(Message::system("You are a coding agent.")));
        request.tools = vec![Arc::new(ToolDefinition {
            name: ToolName::new("Read").expect("non-empty"),
            description: "read a file".to_string(),
            parameters: serde_json::json!({"type": "object"}),
            annotations: Default::default(),
        })];
        request.max_tokens = Some(64);

        let config = anthropic::AnthropicConfig::new("test-key");
        let wire = anthropic_caching_wire(config.completion("some-model"));
        let encoded = wire
            .encode(request, rig_core::wire::Mode::Unary)
            .expect("anthropic wire request should build");

        let bytes = match encoded.request.body() {
            rig_core::wire::Body::Bytes(bytes) => bytes.to_vec(),
            other => panic!("anthropic body should be bytes, got {other:?}"),
        };
        let body: serde_json::Value =
            serde_json::from_slice(&bytes).expect("anthropic request should serialize");

        // system 末块有断点。
        let system = body
            .get("system")
            .and_then(|v| v.as_array())
            .expect("system should be an array of blocks");
        let system_marked = system.last().and_then(|b| b.get("cache_control")).is_some();
        assert!(system_marked, "system 末块必须带 cache_control: {body}");

        // tools 末项有断点。
        let tools_marked = body["tools"]
            .as_array()
            .and_then(|t| t.last())
            .and_then(|t| t.get("cache_control"))
            .is_some();
        assert!(tools_marked, "tools 末项必须带 cache_control");

        // 最后一条消息的块有断点（会话历史的移动断点）。
        let last_message_marked = body["messages"]
            .as_array()
            .and_then(|m| m.last())
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array().or_else(|| None))
            .and_then(|blocks| blocks.last())
            .and_then(|b| b.get("cache_control"))
            .is_some();
        assert!(
            last_message_marked,
            "最后一条消息必须带移动断点（0.46 的 manual 模式补打）"
        );
    }
}
