use crate::core::prompts;
use crate::core::prompts::SystemPrompts;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

// ── Scope-aware editing strategy thresholds ──
/// 1–N files → tight scope (fast path: search → read → replace).
const TIGHT_SCOPE_MAX_FILES: usize = 2;
/// N+1–M files → moderate scope (impact-check → multi_edit).
const MODERATE_SCOPE_MAX_FILES: usize = 5;
/// Filesystem history depth for git context.
const GIT_FILE_HISTORY_DEPTH: usize = 3;
/// Number of recent git changes to pull for context lineage.
const GIT_RECENT_CHANGES_COUNT: usize = 5;

/// Final Override Rules - loaded from file for centralized management.
/// (external dir overrides embedded, cached via loader)
fn final_override_rules() -> String {
    crate::core::prompts::loader::load_prompt("final-override-rules.md")
}

/// Whether Prompt Cache (Anthropic-style cache_control) is enabled.
/// Default: enabled. Set STAR_PROMPT_CACHE_ENABLED=0 to disable.
fn prompt_cache_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("STAR_PROMPT_CACHE_ENABLED")
            .ok()
            .map(|v| {
                let v = v.trim().to_lowercase();
                !(v == "0" || v == "false" || v == "off")
            })
            .unwrap_or(true)
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PromptMode {
    Interactive,
    Agent,
    Plan,
}

#[derive(Clone, Copy, Debug)]
pub struct PromptFlags {
    pub include_core_identity: bool,
    pub include_security_policy: bool,
    pub include_tool_catalog: bool,
    pub include_recent_git_context: bool,
}

impl PromptFlags {
    pub fn from_env() -> Self {
        static FLAGS: OnceLock<PromptFlags> = OnceLock::new();
        *FLAGS.get_or_init(|| {
            let b = |k: &str, default_val: bool| -> bool {
                env_flag(std::env::var(k).ok(), default_val)
            };
            Self {
                include_core_identity: b("STAR_PROMPT_CORE_IDENTITY", true),
                include_security_policy: b("STAR_PROMPT_SECURITY", true),
                include_tool_catalog: b("STAR_PROMPT_TOOL_CATALOG", true),
                include_recent_git_context: b("STAR_PROMPT_RECENT_GIT_CONTEXT", false),
            }
        })
    }
}

pub struct PromptBuilder;

impl PromptBuilder {
    /// 动态加载 Prompt Bundle，支持根据工具和模式按需加载
    fn load_selective_system_prompts(
        mode: PromptMode,
        active_tools: Option<&HashSet<String>>,
    ) -> Option<String> {
        let enabled = cached_system_prompts_dir_enabled();

        if !enabled {
            return None;
        }

        let max_chars = cached_system_prompts_max_chars();
        let max_file_chars = cached_system_prompts_max_file_chars();

        let cache_key = build_bundle_cache_key(mode, active_tools, max_chars, max_file_chars);
        if let Some(cached) = system_prompt_bundle_cache()
            .lock()
            .expect("system prompt bundle cache lock poisoned")
            .get(&cache_key)
            .cloned()
        {
            return cached;
        }

        let mut out = String::new();
        out.push_str("\n\n# System Prompts Bundle (Dynamic)\n");

        let mut all_filenames: Vec<String> = SystemPrompts::iter()
            .map(|f| f.to_string())
            .filter(|f| f.ends_with(".md"))
            .collect();
        all_filenames.sort_by(|a, b| {
            let a_lower = a.to_lowercase();
            let b_lower = b.to_lowercase();
            embedded_prompt_dedupe_key(&a_lower)
                .cmp(&embedded_prompt_dedupe_key(&b_lower))
                .then_with(|| {
                    prompt_version_priority(&a_lower).cmp(&prompt_version_priority(&b_lower))
                })
                .then_with(|| a_lower.cmp(&b_lower))
        });

        let mut selected_filenames: Vec<String> = Vec::new();
        let mut selected_dedupe_keys = HashSet::new();

        for filename in all_filenames {
            let name = filename.to_lowercase();

            if should_skip_embedded_prompt_file(&name) {
                continue;
            }

            if name.starts_with("tool-description-") {
                if let Some(tools) = active_tools {
                    let tool_name_part = name
                        .trim_start_matches("tool-description-")
                        .trim_end_matches(".md")
                        .split('-')
                        .next()
                        .unwrap_or("");

                    // 与工具 schema 描述共用同一映射（tool_descriptions::tool_description_key_map）。
                    // 该映射覆盖了旧版逐分支匹配（bash/grep/readfile/edit/websearch 等别名）。
                    let is_active =
                        crate::core::prompts::tool_descriptions::description_key_matches_active_tools(
                            tool_name_part,
                            tools,
                        );

                    if !is_active {
                        continue;
                    }
                }
            }

            if name.contains("plan-mode") {
                if mode != PromptMode::Plan {
                    continue;
                }
            }

            if name.contains("agent-prompt") {
                if mode == PromptMode::Interactive {
                    continue;
                }

                let specialist_prompts = [
                    "agent-prompt-explore.md",
                    "agent-prompt-star-guide-agent.md",
                    "agent-prompt-starmd-creation.md",
                    "agent-prompt-prompt-suggestion-generator-v2.md",
                    "agent-prompt-session-search-assistant.md",
                    "agent-prompt-update-magic-docs.md",
                    "agent-prompt-bash-command-file-path-extraction.md",
                    "agent-prompt-bash-command-prefix-detection.md",
                    "agent-prompt-bash-output-summarization.md",
                    "agent-prompt-conversation-summarization.md",
                    "agent-prompt-conversation-summarization-with-additional-instructions.md",
                    "agent-prompt-session-title-and-branch-generation.md",
                    "agent-prompt-session-notes-template.md",
                    "agent-prompt-session-notes-update-instructions.md",
                    "agent-prompt-pr-comments-slash-command.md",
                    "agent-prompt-review-pr-slash-command.md",
                    "agent-prompt-security-review-slash.md",
                    "agent-prompt-webfetch-summarizer.md",
                    "agent-prompt-user-sentiment-analysis.md",
                    "agent-prompt-prompt-hook-execution.md",
                    "agent-prompt-agent-hook.md",
                    "agent-prompt-agent-creation-architect.md",
                ];

                if specialist_prompts.iter().any(|&p| name.contains(p)) {
                    continue;
                }
            }

            let dedupe_key = embedded_prompt_dedupe_key(&name);
            if !selected_dedupe_keys.insert(dedupe_key) {
                continue;
            }
            selected_filenames.push(filename);
        }

        selected_filenames.sort();

        for filename in selected_filenames {
            if out.len() >= max_chars {
                break;
            }

            if let Some(file_content) =
                crate::core::prompts::loader::try_load_prompt(&filename)
            {
                let content = if file_content.len() > max_file_chars {
                    let safe_end = file_content
                        .char_indices()
                        .nth(max_file_chars)
                        .map(|(i, _)| i)
                        .unwrap_or(file_content.len());
                    &file_content[..safe_end]
                } else {
                    file_content.as_str()
                };

                out.push_str(&format!("\n--- START OF {} ---\n", filename));
                out.push_str(content.trim());
                out.push_str(&format!("\n--- END OF {} ---", filename));
            }
        }

        let bundle = if out.len() > 100 { Some(out) } else { None };

        system_prompt_bundle_cache()
            .lock()
            .expect("system prompt bundle cache lock poisoned")
            .insert(cache_key, bundle.clone());

        bundle
    }

    pub fn build_system_prompt(
        mode: PromptMode,
        today: &str,
        platform: &str,
        cwd: &str,
        active_tools: Option<&HashSet<String>>,
        active_files: Option<&[String]>,
        project_context_override: Option<String>,
        complexity_hint: Option<String>,
        is_thinking_model: bool,
        include_extended_bundle: bool,
    ) -> String {
        let flags = PromptFlags::from_env();

        let mut parts: Vec<String> = Vec::new();

        if let Some(ctx) = project_context_override {
            parts.push(format!(
                "### Project Instructions (Dynamic Context)\n\n{}",
                ctx
            ));
        } else if let Some(ctx) =
            crate::utils::project_context::load_merged_project_context(std::path::Path::new(cwd))
        {
            parts.push(format!(
                "### Project Instructions (from STAR.md)\n\n{}",
                ctx
            ));
        }

        if let Some(files) = active_files {
            let mut file_history = String::new();
            for file in files {
                if let Some(hist) =
                    crate::core::services::git_service::get_file_history(std::path::Path::new(file), GIT_FILE_HISTORY_DEPTH)
                {
                    file_history.push_str(&format!("\n#### History for {}\n{}\n", file, hist));
                }
            }
            if !file_history.is_empty() {
                parts.push(format!(
                    "### Active Files Git Context (Insight)\n\n{}",
                    file_history
                ));
            }
        } else if flags.include_recent_git_context {
            if let Some(recent_git) = crate::core::services::git_service::get_recent_changes(GIT_RECENT_CHANGES_COUNT) {
                parts.push(format!(
                    "### Recent Git Context (Context Lineage)\n\n{}",
                    recent_git
                ));
            }
        }

        // Adaptive context strategy: adjust editing approach based on scope
        if let Some(files) = active_files {
            if let Some(scope_strategy) = render_scope_strategy(files.len()) {
                parts.push(scope_strategy);
            }
        }

        if let Some(hint) = complexity_hint {
            parts.push(format!(
                "### Task Complexity & Planning Strategy\n\n{}",
                hint
            ));
        }

        // Note: Language response is handled by core_identity prompt which instructs
        // the model to respond in the same language as the user's input.

        if flags.include_core_identity {
            parts.push(prompts::core_identity::render(is_thinking_model));
        }

        match mode {
            PromptMode::Interactive => {
                parts.push(prompts::main_system::render(is_thinking_model));
                maybe_push_key_scenarios(&mut parts, is_thinking_model);
            }
            PromptMode::Agent | PromptMode::Plan => {
                parts.push(prompts::agent_mode::render(is_thinking_model));
                maybe_push_key_scenarios(&mut parts, is_thinking_model);
            }
        }

        if flags.include_security_policy {
            let sp = prompts::security_policy::render();
            if !sp.trim().is_empty() {
                parts.push(sp);
            }
        }

        let reminders = prompts::reminders::render(is_thinking_model);
        if !reminders.trim().is_empty() {
            parts.push(reminders);
        }

        // Detect environment info (lightweight, ~50ms)
        let shell = prompts::env_info::detect_shell();

        // Get git info (blocking, runs in prompt builder context)
        let cwd_path = std::path::Path::new(cwd);
        let is_git = std::process::Command::new("git")
            .args(&["rev-parse", "--is-inside-work-tree"])
            .current_dir(cwd_path)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        let git_branch = if is_git {
            std::process::Command::new("git")
                .args(&["rev-parse", "--abbrev-ref", "HEAD"])
                .current_dir(cwd_path)
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
        } else {
            None
        };

        let git_status = if is_git {
            std::process::Command::new("git")
                .args(&["status", "--short"])
                .current_dir(cwd_path)
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| {
                    let lines: Vec<&str> = s.lines().collect();
                    if lines.is_empty() {
                        "(clean)".to_string()
                    } else {
                        format!("{} files changed", lines.len())
                    }
                })
        } else {
            None
        };

        let recent_commits = if is_git {
            std::process::Command::new("git")
                .args(&["log", "--oneline", "-5"])
                .current_dir(cwd_path)
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
        } else {
            None
        };

        parts.push(prompts::env_info::render(prompts::env_info::EnvInfo {
            today,
            platform,
            cwd,
            shell: &shell,
            is_git_repo: is_git,
            git_branch: git_branch.as_deref(),
            git_status: git_status.as_deref(),
            recent_commits: recent_commits.as_deref(),
        }));

        if flags.include_tool_catalog {
            parts.push(prompts::tool_catalog::render_for_tools(
                is_thinking_model,
                active_tools,
            ));
        }

        if include_extended_bundle {
            if let Some(bundle) = Self::load_selective_system_prompts(mode, active_tools) {
                if !bundle.trim().is_empty() {
                    parts.push(bundle);
                }
            }
        }

        parts.push(final_override_rules());

        parts.join("\n\n")
    }

    /// 第 N 轮起才加载扩展提示包（默认首轮即加载，可用环境变量调回）。
    /// 首轮注入 tool-description bundle，让 agent 一开始就清楚每个工具的用途。
    pub fn include_extended_bundle_for_history_len(history_len: usize) -> bool {
        let min_history = std::env::var("STAR_PROMPT_EXTENDED_BUNDLE_MIN_HISTORY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);
        history_len >= min_history || cached_extended_bundle_first_turn_enabled()
    }

    /// Build system messages with prompt cache optimization.
    ///
    /// Returns two system messages:
    /// 1. Static parts (core identity, security policy, env info, etc.)
    ///    marked with `cache_control: {"type": "ephemeral"}` — these are
    ///    reused across turns, saving 30-50% token costs on Anthropic API.
    /// 2. Dynamic parts (project context, git context, complexity hints) —
    ///    not cached, changes per turn.
    ///
    /// If `STAR_PROMPT_CACHE_ENABLED=0`, both messages are plain system messages
    /// without cache_control.
    pub fn build_cached_system_messages(
        mode: PromptMode,
        today: &str,
        platform: &str,
        cwd: &str,
        active_tools: Option<&HashSet<String>>,
        active_files: Option<&[String]>,
        project_context_override: Option<String>,
        complexity_hint: Option<String>,
        is_thinking_model: bool,
        include_extended_bundle: bool,
    ) -> Vec<crate::types::StarMessage> {
        let flags = PromptFlags::from_env();
        let cache_enabled = prompt_cache_enabled();

        // ── Dynamic parts (NOT cached) ──
        let mut dynamic_parts: Vec<String> = Vec::new();

        if let Some(ctx) = project_context_override {
            dynamic_parts.push(format!(
                "### Project Instructions (Dynamic Context)\n\n{}", ctx
            ));
        } else if let Some(ctx) =
            crate::utils::project_context::load_merged_project_context(std::path::Path::new(cwd))
        {
            dynamic_parts.push(format!(
                "### Project Instructions (from STAR.md)\n\n{}", ctx
            ));
        }

        if let Some(files) = active_files {
            let mut file_history = String::new();
            for file in files {
                if let Some(hist) =
                    crate::core::services::git_service::get_file_history(std::path::Path::new(file), GIT_FILE_HISTORY_DEPTH)
                {
                    file_history.push_str(&format!("\n#### History for {}\n{}\n", file, hist));
                }
            }
            if !file_history.is_empty() {
                dynamic_parts.push(format!(
                    "### Active Files Git Context (Insight)\n\n{}", file_history
                ));
            }
        } else if flags.include_recent_git_context {
            if let Some(recent_git) = crate::core::services::git_service::get_recent_changes(GIT_RECENT_CHANGES_COUNT) {
                dynamic_parts.push(format!(
                    "### Recent Git Context (Context Lineage)\n\n{}", recent_git
                ));
            }
        }

        if let Some(files) = active_files {
            if let Some(scope_strategy) = render_scope_strategy(files.len()) {
                dynamic_parts.push(scope_strategy);
            }
        }

        if let Some(hint) = complexity_hint {
            dynamic_parts.push(format!(
                "### Task Complexity & Planning Strategy\n\n{}", hint
            ));
        }

        // Note: Language response is handled by core_identity prompt which instructs
        // the model to respond in the same language as the user's input.

        // ── Static parts (CACHEABLE with cache_control) ──
        // These parts MUST NOT change between turns to maximize cache hits.
        let mut static_parts: Vec<String> = Vec::new();

        if flags.include_core_identity {
            static_parts.push(prompts::core_identity::render(is_thinking_model));
        }

        match mode {
            PromptMode::Interactive => {
                static_parts.push(prompts::main_system::render(is_thinking_model));
                maybe_push_key_scenarios(&mut static_parts, is_thinking_model);
            }
            PromptMode::Agent | PromptMode::Plan => {
                static_parts.push(prompts::agent_mode::render(is_thinking_model));
                maybe_push_key_scenarios(&mut static_parts, is_thinking_model);
            }
        }

        if flags.include_security_policy {
            let sp = prompts::security_policy::render();
            if !sp.trim().is_empty() {
                static_parts.push(sp);
            }
        }

        let reminders = prompts::reminders::render(is_thinking_model);
        if !reminders.trim().is_empty() {
            static_parts.push(reminders);
        }

        // Static env info (platform, shell — does NOT change between turns)
        let shell = prompts::env_info::detect_shell();
        static_parts.push(prompts::env_info::render_static_env_info(
            today,
            platform,
            cwd,
            &shell,
        ));

        if flags.include_tool_catalog {
            static_parts.push(prompts::tool_catalog::render_for_tools(
                is_thinking_model,
                active_tools,
            ));
        }

        if include_extended_bundle {
            if let Some(bundle) = Self::load_selective_system_prompts(mode, active_tools) {
                if !bundle.trim().is_empty() {
                    static_parts.push(bundle);
                }
            }
        }

        // ── Dynamic parts (NOT cached, changes per turn) ──
        // Git status, branch, recent commits change between turns,
        // so they MUST be in dynamic_parts to avoid cache invalidation.
        let cwd_path = std::path::Path::new(cwd);
        let is_git = std::process::Command::new("git")
            .args(&["rev-parse", "--is-inside-work-tree"])
            .current_dir(cwd_path)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if is_git {
            let git_branch = std::process::Command::new("git")
                .args(&["rev-parse", "--abbrev-ref", "HEAD"])
                .current_dir(cwd_path)
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string());

            let git_status = std::process::Command::new("git")
                .args(&["status", "--short"])
                .current_dir(cwd_path)
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| {
                    let lines: Vec<&str> = s.lines().collect();
                    if lines.is_empty() { "(clean)".to_string() } else { format!("{} files changed", lines.len()) }
                });

            let recent_commits = std::process::Command::new("git")
                .args(&["log", "--oneline", "-5"])
                .current_dir(cwd_path)
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string());

            dynamic_parts.push(prompts::env_info::render_dynamic_git_info(
                git_branch.as_deref(),
                git_status.as_deref(),
                recent_commits.as_deref(),
            ));
        }

        // Override rules
        dynamic_parts.push(final_override_rules());

        // ── Assemble messages ──
        let mut messages: Vec<crate::types::StarMessage> = Vec::new();

        if !static_parts.is_empty() {
            let static_content = static_parts.join("\n\n");
            if cache_enabled {
                messages.push(crate::types::StarMessage::cached_system(static_content));
            } else {
                messages.push(crate::types::StarMessage::system(static_content));
            }
        }

        if !dynamic_parts.is_empty() {
            let dynamic_content = dynamic_parts.join("\n\n");
            messages.push(crate::types::StarMessage::system(dynamic_content));
        }

        messages
    }
}

fn system_prompt_bundle_cache() -> &'static Mutex<HashMap<String, Option<String>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn system_prompts_dir_enabled_from_env(raw: Option<String>) -> bool {
    raw.map(|value| {
        let value = value.trim().to_lowercase();
        !(value == "0" || value == "false" || value == "off")
    })
    .unwrap_or(true)
}

fn cached_system_prompts_dir_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        system_prompts_dir_enabled_from_env(
            std::env::var("STAR_PROMPT_USE_SYSTEM_PROMPTS_DIR").ok(),
        )
    })
}

fn cached_system_prompts_max_chars() -> usize {
    static MAX_CHARS: OnceLock<usize> = OnceLock::new();
    *MAX_CHARS.get_or_init(|| {
        std::env::var("STAR_SYSTEM_PROMPTS_MAX_CHARS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(15_000)
    })
}

fn cached_system_prompts_max_file_chars() -> usize {
    static MAX_FILE_CHARS: OnceLock<usize> = OnceLock::new();
    *MAX_FILE_CHARS.get_or_init(|| {
        std::env::var("STAR_SYSTEM_PROMPTS_MAX_FILE_CHARS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(6_000)
    })
}

fn build_bundle_cache_key(
    mode: PromptMode,
    active_tools: Option<&HashSet<String>>,
    max_chars: usize,
    max_file_chars: usize,
) -> String {
    let mut tool_names = active_tools
        .map(|tools| {
            tools
                .iter()
                .map(|tool| tool.to_lowercase())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    tool_names.sort();
    tool_names.dedup();

    format!(
        "mode={mode:?};max_chars={max_chars};max_file_chars={max_file_chars};tools={}",
        tool_names.join(",")
    )
}

fn should_skip_embedded_prompt_file(filename: &str) -> bool {
    filename.starts_with("system-prompt-")
}

fn embedded_prompt_dedupe_key(filename: &str) -> String {
    if filename.contains("tool-description-enterplanmode") {
        return "tool-description-enterplanmode".to_string();
    }
    if filename.contains("tool-description-exitplanmode") {
        return "tool-description-exitplanmode".to_string();
    }
    filename.to_string()
}

fn prompt_version_priority(filename: &str) -> u8 {
    if filename.contains("-v2.") {
        0
    } else {
        1
    }
}

fn env_flag(value: Option<String>, default_val: bool) -> bool {
    value
        .map(|v| {
            let normalized = v.trim().to_lowercase();
            !(normalized == "0" || normalized == "false" || normalized == "off")
        })
        .unwrap_or(default_val)
}

fn extended_bundle_first_turn_enabled_from_env(raw: Option<String>) -> bool {
    raw.map(|value| {
        let normalized = value.trim().to_lowercase();
        matches!(normalized.as_str(), "1" | "true" | "on" | "yes")
    })
    .unwrap_or(false)
}

fn cached_extended_bundle_first_turn_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        extended_bundle_first_turn_enabled_from_env(
            std::env::var("STAR_PROMPT_EXTENDED_FIRST_TURN").ok(),
        )
    })
}

/// 从 scope-strategy.md 加载当前文件数的编辑策略段落（外部目录可覆盖）。
fn scope_strategy_for(file_count: usize) -> String {    if file_count == 0 {
        return String::new();
    }
    let template = crate::core::prompts::loader::load_prompt("scope-strategy.md");
    let mut lines = template.lines();
    let section = match file_count {
        1..=TIGHT_SCOPE_MAX_FILES => lines.find(|l| l.starts_with("- **Tight scope**")),
        file_count if file_count <= MODERATE_SCOPE_MAX_FILES => {
            lines.find(|l| l.starts_with("- **Moderate scope**"))
        }
        _ => lines.find(|l| l.starts_with("- **Broad scope**")),
    };
    section
        .map(|s| s.trim_start_matches("- ").to_string())
        .unwrap_or_default()
}

/// 渲染"Context Scope Strategy"段落
fn render_scope_strategy(file_count: usize) -> Option<String> {
    let strategy = scope_strategy_for(file_count);
    if strategy.is_empty() {
        return None;
    }
    Some(format!(
        "### Context Scope Strategy\n\nEditing involves {} active file(s). {}",
        file_count, strategy
    ))
}

/// Key Scenarios — 包含具体的工作流示例，帮助模型理解正确的工具使用方式。
/// 默认启用。设置 `STAR_PROMPT_KEY_SCENARIOS=0` 可禁用。
fn maybe_push_key_scenarios(parts: &mut Vec<String>, is_thinking_model: bool) {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    let enabled = *ENABLED.get_or_init(|| {
        env_flag(std::env::var("STAR_PROMPT_KEY_SCENARIOS").ok(), true)
    });
    if enabled {
        parts.push(prompts::key_scenarios::render(is_thinking_model));
    }
}

 