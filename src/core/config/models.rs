use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProviderConfig {
    pub providers: HashMap<String, ProviderSettings>,
    #[serde(alias = "activeProviderId")]
    pub active_provider_id: Option<String>,
    #[serde(alias = "activeModel")]
    pub active_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ModelConfig {
    pub name: Option<String>,
    #[serde(alias = "contextWindow")]
    pub context_window: Option<usize>,
    pub alias: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProviderSettings {
    #[serde(alias = "apiKey")]
    pub api_key: Option<String>,
    #[serde(alias = "baseUrl", alias = "baseURL")]
    pub base_url: Option<String>,
    #[serde(alias = "selectedModel", alias = "currentModel")]
    pub selected_model: Option<String>,
    pub models: Option<HashMap<String, ModelConfig>>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub r#type: Option<String>,
    /// 创建时刻的 Unix 秒，越新越靠前。`None` 是老条目（升级前就存在的配置），
    /// 排在后面。provider 列表用它把新加的 provider 顶到最前面。
    pub order: Option<u64>,
}

/// 一个 `model 文本` 输入解析出来的结果：`selected_model` 取第一个，其余进 `models`。
///
/// 返回 `None` 表示"用户什么都没填" —— 那是"完全交给 API 自动发现"，不是错误。
pub type ModelCandidates = Option<(String, HashMap<String, ModelConfig>)>;

/// 把逗号 / 换行 / 分号分隔的模型名解析成 `[首项, 其余…]`。
///
/// 只做机械清洗，不判断名字是否合法 —— 上游到底认不认这个名字只有请求能回答，
/// 在这里校验等于凭空拒绝用户。逐项 `trim`、丢空项、按首次出现去重。
pub fn split_model_names(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.split([',', '\n', '\r', ';', ' ']) {
        let name = raw.trim();
        if name.is_empty() {
            continue;
        }
        if !out.iter().any(|seen| seen == name) {
            out.push(name.to_string());
        }
    }
    out
}

impl ProviderSettings {
    /// 这个 provider 声明的候选模型，**按确定顺序**返回，第一个是默认项。
    ///
    /// 返回空列表 = 没有手工声明，完全交给 API 自动发现（模型面板会自己去拉）。
    ///
    /// # 为什么顺序必须是现算的
    ///
    /// `models` 是 `HashMap`，迭代顺序不确定。直接拿它当有序列表，模型面板每次
    /// 刷新换一次顺序 —— `provider_store::configured_provider_ids_from` 里已经踩过
    /// 同一个坑（`providers` 也是 `HashMap`）。所以顺序由"默认项优先 + 键名升序"
    /// 这条规则现算，任何写法（哪怕 `models` 写得毫无顺序）都得到稳定结果。
    ///
    /// # 为什么不用 Vec
    ///
    /// 加 `model_list: Vec<…>` 字段需要迁移所有已有配置，而 `models` + `selected_model`
    /// 已经在配置文件里了，只是 `models` 此前无人读。让它生效比再加一个字段划算得多。
    pub fn candidate_models(&self) -> Vec<String> {
        let default = self
            .selected_model
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty());

        // 键名升序 —— 字节序，确定性来源。`selected_model` 之外的按这个排。
        let mut rest: Vec<String> = self
            .models
            .as_ref()
            .map(|models| models.keys().cloned().collect())
            .unwrap_or_default();
        rest.sort();

        let mut out = Vec::with_capacity(rest.len() + 1);
        if let Some(default) = default {
            out.push(default.to_string());
        }
        out.extend(
            rest.into_iter()
                .filter(|name| Some(name.as_str()) != default),
        );
        out
    }

    /// 用一个 `model 文本` 输入装配 `selected_model` + `models`。
    ///
    /// - 空输入 → 两个字段都清成 `None`（自动发现）。
    /// - 恰好一个名字 → 只写 `selected_model`，**不造 `models` map**。单值场景保持
    ///   与旧配置一模一样的形状，避免把简单配置写复杂。
    /// - 多个名字 → 第一个当默认，其余进 `models`；`models` 里已存在的
    ///   `ModelConfig`（`context_window` / `alias` / `name`）**原样保留**，
    ///   不被默认值覆盖。
    pub fn set_candidate_models_from_text(&mut self, text: &str) {
        let names = split_model_names(text);
        self.selected_model = names.first().cloned();
        if names.len() <= 1 {
            self.models = None;
            return;
        }
        let models = self.models.get_or_insert_with(HashMap::new);
        // 只增不删：手写的 `context_window` / `alias` 是用户花力气配的，
        // 从文本里看不到这个信息，删掉等于静默丢配置。
        for name in names.iter().skip(1) {
            models.entry(name.clone()).or_default();
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub name: &'static str,
    pub max_tokens: u32,
    pub context_window: u32,
    pub supports_images: bool,
    pub supports_computer_use: bool,
    pub supports_prompt_cache: bool,
    pub input_price: f64,  // Price per million tokens
    pub output_price: f64, // Price per million tokens
    pub cache_write_price: Option<f64>,
    pub cache_read_price: Option<f64>,
    pub description: Option<&'static str>,
}

pub fn is_preview_model(_model: &str) -> bool {
    false
}

fn normalized_model_name(model_name: &str) -> String {
    let name = model_name.to_lowercase();
    if let Some(idx) = name.find('/') {
        name[idx + 1..].to_string()
    } else {
        name
    }
}

pub fn is_deepseek_reasoner_model(model_name: &str) -> bool {
    let clean_name = normalized_model_name(model_name);
    clean_name.starts_with("deepseek-r1") || clean_name.starts_with("deepseek-reasoner")
}

pub fn is_thinking_model(model_name: &str) -> bool {
    let clean_name = normalized_model_name(model_name);

    // DeepSeek models with reasoning capability
    if clean_name.starts_with("deepseek-r1")
        || clean_name.starts_with("deepseek-reasoner")
        || clean_name.starts_with("deepseek-coder-reasoner")
        || clean_name.starts_with("deepseek-v4")
    {
        return true;
    }

    // OpenAI o-series models (o1, o3, o4-mini, etc.)
    if clean_name.starts_with("o1") || clean_name.starts_with("o3") || clean_name.starts_with("o4")
    {
        return true;
    }

    // GPT-5 series with reasoning capability
    if clean_name.starts_with("gpt-5") {
        return true;
    }

    // Qwen reasoning models (QwQ, Qwen-QwQ)
    if clean_name.contains("qwq") || clean_name.contains("qwen-qwq") {
        return true;
    }

    // Kimi thinking models (Moonshot)
    if clean_name.starts_with("kimi") && clean_name.contains("thinking") {
        return true;
    }

    // Gemini thinking models
    if clean_name.starts_with("gemini") && clean_name.contains("thinking") {
        return true;
    }

    // Grok models with reasoning
    if clean_name.starts_with("grok-") {
        return true;
    }

    // Generic patterns — covers other thinking/reasoning models
    if clean_name.contains("thinking")
        || clean_name.contains("reasoner")
        || clean_name.contains("reasoning")
    {
        return true;
    }

    // OpenCode Zen：命名里没有 thinking/reasoning 关键词，但上游会回
    // reasoning_content（实测 space-bunny-free 思考 17*23 时返回了
    // "We need answer simple multiplication..."）。复用同一份判定，
    // 免得这里和 thinking_capability 各认一半模型。
    matches!(
        zen_thinking_capability(&clean_name),
        Some(ThinkingCapability::Granular) | Some(ThinkingCapability::Binary)
    )
}

/// Thinking capability levels for different models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingCapability {
    /// Supports granular thinking levels: Off, Low, Medium, High
    Granular,
    /// Supports only binary thinking: Off, On
    Binary,
    /// Does not support thinking/reasoning
    None,
}

/// Detect the thinking capability of a model based on its name.
pub fn thinking_capability(model_name: &str) -> ThinkingCapability {
    let clean_name = normalized_model_name(model_name);

    // Claude models — full granular thinking support (budget_tokens)
    if clean_name.starts_with("claude-") {
        return ThinkingCapability::Granular;
    }

    // OpenAI o-series — reasoning effort parameter (low/medium/high)
    if clean_name.starts_with("o1") || clean_name.starts_with("o3") || clean_name.starts_with("o4")
    {
        return ThinkingCapability::Granular;
    }

    // GPT-5 — granular reasoning
    if clean_name.starts_with("gpt-5") {
        return ThinkingCapability::Granular;
    }

    // DeepSeek reasoner models — binary thinking
    if clean_name.starts_with("deepseek-r1")
        || clean_name.starts_with("deepseek-reasoner")
        || clean_name.starts_with("deepseek-coder-reasoner")
        || clean_name.starts_with("deepseek-v4")
    {
        return ThinkingCapability::Binary;
    }

    // Qwen QwQ — binary thinking
    if clean_name.contains("qwq") || clean_name.contains("qwen-qwq") {
        return ThinkingCapability::Binary;
    }

    // Gemini thinking — binary
    if clean_name.starts_with("gemini") && clean_name.contains("thinking") {
        return ThinkingCapability::Binary;
    }

    // Kimi thinking — binary
    if clean_name.starts_with("kimi") && clean_name.contains("thinking") {
        return ThinkingCapability::Binary;
    }

    // Grok reasoning — binary
    if clean_name.starts_with("grok-") {
        return ThinkingCapability::Binary;
    }

    // Generic thinking/reasoner/keyword models — binary
    if clean_name.contains("thinking")
        || clean_name.contains("reasoner")
        || clean_name.contains("reasoning")
    {
        return ThinkingCapability::Binary;
    }

    // OpenCode Zen 的模型命名不带 thinking/reasoning 关键词，但上游确实会回
    // reasoning_content —— 实测 `space-bunny-free` 思考 "17*23" 时返回了
    // "We need answer simple multiplication..."。只靠关键词判定的话这些模型
    // 会被判成 None，思考面板只剩一个 Off，用户根本没法调档位。
    if let Some(cap) = zen_thinking_capability(&clean_name) {
        return cap;
    }

    ThinkingCapability::None
}

/// Zen 各模型族的思考能力。`None` = 该族不认识。
///
/// 判定依据是 Zen 官方模型表（`/zen/v1/models` 与文档端点列）：
/// - `gpt-5*` / `gpt-6*` / `muse-spark-*` 走 OpenAI Responses，档位由
///   reasoning_effort 控制 → Granular
/// - `deepseek-v4*` / `glm-5*` / `kimi-k*` / `minimax-m*` / `mistral-large*`
///   / `qwen3.*` 走 chat/completions，开关型 → Binary
/// - `*-free` 系列实测会回 reasoning_content → Binary
///
/// # 为什么要写这么窄
///
/// 不能只按家族前缀匹配。`starts_with("gpt-")` 会把不推理的 `gpt-4o` 判成可
/// 思考，`starts_with("qwen")` 会把 `qwen2.5-coder-32b-instruct` 判成可思考 ——
/// 两者都会让 UI 给出档位、让请求层发出 `reasoning_effort` / thinking 开关，
/// 而这些模型不认那些字段，直接 400。这里因此只认 Zen 实际提供的那些版本号。
///
/// 故意**不**在这里处理 `grok-*` 和 `claude-*`：它们已被上面的通用规则先一步
/// 判定（grok → Binary、claude → Granular），而那份判定对 xAI / Anthropic
/// 官方端点同样成立，Zen 复用即可，不必也不该在这里改写。
fn zen_thinking_capability(clean_name: &str) -> Option<ThinkingCapability> {
    if clean_name.starts_with("gpt-5")
        || clean_name.starts_with("gpt-6")
        || clean_name.starts_with("muse-spark-")
    {
        return Some(ThinkingCapability::Granular);
    }
    // qwen3 才是 Zen 上有思考的 qwen；qwen2.x / qwQ 由上面的通用规则处理。
    if clean_name.starts_with("deepseek-v4")
        || clean_name.starts_with("glm-5")
        || clean_name.starts_with("kimi-k")
        || clean_name.starts_with("minimax-m")
        || clean_name.starts_with("mistral-large")
        || clean_name.starts_with("qwen3")
    {
        return Some(ThinkingCapability::Binary);
    }
    // 免费层实测有思考输出（space-bunny-free 等）。
    if clean_name.ends_with("-free")
        || clean_name.contains("-free-")
        || clean_name.starts_with("big-pickle")
    {
        return Some(ThinkingCapability::Binary);
    }
    None
}

/// UI 是否该显示思考档位（对标 Claude Code 的 `modelSupportsEffort`）。
///
/// 启动阶段模型名还没解析出来（空串），先当作支持 —— 指示器早一点出现，用户才知道
/// 有这个档位可调；解析完成后这一格会跟着重画。
pub fn supports_thinking_ui(model_name: &str) -> bool {
    model_name.trim().is_empty()
        || !matches!(thinking_capability(model_name), ThinkingCapability::None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unresolved_model_name_still_shows_the_indicator() {
        // 启动阶段模型名是空串 —— 这时候藏起来，用户第一眼就看不到档位在哪
        assert!(supports_thinking_ui(""));
        assert!(supports_thinking_ui("   "));
    }

    #[test]
    fn thinking_models_show_the_indicator() {
        assert!(supports_thinking_ui("claude-opus-5"));
        assert!(supports_thinking_ui("deepseek-reasoner"));
        assert!(supports_thinking_ui("gpt-5"));
    }

    #[test]
    fn a_model_without_thinking_hides_the_indicator() {
        assert!(!supports_thinking_ui("gpt-4o"));
        assert!(!supports_thinking_ui("qwen2.5-coder-32b-instruct"));
    }

    /// Zen 的模型名不带 thinking/reasoning 关键词，但上游确实会思考 ——
    /// 只靠关键词判定会让思考面板只剩一个 Off，用户没法调档位。
    #[test]
    fn zen_models_that_reason_still_offer_the_effort_gears() {
        // 实测会回 reasoning_content 的免费模型。
        for m in [
            "space-bunny-free",
            "big-pickle",
            "exo-free",
            "mimo-v2.6-flash-free",
            "nemotron-3-ultra-free",
            "muse-spark-1.3-contributor-free",
        ] {
            assert!(
                supports_thinking_ui(m),
                "{m} reasons upstream, so the gear must be adjustable"
            );
        }

        // chat/completions 系：开关型。
        for m in [
            "deepseek-v4-pro",
            "glm-5.3",
            "kimi-k3",
            "minimax-m3",
            "qwen3.8-max",
        ] {
            assert_eq!(
                thinking_capability(m),
                ThinkingCapability::Binary,
                "model {m}"
            );
        }

        // Responses 系：reasoning_effort 精细档。
        for m in ["gpt-5.5", "gpt-6-astra", "muse-spark-1.3"] {
            assert_eq!(
                thinking_capability(m),
                ThinkingCapability::Granular,
                "model {m}"
            );
        }

        // grok / claude 走通用规则，不被 Zen 分支改写：那份判定对 xAI 官方
        // 端点同样成立，grok 是开关型（Alt+T 只切 Off/Medium）。
        assert_eq!(thinking_capability("grok-4.7"), ThinkingCapability::Binary);
        assert_eq!(
            thinking_capability("claude-opus-5-5"),
            ThinkingCapability::Granular
        );
    }

    /// `opencode/<model>` 这种带命名空间前缀的写法也要认出来。
    #[test]
    fn zen_models_are_recognized_through_their_namespace_prefix() {
        assert!(supports_thinking_ui("opencode/space-bunny-free"));
        assert_eq!(
            thinking_capability("opencode/gpt-5.5"),
            ThinkingCapability::Granular
        );
        assert_eq!(
            thinking_capability("opencode/deepseek-v4-pro"),
            ThinkingCapability::Binary
        );
    }

    /// is_thinking_model 决定请求层发不发 reasoning_effort，必须和
    /// thinking_capability 认同一批模型，否则面板给了档位却发不出去。
    #[test]
    fn the_runtime_hint_agrees_with_the_ui_capability() {
        for m in [
            "space-bunny-free",
            "deepseek-v4-pro",
            "glm-5.3",
            "gpt-5.5",
            "grok-4.7",
        ] {
            assert!(is_thinking_model(m), "{m} must be reported as reasoning");
            assert!(
                supports_thinking_ui(m),
                "{m}: UI and runtime detection must agree"
            );
        }
    }

    /// 没接入的协议不该被误判成可调 —— 选了也发不出去。
    #[test]
    fn a_made_up_model_name_is_still_none() {
        assert_eq!(
            thinking_capability("some-unknown-model"),
            ThinkingCapability::None
        );
        assert!(!is_thinking_model("some-unknown-model"));
    }

    // ---- candidate_models / set_candidate_models_from_text ----

    fn settings(default: Option<&str>, models: &[&str]) -> ProviderSettings {
        ProviderSettings {
            selected_model: default.map(str::to_string),
            models: (!models.is_empty()).then(|| {
                models
                    .iter()
                    .map(|m| (m.to_string(), ModelConfig::default()))
                    .collect()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn candidate_models_puts_the_default_first() {
        let s = settings(Some("deepseek-v4.1-flash"), &["glm-5.3", "deepseek-v4-pro"]);
        assert_eq!(
            s.candidate_models(),
            vec!["deepseek-v4.1-flash", "deepseek-v4-pro", "glm-5.3"]
        );
    }

    /// 顺序必须只取决于配置内容，不取决于 `HashMap` 的迭代顺序。
    /// `models` 是 HashMap，直接用它的键序会让面板每次刷新换一次排列。
    #[test]
    fn candidate_models_are_deterministically_ordered() {
        let s = settings(
            None,
            &["m9", "m1", "m5", "m3", "m7", "m2", "m8", "m4", "m6"],
        );
        let first = s.candidate_models();
        for _ in 0..20 {
            assert_eq!(s.candidate_models(), first, "顺序必须可复现");
        }
        assert_eq!(
            first,
            vec!["m1", "m2", "m3", "m4", "m5", "m6", "m7", "m8", "m9"]
        );
    }

    /// 默认项同时也在 `models` 里时，尾部不能重复出现它。
    #[test]
    fn candidate_models_skip_the_default_in_the_sorted_tail() {
        let s = settings(Some("b"), &["a", "b", "c"]);
        assert_eq!(s.candidate_models(), vec!["b", "a", "c"]);
    }

    /// 什么都没配 = 交给 API 自动发现，调用方靠空列表判断。
    #[test]
    fn candidate_models_is_empty_when_nothing_is_configured() {
        assert!(ProviderSettings::default().candidate_models().is_empty());
        // 空白串不算配了。
        assert!(settings(Some("   "), &[]).candidate_models().is_empty());
    }

    #[test]
    fn candidate_models_trims_the_default() {
        let s = settings(Some("  gpt-5  "), &[]);
        assert_eq!(s.candidate_models(), vec!["gpt-5"]);
    }

    #[test]
    fn split_model_names_accepts_commas_newlines_and_dedupes() {
        assert_eq!(
            split_model_names("a, b\nc\r\nd; e  a"),
            vec!["a", "b", "c", "d", "e"],
            "逗号/换行/分号/空格都算分隔，且按首次出现去重"
        );
        assert!(split_model_names("  \n , ; ").is_empty());
    }

    /// 单值场景保持与旧配置一模一样的形状：不为了"看起来完整"造一个单键 map。
    #[test]
    fn a_single_configured_model_writes_no_models_map() {
        let mut s = ProviderSettings::default();
        s.set_candidate_models_from_text("gpt-5");
        assert_eq!(s.selected_model.as_deref(), Some("gpt-5"));
        assert!(s.models.is_none(), "只配一个模型时不该造 models map");
    }

    #[test]
    fn multiple_names_split_into_default_plus_map() {
        let mut s = ProviderSettings::default();
        s.set_candidate_models_from_text("a, b, c");
        assert_eq!(s.selected_model.as_deref(), Some("a"));
        let keys: Vec<&String> = s.models.as_ref().unwrap().keys().collect();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&&"b".to_string()));
        assert!(keys.contains(&&"c".to_string()));
    }

    /// `ModelConfig` 里的 `context_window` / `alias` 是用户花力气配的，
    /// 文本里看不到这个信息 —— 重新提交表单不能静默抹掉。
    #[test]
    fn rewriting_the_model_text_preserves_hand_written_model_config() {
        let mut s = ProviderSettings::default();
        s.models = Some(
            [(
                "b".to_string(),
                ModelConfig {
                    name: Some("B 名称".to_string()),
                    context_window: Some(128_000),
                    alias: Some(vec!["bee".to_string()]),
                },
            )]
            .into_iter()
            .collect(),
        );
        s.set_candidate_models_from_text("a, b, c");
        let b = &s.models.as_ref().unwrap()["b"];
        assert_eq!(b.name.as_deref(), Some("B 名称"));
        assert_eq!(b.context_window, Some(128_000));
        assert_eq!(b.alias.as_ref().unwrap(), &vec!["bee".to_string()]);
    }

    /// 清空模型字段 = 回到"自动发现"，必须把 `models` 一起清掉，
    /// 否则删掉的那个名字会从 `models` 里复活。
    #[test]
    fn clearing_the_model_text_clears_both_fields() {
        let mut s = ProviderSettings::default();
        s.set_candidate_models_from_text("a, b, c");
        s.set_candidate_models_from_text("  ");
        assert!(s.selected_model.is_none());
        assert!(
            s.models.is_none(),
            "清空后 models 必须一起清，否则删掉的名字会复活"
        );
        assert!(s.candidate_models().is_empty());
    }

    /// 上一版把 Zen 的 gpt 规则写成 `starts_with("gpt-")`，把不推理的
    /// `gpt-4o` 也判成可思考 —— UI 会给出档位，请求层会发 reasoning_effort，
    /// 而 gpt-4o 收到该字段会 400。非推理的 gpt 必须继续判成 None。
    #[test]
    fn non_reasoning_gpt_models_stay_non_thinking() {
        for m in ["gpt-4o", "gpt-4o-mini", "gpt-3.5-turbo", "gpt-4-turbo"] {
            assert_eq!(
                thinking_capability(m),
                ThinkingCapability::None,
                "{m} does not reason, so it must not be offered effort gears"
            );
            assert!(!is_thinking_model(m), "{m}");
            assert!(!supports_thinking_ui(m), "{m}");
        }
    }

    /// 同理：Zen 只提供 qwen3.x，`qwen2.5-coder-32b-instruct` 这类不推理的
    /// qwen 不能因为"同属 qwen 家族"就被判成可思考。
    #[test]
    fn non_reasoning_qwen_models_stay_non_thinking() {
        for m in ["qwen2.5-coder-32b-instruct", "qwen2.5-72b-instruct"] {
            assert_eq!(
                thinking_capability(m),
                ThinkingCapability::None,
                "{m} does not reason"
            );
            assert!(!is_thinking_model(m), "{m}");
        }
    }
}
