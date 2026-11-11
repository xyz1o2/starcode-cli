//! OpenAI 兼容路径的 prompt cache 断点注入。
//!
//! # 为什么需要这一层
//!
//! 项目通过 `StarMessage::cached_system()`（`types/mod.rs:492`）给 system prompt 打上
//! `cache_control: {"type":"ephemeral"}`，但这条标记到 rig 手上会**原样蒸发**：
//!
//! - `rig_adapter::convert_one_message` 把 system 转成 `Message::System { content: String }`，
//!   而 rig 的这个变体没有 `cache_control` 字段；
//! - Anthropic 分支靠 `Messages::with_prompt_caching()` 打自己的断点，OpenAI 兼容分支不经过它；
//! - rig 的 OpenAI 线格式本身也无法在类型层表达 system 断点：
//!   `Chat::encode` 把 system 序列化成 `[{"type":"text","text":…}]`
//!   （`providers/openai/wire/chat.rs` 的 `messages()`），既没有
//!   `cache_control` 也没有 `additional_params` 逃生口。
//!
//! 结果是：发往 openai-compatible 网关的请求里**一个断点都没有**，缓存完全靠服务端
//! 隐式前缀匹配。实测命中率因此偏低。
//!
//! # rig 0.46 下的注入点：装饰 Transport
//!
//! rig 0.46 推翻了 0.42 的 `OpenAICompatibleProvider::finalize_request_body` 扩展机制，
//! 换成 wire 架构：`Chat` 是**纯数据**端点，一个 `Wire` impl 负责 `encode`
//! （请求体序列化）和 `decoder`（回复解码）。0.46 的 `Chat::decoder` 返回的
//! `ChatDecoder` 位于 `pub(crate)` 模块里 —— 外部 crate **无法命名**这个类型，
//! 所以「包一层 `Chat` wire、只覆写 `encode`」的 0.42 式做法不再可行
//! （`Wire` impl 的关联类型必须显式写出来）。
//!
//! 替代注入点是 `Transport`（`driver.rs:205`）：transport 在 wire 编码**之后**、
//! 网络发送**之前**拿到 `Encoded`（已序列化的 HTTP 请求），此时 body 就是
//! 最终上线的 JSON。`PatchTransport` 对 `Encoded::request` 做
//! 解 JSON → 打点 → 重装 bytes，其余字段（headers、framing、request-id header、
//! projection）原样转发给内层 transport —— 传输、重试、观测行为全部跟随 rig。
//!
//! 这个注入点天然覆盖流式与非流式：两条路径共用同一个 `Chat::encode`，
//! `Rewrite::StreamUsage` / `Rewrite::Stream(true)` 已在 encode 阶段写进 body，
//! 我们在它之后打点，不会碰坏这两个字段。
//!
//! # 为什么不能走 `additional_params`
//!
//! `CompletionRequest::additional_params` 在 0.46 由 `request_params` 的
//! `raw_layer` `deep_merge` 到**顶层**。往里面塞 `messages` 会**整体覆盖**整个消息数组。
//! 对已编码 body 直接改写天然绕开这个陷阱。
//!
//! # 为什么不能用 `CompletionRequest::cache_points`
//!
//! `cache_points` 是 0.46 的 portable 断点字段，但 `Chat` wire **不消费**它 ——
//! 0.46 里只有 Anthropic `Messages`（`anthropic/completion.rs:264`）和 OpenAI
//! Responses API（`responses_api/mod.rs:577`）实现了标记逻辑。Chat Completions
//! 这条线没有实现。
//!
//! # 断点形状照抄 OpenRouter
//!
//! 0.42 时代沿用 rig `providers/openrouter` 已对真实网关验证过的形状
//! （字符串 content 换成带标记的文本块数组；数组 content 只给最后一块加标记），
//! 并额外加一个 tools 断点。形状不变，继续沿用。
//!
//! # HTTP 客户端
//!
//! 0.46 的 `HttpClientExt` 是 `rig-http` 的 trait，`rig-reqwest` 提供
//! `ReqwestClient::from(reqwest::Client)` 包装。我们把项目自己的
//! `build_rig_http_client()`（统一超时/代理配置）包装后经
//! `OpenAIConfig::connect(...)` 装上，再叠加 `PatchTransport`。

use std::sync::OnceLock;

use rig_core::driver::{Exchange, Opening, Transport};
use rig_core::error::EncodeError;
use rig_core::http_client::HttpClientExt;
use rig_core::wire::{Body, Encoded, Wire, WireFrame};

/// Anthropic 一次请求的断点上限（anthropic/wire.rs 的断点预算）。我们只用 2 个。
const MAX_BREAKPOINTS: usize = 2;

/// Anthropic 的标准临时断点标记。
fn ephemeral() -> serde_json::Value {
    serde_json::json!({ "type": "ephemeral" })
}

/// 是否在 OpenAI 兼容路径上发送 `cache_control` 断点。
///
/// **默认关。** 这注入的是 OpenAI Chat Completions 规范**之外**的字段：官方端点会严格
/// 校验 content part，遇到未知键很可能 400。只有明确知道上游网关接受它的用户才该打开
/// （`~/.star/settings.json` 里 `type: "openai-compatible"` 的 provider 大多是聚合网关）。
///
/// 逃生口就是这个开关：设成 `0` 立刻回到修复前的行为。
///
/// 解析惯例与 `prompt_builder::prompt_cache_enabled()` 一致（`0` / `false` / `off` 视为关）。
pub fn enabled_from_env() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("STAR_OPENAI_PROMPT_CACHE")
            .ok()
            .map(|v| {
                let v = v.trim().to_lowercase();
                !(v == "0" || v == "false" || v == "off")
            })
            .unwrap_or(false)
    })
}

/// 给已序列化的请求体打 prompt cache 断点。
///
/// 打两处：
/// 1. `tools` 数组的**最后一个** tool definition —— 工具目录是仅次于 system prompt 的最大
///    静态块，会话内不变，性价比最高。
/// 2. 第一条 `role == "system"` 消息的**最后一个** content block。
///
/// 断点标最后一块而不是第一块是有意的：断点缓存的是「到该块为止」的前缀，落在最后一块
/// 才能最大化复用长度。
///
/// 不加第三个断点：Anthropic 的第三个是**顶层** `automatic_caching` 断点，由服务端实现，
/// 在这条线上没有对应表示；在最后一个 user 消息上打断点只会给每轮都变的块加写入溢价，
/// 永远换不到命中。
///
/// 全程优雅降级，**从不报错** —— 一个不该发的断点不能变成一次请求失败。
pub(crate) fn apply_cache_breakpoints(body: &mut serde_json::Value) {
    let mut marked = 0usize;

    // ── 断点 1：工具目录 ──
    // 形状是 `{ "type": "function", "function": {...} }`，加个兄弟键不会与 `strict` 冲突
    // （我们从不调 `with_strict_tools()`）。
    if let Some(tools) = body.get_mut("tools").and_then(|v| v.as_array_mut()) {
        if let Some(last) = tools.last_mut().and_then(|t| t.as_object_mut()) {
            last.insert("cache_control".to_string(), ephemeral());
            marked += 1;
        }
    }

    // ── 断点 2：system prompt ──
    // 提示词拆成 [静态块, 准静态块] 两条（见 prompt_builder::build_cached_system_messages）。
    // 断点标记**第一条** system 消息的最后一块（静态块）：项目指令（STAR.md）等
    // 准静态内容变了，只失效准静态块，静态前缀继续命中。压缩可能在数组中部插入
    // system 摘要（`message_pipeline.rs`）——那些在缓存前缀之外，标记第一条即可，
    // 不要试图全标。
    if let Some(messages) = body.get_mut("messages").and_then(|v| v.as_array_mut()) {
        if let Some(system) = messages
            .iter_mut()
            .find(|m| m.get("role").and_then(|v| v.as_str()) == Some("system"))
        {
            if mark_system_content(system) {
                marked += 1;
            }
        }
    }

    if marked > 0 && marked <= MAX_BREAKPOINTS {
        crate::utils::logging::append_debug_log_line(&format!(
            "[PromptCache] openai cache_control applied (breakpoints={marked})"
        ));
    }
}

/// 给一条 system 消息的 content 打标记。返回是否真的打了。
///
/// rig 0.46 的 `Chat::encode` 把 `Message::System` 序列化成
/// `[{"type":"text","text":…}]` 块数组，所以热路径永远是数组。字符串分支是为
/// `preamble` 来源的 system 消息等场景保留的防御性路径。
fn mark_system_content(message: &mut serde_json::Value) -> bool {
    let Some(object) = message.as_object_mut() else {
        return false;
    };
    let Some(content) = object.get_mut("content") else {
        return false;
    };

    match content {
        // 已经是块数组：只标记最后一块，前面的块（含 image 等非文本块）原样保留。
        serde_json::Value::Array(blocks) => {
            let Some(last) = blocks.last_mut().and_then(|b| b.as_object_mut()) else {
                return false;
            };
            // insert 天然覆盖，重复调用幂等。
            last.insert("cache_control".to_string(), ephemeral());
            true
        }
        // 纯字符串：换成带标记的文本块数组（与 OpenRouter 的形状一致）。
        serde_json::Value::String(text) => {
            let marked = serde_json::json!({
                "type": "text",
                "text": text,
                "cache_control": ephemeral(),
            });
            *content = serde_json::Value::Array(vec![marked]);
            true
        }
        // 既不是字符串也不是数组（理论上不会发生）：不动它。
        _ => false,
    }
}

/// 断点注入 transport：在 rig 编码之后、发送之前给请求体打 `cache_control`。
///
/// 装饰任意 `Transport`（通常是 `DynHttpClient`），`send` 的 `payload: Encoded`
/// 里 `request.body` 已是 rig 编好的最终 JSON bytes。`enabled == false` 时
/// 与内层 transport 行为完全一致，便于让官方 OpenAI 分支复用同一份代码路径。
#[derive(Clone, Debug)]
pub struct PatchTransport<H> {
    inner: H,
    enabled: bool,
}

impl<H> PatchTransport<H> {
    pub fn new(inner: H, enabled: bool) -> Self {
        Self { inner, enabled }
    }
}

impl<W, H> Transport<W> for PatchTransport<H>
where
    // 与 rig 自己的 HTTP transport 同一约束（driver/http_transport.rs:16）。
    W: Wire<Payload = Encoded, Frame = WireFrame>,
    H: Transport<W>,
{
    fn send(&self, mut payload: Encoded, exchange: Exchange) -> Opening<W::Frame> {
        if self.enabled {
            if let Err(error) = patch_request(&mut payload.request) {
                // 编码期失败（JSON 坏体）走 Opening 失败路径，与 wire 的
                // EncodeError 同语义。
                return Opening::failed(rig_core::error::ProviderError::request(error));
            }
        }
        self.inner.send(payload, exchange)
    }
}

/// 解出 `Encoded.request` 的 JSON body → 打点 → 装回 bytes。
/// method / uri / headers / extensions 经 `into_parts` 原样保留 —— 只换 body。
fn patch_request(request: &mut rig_core::http_client::Request<Body>) -> Result<(), String> {
    let taken = std::mem::replace(request, rig_core::http_client::Request::new(Body::empty()));
    let (parts, body) = taken.into_parts();
    let body = match body {
        Body::Bytes(raw) => raw,
        // chat 线格式的 body 恒为 JSON bytes（multipart 只出现在 audio/images
        // 端点，chat 不会走到这里）。防御性分支：原样放回。
        other => {
            *request = rig_core::http_client::Request::from_parts(parts, other);
            return Ok(());
        }
    };
    let byte_vec: Vec<u8> = body.into();
    let mut value: serde_json::Value = serde_json::from_slice(&byte_vec)
        .map_err(|e| format!("patching cache breakpoints: chat body is not JSON: {e}"))?;
    apply_cache_breakpoints(&mut value);
    let patched = serde_json::to_vec(&value)
        .map_err(|e| format!("patching cache breakpoints: re-serialize body: {e}"))?;
    *request = rig_core::http_client::Request::from_parts(parts, Body::Bytes(patched));
    Ok(())
}

/// 打好断点的 HTTP transport：先包项目的 reqwest::Client 成 rig 的
/// `ReqwestClient`（HttpClientExt），再叠 `PatchTransport`。
///
/// `PatchTransport` 本身满足 `Transport`（rig 0.46 里 `HttpClientExt`
/// 自动就是 transport），所以直接返回，不需要再擦一层 `DynHttpClient`。
pub fn patched_transport(
    client: reqwest::Client,
    enabled: bool,
) -> PatchTransport<rig_core::http_client::DynHttpClient> {
    let http = rig_core::http_client::DynHttpClient::new(rig_reqwest::ReqwestClient::from(
        client,
    ));
    PatchTransport::new(http, enabled)
}

// 编码失败路径的类型。
#[allow(unused)]
use EncodeError as _DocEncodeError;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 遍历整个 body 数 `cache_control` 出现次数。
    fn count_breakpoints(value: &serde_json::Value) -> usize {
        match value {
            serde_json::Value::Object(map) => {
                let here = usize::from(map.contains_key("cache_control"));
                map.values().map(count_breakpoints).sum::<usize>() + here
            }
            serde_json::Value::Array(items) => items.iter().map(count_breakpoints).sum(),
            _ => 0,
        }
    }

    #[test]
    fn marks_last_system_block_when_content_is_an_array() {
        let mut body = json!({
            "model": "m",
            "messages": [{"role": "system", "content": [{"type": "text", "text": "x"}]}]
        });
        apply_cache_breakpoints(&mut body);

        assert_eq!(
            body["messages"][0]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
        // 文本本身不能被改动。
        assert_eq!(body["messages"][0]["content"][0]["text"], "x");
    }

    #[test]
    fn converts_string_system_content_to_a_marked_block() {
        let mut body = json!({
            "messages": [{"role": "system", "content": "hello"}]
        });
        apply_cache_breakpoints(&mut body);

        let blocks = body["messages"][0]["content"]
            .as_array()
            .expect("string content should become an array");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[0]["text"], "hello");
        assert_eq!(blocks[0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn preserves_non_text_blocks_in_system_content() {
        let mut body = json!({
            "messages": [{
                "role": "system",
                "content": [
                    {"type": "image_url", "image_url": {"url": "data:..."}},
                    {"type": "text", "text": "tail"}
                ]
            }]
        });
        let image_before = body["messages"][0]["content"][0].clone();
        apply_cache_breakpoints(&mut body);

        // 非文本块逐字节不变。
        assert_eq!(body["messages"][0]["content"][0], image_before);
        assert!(body["messages"][0]["content"][0]
            .get("cache_control")
            .is_none());
        // 断点落在最后一块。
        assert_eq!(
            body["messages"][0]["content"][1]["cache_control"]["type"],
            "ephemeral"
        );
    }

    #[test]
    fn marks_last_tool_definition_only() {
        let tool = |name: &str| json!({"type": "function", "function": {"name": name}});
        let mut body = json!({
            "messages": [{"role": "system", "content": [{"type": "text", "text": "s"}]}],
            "tools": [tool("a"), tool("b"), tool("c")]
        });
        apply_cache_breakpoints(&mut body);

        assert!(body["tools"][0].get("cache_control").is_none());
        assert!(body["tools"][1].get("cache_control").is_none());
        assert_eq!(body["tools"][2]["cache_control"]["type"], "ephemeral");
        // 工具定义本体不能被破坏。
        assert_eq!(body["tools"][2]["function"]["name"], "c");
    }

    #[test]
    fn emits_exactly_two_breakpoints_for_a_typical_request() {
        let mut body = json!({
            "model": "m",
            "messages": [
                {"role": "system", "content": [{"type": "text", "text": "s"}]},
                {"role": "user", "content": [{"type": "text", "text": "u"}]}
            ],
            "tools": [{"type": "function", "function": {"name": "t"}}]
        });
        apply_cache_breakpoints(&mut body);

        assert_eq!(count_breakpoints(&body), 2);
        assert!(count_breakpoints(&body) <= MAX_BREAKPOINTS);
    }

    #[test]
    fn no_system_message_is_a_noop() {
        let mut body = json!({
            "messages": [{"role": "user", "content": [{"type": "text", "text": "u"}]}]
        });
        let before = body.clone();
        apply_cache_breakpoints(&mut body);

        assert_eq!(body, before);
    }

    #[test]
    fn preserves_stream_flag_and_stream_options() {
        // 流式请求的这两个字段由 rig 的 Rewrite 在 encode 阶段写入 body，
        // patch 必须原样放过它们，否则流式与非流式就不等价了。
        let mut body = json!({
            "stream": true,
            "stream_options": {"include_usage": true},
            "messages": [{"role": "system", "content": [{"type": "text", "text": "s"}]}]
        });
        apply_cache_breakpoints(&mut body);

        assert_eq!(body["stream"], json!(true));
        assert_eq!(body["stream_options"]["include_usage"], json!(true));
    }

    #[test]
    fn does_not_disturb_additional_params() {
        // additional_params 是 deep_merge 到顶层的，patch 拿到的是合并后的 body ——
        // 这条守住「别去动它」的约定。
        let mut body = json!({
            "model": "m",
            "some_custom_top_level_key": {"nested": 1},
            "messages": [{"role": "system", "content": [{"type": "text", "text": "s"}]}]
        });
        apply_cache_breakpoints(&mut body);

        assert_eq!(body["some_custom_top_level_key"]["nested"], json!(1));
        assert_eq!(body["model"], json!("m"));
    }

    #[test]
    fn marking_is_idempotent() {
        let mut body = json!({
            "messages": [{"role": "system", "content": [{"type": "text", "text": "s"}]}]
        });
        apply_cache_breakpoints(&mut body);
        let after_first = body.clone();
        apply_cache_breakpoints(&mut body);

        assert_eq!(body, after_first);
        assert_eq!(count_breakpoints(&body), 1);
    }

    /// 端到端形态：走 rig 真实的 OpenAI 线格式序列化，再打断点。
    ///
    /// 这条锁住最重要的契约 —— 断点必须真的落在**发上线的那个 JSON** 里，
    /// 而不是某个中间结构上。前面的测试都是手搓 `serde_json::Value`，可能
    /// 与 rig 实际生成的形状脱节。
    #[test]
    fn breakpoints_land_in_the_real_openai_wire_format() {
        use std::sync::Arc;

        let core = rig_core::completion::CompletionRequest {
            model: None,
            chat_history: vec![
                Arc::new(rig_core::message::Message::System {
                    content: "You are a coding agent.".to_string(),
                }),
                Arc::new(rig_core::message::Message::User {
                    content: vec![rig_core::message::UserContent::Text(
                        rig_core::message::Text {
                            text: "hello".to_string(),
                            ..Default::default()
                        },
                    )],
                }),
            ],
            ..rig_core::completion::CompletionRequest::default()
        };

        let config = rig_core::providers::openai::wire::OpenAIConfig::new("test-key");
        let chat = rig_core::providers::openai::wire::Chat::new(config, "some-model");
        let encoded = chat
            .encode(core, rig_core::wire::Mode::Unary)
            .expect("wire request should build");

        let bytes = match encoded.request.body() {
            Body::Bytes(bytes) => bytes.clone(),
            _ => panic!("chat body should be bytes"),
        };
        let mut body: serde_json::Value =
            serde_json::from_slice(&bytes).expect("wire request should serialize");

        // rig 的 system 消息在 OpenAI 线格式里是**块数组**，不是裸字符串 ——
        // 这一点如果变了，断点就会落空。
        assert!(
            body["messages"][0]["content"].as_array().is_some(),
            "rig 序列化 system content 时必须用块数组，否则断点会落空"
        );

        apply_cache_breakpoints(&mut body);

        let system_content = body["messages"][0]["content"]
            .as_array()
            .expect("system content stays an array");
        assert_eq!(
            system_content
                .last()
                .expect("system content has a block")
                .get("cache_control")
                .and_then(|v| v.get("type"))
                .and_then(|v| v.as_str()),
            Some("ephemeral"),
            "断点必须落在 system 最后一个块上"
        );
    }

    #[test]
    fn env_gate_defaults_to_off() {
        // 未设置时必须是关的——官方 OpenAI 端点会拒绝这个规范外字段。
        // 只断言解析逻辑，不改进程环境。
        assert!(!enabled_from_env(), "默认应为关闭，需显式打开");
    }
}
