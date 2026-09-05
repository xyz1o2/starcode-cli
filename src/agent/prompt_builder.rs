use crate::core::prompts;
use crate::core::prompts::SystemPrompts;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

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

// ── Git 快照缓存 ──
//
// 之前每构建一次 system prompt 就 fork 4 个 git 子进程（rev-parse ×2 / status / log），
// 每条用户消息一次。更要命的是 `git status --short` 的行数在 coding agent 里几乎每次
// 编辑之后都会变，而它落在 system 消息里 —— 等于每轮都把 prompt 缓存前缀改掉一截，
// 于是那 4 万多字符的 system prompt 每轮都在按原价重新计费。
//
// 这里加一层 TTL 缓存，默认 300 秒，**正好是 Anthropic ephemeral 缓存的存活时间**：
// 同一个缓存窗口内 git 段落逐字节相同，缓存前缀不会因为「刚改了个文件」而失效；
// 窗口过期时缓存本来就要重建，那一刻刷新 git 信息不损失任何东西。
// 用 STAR_PROMPT_GIT_SNAPSHOT_SECS 调整（0 = 关掉缓存，每次都真跑 git）。
const GIT_SNAPSHOT_TTL_SECS: u64 = 300;

/// 一次 git 探测的结果（分支 / 工作区状态 / 最近提交）
#[derive(Clone, Default)]
struct GitSnapshot {
    is_repo: bool,
    branch: Option<String>,
    status: Option<String>,
    recent_commits: Option<String>,
}

fn git_snapshot_ttl() -> Duration {
    Duration::from_secs(
        std::env::var("STAR_PROMPT_GIT_SNAPSHOT_SECS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(GIT_SNAPSHOT_TTL_SECS),
    )
}

fn git_snapshot_cache() -> &'static Mutex<HashMap<PathBuf, (Instant, GitSnapshot)>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, (Instant, GitSnapshot)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 真正去跑 git 的那一层（4 个子进程，只在缓存未命中时执行）
fn probe_git_snapshot(cwd: &Path) -> GitSnapshot {
    let run = |args: &[&str]| -> Option<String> {
        std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
    };

    let is_repo = run(&["rev-parse", "--is-inside-work-tree"]).is_some();
    if !is_repo {
        return GitSnapshot::default();
    }

    GitSnapshot {
        is_repo: true,
        branch: run(&["rev-parse", "--abbrev-ref", "HEAD"]).map(|s| s.trim().to_string()),
        // 只报「几个文件改了」而不是文件名列表：既省 token，也让这一段在
        // 同一批改动里更容易保持逐字节稳定。
        status: run(&["status", "--short"]).map(|s| {
            let changed = s.lines().filter(|l| !l.trim().is_empty()).count();
            if changed == 0 {
                "(clean)".to_string()
            } else {
                format!("{} files changed", changed)
            }
        }),
        recent_commits: run(&["log", "--oneline", "-5"]).map(|s| s.trim().to_string()),
    }
}

/// 带 TTL 的 git 快照；同一个 cwd 在 TTL 内复用上一次的结果
fn cached_git_snapshot(cwd: &Path) -> GitSnapshot {
    let ttl = git_snapshot_ttl();
    if ttl.is_zero() {
        return probe_git_snapshot(cwd);
    }

    let key = cwd.to_path_buf();
    if let Ok(cache) = git_snapshot_cache().lock() {
        if let Some((checked_at, snapshot)) = cache.get(&key) {
            if checked_at.elapsed() < ttl {
                return snapshot.clone();
            }
        }
    }

    let snapshot = probe_git_snapshot(cwd);
    if let Ok(mut cache) = git_snapshot_cache().lock() {
        cache.insert(key, (Instant::now(), snapshot.clone()));
    }
    snapshot
}

// ── System prompt 逃生口（对标 pi 的 SYSTEM.md / APPEND_SYSTEM.md）──
//
// 本项目生成的 system prompt 约 4 万字符（≈1.2 万 token），还没算工具 schema。
// 有的场景（跑评测、接小上下文模型、只想要个纯粹的 shell agent）根本不需要这一整套，
// 但之前没有任何办法绕开它。现在给两个钩子：
//
// - `SYSTEM.md`：**整体替换**生成好的 system prompt；
// - `APPEND_SYSTEM.md`：在生成内容**后面追加**（保留全部内建行为，只加自己的规则）。
//
// 查找顺序：`$STAR_SYSTEM_PROMPT_FILE` / `$STAR_APPEND_SYSTEM_PROMPT_FILE` 指定的路径
// > 项目 `./.star/` > 全局 `~/.star/`。两个钩子可以同时用。
fn read_prompt_override(cwd: &Path, env_key: &str, filename: &str) -> Option<String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(explicit) = std::env::var(env_key).ok().filter(|v| !v.trim().is_empty()) {
        candidates.push(PathBuf::from(explicit.trim()));
    }
    candidates.push(
        crate::core::config::storage::Storage::new(cwd.to_path_buf())
            .star_dir()
            .join(filename),
    );
    candidates.push(crate::core::config::storage::Storage::global_star_dir().join(filename));

    for path in candidates {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if !content.trim().is_empty() {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[Prompt] Using {} override from {}",
                    filename,
                    path.display()
                ));
                return Some(content.trim().to_string());
            }
        }
    }
    None
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

            if let Some(file_content) = crate::core::prompts::loader::try_load_prompt(&filename) {
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
                if let Some(hist) = crate::core::services::git_service::get_file_history(
                    std::path::Path::new(file),
                    GIT_FILE_HISTORY_DEPTH,
                ) {
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
            if let Some(recent_git) =
                crate::core::services::git_service::get_recent_changes(GIT_RECENT_CHANGES_COUNT)
            {
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

        // Git 信息走 TTL 缓存（见 cached_git_snapshot 的注释）
        let git = cached_git_snapshot(std::path::Path::new(cwd));

        parts.push(prompts::env_info::render(prompts::env_info::EnvInfo {
            today,
            platform,
            cwd,
            shell: &shell,
            is_git_repo: git.is_repo,
            git_branch: git.branch.as_deref(),
            git_status: git.status.as_deref(),
            recent_commits: git.recent_commits.as_deref(),
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
    /// # 缓存前缀必须逐字节稳定
    ///
    /// Anthropic 的 prompt cache 是**前缀**缓存，而 rig 只在 system 数组的最后一块上
    /// 打断点（`apply_system_cache_control` → `system.last_mut()`），所以**整个 system
    /// 数组都属于被缓存的前缀**：其中任何一个字节变了，这一整段（本项目约 4 万字符）
    /// 就要按原价重算。
    ///
    /// 因此这里只往 system 里放"一个缓存窗口内不会变"的东西：
    ///
    /// - 静态段：身份 / 模式提示 / 安全策略 / reminders / 平台信息 / 工具目录 /
    ///   扩展提示包 / final override rules；
    /// - 准静态段：项目指令（STAR.md，改了会变但不会每轮变）+ git 快照
    ///   （TTL 300s ≈ 缓存存活时间，见 [`cached_git_snapshot`]）。
    ///
    /// 真正每轮都变的东西（`complexity_hint`、检索出来的动态上下文）不再进 system，
    /// 由调用方挂到当轮 user 消息上 —— 见 [`PromptBuilder::build_turn_context`]。
    ///
    /// `.star/SYSTEM.md` 可以整体替换这里生成的内容，`.star/APPEND_SYSTEM.md` 可以在
    /// 后面追加 —— 见 [`read_prompt_override`]。
    ///
    /// If `STAR_PROMPT_CACHE_ENABLED=0`, the messages are plain system messages
    /// without cache_control.
    pub fn build_cached_system_messages(
        mode: PromptMode,
        today: &str,
        platform: &str,
        cwd: &str,
        active_tools: Option<&HashSet<String>>,
        active_files: Option<&[String]>,
        project_context_override: Option<String>,
        is_thinking_model: bool,
        include_extended_bundle: bool,
    ) -> Vec<crate::types::StarMessage> {
        let cwd_path = std::path::Path::new(cwd);

        // SYSTEM.md 整体替换：命中就直接返回，连 git / 扩展包 / 项目指令都不用去读。
        if let Some(replacement) =
            read_prompt_override(cwd_path, "STAR_SYSTEM_PROMPT_FILE", "SYSTEM.md")
        {
            return vec![Self::finish_system_message(cwd_path, replacement)];
        }

        let flags = PromptFlags::from_env();

        // ── 准静态段（跟着静态段一起进缓存前缀）──
        // 这些内容会变，但不是每轮都变；跟静态段放在同一条 system 消息里，
        // 变的时候重建一次缓存即可，比每轮都变要划算得多。
        let mut semi_static_parts: Vec<String> = Vec::new();

        if let Some(ctx) = project_context_override {
            semi_static_parts.push(format!("### Project Instructions\n\n{}", ctx));
        } else if let Some(ctx) =
            crate::utils::project_context::load_merged_project_context(std::path::Path::new(cwd))
        {
            semi_static_parts.push(format!(
                "### Project Instructions (from STAR.md)\n\n{}",
                ctx
            ));
        }

        if let Some(files) = active_files {
            let mut file_history = String::new();
            for file in files {
                if let Some(hist) = crate::core::services::git_service::get_file_history(
                    std::path::Path::new(file),
                    GIT_FILE_HISTORY_DEPTH,
                ) {
                    file_history.push_str(&format!("\n#### History for {}\n{}\n", file, hist));
                }
            }
            if !file_history.is_empty() {
                semi_static_parts.push(format!(
                    "### Active Files Git Context (Insight)\n\n{}",
                    file_history
                ));
            }
        } else if flags.include_recent_git_context {
            if let Some(recent_git) =
                crate::core::services::git_service::get_recent_changes(GIT_RECENT_CHANGES_COUNT)
            {
                semi_static_parts.push(format!(
                    "### Recent Git Context (Context Lineage)\n\n{}",
                    recent_git
                ));
            }
        }

        if let Some(files) = active_files {
            if let Some(scope_strategy) = render_scope_strategy(files.len()) {
                semi_static_parts.push(scope_strategy);
            }
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
            today, platform, cwd, &shell,
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

        // ── Git 快照（准静态：TTL 内逐字节稳定）──
        let git = cached_git_snapshot(cwd_path);
        if git.is_repo {
            semi_static_parts.push(prompts::env_info::render_dynamic_git_info(
                git.branch.as_deref(),
                git.status.as_deref(),
                git.recent_commits.as_deref(),
            ));
        }

        // ── Assemble messages ──
        // 静态 + 准静态合成**一条** system 消息：rig 只会在 system 数组最后一块上打
        // 缓存断点，拆成两条并不会让前面那条单独进缓存，反而白白多一条消息。
        // final override rules 压在最后，保持"最后读到"的位置优势。
        let mut parts = static_parts;
        parts.append(&mut semi_static_parts);
        parts.push(final_override_rules());

        vec![Self::finish_system_message(cwd_path, parts.join("\n\n"))]
    }

    /// 收尾：追加 `APPEND_SYSTEM.md`，再按开关决定是否打缓存标记
    fn finish_system_message(cwd: &Path, mut content: String) -> crate::types::StarMessage {
        if let Some(appended) =
            read_prompt_override(cwd, "STAR_APPEND_SYSTEM_PROMPT_FILE", "APPEND_SYSTEM.md")
        {
            content.push_str("\n\n");
            content.push_str(&appended);
        }

        if prompt_cache_enabled() {
            crate::types::StarMessage::cached_system(content)
        } else {
            crate::types::StarMessage::system(content)
        }
    }

    /// 构造"当轮上下文"—— 每条用户消息都会变、因此**不能**进 system prompt 的那部分
    ///
    /// 复杂度提示会在 Simple/Medium/Complex 之间来回跳，检索出来的动态上下文更是每问
    /// 一次就换一批。这两样只要放进 system 数组，缓存前缀就每轮都被击穿。对标 Claude
    /// Code 的做法：挂到当轮 user 消息上，用 `<system-reminder>` 包起来。
    ///
    /// 返回 `None` 表示这一轮没有额外上下文，调用方直接发原始输入即可。
    /// 总长度受 `STAR_TURN_CONTEXT_MAX_CHARS` 限制（默认 4000），避免长会话里
    /// 一轮一轮堆积。
    pub fn build_turn_context(
        dynamic_context: Option<&str>,
        complexity_hint: Option<&str>,
    ) -> Option<String> {
        let mut blocks: Vec<String> = Vec::new();

        if let Some(hint) = complexity_hint.map(str::trim).filter(|s| !s.is_empty()) {
            blocks.push(format!(
                "### Task Complexity & Planning Strategy\n\n{}",
                hint
            ));
        }
        if let Some(ctx) = dynamic_context.map(str::trim).filter(|s| !s.is_empty()) {
            blocks.push(format!("### Retrieved Project Context\n\n{}", ctx));
        }

        if blocks.is_empty() {
            return None;
        }

        let max_chars = std::env::var("STAR_TURN_CONTEXT_MAX_CHARS")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(4000);

        let mut body = blocks.join("\n\n");
        if body.chars().count() > max_chars {
            let cut = body
                .char_indices()
                .nth(max_chars)
                .map(|(i, _)| i)
                .unwrap_or(body.len());
            body.truncate(cut);
            body.push_str("\n\n... [turn context truncated]");
        }

        Some(format!("<system-reminder>\n{}\n</system-reminder>", body))
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
fn scope_strategy_for(file_count: usize) -> String {
    if file_count == 0 {
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
    let enabled =
        *ENABLED.get_or_init(|| env_flag(std::env::var("STAR_PROMPT_KEY_SCENARIOS").ok(), true));
    if enabled {
        parts.push(prompts::key_scenarios::render(is_thinking_model));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_context_is_none_when_nothing_to_say() {
        assert!(PromptBuilder::build_turn_context(None, None).is_none());
        assert!(PromptBuilder::build_turn_context(Some("  "), Some("\n")).is_none());
    }

    #[test]
    fn turn_context_wraps_blocks_in_system_reminder() {
        let out = PromptBuilder::build_turn_context(Some("retrieved doc"), Some("plan first"))
            .expect("both blocks present");
        assert!(out.starts_with("<system-reminder>"));
        assert!(out.ends_with("</system-reminder>"));
        // 复杂度提示排在检索上下文之前
        let hint_at = out.find("plan first").unwrap();
        let ctx_at = out.find("retrieved doc").unwrap();
        assert!(hint_at < ctx_at);
    }

    #[test]
    fn turn_context_respects_char_budget() {
        // 不动环境变量：默认上限 4000，直接喂一段更长的进去。
        // （单测并行跑，set_var 会漏给同文件里的其它测试。）
        let long = "x".repeat(9000);
        let out = PromptBuilder::build_turn_context(Some(&long), None).unwrap();
        assert!(out.contains("turn context truncated"));
        assert!(out.chars().count() < 5000);
    }
}
