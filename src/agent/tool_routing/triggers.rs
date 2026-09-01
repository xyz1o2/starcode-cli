use crate::agent::tool_routing::has_action_intent;
use std::collections::HashSet;
use std::sync::OnceLock;

/// 自动触发器类型——按优先级从高到低排列
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum AutoTriggerKind {
    /// 验证（最高优先级——编辑后立即检查）
    Verification = 10,
    /// 语义搜索（概念性查询，最通用）
    SemanticSearch = 5,
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

/// 触发器评估结果
pub(crate) struct TriggerEval {
    pub kind: AutoTriggerKind,
    /// 0-10 的相关性分数，越高越应该触发
    pub score: u32,
    pub reason: &'static str,
}

/// 评估所有可用的自动触发器，返回最高优先级+最高分的那个。
/// 替代原来的串行 if-else 链，确保每轮只触发最相关的操作。
pub(crate) fn select_best_auto_trigger(
    verification_required: bool,
    skip_verification: bool,
    user_input: &str,
    current_content: &str,
    active_tools: &HashSet<String>,
    semantic_search_attempted: bool,
    navigator_skill_attempted: bool,
    analyzer_skill_attempted: bool,
    editor_skill_attempted: bool,
    project_map_attempted: bool,
) -> Option<TriggerEval> {
    let mut candidates: Vec<TriggerEval> = Vec::new();

    // 验证触发器 — 最高优先级（编辑后必须检查）
    if verification_required && !skip_verification {
        candidates.push(TriggerEval {
            kind: AutoTriggerKind::Verification,
            score: 10,
            reason: "edits_detected_requires_verification",
        });
    }

    // 语义搜索 — 概念性查询的通用最佳选择
    if should_trigger_semantic_search(user_input, current_content, active_tools, semantic_search_attempted) {
        let score = score_semantic_search_relevance(user_input, current_content);
        candidates.push(TriggerEval {
            kind: AutoTriggerKind::SemanticSearch,
            score,
            reason: "conceptual_query_detected",
        });
    }

    // JSON回退 — 检测到行动意图但模型没返回工具调用
    if json_fallback_enabled() && has_action_intent(current_content) {
        candidates.push(TriggerEval {
            kind: AutoTriggerKind::JsonFallback,
            score: 4,
            reason: "action_intent_without_tool_calls",
        });
    }

    // 导航器技能 — 需要代码结构理解
    if should_trigger_navigator_skill(
        user_input, current_content, active_tools,
        semantic_search_attempted, navigator_skill_attempted,
    ) {
        candidates.push(TriggerEval {
            kind: AutoTriggerKind::NavigatorSkill,
            score: 3,
            reason: "dependency_tracing_needed",
        });
    }

    // 项目地图 — 需要项目结构总览
    if should_trigger_project_map(user_input, current_content, active_tools, project_map_attempted) {
        candidates.push(TriggerEval {
            kind: AutoTriggerKind::ProjectMap,
            score: 2,
            reason: "project_overview_needed",
        });
    }

    // 分析器技能 — 需要结构化分析
    if should_trigger_analyzer_skill(
        user_input, current_content, active_tools,
        project_map_attempted, analyzer_skill_attempted,
    ) {
        candidates.push(TriggerEval {
            kind: AutoTriggerKind::AnalyzerSkill,
            score: 1,
            reason: "broad_analysis_needed",
        });
    }

    // 编辑器技能 — 最后手段
    if should_trigger_editor_skill(
        user_input, current_content, active_tools,
        semantic_search_attempted, navigator_skill_attempted, editor_skill_attempted,
    ) {
        candidates.push(TriggerEval {
            kind: AutoTriggerKind::EditorSkill,
            score: 1,
            reason: "edit_context_insufficient",
        });
    }

    // 按优先级排序：先比 kind 的固有优先级（Ord），再比相关性分数
    candidates.sort_by(|a, b| {
        b.kind.cmp(&a.kind).then(b.score.cmp(&a.score))
    });

    candidates.into_iter().next()
}

/// 评估语义搜索的相关性分数（0-10）
fn score_semantic_search_relevance(user_input: &str, current_content: &str) -> u32 {
    let lower = user_input.to_lowercase();
    let combined = format!("{} {}", lower, current_content.to_lowercase());
    let mut score = 5u32; // 基准分

    let high_indicators = [
        "how", "why", "what", "explain", "understand",
        "概念", "原理", "怎么", "为什么", "是什么", "理解",
        "架构", "architecture", "design", "pattern",
    ];
    let medium_indicators = [
        "find", "Grep", "locate", "where",
        "找", "搜索", "定位", "哪里",
    ];

    for kw in &high_indicators {
        if combined.contains(kw) {
            score += 3;
            break;
        }
    }
    for kw in &medium_indicators {
        if combined.contains(kw) {
            score += 1;
            break;
        }
    }

    score.min(10)
}

// ── 原触发器条件检查函数（供 select_best_auto_trigger 和旧代码复用）──

/// 检查是否应该触发语义搜索
fn should_trigger_semantic_search(
    user_input: &str,
    current_content: &str,
    active_tools: &HashSet<String>,
    already_attempted: bool,
) -> bool {
    should_trigger_semantic_search_with_flag(
        user_input,
        current_content,
        active_tools,
        already_attempted,
        auto_semantic_search_enabled(),
    )
}

fn should_trigger_semantic_search_with_flag(
    _user_input: &str,
    _current_content: &str,
    active_tools: &HashSet<String>,
    already_attempted: bool,
    auto_enabled: bool,
) -> bool {
    auto_enabled && !already_attempted && active_tools.contains("SemanticSearch")
}

/// 检查是否应该预取语义搜索
pub(crate) fn should_prefetch_semantic_search(
    user_input: &str,
    active_tools: &HashSet<String>,
    history_len: usize,
) -> bool {
    should_prefetch_semantic_search_with_flag(
        user_input,
        active_tools,
        history_len,
        first_turn_prefetch_enabled(),
    )
}

fn should_prefetch_semantic_search_with_flag(
    _user_input: &str,
    active_tools: &HashSet<String>,
    history_len: usize,
    prefetch_enabled: bool,
) -> bool {
    prefetch_enabled && history_len == 0 && active_tools.contains("SemanticSearch")
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
    _user_input: &str,
    active_tools: &HashSet<String>,
    history_len: usize,
    prefetch_enabled: bool,
) -> bool {
    prefetch_enabled && history_len == 0 && active_tools.contains("ProjectMap")
}

/// 检查是否应该触发导航技能
fn should_trigger_navigator_skill(
    user_input: &str,
    current_content: &str,
    active_tools: &HashSet<String>,
    semantic_search_attempted: bool,
    already_attempted: bool,
) -> bool {
    should_trigger_navigator_skill_with_flag(
        user_input,
        current_content,
        active_tools,
        semantic_search_attempted,
        already_attempted,
        auto_skill_fallbacks_enabled(),
    )
}

fn should_trigger_navigator_skill_with_flag(
    _user_input: &str,
    _current_content: &str,
    active_tools: &HashSet<String>,
    semantic_search_attempted: bool,
    already_attempted: bool,
    auto_enabled: bool,
) -> bool {
    auto_enabled
        && !already_attempted
        && semantic_search_attempted
        && active_tools.contains("skill")
}

/// 检查是否应该触发分析器技能
fn should_trigger_analyzer_skill(
    user_input: &str,
    current_content: &str,
    active_tools: &HashSet<String>,
    project_map_attempted: bool,
    already_attempted: bool,
) -> bool {
    should_trigger_analyzer_skill_with_flag(
        user_input,
        current_content,
        active_tools,
        project_map_attempted,
        already_attempted,
        auto_skill_fallbacks_enabled(),
    )
}

fn should_trigger_analyzer_skill_with_flag(
    _user_input: &str,
    _current_content: &str,
    active_tools: &HashSet<String>,
    project_map_attempted: bool,
    already_attempted: bool,
    auto_enabled: bool,
) -> bool {
    auto_enabled
        && !already_attempted
        && project_map_attempted
        && active_tools.contains("skill")
}

/// 检查是否应该触发项目地图
fn should_trigger_project_map(
    user_input: &str,
    current_content: &str,
    active_tools: &HashSet<String>,
    already_attempted: bool,
) -> bool {
    should_trigger_project_map_with_flag(
        user_input,
        current_content,
        active_tools,
        already_attempted,
        auto_skill_fallbacks_enabled(),
    )
}

fn should_trigger_project_map_with_flag(
    _user_input: &str,
    _current_content: &str,
    active_tools: &HashSet<String>,
    already_attempted: bool,
    auto_enabled: bool,
) -> bool {
    auto_enabled && !already_attempted && active_tools.contains("ProjectMap")
}

/// 检查是否应该触发编辑器技能
fn should_trigger_editor_skill(
    user_input: &str,
    current_content: &str,
    active_tools: &HashSet<String>,
    semantic_search_attempted: bool,
    navigator_skill_attempted: bool,
    already_attempted: bool,
) -> bool {
    should_trigger_editor_skill_with_flag(
        user_input,
        current_content,
        active_tools,
        semantic_search_attempted,
        navigator_skill_attempted,
        already_attempted,
        auto_skill_fallbacks_enabled(),
    )
}

fn should_trigger_editor_skill_with_flag(
    _user_input: &str,
    _current_content: &str,
    active_tools: &HashSet<String>,
    semantic_search_attempted: bool,
    navigator_skill_attempted: bool,
    already_attempted: bool,
    auto_enabled: bool,
) -> bool {
    auto_enabled
        && !already_attempted
        && semantic_search_attempted
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

/// 检查是否启用自动语义搜索
fn auto_semantic_search_enabled() -> bool {
    bool_env_flag("STAR_ENABLE_AUTO_SEMANTIC_SEARCH", false)
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
        "STAR_ENABLE_AUTO_SEMANTIC_SEARCH" => {
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
