use crate::agent::tool_routing::has_action_intent;
use std::collections::HashSet;
use std::sync::OnceLock;

/// 自动触发器类型——**同分时的**优先级（从高到低排列）。
///
/// # 排序规则改了：分数优先，kind 只做平手裁决
///
/// 以前这里是 `b.kind.cmp(&a.kind).then(b.score.cmp(&a.score))`——kind 是固定
/// 优先级枚举、每个 kind 又最多只 push 一个候选，于是 `score` **永远打不破任何
/// 平局**，`score_codebase_search_relevance` 那一整套关键词打分是死代码。
/// 现在 [`TriggerEval::score`] 是第一排序键（`RouteScore`，0..=100，跨 kind 可比），
/// kind 退化为第二键。两个工具抢同一个活儿时，分数高的赢；只有真的打平时
/// 才按 Verification > CodebaseSearch > … 的安全序兜底。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum AutoTriggerKind {
    /// 验证（编辑后必须检查，语义上最硬）
    Verification = 10,
    /// 语义搜索
    CodebaseSearch = 5,
    /// JSON回退（已检测到行动意图但无工具调用）
    JsonFallback = 4,
    /// 编辑器技能（最后手段——用于编辑类任务）
    EditorSkill = 3,
    /// 导航器技能（需要追踪依赖）
    NavigatorSkill = 2,
    /// 分析器技能（需要结构化分析）
    AnalyzerSkill = 1,
    /// 项目地图（结构概览，信息密度最低）
    ProjectMap = 0,
}

/// 触发器评估结果。
#[derive(Debug, Clone)]
pub(crate) struct TriggerEval {
    pub kind: AutoTriggerKind,
    /// 0-100 的路由分，**跨 kind 可比**，是 `select_best_auto_trigger` 的主排序键。
    pub score: u32,
    pub reason: &'static str,
    /// 人类可读的判分依据，写进 debug log——这是唯一能回答"它到底为什么触发/
    /// 为什么没触发"的地方。eval 基座已删（见 docs/starlens-design.md §12 末），
    /// 所以这行日志是当下唯一的可观测面，别省。
    pub detail: String,
}

/// 三个自动触发开关的取值。
///
/// 把它们从 [`select_best_auto_trigger`] 的参数里抽出来，是为了能在测试里直接指定
/// 组合——以前三个开关各自散在函数体里读环境变量，测试只能依赖进程环境（Rust 单测
/// 并行跑 + `OnceLock` 缓存，基本没法测），于是路由逻辑长期处于"看起来在跑、
/// 实际没被测过"的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RouteFlags {
    pub auto_codebase_search: bool,
    pub auto_skill_fallbacks: bool,
    pub json_fallback: bool,
}

impl RouteFlags {
    pub(crate) fn from_env() -> Self {
        Self {
            auto_codebase_search: auto_codebase_search_enabled(),
            auto_skill_fallbacks: auto_skill_fallbacks_enabled(),
            json_fallback: json_fallback_enabled(),
        }
    }
}

/// 评估所有可用的自动触发器，返回**分数最高**的那个。
///
/// `verification_required` 走短路：编辑后必须验证，不是打分能压过去的。
pub(crate) fn select_best_auto_trigger(
    verification_required: bool,
    skip_verification: bool,
    user_input: &str,
    current_content: &str,
    active_tools: &HashSet<String>,
    codebase_search_attempted: bool,
    navigator_skill_attempted: bool,
    analyzer_skill_attempted: bool,
    editor_skill_attempted: bool,
    project_map_attempted: bool,
) -> Option<TriggerEval> {
    select_best_auto_trigger_with_flags(
        verification_required,
        skip_verification,
        user_input,
        current_content,
        active_tools,
        codebase_search_attempted,
        navigator_skill_attempted,
        analyzer_skill_attempted,
        editor_skill_attempted,
        project_map_attempted,
        RouteFlags::from_env(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn select_best_auto_trigger_with_flags(
    verification_required: bool,
    skip_verification: bool,
    user_input: &str,
    current_content: &str,
    active_tools: &HashSet<String>,
    codebase_search_attempted: bool,
    navigator_skill_attempted: bool,
    analyzer_skill_attempted: bool,
    editor_skill_attempted: bool,
    project_map_attempted: bool,
    flags: RouteFlags,
) -> Option<TriggerEval> {
    let mut candidates: Vec<TriggerEval> = Vec::new();

    // 验证触发器 — 短路：安全约束优先于任何相关性打分
    if verification_required && !skip_verification {
        candidates.push(TriggerEval {
            kind: AutoTriggerKind::Verification,
            score: 100,
            reason: "edits_detected_requires_verification",
            detail: "verification_required && !skip_verification".to_string(),
        });
    }

    // 语义搜索 — 只有真的像"跨文件排序定位"时才进候选
    if let Some(score) = score_rank_locate_route(user_input) {
        if should_trigger_codebase_search_with_flag(
            user_input,
            current_content,
            active_tools,
            codebase_search_attempted,
            flags.auto_codebase_search,
        ) {
            candidates.push(TriggerEval {
                kind: AutoTriggerKind::CodebaseSearch,
                score,
                reason: "ranked_cross_file_locate_shape",
                detail: describe_rank_locate_route(user_input),
            });
        }
    }

    // JSON回退 — 检测到行动意图但模型没返回工具调用
    if flags.json_fallback && has_action_intent(current_content) {
        candidates.push(TriggerEval {
            kind: AutoTriggerKind::JsonFallback,
            score: 40,
            reason: "action_intent_without_tool_calls",
            detail: "has_action_intent(current_content)".to_string(),
        });
    }

    // 导航器技能 — 需要代码结构理解
    if should_trigger_navigator_skill_with_flag(
        user_input,
        current_content,
        active_tools,
        codebase_search_attempted,
        navigator_skill_attempted,
        flags.auto_skill_fallbacks,
    ) {
        candidates.push(TriggerEval {
            kind: AutoTriggerKind::NavigatorSkill,
            score: 30,
            reason: "dependency_tracing_needed",
            detail: "downstream of codebase_search".to_string(),
        });
    }

    // 项目地图 — 需要项目结构总览
    if should_trigger_project_map_with_flag(
        user_input,
        current_content,
        active_tools,
        project_map_attempted,
        flags.auto_skill_fallbacks,
    ) {
        candidates.push(TriggerEval {
            kind: AutoTriggerKind::ProjectMap,
            score: 20,
            reason: "project_overview_needed",
            detail: "downstream of auto_skill_fallbacks".to_string(),
        });
    }

    // 分析器技能 — 需要结构化分析
    if should_trigger_analyzer_skill_with_flag(
        user_input,
        current_content,
        active_tools,
        project_map_attempted,
        analyzer_skill_attempted,
        flags.auto_skill_fallbacks,
    ) {
        candidates.push(TriggerEval {
            kind: AutoTriggerKind::AnalyzerSkill,
            score: 10,
            reason: "broad_analysis_needed",
            detail: "downstream of project_map".to_string(),
        });
    }

    // 编辑器技能 — 最后手段
    if should_trigger_editor_skill_with_flag(
        user_input,
        current_content,
        active_tools,
        codebase_search_attempted,
        navigator_skill_attempted,
        editor_skill_attempted,
        flags.auto_skill_fallbacks,
    ) {
        candidates.push(TriggerEval {
            kind: AutoTriggerKind::EditorSkill,
            score: 5,
            reason: "edit_context_insufficient",
            detail: "downstream of codebase_search + navigator".to_string(),
        });
    }

    // 分数优先，kind 只在同分时兜底（见 AutoTriggerKind 的文档）。
    candidates.sort_by(|a, b| b.score.cmp(&a.score).then(b.kind.cmp(&a.kind)));

    candidates.into_iter().next()
}

// ── 「跨文件排序定位」路由打分 ─────────────────────────────────────────────
//
// 这是 `ranked-cross-file-locate` 槽（`core::prompts::routing_matrix`）在
// **自动触发路径**上的判分器。描述文件告诉模型"什么时候自己调"，这里决定
// "模型不说话时我们替它调不调"——两边必须指向同一个活儿，否则又是一次错配。
//
// 判的是**句子的形状**，不是主题词：主题词永远是代码里的词，我们看不到。

/// 分数低于这个值的候选根本不进 `candidates`。
fn route_score_threshold() -> u32 {
    static THRESHOLD: OnceLock<u32> = OnceLock::new();
    *THRESHOLD.get_or_init(|| {
        std::env::var("STARCODE_ROUTE_SCORE_MIN")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(60)
    })
}

/// 归一化：小写 + 去掉冠词，让 `all the places` 能匹配上 `all places` 这类标记。
///
/// 上一版按原样子串匹配，于是 "all the places that build the prompt" 不含
/// "all places" 而漏判——标记词表写得再全也会被一个冠词击穿。
fn normalize_for_markers(input: &str) -> String {
    let lower = input.to_lowercase();
    let mut out: Vec<&str> = Vec::new();
    for token in lower.split_whitespace() {
        // 冠词一律摘掉：它出现在标记词前面只会挡住匹配（"all **the** places" 匹配不上
        // `all place`），而在别处对判分也没有判别力。
        if matches!(token, "the" | "a" | "an") {
            continue;
        }
        out.push(token);
    }
    out.join(" ")
}

/// 判分词表。提到模块级是因为**打分**和**解释**必须用同一份——
/// 以前"看起来在路由"的打分函数和真正的谓词是各写各的，现在两者共用下列常量。
///
/// 注意这些都跑在 [`normalize_for_markers`] 的输出上：冠词已被摘掉，
/// 因此这里写 `all places` 就能命中 "all the places"。
const SWEEP_MARKERS: &[&str] = &[
    // ── 跨文件定位：问的是"在哪 / 有哪些地方" ──
    "where is",
    "where are",
    "where does",
    "where do",
    "where did",
    "where can i find",
    "where should i",
    "which file",
    "what file",
    "all place",
    "every place",
    "across repo",
    "across codebase",
    "whole codebase",
    "how does",
    "how do",
    "how is",
    "how are",
    // 中文：「在哪 / 哪里 / 哪些」
    "在哪",
    "哪里",
    "哪儿",
    "何处",
    "哪些",
    "几个地方",
    "都哪里",
];

/// 「找的是干这件事的地方，而不是某个符号」的功能动词。
const FUNCTIONAL_VERBS: &[&str] = &[
    "handle",
    "implement",
    "responsible",
    "responsibility",
    "process",
    "managed",
    "enforced",
    "wired",
    "plumbed",
    "configured",
    "处理",
    "实现",
    "负责",
    "相关",
    "逻辑",
];

/// 概念问答：**词法引擎结构上答不了**的形状。
///
/// 这些不是"分数低"，而是"检索必然零命中"——提问里的词一个都不会出现在代码里
///（"为什么" "架构是什么" "解释一下"）。硬否决，不参与打分。
const CONCEPTUAL_MARKERS: &[&str] = &[
    "why ",
    "why?",
    "explain",
    "what is the architecture",
    "what's the architecture",
    "understand the",
    "make sense",
    "是什么",
    "为什么",
    "怎么回事",
    "怎么理解",
    "解释",
    "原理",
    "架构是",
    "凭什么",
];

/// 判断「像不像跨文件排序定位」，返回 0-100；不适用返回 `None`。
///
/// # 为什么不是"加分求和"
///
/// 上一版是 `基准20 + 扫面35 + 疑问15 + 动词15 + 多词10 − 精确55 − 点名45`，
/// 结果**最典型的措辞正好卡在阈值下面**：`where is retry handled` = 20+35 = 55，
/// 阈值 60 → 不触发。也就是说那个打分器的主要作用是漏判，而不是拦住不该触发的。
/// 而且标记词按原样子串匹配，"all the places" 匹配不上 `all places`。
///
/// 现在改成 **硬否决 + 分档**：先排除结构上不该管的，剩下的按证据强度落到两档之一。
/// 档位（75 / 62）离阈值（60）都有富余，任何单条规则的波动都不会把常见问法打出界。
/// 加成只用于同档内排序，不参与"够不够格"的判定。
fn score_rank_locate_route(user_input: &str) -> Option<u32> {
    let raw = user_input.trim();
    if raw.is_empty() {
        return None;
    }
    let lower = raw.to_lowercase();
    let norm = normalize_for_markers(raw);

    // ── 一、硬否决：这些有更合适的工具，或这个引擎结构上答不了 ──
    // 用户点名了别的工具：他指定了，别替他改。
    if names_another_tool(&lower) {
        return None;
    }
    // 精确标识符形状（`::`、路径、glob、正则、snake_case）→ Grep / Glob / CodeGraph。
    if has_exact_identifier_shape(raw) {
        return None;
    }
    // 纯概念问答 → 词法检索必然零命中，白烧一个回合。
    if CONCEPTUAL_MARKERS.iter().any(|m| lower.contains(m)) {
        return None;
    }

    let is_question = raw.contains('?') || raw.contains('？');
    let has_sweep = SWEEP_MARKERS.iter().any(|m| norm.contains(m));
    let has_verb = FUNCTIONAL_VERBS.iter().any(|v| norm.contains(v));
    let word_count = raw
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != ':' && c != '.')
        .filter(|w| w.chars().count() >= 3)
        .count();

    // ── 二、分档 ──
    // 每档给一个远高于阈值的分：`STARCODE_ROUTE_SCORE_MIN` 仍可整体调高/调低，
    // 但档内不会因为某条规则差几分就掉出触发范围。
    let mut score: u32 = if has_sweep {
        // 一档：明确的跨文件定位问法。这一个信号就够，不必再叠加别的。
        75
    } else if is_question && has_verb {
        // 二档：问句 + 功能动词（"限流怎么处理？"）——同样是"找干这件事的地方"。
        62
    } else {
        // 三档：证据不足，不够格。宁可漏一次，也不要在无关提问上白烧一轮检索。
        return None;
    };

    // 加成只用来在同一档内排序，不参与"够不够格"的判定。
    if is_question && !has_sweep {
        score += 5;
    }
    if word_count >= 3 {
        score += 5;
    }

    let score = score.min(100);
    (score >= route_score_threshold()).then_some(score)
}

/// 这句话里是否含"精确 token"——是的话 Grep 才是对的那个工具。
fn has_exact_identifier_shape(raw: &str) -> bool {
    // glob / 正则 / 路径
    if raw.contains("::")
        || raw.contains('/')
        || raw.contains('*')
        || raw.contains('\\')
        || raw.contains(".*")
        || raw.contains("^")
        || raw.contains("$")
    {
        return true;
    }
    // snake_case 或 CamelCase 标识符（要求长度 ≥ 8，避免 "codebase_search" 之外的
    // 普通下划线词误判成标识符 —— 下划线词常常本身就是关键词）
    raw.split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|w| {
            w.chars().count() >= 8
                && ((w.contains('_') && w.chars().any(|c| c.is_uppercase()))
                    || (w.chars().any(|c| c.is_uppercase())
                        && w.chars().any(|c| c.is_lowercase())
                        && !w.chars().all(|c| c.is_alphanumeric())))
        })
}

/// 用户是否点名了别的工具——他指定了，就别替他改。
fn names_another_tool(lower: &str) -> bool {
    const TOOL_WORDS: &[&str] = &[
        "grep",
        "glob",
        "read ",
        " codebase_search",
        "codegraph",
        "projectmap",
        "sed ",
        "awk ",
    ];
    TOOL_WORDS.iter().any(|t| lower.contains(t))
}

/// 把判分依据写成人能读的一句话，进 debug log。
///
/// 这是当下唯一能回答"它为什么触发 / 为什么没触发"的出口——eval 基座已随
/// eval 体系删除（见 docs/starlens-design.md §12 末），所以每条判定都必须可被人复核。
/// **被否决的输入要报出否决理由**，否则排障时只看到 "no strong signal"，
/// 分不清是词表没覆盖还是被硬否决拒了。
fn describe_rank_locate_route(user_input: &str) -> String {
    let lower = user_input.to_lowercase();
    let norm = normalize_for_markers(user_input);

    // 先报否决原因。
    if names_another_tool(&lower) {
        return format!("veto: named another tool => [{lower}]");
    }
    if has_exact_identifier_shape(user_input) {
        return format!("veto: exact-token shape (Grep/Glob/CodeGraph owns this) => [{lower}]");
    }
    if let Some(m) = CONCEPTUAL_MARKERS.iter().find(|m| lower.contains(*m)) {
        return format!(
            "veto: conceptual QA ({m:?}) — lexical search cannot answer it => [{lower}]"
        );
    }

    let mut why: Vec<String> = Vec::new();
    if let Some(m) = SWEEP_MARKERS.iter().find(|m| norm.contains(**m)) {
        // 报出具体命中的标记：调词表时要知道是哪一条在起作用。
        why.push(format!("sweep({m:?})"));
    }
    if user_input.contains('?') || user_input.contains('？') {
        why.push("question".to_string());
    }
    if FUNCTIONAL_VERBS.iter().any(|v| norm.contains(*v)) {
        why.push("functional-verb".to_string());
    }
    if why.is_empty() {
        why.push(
            "insufficient signal (need sweep phrasing, or question + functional verb)".to_string(),
        );
    }
    format!("{} => [{lower}]", why.join(" + "))
}

/// 自动触发闸门。
///
/// 注意这次 `_user_input` **不再是下划线参数** —— 以前它带下划线是因为谓词压根不看
/// 提问内容（真表达式只有 `enabled && !attempted && tool_present`），于是"概念性查询
/// 检测"是一句空话，开关一打开就是每轮盲打。现在判分在
/// [`score_rank_locate_route`] 里做，这里只做廉价的旁路检查。
fn should_trigger_codebase_search_with_flag(
    _user_input: &str,
    _current_content: &str,
    active_tools: &HashSet<String>,
    already_attempted: bool,
    auto_enabled: bool,
) -> bool {
    auto_enabled && !already_attempted && active_tools.contains("CodebaseSearch")
}

/// 检查是否应该预取语义搜索
pub(crate) fn should_prefetch_codebase_search(
    user_input: &str,
    active_tools: &HashSet<String>,
    history_len: usize,
) -> bool {
    should_prefetch_codebase_search_with_flag(
        user_input,
        active_tools,
        history_len,
        first_turn_prefetch_enabled(),
    )
}

/// 首轮预取：只有在首轮、开关开着、且提问**确实像**跨文件排序定位时才预取。
///
/// 以前 `user_input` 同样没被读，`history_len == 0` 就无条件预取一遍——那不是预取，
/// 那是在每次新会话的开头强塞一次昂贵的全仓搜索。
fn should_prefetch_codebase_search_with_flag(
    user_input: &str,
    active_tools: &HashSet<String>,
    history_len: usize,
    prefetch_enabled: bool,
) -> bool {
    prefetch_enabled
        && history_len == 0
        && active_tools.contains("CodebaseSearch")
        && score_rank_locate_route(user_input).is_some()
}

/// 检查是否应该预取项目地图
pub(crate) fn should_prefetch_project_map(
    user_input: &str,
    active_tools: &HashSet<String>,
    history_len: usize,
) -> bool {
    should_prefetch_project_map_with_flag(
        user_input,
        active_tools,
        history_len,
        first_turn_prefetch_enabled(),
    )
}

fn should_prefetch_project_map_with_flag(
    user_input: &str,
    active_tools: &HashSet<String>,
    history_len: usize,
    prefetch_enabled: bool,
) -> bool {
    prefetch_enabled
        && history_len == 0
        && active_tools.contains("ProjectMap")
        && looks_like_overview_question(user_input)
}

/// 「这个仓大概是什么样」——ProjectMap 预取的唯一正当理由。
fn looks_like_overview_question(user_input: &str) -> bool {
    let lower = user_input.to_lowercase();
    const MARKERS: &[&str] = &[
        "overview",
        "structure",
        "layout",
        "what is this repo",
        "what's this repo",
        "get started",
        "onboard",
        "familiar",
        "big picture",
        "high level",
        "架构",
        "结构",
        "概览",
        "这个项目",
        "这个仓库",
        "上手",
    ];
    MARKERS.iter().any(|m| lower.contains(m))
}

/// 检查是否应该触发导航技能
fn should_trigger_navigator_skill_with_flag(
    _user_input: &str,
    _current_content: &str,
    active_tools: &HashSet<String>,
    codebase_search_attempted: bool,
    already_attempted: bool,
    auto_enabled: bool,
) -> bool {
    auto_enabled
        && !already_attempted
        && codebase_search_attempted
        && active_tools.contains("skill")
}

/// 检查是否应该触发分析器技能
fn should_trigger_analyzer_skill_with_flag(
    _user_input: &str,
    _current_content: &str,
    active_tools: &HashSet<String>,
    project_map_attempted: bool,
    already_attempted: bool,
    auto_enabled: bool,
) -> bool {
    auto_enabled && !already_attempted && project_map_attempted && active_tools.contains("skill")
}

/// 检查是否应该触发项目地图
fn should_trigger_project_map_with_flag(
    user_input: &str,
    _current_content: &str,
    active_tools: &HashSet<String>,
    already_attempted: bool,
    auto_enabled: bool,
) -> bool {
    auto_enabled
        && !already_attempted
        && active_tools.contains("ProjectMap")
        && looks_like_overview_question(user_input)
}

/// 检查是否应该触发编辑器技能
fn should_trigger_editor_skill_with_flag(
    _user_input: &str,
    _current_content: &str,
    active_tools: &HashSet<String>,
    codebase_search_attempted: bool,
    navigator_skill_attempted: bool,
    already_attempted: bool,
    auto_enabled: bool,
) -> bool {
    auto_enabled
        && !already_attempted
        && codebase_search_attempted
        && navigator_skill_attempted
        && active_tools.contains("skill")
}

/// 检查是否启用第一轮预取
fn first_turn_prefetch_enabled() -> bool {
    bool_env_flag("STAR_ENABLE_FIRST_TURN_PREFETCH", false)
}

/// 检查是否启用动态上下文第一轮
pub(crate) fn dynamic_context_first_turn_enabled() -> bool {
    bool_env_flag("STAR_DYNAMIC_CONTEXT_FIRST_TURN", false)
}

/// 自动语义搜索闸门，**默认开**。
///
/// 以前默认 `false`，而 `should_trigger_codebase_search_with_flag` 又不看提问内容——
/// 也就是说这套机制唯一的实际状态就是"永远关着"，整条 skill 回退梯（navigator 依赖
/// `codebase_search_attempted`、editor 依赖 navigator）因此全是死代码。现在谓词会
/// 真的判分（[`score_rank_locate_route`]），默认开着才是安全的。
/// 需要恢复旧行为时 `STAR_ENABLE_AUTO_CODEBASE_SEARCH=0`。
fn auto_codebase_search_enabled() -> bool {
    bool_env_flag("STAR_ENABLE_AUTO_CODEBASE_SEARCH", true)
}

/// 检查是否启用自动技能回退
fn auto_skill_fallbacks_enabled() -> bool {
    bool_env_flag("STAR_ENABLE_AUTO_SKILL_FALLBACKS", false)
}

/// 检查是否启用 JSON 回退
pub(crate) fn json_fallback_enabled() -> bool {
    bool_env_flag("STARCODE_JSON_FALLBACK", false)
}

/// 读取布尔环境变量标志
fn bool_env_flag(key: &str, default: bool) -> bool {
    match key {
        "STAR_ENABLE_FIRST_TURN_PREFETCH" => {
            static VALUE: OnceLock<bool> = OnceLock::new();
            *VALUE.get_or_init(|| read_bool_env_flag(key, default))
        }
        "STAR_DYNAMIC_CONTEXT_FIRST_TURN" => {
            static VALUE: OnceLock<bool> = OnceLock::new();
            *VALUE.get_or_init(|| read_bool_env_flag(key, default))
        }
        "STAR_ENABLE_AUTO_CODEBASE_SEARCH" => {
            static VALUE: OnceLock<bool> = OnceLock::new();
            *VALUE.get_or_init(|| read_bool_env_flag(key, default))
        }
        "STAR_ENABLE_AUTO_SKILL_FALLBACKS" => {
            static VALUE: OnceLock<bool> = OnceLock::new();
            *VALUE.get_or_init(|| read_bool_env_flag(key, default))
        }
        "STARCODE_JSON_FALLBACK" => {
            static VALUE: OnceLock<bool> = OnceLock::new();
            *VALUE.get_or_init(|| read_bool_env_flag(key, default))
        }
        _ => read_bool_env_flag(key, default),
    }
}

/// 读取布尔环境变量标志
fn read_bool_env_flag(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    /// 测试用开关：语义搜索开、技能回退开、JSON 回退开。
    /// 真实取值走 [`RouteFlags::from_env`]，与测试无关。
    const FLAGS_ON: RouteFlags = RouteFlags {
        auto_codebase_search: true,
        auto_skill_fallbacks: true,
        json_fallback: true,
    };

    const FLAGS_OFF: RouteFlags = RouteFlags {
        auto_codebase_search: false,
        auto_skill_fallbacks: false,
        json_fallback: false,
    };

    /// 跨文件排序定位的**正向**样本：应该进候选。
    ///
    /// 这批样本就是"判定集"里 codebase_search 的那一类。它锁的是**路由契约**
    /// （这类问题归 CodebaseSearch），不测量真实模型行为——eval 基座已删，
    /// 真正的行为度量得先重建评测基座，见 docs/starlens-design.md §12 末。
    #[test]
    fn cross_file_locate_questions_score_above_threshold() {
        for q in [
            "where is retry handled?",
            "where are rate limits enforced?",
            "which files touch the config parsing?",
            "how does the compaction flow work across the codebase?",
            "配置解析的代码在哪些文件里？",
            "所有用了 cache 的地方都在哪？",
        ] {
            assert!(
                score_rank_locate_route(q).is_some(),
                "应该被判为跨文件排序定位，却没进候选：{q:?}  detail={}",
                describe_rank_locate_route(q)
            );
        }
    }

    /// 回归：上一版是 `基准20 + 扫面35 + …` 的加法，阈值 60。
    /// 于是**最典型的措辞正好卡在阈值下面**：
    /// `where is retry handled` = 20 + 35 = 55 → 不触发。
    /// 也就是说那个打分器的主要作用是漏判，而不是拦住不该触发的。
    ///
    /// 这批样本全部**不带问号、不带冠词以外的加分**，是"有需要就触发"这条要求
    /// 的最低门槛。任何一条掉出去，都意味着最常见的问法用不上这个工具。
    #[test]
    fn canonical_locate_phrasings_all_trigger() {
        for q in [
            // 无问号的最简形式
            "where is retry handled",
            "where is auth",
            "where is retry",
            "where are rate limits enforced",
            "where does compaction get triggered",
            "where do we validate user input",
            "where is the tool routing logic",
            // 冠词把标记词打断的情况（"all the places" vs 标记 "all places"）
            "all the places that build the prompt",
            "every place that touches the filesystem",
            // 同义问法
            "how does the watcher work",
            "how do we handle cancellation",
            "across the codebase, where is retry handled",
            // 中文
            "哪里处理限流",
            "限流在哪里处理",
            "配置解析在哪些文件",
            "错误处理在哪里处理",
        ] {
            assert!(
                score_rank_locate_route(q).is_some(),
                "最典型的定位问法漏判了：{q:?}  detail={}",
                describe_rank_locate_route(q)
            );
        }
    }

    /// 触发也不能变成"什么都触发"——证据不足的短陈述句不该烧一轮检索。
    #[test]
    fn weak_signals_do_not_trigger() {
        for q in [
            "retry",
            "fix it",
            "update the config",
            "rate limiting",
            "把这个跑一下",
            "优化性能",
        ] {
            assert!(
                score_rank_locate_route(q).is_none(),
                "证据不足，不该触发：{q:?}  detail={}",
                describe_rank_locate_route(q)
            );
        }
    }

    /// 归一化必须真的摘掉冠词，否则标记词表会被一个 "the" 击穿。
    #[test]
    fn markers_survive_articles() {
        assert_eq!(normalize_for_markers("all the places"), "all places");
        assert_eq!(normalize_for_markers("in a repo"), "in repo");
        assert_eq!(normalize_for_markers("where is x"), "where is x");
        assert_eq!(normalize_for_markers("the where is x"), "where is x");
    }

    /// 精确 token 的**反向**样本：Grep / Glob / CodeGraph 才是对的那个工具。
    ///
    /// 这一条是本次修复的核心守卫：以前谓词根本不读提问内容，打开开关就对
    /// 这些问题也盲打一发。
    #[test]
    fn exact_token_questions_do_not_score_above_threshold() {
        for q in [
            "grep for parse_toml_config",
            "find src/**/*.rs",
            "where is crate::core::tools::codebase_search::search_codebase defined?",
            "run cargo test for the triggers module",
            // 用户点名了别的工具：他指定了，别替他改。
            "use glob to find where the config files are",
            "grep 一下 token budget 的实现",
        ] {
            assert!(
                score_rank_locate_route(q).is_none(),
                "精确 token / 点名别的工具，不该触发排序定位：{q:?}  detail={}",
                describe_rank_locate_route(q)
            );
        }
    }

    /// 概念问答是**硬否决**，不是"分数低"。
    ///
    /// 这些提问里的词一个都不会出现在代码里（为什么 / 是什么 / 解释），
    /// 词法检索必然零命中 —— 触发它只是白烧一个回合。
    /// 注意 `token 预算是怎么算的` 带问号也进不来，正因为否决发生在打分之前。
    #[test]
    fn conceptual_qa_is_vetoed_not_merely_low_scored() {
        for q in [
            "why does the watcher use a debounce",
            "explain the tool routing architecture",
            "为什么这里用 OnceLock",
            "架构是怎样的",
            "解释一下 compaction",
        ] {
            let detail = describe_rank_locate_route(q);
            assert!(
                score_rank_locate_route(q).is_none(),
                "概念问答不该路由到词法引擎：{q:?}  detail={detail}"
            );
            assert!(
                detail.starts_with("veto:"),
                "被拒的 query 必须在 detail 里说明否决理由，否则没法排障：{detail}"
            );
        }
    }

    #[test]
    fn empty_input_never_scores() {
        assert!(score_rank_locate_route("").is_none());
        assert!(score_rank_locate_route("   ").is_none());
    }

    /// 回归：分数必须真的影响选择。
    ///
    /// 以前 `sort_by(|a,b| b.kind.cmp(&a.kind).then(b.score.cmp(&a.score)))` 里 kind 是
    /// 唯一键、每个 kind 又只出现一次，于是 score 恒为死字段。
    ///
    /// 这里造两个能同时进候选的：AnalyzerSkill（分 10、kind=1）与 EditorSkill
    /// （分 5、kind=3）。**kind 更低的那个必须赢**——旧排序会选 EditorSkill。
    #[test]
    fn score_beats_kind_priority() {
        let picked = select_best_auto_trigger_with_flags(
            false,
            false,
            "随便一句",
            "",
            &tools(&["skill"]),
            true,  // codebase_search_attempted → EditorSkill 的前置满足
            true,  // navigator_skill_attempted → EditorSkill 的前置满足
            false, // analyzer 未尝试
            false, // editor 未尝试
            true,  // project_map_attempted → AnalyzerSkill 的前置满足
            FLAGS_ON,
        );
        let picked = picked.expect("两个候选都该进池");
        assert_eq!(
            picked.kind,
            AutoTriggerKind::AnalyzerSkill,
            "分数 10 必须压过 kind 更高的分数 5：{picked:?}"
        );
        assert_eq!(picked.score, 10);
    }

    /// 回归：编辑后必须验证，短路优先级不能被分数压过去。
    #[test]
    fn verification_short_circuits_scoring() {
        let picked = select_best_auto_trigger_with_flags(
            true,
            false,
            "where is retry handled?",
            "",
            &tools(&["CodebaseSearch"]),
            false,
            false,
            false,
            false,
            false,
            FLAGS_ON,
        );
        assert_eq!(
            picked.as_ref().map(|p| p.kind),
            Some(AutoTriggerKind::Verification)
        );
        assert_eq!(picked.map(|p| p.score), Some(100));
    }

    /// 强扫面信号 + 模型沉默 → 选 CodebaseSearch，且带可解释的 detail。
    #[test]
    fn strong_sweep_signal_selects_codebase_search() {
        let picked = select_best_auto_trigger_with_flags(
            false,
            false,
            "where is retry handled across the codebase?",
            "",
            &tools(&["CodebaseSearch"]),
            false,
            false,
            false,
            false,
            false,
            FLAGS_ON,
        );
        let picked = picked.expect("强扫面信号应产生候选");
        assert_eq!(picked.kind, AutoTriggerKind::CodebaseSearch);
        assert!(
            picked.detail.contains("sweep("),
            "判分依据要能解释这次触发：{}",
            picked.detail
        );
        assert!(
            !picked.detail.starts_with("veto:"),
            "被触发的东西不该带否决理由：{}",
            picked.detail
        );
    }

    /// 弱信号 + JSON 回退开着 → 选回退，而不是排序定位。
    /// 这是"score 压过 kind"的另一个面：kind 更高的 CodebaseSearch 也会落选。
    #[test]
    fn json_fallback_wins_when_no_sweep_signal() {
        let picked = select_best_auto_trigger_with_flags(
            false,
            false,
            "find the thing",
            "I will update this file",
            &tools(&["CodebaseSearch"]),
            false,
            false,
            false,
            false,
            false,
            FLAGS_ON,
        );
        assert_eq!(
            picked.as_ref().map(|p| p.kind),
            Some(AutoTriggerKind::JsonFallback),
            "没有扫面信号时不该触发 CodebaseSearch：{picked:?}"
        );
    }

    /// 已经搜过一次就不再自动搜（避免每轮重复消费同一份索引）。
    #[test]
    fn already_attempted_disables_the_route() {
        let picked = select_best_auto_trigger_with_flags(
            false,
            false,
            "where is retry handled across the codebase?",
            "",
            &tools(&["CodebaseSearch"]),
            true, // attempted
            false,
            false,
            false,
            false,
            FLAGS_ON,
        );
        assert!(picked.is_none(), "已尝试过就不该再选：{picked:?}");
    }

    #[test]
    fn tool_must_be_active() {
        let picked = select_best_auto_trigger_with_flags(
            false,
            false,
            "where is retry handled across the codebase?",
            "",
            &tools(&["Grep"]),
            false,
            false,
            false,
            false,
            false,
            FLAGS_ON,
        );
        assert!(picked.is_none(), "工具没激活就不该选：{picked:?}");
    }

    /// 回归：`STAR_ENABLE_AUTO_CODEBASE_SEARCH` 曾默认 false，等于这套机制唯一的
    /// 实际状态就是"永远关着"，整条 skill 回退梯全是死代码。
    #[test]
    fn auto_codebase_search_is_on_by_default() {
        assert!(
            RouteFlags::from_env().auto_codebase_search,
            "语义搜索自动触发必须默认开；恢复旧行为请显式设 \
             STAR_ENABLE_AUTO_CODEBASE_SEARCH=0"
        );
    }

    /// 逃生门仍然有效：显式写 0 就是关。
    ///
    /// 这里测 [`read_bool_env_flag`] 而不是 [`bool_env_flag`]——后者按 key 用
    /// `OnceLock` 缓存**首次**读取的结果，同进程内第二次调用拿到的是上一次的答案。
    /// 开关一旦被缓存，测试就既测不了"默认值"也测不了"显式关闭"（本文件踩过）。
    #[test]
    fn auto_flags_are_documented_as_escapable() {
        assert!(read_bool_env_flag(
            "STAR_ENABLE_AUTO_CODEBASE_SEARCH_UNSET",
            true
        ));
        assert!(!read_bool_env_flag(
            "STAR_ENABLE_AUTO_CODEBASE_SEARCH_UNSET",
            false
        ));
        assert_eq!(
            RouteFlags::from_env().json_fallback,
            json_fallback_enabled()
        );
    }

    /// 预取只有在首轮**且**提问像扫面时才发生——以前是无条件预取。
    #[test]
    fn prefetch_requires_sweep_shape_and_first_turn() {
        let active = tools(&["CodebaseSearch"]);
        assert!(should_prefetch_codebase_search_with_flag(
            "where is retry handled across the codebase?",
            &active,
            0,
            true
        ));
        assert!(!should_prefetch_codebase_search_with_flag(
            "grep for parse_toml_config",
            &active,
            0,
            true
        ));
        assert!(!should_prefetch_codebase_search_with_flag(
            "where is retry handled across the codebase?",
            &active,
            3, // 非首轮
            true
        ));
    }

    #[test]
    fn project_map_route_requires_overview_question() {
        let active = tools(&["ProjectMap"]);
        assert!(should_trigger_project_map_with_flag(
            "give me an overview of this repo",
            "",
            &active,
            false,
            true
        ));
        assert!(!should_trigger_project_map_with_flag(
            "where is retry handled?",
            "",
            &active,
            false,
            true
        ));
    }
}
