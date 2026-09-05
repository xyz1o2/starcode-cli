//! 思考力度（thinking effort）→ 各家 provider 的请求参数。
//!
//! ## 为什么需要这个模块
//!
//! UI 侧的档位（`ChatState::thinking_effort`，来自 Alt+T / `/effort` /
//! 命令面板）以前只是个显示项：改完写进 `~/.star/settings.json`，欢迎抬头
//! 和状态栏也跟着变，但没有任何路径把它送进请求 —— 唯一读它的地方是
//! `rig_adapter::build_request` 里的 `STAR_THINKING_EFFORT` 环境变量，而
//! 全仓没有代码设置这个变量。本模块补上会话级存放点
//! （[`set_session_effort`] / [`current_effort`]），并把档位翻译成各家
//! provider 真正认识的字段。
//!
//! ## 为什么要分方言（dialect）
//!
//! rig 的 `CompletionRequest::additional_params` 是 `#[serde(flatten)]`，
//! 里面的 key 会原样出现在请求体顶层。所以"把 reasoning_effort、thinking、
//! reasoning 三个字段一起发出去，谁认识谁生效"是错的：严格的服务端
//! （Anthropic 官方、NVIDIA NIM 等）看到不认识的字段直接 400。这里按
//! provider + 模型名只挑**一种**方言发。
//!
//! ## `Off` 的语义
//!
//! `Off` 一律不发字段（= 用服务端默认），只有字段名明确的国内厂商方言
//! （Qwen / 智谱 / 火山）才发显式关闭 —— 对未知的兼容端点来说，多发一个
//! 字段导致整轮 400 比"思考没关掉"更糟。
//!
//! 自动判断猜错时用 `STAR_THINKING_DIALECT` 强制指定方言，
//! `STAR_THINKING_EFFORT` 仍然作为没设过会话档位时的默认值。

use crate::types::ThinkingEffort;
use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicU8, Ordering};

/// 手动指定思考参数方言，取值见 [`parse_dialect`]。
pub const ENV_STAR_THINKING_DIALECT: &str = "STAR_THINKING_DIALECT";

/// 会话级思考力度。`UNSET` 表示本进程没设过，回落到
/// `STAR_THINKING_EFFORT`。写入方是 `runtime::control_requests`
/// 处理 `AgentRequest::SetThinkingEffort` 时，读取方是构造请求时。
static SESSION_EFFORT: AtomicU8 = AtomicU8::new(UNSET);

const UNSET: u8 = 0;

fn encode(effort: &ThinkingEffort) -> u8 {
    match effort {
        ThinkingEffort::Off => 1,
        ThinkingEffort::Low => 2,
        ThinkingEffort::Medium => 3,
        ThinkingEffort::High => 4,
    }
}

fn decode(raw: u8) -> Option<ThinkingEffort> {
    match raw {
        1 => Some(ThinkingEffort::Off),
        2 => Some(ThinkingEffort::Low),
        3 => Some(ThinkingEffort::Medium),
        4 => Some(ThinkingEffort::High),
        _ => None,
    }
}

/// 记录用户新选的档位，之后每一次请求都按它构造。
pub fn set_session_effort(effort: &ThinkingEffort) {
    SESSION_EFFORT.store(encode(effort), Ordering::Relaxed);
}

/// 解析档位字符串，覆盖各处命令/环境变量里出现过的写法。
pub fn parse_effort(raw: &str) -> Option<ThinkingEffort> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "off" | "none" | "disable" | "disabled" | "false" | "0" => Some(ThinkingEffort::Off),
        "low" | "minimal" | "think" => Some(ThinkingEffort::Low),
        "medium" | "mid" | "auto" | "on" | "true" => Some(ThinkingEffort::Medium),
        "high" | "xhigh" | "max" | "ultra" | "ultrathink" => Some(ThinkingEffort::High),
        _ => None,
    }
}

/// 当前生效档位：会话设置 > `STAR_THINKING_EFFORT` > `Off`。
pub fn current_effort() -> ThinkingEffort {
    if let Some(effort) = decode(SESSION_EFFORT.load(Ordering::Relaxed)) {
        return effort;
    }
    std::env::var(super::ENV_STAR_THINKING_EFFORT)
        .ok()
        .and_then(|raw| parse_effort(&raw))
        .unwrap_or_default()
}

// ── provider 画像与方言 ────────────────────────────────────────────

/// `RigAdapter` 的四种取向，决定思考参数往哪种方言翻。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Anthropic,
    OpenAi,
    DeepSeek,
    /// 任意 OpenAI 兼容端点（聚合器、国内厂商、自建推理服务）
    Compatible,
}

/// 判断方言需要的最少信息，由 `RigAdapter::thinking_profile` 现算。
#[derive(Debug, Clone, Copy)]
pub struct ProviderProfile<'a> {
    pub kind: ProviderKind,
    pub model: &'a str,
    pub base_url: Option<&'a str>,
    pub provider_name: Option<&'a str>,
}

/// 各家开思考的字段长得完全不一样，只能挑一种发。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ThinkingDialect {
    /// 不发任何思考字段
    #[default]
    None,
    /// Anthropic 新一代：`thinking:{type:"adaptive"}` + `output_config:{effort}`
    AnthropicEffort,
    /// Anthropic 老一代：`thinking:{type:"enabled",budget_tokens:N}`，要求 `max_tokens > N`
    AnthropicBudget,
    /// OpenAI / Groq / xAI：`reasoning_effort:"low"|"medium"|"high"`
    OpenAiEffort,
    /// OpenRouter：`reasoning:{effort}`
    OpenRouterReasoning,
    /// 阿里云百炼兼容模式：`enable_thinking` + `thinking_budget`
    QwenEnableThinking,
    /// 智谱 / 火山方舟：`thinking:{type:"enabled"|"disabled"}`（只有开关，没有档位）
    ThinkingToggle,
}

/// `STAR_THINKING_DIALECT` 的取值。自动判断认错服务端时用它兜底。
pub fn parse_dialect(raw: &str) -> Option<ThinkingDialect> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "off" | "none" | "disabled" => Some(ThinkingDialect::None),
        "anthropic" | "anthropic-effort" | "adaptive" | "output_config" => {
            Some(ThinkingDialect::AnthropicEffort)
        }
        "anthropic-budget" | "budget" | "budget_tokens" => Some(ThinkingDialect::AnthropicBudget),
        "openai" | "reasoning_effort" | "groq" | "xai" => Some(ThinkingDialect::OpenAiEffort),
        "openrouter" | "reasoning" => Some(ThinkingDialect::OpenRouterReasoning),
        "qwen" | "dashscope" | "enable_thinking" => Some(ThinkingDialect::QwenEnableThinking),
        "thinking" | "toggle" | "glm" | "zhipu" | "doubao" => Some(ThinkingDialect::ThinkingToggle),
        _ => None,
    }
}

/// 取模型名的最后一段并小写：聚合器会带 `anthropic/` 这类前缀。
fn canonical_model(model: &str) -> String {
    model
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .trim()
        .to_ascii_lowercase()
}

/// 取 `claude-opus-4-7-20260101` 里 `claude-opus-4` 之后的小版本号。
/// 只认 1~2 位数字，否则 `claude-opus-4-20250514` 的日期会被当成版本号。
fn family_minor(model: &str, prefix: &str) -> Option<u32> {
    let rest = model.strip_prefix(prefix)?.strip_prefix('-')?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() || digits.len() > 2 {
        return None;
    }
    digits.parse().ok()
}

/// `output_config.effort` 的支持范围：Fable 系列、Opus 4.5+、Sonnet 4.6+。
/// Sonnet 4.5 / Haiku 4.5 收到这个字段会报错，所以只能白名单。
fn anthropic_effort_capable(model: &str) -> bool {
    if model.contains("fable") {
        return true;
    }
    if let Some(minor) = family_minor(model, "claude-opus-4") {
        return minor >= 5;
    }
    if let Some(minor) = family_minor(model, "claude-sonnet-4") {
        return minor >= 6;
    }
    false
}

/// 老一代 extended thinking（`budget_tokens`）的支持范围。
/// Claude 3.5 及更早不支持思考，发了会 400。
fn anthropic_budget_capable(model: &str) -> bool {
    model.starts_with("claude-3-7")
        || model.starts_with("claude-sonnet-4")
        || model.starts_with("claude-opus-4")
        || model.starts_with("claude-haiku-4-5")
}

fn anthropic_dialect(model: &str) -> ThinkingDialect {
    if anthropic_effort_capable(model) {
        ThinkingDialect::AnthropicEffort
    } else if anthropic_budget_capable(model) {
        ThinkingDialect::AnthropicBudget
    } else {
        ThinkingDialect::None
    }
}

/// OpenAI 兼容端点：先按服务端认，认不出来再看模型名。
fn compatible_dialect(profile: &ProviderProfile<'_>, model: &str) -> ThinkingDialect {
    let host = format!(
        "{} {}",
        profile.base_url.unwrap_or_default(),
        profile.provider_name.unwrap_or_default()
    )
    .to_ascii_lowercase();
    let has = |needle: &str| host.contains(needle);

    if has("openrouter") {
        return ThinkingDialect::OpenRouterReasoning;
    }
    if has("dashscope") || has("aliyuncs") || has("bailian") {
        return ThinkingDialect::QwenEnableThinking;
    }
    if has("bigmodel") || has("zhipu") || has("glm") || has("volces") || has("volcengine") {
        return ThinkingDialect::ThinkingToggle;
    }
    if has("anthropic") {
        return anthropic_dialect(model);
    }
    // DeepSeek、Moonshot 都是按模型名区分思不思考，没有请求参数可调。
    if has("deepseek") || has("moonshot") || has("kimi") {
        return ThinkingDialect::None;
    }
    // 其余（Groq / xAI / vLLM / SGLang / LiteLLM / 自建）按 OpenAI 的
    // `reasoning_effort` 走，但只在模型名看着会思考时发 —— 非思考模型
    // 收到这个字段一样会 400。
    if crate::core::config::models::is_thinking_model(model) {
        ThinkingDialect::OpenAiEffort
    } else {
        ThinkingDialect::None
    }
}

/// 选定这次请求该用的方言。`STAR_THINKING_DIALECT` 优先。
pub fn resolve_dialect(profile: &ProviderProfile<'_>) -> ThinkingDialect {
    if let Some(forced) = std::env::var(ENV_STAR_THINKING_DIALECT)
        .ok()
        .and_then(|raw| parse_dialect(&raw))
    {
        return forced;
    }
    let model = canonical_model(profile.model);
    match profile.kind {
        ProviderKind::Anthropic => anthropic_dialect(&model),
        ProviderKind::OpenAi => {
            if crate::core::config::models::is_thinking_model(&model) {
                ThinkingDialect::OpenAiEffort
            } else {
                ThinkingDialect::None
            }
        }
        // DeepSeek 官方用模型名区分（deepseek-chat / deepseek-reasoner），
        // 档位在这里天然无效 —— 要换思考深度得换模型。
        ProviderKind::DeepSeek => ThinkingDialect::None,
        ProviderKind::Compatible => compatible_dialect(profile, &model),
    }
}

// ── 档位 → 参数 ───────────────────────────────────────────────────

/// `low` / `medium` / `high` 三档。`Off` 不会走到这里（调用点已判空）。
fn effort_word(effort: &ThinkingEffort) -> &'static str {
    match effort {
        ThinkingEffort::Off | ThinkingEffort::Low => "low",
        ThinkingEffort::Medium => "medium",
        ThinkingEffort::High => "high",
    }
}

/// `(budget_tokens, max_tokens)`。`budget_tokens` 必须小于 `max_tokens`，
/// 而 `max_tokens` 是"思考 + 回答"的总额度，所以要给回答留出余量。
/// 上限压在 32000：老一代里 Opus 4 / 4.1 的输出上限就是 32000，再高会 400。
fn budget_ladder(effort: &ThinkingEffort) -> (u64, u64) {
    match effort {
        ThinkingEffort::Off | ThinkingEffort::Low => (4_096, 16_384),
        ThinkingEffort::Medium => (10_000, 24_576),
        ThinkingEffort::High => (24_576, 32_000),
    }
}

/// 一次请求要带的思考参数。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ThinkingParams {
    /// 合并进 rig 的 `additional_params`（`#[serde(flatten)]`，等于请求体顶层）
    pub extra: Map<String, Value>,
    /// budget 方言下必须显式给的 `max_tokens`；其余方言为 `None`
    pub max_tokens: Option<u64>,
    /// 实际选中的方言，写日志用
    pub dialect: ThinkingDialect,
}

impl ThinkingParams {
    /// 没有任何字段要发（`Off` 或方言为 `None`）。
    pub fn is_empty(&self) -> bool {
        self.extra.is_empty() && self.max_tokens.is_none()
    }
}

/// 把档位翻成这个 provider 认识的字段。
pub fn thinking_params(profile: &ProviderProfile<'_>, effort: &ThinkingEffort) -> ThinkingParams {
    let dialect = resolve_dialect(profile);
    let mut params = ThinkingParams {
        dialect,
        ..Default::default()
    };
    let on = !matches!(effort, ThinkingEffort::Off);

    match dialect {
        ThinkingDialect::None => {}
        ThinkingDialect::AnthropicEffort => {
            if on {
                params
                    .extra
                    .insert("thinking".to_string(), json!({ "type": "adaptive" }));
                params.extra.insert(
                    "output_config".to_string(),
                    json!({ "effort": effort_word(effort) }),
                );
            }
        }
        ThinkingDialect::AnthropicBudget => {
            if on {
                let (budget, max_tokens) = budget_ladder(effort);
                params.extra.insert(
                    "thinking".to_string(),
                    json!({ "type": "enabled", "budget_tokens": budget }),
                );
                params.max_tokens = Some(max_tokens);
            }
        }
        ThinkingDialect::OpenAiEffort => {
            if on {
                params
                    .extra
                    .insert("reasoning_effort".to_string(), json!(effort_word(effort)));
            }
        }
        ThinkingDialect::OpenRouterReasoning => {
            if on {
                params.extra.insert(
                    "reasoning".to_string(),
                    json!({ "effort": effort_word(effort) }),
                );
            }
        }
        ThinkingDialect::QwenEnableThinking => {
            params
                .extra
                .insert("enable_thinking".to_string(), json!(on));
            if on {
                params.extra.insert(
                    "thinking_budget".to_string(),
                    json!(budget_ladder(effort).0),
                );
            }
        }
        ThinkingDialect::ThinkingToggle => {
            let mode = if on { "enabled" } else { "disabled" };
            params
                .extra
                .insert("thinking".to_string(), json!({ "type": mode }));
        }
    }
    params
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 会话档位是进程级的，改它的用例必须串行跑。
    static SERIAL: Mutex<()> = Mutex::new(());

    fn profile<'a>(kind: ProviderKind, model: &'a str, host: &'a str) -> ProviderProfile<'a> {
        ProviderProfile {
            kind,
            model,
            base_url: if host.is_empty() { None } else { Some(host) },
            provider_name: None,
        }
    }

    fn params(kind: ProviderKind, model: &str, host: &str, effort: ThinkingEffort) -> Value {
        let p = profile(kind, model, host);
        Value::Object(thinking_params(&p, &effort).extra)
    }

    #[test]
    fn session_effort_wins_over_the_env_default() {
        let _guard = SERIAL.lock().unwrap();
        set_session_effort(&ThinkingEffort::High);
        assert_eq!(current_effort(), ThinkingEffort::High);
        set_session_effort(&ThinkingEffort::Off);
        assert_eq!(current_effort(), ThinkingEffort::Off);
    }

    #[test]
    fn effort_strings_parse_the_way_the_commands_spell_them() {
        assert_eq!(parse_effort("OFF"), Some(ThinkingEffort::Off));
        assert_eq!(parse_effort(" none "), Some(ThinkingEffort::Off));
        assert_eq!(parse_effort("medium"), Some(ThinkingEffort::Medium));
        assert_eq!(parse_effort("ultrathink"), Some(ThinkingEffort::High));
        assert_eq!(parse_effort("banana"), None);
    }

    #[test]
    fn newer_claude_takes_adaptive_thinking_plus_output_config_effort() {
        let body = params(
            ProviderKind::Anthropic,
            "claude-opus-4-6",
            "",
            ThinkingEffort::High,
        );
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["output_config"]["effort"], "high");
        assert!(body.get("budget_tokens").is_none());
    }

    #[test]
    fn older_claude_takes_a_budget_that_stays_under_max_tokens() {
        let p = profile(ProviderKind::Anthropic, "claude-sonnet-4-5-20250929", "");
        let got = thinking_params(&p, &ThinkingEffort::High);
        assert_eq!(got.dialect, ThinkingDialect::AnthropicBudget);
        let budget = got.extra["thinking"]["budget_tokens"].as_u64().unwrap();
        let max_tokens = got.max_tokens.expect("budget 方言必须显式给 max_tokens");
        assert!(budget >= 1024, "Anthropic 要求 budget_tokens 至少 1024");
        assert!(budget < max_tokens, "budget_tokens 必须小于 max_tokens");
        assert!(max_tokens <= 32_000, "老一代 Opus 4 的输出上限是 32000");
    }

    #[test]
    fn a_date_suffix_is_not_mistaken_for_a_minor_version() {
        // claude-opus-4-20250514 是 Opus 4 + 日期，不是 "Opus 4.20250514"，
        // 认成新模型就会发 output_config 然后 400。
        let p = profile(ProviderKind::Anthropic, "claude-opus-4-20250514", "");
        assert_eq!(resolve_dialect(&p), ThinkingDialect::AnthropicBudget);
        let newer = profile(ProviderKind::Anthropic, "claude-opus-4-5-20251101", "");
        assert_eq!(resolve_dialect(&newer), ThinkingDialect::AnthropicEffort);
    }

    #[test]
    fn claude_without_thinking_support_gets_no_parameters() {
        let p = profile(ProviderKind::Anthropic, "claude-3-5-sonnet-20241022", "");
        assert!(thinking_params(&p, &ThinkingEffort::High).is_empty());
    }

    #[test]
    fn openai_uses_snake_case_reasoning_effort_only_on_reasoning_models() {
        let body = params(ProviderKind::OpenAi, "o3-mini", "", ThinkingEffort::Medium);
        assert_eq!(body["reasoning_effort"], "medium");
        // 之前发的是 camelCase 的 reasoningEffort，OpenAI 根本不认。
        assert!(body.get("reasoningEffort").is_none());

        let plain = profile(ProviderKind::OpenAi, "gpt-4o", "");
        assert!(thinking_params(&plain, &ThinkingEffort::High).is_empty());
    }

    #[test]
    fn openrouter_gets_its_own_reasoning_object() {
        let body = params(
            ProviderKind::Compatible,
            "anthropic/claude-sonnet-4.6",
            "https://openrouter.ai/api/v1",
            ThinkingEffort::Low,
        );
        assert_eq!(body["reasoning"]["effort"], "low");
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn only_one_dialect_is_ever_sent() {
        // 回归点：老实现把 reasoningEffort + thinking + reasoning 三个字段
        // 一起塞进请求体，严格的服务端直接 400。
        let cases = [
            (ProviderKind::Anthropic, "claude-opus-4-6", ""),
            (ProviderKind::Anthropic, "claude-sonnet-4-20250514", ""),
            (ProviderKind::OpenAi, "gpt-5", ""),
            (ProviderKind::Compatible, "grok-4", "https://openrouter.ai"),
            (
                ProviderKind::Compatible,
                "qwen3-max",
                "https://dashscope.aliyuncs.com/compatible-mode/v1",
            ),
            (
                ProviderKind::Compatible,
                "glm-4.6",
                "https://open.bigmodel.cn/api/paas/v4",
            ),
            (
                ProviderKind::Compatible,
                "qwq-32b",
                "http://localhost:8000/v1",
            ),
        ];
        let knobs = [
            "thinking",
            "reasoning",
            "reasoning_effort",
            "enable_thinking",
        ];
        for (kind, model, host) in cases {
            let body = params(kind, model, host, ThinkingEffort::High);
            let hits = knobs.iter().filter(|k| body.get(**k).is_some()).count();
            assert_eq!(hits, 1, "{model} @ {host} 应当只发一种思考字段：{body}");
        }
    }

    #[test]
    fn off_stays_silent_on_endpoints_we_cannot_be_sure_about() {
        for (kind, model, host) in [
            (ProviderKind::Anthropic, "claude-opus-4-6", ""),
            (ProviderKind::Anthropic, "claude-sonnet-4-20250514", ""),
            (ProviderKind::OpenAi, "o3", ""),
            (ProviderKind::Compatible, "grok-4", "https://openrouter.ai"),
            (
                ProviderKind::Compatible,
                "qwq-32b",
                "http://localhost:8000/v1",
            ),
        ] {
            let p = profile(kind, model, host);
            assert!(
                thinking_params(&p, &ThinkingEffort::Off).is_empty(),
                "{model} @ {host}：Off 应当什么都不发"
            );
        }
    }

    #[test]
    fn vendors_with_a_documented_switch_can_actually_be_turned_off() {
        let qwen = params(
            ProviderKind::Compatible,
            "qwen3-max",
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
            ThinkingEffort::Off,
        );
        assert_eq!(qwen["enable_thinking"], false);
        assert!(qwen.get("thinking_budget").is_none());

        let glm = params(
            ProviderKind::Compatible,
            "glm-4.6",
            "https://open.bigmodel.cn/api/paas/v4",
            ThinkingEffort::Off,
        );
        assert_eq!(glm["thinking"]["type"], "disabled");
    }

    #[test]
    fn deepseek_official_has_no_knob_to_turn() {
        // 思不思考由模型名决定（deepseek-chat / deepseek-reasoner），
        // 多发字段只会 400。
        let p = profile(ProviderKind::DeepSeek, "deepseek-reasoner", "");
        assert!(thinking_params(&p, &ThinkingEffort::High).is_empty());
    }
}
