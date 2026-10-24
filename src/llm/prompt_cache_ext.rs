//! OpenAI 兼容路径的 prompt cache 断点注入。
//!
//! # 为什么需要这一层
//!
//! 项目通过 `StarMessage::cached_system()`（`types/mod.rs:492`）给 system prompt 打上
//! `cache_control: {"type":"ephemeral"}`，但这条标记到 rig 手上会**原样蒸发**：
//!
//! - `rig_adapter::convert_one_message` 把 system 转成 `Message::System { content: String }`，
//!   而 rig 的这个变体没有 `cache_control` 字段；
//! - Anthropic 分支靠 `with_prompt_caching()` 打自己的断点，OpenAI 兼容分支不经过它；
//! - rig 的 OpenAI 线格式本身也无法在类型层表达 system 断点：
//!   `SystemContent`（`openai/completion/mod.rs:269-273`）只有 `type` + `text`，
//!   既没有 `cache_control` 也没有 `additional_params` 逃生口。
//!
//! 结果是：发往 openai-compatible 网关的请求里**一个断点都没有**，缓存完全靠服务端
//! 隐式前缀匹配。实测命中率因此偏低。
//!
//! # 为什么注入点是 `finalize_request_body`
//!
//! 这是 `OpenAICompatibleProvider` trait 上唯一能拿到**已完整序列化**请求体的钩子，
//! 而且两条路径都会经过它：
//!
//! - 非流式：`openai/completion/mod.rs:2263`
//! - 流式：`openai/completion/streaming.rs:368`（在 `stream` / `stream_options` 合并**之后**）
//!
//! 换扩展的方式是 `Client::with_ext`（`client/mod.rs:536`）—— public 且**无 bounds**，
//! 所以 base_url / headers / HTTP backend 全部原样保留，不需要自己再造一套 builder。
//!
//! # 为什么不能走 `additional_params`
//!
//! `CompletionRequest::additional_params` 带 `#[serde(flatten)]`（`openai/completion/mod.rs:1826`），
//! 它展平到**顶层**。往里面塞 `messages` 会**整体覆盖**整个消息数组。
//! `finalize_request_body` 拿到的是已经展平好的 `serde_json::Value`，天然绕开这个陷阱。
//!
//! # 断点形状照抄 OpenRouter
//!
//! `providers/openrouter/completion.rs:1351-1394` 的 `apply_prompt_caching` 已经对真实网关
//! 验证过。这里沿用同样的形状（把字符串 content 换成带标记的文本块数组；数组 content
//! 则只给最后一块加标记），并额外加一个 tools 断点。

use std::sync::OnceLock;

use rig_core::client::{
    Capabilities, Capable, ClientBuilder, DebugExt, Nothing, Provider, ProviderBuilder,
};
use rig_core::completion::CompletionError;
use rig_core::http_client::HttpClientExt;
use rig_core::providers::openai;
use rig_core::providers::openai::completion::{
    CompletionModelOptions, CompletionRequest, OpenAICompatibleProvider,
};

/// Anthropic 一次请求的断点上限（`anthropic/completion.rs:2382` 的 `MAX_CACHE_CONTROL_MARKERS`）。
/// 我们只用 2 个，远低于上限。
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
/// 全程优雅降级，**从不返回 `Err`** —— 一个不该发的断点不能变成一次请求失败。
fn apply_cache_breakpoints(body: &mut serde_json::Value) {
    let mut marked = 0usize;

    // ── 断点 1：工具目录 ──
    // 形状是 `{ "type": "function", "function": {...} }`，加个兄弟键不会与 `strict` 冲突
    // （项目从不调 `with_strict_tools()`，`tool_choice` 恒为 None）。
    if let Some(tools) = body.get_mut("tools").and_then(|v| v.as_array_mut()) {
        if let Some(last) = tools.last_mut().and_then(|t| t.as_object_mut()) {
            last.insert("cache_control".to_string(), ephemeral());
            marked += 1;
        }
    }

    // ── 断点 2：system prompt ──
    // 每条请求只有一条 system 消息（`prompt_builder.rs:536` 返回 `vec![单条]`），
    // 但压缩可能在数组中部插入 system 摘要（`message_pipeline.rs:404-434`）——那些在
    // 缓存前缀之外，标记第一条即可，不要试图全标。
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
/// rig 的 `Message::System` 序列化成 `Vec<SystemContent>`（mod.rs:141-146，**没有**
/// `serialize_with`，而 User 有），所以热路径永远是数组 `[{"type":"text","text":…}]`。
/// 字符串分支是为 `preamble` 来源的 system 消息等场景保留的防御性路径。
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

/// 在 OpenAI 兼容请求上打 prompt cache 断点的 provider 扩展。
///
/// 结构上包一层 `OpenAICompletionsExt` 并把其余行为全部委托给它——只有
/// `finalize_request_body` 是真正覆写的。这样任何 OpenAI 侧的行为变更
/// （方言修正、模型家族判断、遥测字段）都自动跟随，不会在两份实现之间漂移。
#[derive(Debug, Clone, Default)]
pub struct PromptCacheExt {
    inner: openai::OpenAICompletionsExt,
    enabled: bool,
}

impl PromptCacheExt {
    /// `enabled == false` 时本扩展与 `OpenAICompletionsExt` 行为完全一致，
    /// 便于让官方 OpenAI 分支复用同一份代码路径。
    pub fn new(enabled: bool) -> Self {
        Self {
            inner: openai::OpenAICompletionsExt::default(),
            enabled,
        }
    }
}

impl Provider for PromptCacheExt {
    type Builder = PromptCacheExtBuilder;
    const VERIFY_PATH: &'static str = "/models";
}

impl DebugExt for PromptCacheExt {}

impl ProviderBuilder for PromptCacheExtBuilder {
    type Extension<H>
        = PromptCacheExt
    where
        H: HttpClientExt;
    // 断点由 `PromptCacheExt::new` 在换扩展时注入，builder 本身不带凭证语义。
    type ApiKey = Nothing;

    const BASE_URL: &'static str = "https://api.openai.com/v1";

    fn build<H>(
        _builder: &ClientBuilder<Self, Self::ApiKey, H>,
    ) -> rig_core::http_client::Result<Self::Extension<H>>
    where
        H: HttpClientExt,
    {
        Ok(PromptCacheExt::default())
    }
}

// 只为默认 HTTP 后端实现：rig 的 `CompletionClient::completion_model` 通过
// `Capabilities<H, Completion = Capable<M>>` 反查模型类型，泛型 `H` 会让调用点
// 无法推断（报错里表现为 `Client<PromptCacheExt>` 只剩一个参数）。本项目的
// OpenAI 路径本来就固定用 `reqwest::Client`（`openai::CompletionsClient`），
// 所以这不是限制，是精确描述。
impl Capabilities<reqwest::Client> for PromptCacheExt {
    type Completion = Capable<openai::completion::GenericCompletionModel<PromptCacheExt>>;
    type Embeddings = Nothing;
    type Transcription = Nothing;
    type ModelListing = Nothing;
    type Rerank = Nothing;
}

impl OpenAICompatibleProvider for PromptCacheExt {
    const PROVIDER_NAME: &'static str =
        <openai::OpenAICompletionsExt as OpenAICompatibleProvider>::PROVIDER_NAME;
    type StreamingUsage =
        <openai::OpenAICompletionsExt as OpenAICompatibleProvider>::StreamingUsage;
    type Response = <openai::OpenAICompletionsExt as OpenAICompatibleProvider>::Response;

    /// 必须转发：默认实现恒返回 `false`，OpenAI 覆写为按模型家族判断
    /// （`is_openai_reasoning_model`，mod.rs:1688-1690）。不转发会让 gpt-5 / o-series
    /// 从 `max_completion_tokens` 退回被这些模型拒绝的 `max_tokens`。该 helper 是
    /// `pub(crate)`，只能委托。
    fn requires_modern_output_cap(&self, model: &str) -> bool {
        self.inner.requires_modern_output_cap(model)
    }

    /// 唯一真正覆写的方法：先让内层跑完 OpenAI 原有的 body 收尾，再打我们的断点。
    fn finalize_request_body(&self, body: &mut serde_json::Value) -> Result<(), CompletionError> {
        self.inner.finalize_request_body(body)?;
        if self.enabled {
            apply_cache_breakpoints(body);
        }
        Ok(())
    }
}

// 让未使用的导入保持诚实：CompletionRequest / CompletionModelOptions 出现在文档里，
// 但本文件不需要直接引用它们。
#[allow(unused)]
type _Doc = (CompletionRequest, CompletionModelOptions);

/// 与 `OpenAICompletionsExtBuilder` 同形的单元 builder。
///
/// **注意**:不能把 `Provider::Builder` 指向 `client::Nothing` —— 它只有 `impl ApiKey`,
/// 没有 `ProviderBuilder` / `Provider` 实现(已核对 `client/mod.rs:158-163`)。
#[derive(Debug, Default, Clone, Copy)]
pub struct PromptCacheExtBuilder;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 直接调 finalize 钩子，不需要网络。
    fn finalize(enabled: bool, body: &mut serde_json::Value) -> Result<(), CompletionError> {
        PromptCacheExt::new(enabled).finalize_request_body(body)
    }

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
        finalize(true, &mut body).expect("finalize should succeed");

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
        finalize(true, &mut body).expect("finalize should succeed");

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
        finalize(true, &mut body).expect("finalize should succeed");

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
        finalize(true, &mut body).expect("finalize should succeed");

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
        finalize(true, &mut body).expect("finalize should succeed");

        assert_eq!(count_breakpoints(&body), 2);
        assert!(count_breakpoints(&body) <= MAX_BREAKPOINTS);
    }

    #[test]
    fn no_system_message_is_a_noop() {
        let mut body = json!({
            "messages": [{"role": "user", "content": [{"type": "text", "text": "u"}]}]
        });
        let before = body.clone();
        finalize(true, &mut body).expect("finalize should succeed");

        assert_eq!(body, before);
    }

    #[test]
    fn disabled_ext_leaves_body_untouched() {
        let mut body = json!({
            "messages": [{"role": "system", "content": [{"type": "text", "text": "s"}]}],
            "tools": [{"type": "function", "function": {"name": "t"}}]
        });
        let before = body.clone();
        finalize(false, &mut body).expect("finalize should succeed");

        assert_eq!(body, before, "关闭时必须与修复前逐字节一致");
        assert_eq!(count_breakpoints(&body), 0);
    }

    #[test]
    fn preserves_stream_flag_and_stream_options() {
        // 流式路径在 finalize 之前才合并这两个字段(streaming.rs:346-368)，
        // 钩子必须原样放过它们，否则流式与非流式就不等价了。
        let mut body = json!({
            "stream": true,
            "stream_options": {"include_usage": true},
            "messages": [{"role": "system", "content": [{"type": "text", "text": "s"}]}]
        });
        finalize(true, &mut body).expect("finalize should succeed");

        assert_eq!(body["stream"], json!(true));
        assert_eq!(body["stream_options"]["include_usage"], json!(true));
    }

    #[test]
    fn does_not_disturb_additional_params() {
        // additional_params 是 #[serde(flatten)] 到顶层的，finalize 拿到的是展平后的
        // body —— 这条守住「别去动它」的约定。
        let mut body = json!({
            "model": "m",
            "some_custom_top_level_key": {"nested": 1},
            "messages": [{"role": "system", "content": [{"type": "text", "text": "s"}]}]
        });
        finalize(true, &mut body).expect("finalize should succeed");

        assert_eq!(body["some_custom_top_level_key"]["nested"], json!(1));
        assert_eq!(body["model"], json!("m"));
    }

    #[test]
    fn marking_is_idempotent() {
        let mut body = json!({
            "messages": [{"role": "system", "content": [{"type": "text", "text": "s"}]}]
        });
        finalize(true, &mut body).expect("first pass");
        let after_first = body.clone();
        finalize(true, &mut body).expect("second pass");

        assert_eq!(body, after_first);
        assert_eq!(count_breakpoints(&body), 1);
    }

    #[test]
    fn forwards_requires_modern_output_cap() {
        // 不转发就会让 gpt-5 / o-series 退回 max_tokens 而被服务端拒绝。
        let ext = PromptCacheExt::new(true);
        assert!(ext.requires_modern_output_cap("gpt-5"));
        assert!(ext.requires_modern_output_cap("o3-mini"));
        assert!(!ext.requires_modern_output_cap("gpt-4o"));
    }

    /// 端到端形态：走 rig 真实的 OpenAI 线格式序列化，再打断点。
    ///
    /// 这条锁住最重要的契约 —— 断点必须真的落在**发上线的那个 JSON** 里，
    /// 而不是某个中间结构上。前面的测试都是手搓 `serde_json::Value`，可能
    /// 与 rig 实际生成的形状脱节。
    #[test]
    fn breakpoints_land_in_the_real_openai_wire_format() {
        use rig_core::completion::message::{Message, UserContent};
        use rig_core::completion::request::CompletionRequest as CoreCompletionRequest;
        use rig_core::providers::openai::completion::{
            CompletionModelOptions, CompletionRequest as WireRequest,
        };

        let core = CoreCompletionRequest {
            model: None,
            preamble: None,
            chat_history: vec![
                Message::System {
                    content: "You are a coding agent.".to_string(),
                },
                Message::User {
                    content: vec![UserContent::text("hello")],
                },
            ],
            documents: vec![],
            tools: vec![],
            temperature: None,
            max_tokens: Some(16),
            tool_choice: None,
            additional_params: None,
            output_schema: None,
            record_telemetry_content: false,
        };

        let ext = PromptCacheExt::new(true);
        let wire = ext
            .build_completion_request(
                "some-model".to_string(),
                core,
                CompletionModelOptions::default(),
            )
            .expect("wire request should build");

        // 序列化得到真实线格式 body。
        let mut body = serde_json::to_value(&wire).expect("wire request should serialize");

        // rig 的 system 消息在 OpenAI 线格式里是**块数组**（无 serialize_with），
        // 不是裸字符串 —— 这一点如果变了，断点就会落空。
        assert!(
            body["messages"][0]["content"].as_array().is_some(),
            "rig 序列化 system content 时必须用块数组（没有 serialize_with），\
             否则断点会落空"
        );

        finalize(true, &mut body).expect("finalize should succeed");

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
