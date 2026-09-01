use crate::types::{StarTool, StarToolCall};
use std::collections::HashSet;

mod auto_plan;
mod helpers;
mod loop_detection;
mod sequence_learner;
mod tool_call_builders;
mod triggers;

pub(crate) use auto_plan::{maybe_generate_auto_plan, AutoPlanDecision};
pub(crate) use helpers::{
    build_tool_loop_signature, build_tool_selection_system_message, compute_tool_roles,
    has_action_intent, infer_tool_hints, is_edit_tool_name, is_memory_tool_name,
    is_read_only_tool_name, is_validation_tool_name, request_complexity_label,
    resolved_read_only_turn_limit, select_tools_for_turn, select_tools_for_turn_for_client,
    should_skip_verification, truncate_chars, ToolHints, ToolRoles,
};
pub(crate) use loop_detection::{detect_tool_loop, resolved_tool_loop_repeat_threshold};
pub(crate) use sequence_learner::ToolSequenceLearner;
pub(crate) use tool_call_builders::{
    build_analyzer_skill_tool_call, build_editor_skill_tool_call, build_json_fallback_prompt,
    build_navigator_skill_tool_call, build_project_map_tool_call, build_semantic_search_tool_call,
    build_validation_tool_call, json_fallback_extract_tool_call,
};
pub(crate) use triggers::{
    dynamic_context_first_turn_enabled, json_fallback_enabled, select_best_auto_trigger,
    should_prefetch_project_map, should_prefetch_semantic_search, AutoTriggerKind,
};

/// 工具选择结果
#[derive(Debug, Clone)]
pub(crate) struct ToolSelection {
    pub(crate) tools: Vec<StarTool>,
    pub(crate) selected_names: HashSet<String>,
    pub(crate) rationale: String,
    pub(crate) total_tools: usize,
}
