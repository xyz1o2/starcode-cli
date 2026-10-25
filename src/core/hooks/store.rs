use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ManagedHookEvent {
    #[serde(rename = "SessionStart")]
    SessionStart,
    #[serde(rename = "SessionEnd")]
    SessionEnd,
    #[serde(rename = "BeforeAgent")]
    BeforeAgent,
    #[serde(rename = "AfterAgent")]
    AfterAgent,
    #[serde(rename = "Notification")]
    Notification,
    #[serde(rename = "UserPromptSubmit")]
    UserPromptSubmit,
    #[serde(rename = "Stop")]
    Stop,
    #[serde(rename = "PreCompact", alias = "PreCompress")]
    PreCompact,
    #[serde(rename = "BeforeToolSelection")]
    BeforeToolSelection,
    #[serde(rename = "BeforeModel")]
    BeforeModel,
    #[serde(rename = "AfterModel")]
    AfterModel,
    #[serde(rename = "PreToolUse")]
    PreToolUse,
    #[serde(rename = "PostToolUse")]
    PostToolUse,
    #[serde(rename = "SubagentStop")]
    SubagentStop,
    // ── Claude Code 兼容事件（2026-10-25 接入）──
    //
    // 之前这些事件名只在 `core/hooks/types.rs`（一个从未被任何 `mod` 声明引用、
    // 因而不参与编译的文件）里出现过。看得见、用不着的枚举比没有更糟 ——
    // 审查时会得出「CC 35 个事件覆盖了 22 个」的错误结论。
    //
    // 现在它们进了**真正生效**的注册表，每个都有真实的触发点。
    #[serde(rename = "PermissionRequest")]
    PermissionRequest,
    #[serde(rename = "PermissionDenied")]
    PermissionDenied,
    #[serde(rename = "PostToolUseFailure")]
    PostToolUseFailure,
    #[serde(rename = "StopFailure")]
    StopFailure,
    #[serde(rename = "SubagentStart")]
    SubagentStart,
    #[serde(rename = "PostCompact")]
    PostCompact,
}

impl ManagedHookEvent {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SessionStart => "SessionStart",
            Self::SessionEnd => "SessionEnd",
            Self::BeforeAgent => "BeforeAgent",
            Self::AfterAgent => "AfterAgent",
            Self::Notification => "Notification",
            Self::UserPromptSubmit => "UserPromptSubmit",
            Self::Stop => "Stop",
            Self::PreCompact => "PreCompact",
            Self::BeforeToolSelection => "BeforeToolSelection",
            Self::BeforeModel => "BeforeModel",
            Self::AfterModel => "AfterModel",
            Self::PreToolUse => "PreToolUse",
            Self::PostToolUse => "PostToolUse",
            Self::SubagentStop => "SubagentStop",
            Self::PermissionRequest => "PermissionRequest",
            Self::PermissionDenied => "PermissionDenied",
            Self::PostToolUseFailure => "PostToolUseFailure",
            Self::StopFailure => "StopFailure",
            Self::SubagentStart => "SubagentStart",
            Self::PostCompact => "PostCompact",
        }
    }

    pub fn parse(input: &str) -> Option<Self> {
        match input.trim().to_lowercase().as_str() {
            "sessionstart" | "session_start" | "start" => Some(Self::SessionStart),
            "sessionend" | "session_end" | "end" => Some(Self::SessionEnd),
            "beforeagent" | "before_agent" | "before" => Some(Self::BeforeAgent),
            "afteragent" | "after_agent" | "after" => Some(Self::AfterAgent),
            "notification" | "notify" | "notice" => Some(Self::Notification),
            "userpromptsubmit" | "user_prompt_submit" | "prompt_submit" | "submit" => {
                Some(Self::UserPromptSubmit)
            }
            "stop" | "abort" | "cancel" => Some(Self::Stop),
            "precompact" | "pre_compact" | "precompress" | "pre_compress" | "compress_before" => {
                Some(Self::PreCompact)
            }
            "beforetoolselection" | "before_tool_selection" | "tool_selection_before" => {
                Some(Self::BeforeToolSelection)
            }
            "beforemodel" | "before_model" | "model_before" => Some(Self::BeforeModel),
            "aftermodel" | "after_model" | "model_after" => Some(Self::AfterModel),
            "pretooluse" | "pre_tool_use" | "beforetool" | "before_tool" => Some(Self::PreToolUse),
            "posttooluse" | "post_tool_use" | "aftertool" | "after_tool" => Some(Self::PostToolUse),
            "subagentstop" | "subagent_stop" | "agent_stop" => Some(Self::SubagentStop),
            "permissionrequest" | "permission_request" | "perm_request" => {
                Some(Self::PermissionRequest)
            }
            "permissiondenied" | "permission_denied" | "perm_denied" => {
                Some(Self::PermissionDenied)
            }
            "posttoolusefailure" | "post_tool_use_failure" | "tool_failure" => {
                Some(Self::PostToolUseFailure)
            }
            "stopfailure" | "stop_failure" => Some(Self::StopFailure),
            "subagentstart" | "subagent_start" | "agent_start" => Some(Self::SubagentStart),
            "postcompact" | "post_compact" | "compress_after" => Some(Self::PostCompact),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedHook {
    pub name: String,
    pub event: ManagedHookEvent,
    pub command: String,
    pub timeout_secs: u64,
    #[serde(default)]
    pub blocking: bool,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<PathBuf>,
    /// Claude Code 的 `matcher`：限定这条 hook 在哪些工具上触发。
    /// `None` / `None` 以外的空串 / `"*"` 视为匹配全部 —— 与 CC 一致。
    /// 从 settings.json 导入时带上；`/hooks add` 建的一律 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
}

/// 按 Claude Code 的 matcher 语义判断某条工具事件是否命中。
///
/// CC 的规则是**按字符集分派**的，简单到能背下来：
/// - `"*"` / 空串 / `None` → 全部命中
/// - 只含字母数字 `_ -` 空格 `,` `|` → 精确串，或 `|` / `,` 分隔的精确串列表
/// - 含其他任何字符 → **非锚定**正则（`Regex::is_match`，即任意位置子串命中）
///
/// 第三条意味着 `Edit.*` 同时匹配 `Edit` 和 `NotebookEdit`；需要整串匹配就自己写
/// `^...$`。这里忠实复刻，不加锚点。
pub fn matcher_matches(matcher: Option<&str>, tool_name: Option<&str>) -> bool {
    let Some(raw) = matcher else {
        return true;
    };
    let pattern = raw.trim();
    if pattern.is_empty() || pattern == "*" {
        return true;
    }
    // 无工具名可匹配时，只有「匹配全部」的 matcher 才能通过
    let Some(tool_name) = tool_name else {
        return false;
    };

    const EXACT_CHARS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_- ,|";
    if pattern.chars().all(|c| EXACT_CHARS.contains(c)) {
        pattern
            .split(['|', ','])
            .map(|s| s.trim())
            .any(|exact| !exact.is_empty() && exact == tool_name)
    } else {
        match regex::Regex::new(pattern) {
            Ok(re) => re.is_match(tool_name),
            // 编译失败：宁可不放行未知规则，也不让它拦下所有工具。
            Err(e) => {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[Hooks] matcher `{}` 不是合法正则，已忽略该 hook（{}）",
                    pattern, e
                ));
                false
            }
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HookStore {
    pub hooks: Vec<ManagedHook>,
}

#[derive(Debug, Clone)]
struct HookStoreCacheEntry {
    modified_at_unix_ms: Option<u128>,
    file_len: u64,
    store: HookStore,
    checked_at: Instant,
}

fn hook_store_cache() -> &'static Mutex<HashMap<PathBuf, HookStoreCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, HookStoreCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn file_fingerprint(metadata: &std::fs::Metadata) -> (Option<u128>, u64) {
    let modified_at_unix_ms = metadata
        .modified()
        .ok()
        .and_then(|ts| ts.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis());
    (modified_at_unix_ms, metadata.len())
}

fn hook_store_cache_ttl() -> Duration {
    static TTL: OnceLock<Duration> = OnceLock::new();
    *TTL.get_or_init(|| {
        let ttl_ms = std::env::var("STAR_HOOK_STORE_CACHE_TTL_MS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(1500);
        Duration::from_millis(ttl_ms)
    })
}

pub fn hooks_file_path(project_root: &Path) -> PathBuf {
    project_root.join(".star").join("hooks.json")
}

async fn ensure_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("failed to create {}: {}", parent.display(), e))?;
        }
    }
    Ok(())
}

pub async fn load_store(project_root: &Path) -> Result<HookStore, String> {
    let path = hooks_file_path(project_root);
    if let Ok(cache) = hook_store_cache().lock() {
        if let Some(entry) = cache.get(&path) {
            if entry.checked_at.elapsed() <= hook_store_cache_ttl() {
                return Ok(entry.store.clone());
            }
        }
    }

    if !path.exists() {
        let store = HookStore::default();
        if let Ok(mut cache) = hook_store_cache().lock() {
            cache.insert(
                path,
                HookStoreCacheEntry {
                    modified_at_unix_ms: None,
                    file_len: 0,
                    store: store.clone(),
                    checked_at: Instant::now(),
                },
            );
        }
        return Ok(store);
    }

    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|e| format!("failed to stat {}: {}", path.display(), e))?;
    let (modified_at_unix_ms, file_len) = file_fingerprint(&metadata);

    if let Ok(cache) = hook_store_cache().lock() {
        if let Some(entry) = cache.get(&path) {
            if entry.modified_at_unix_ms == modified_at_unix_ms && entry.file_len == file_len {
                return Ok(entry.store.clone());
            }
        }
    }

    let text = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("failed to read {}: {}", path.display(), e))?;

    if text.trim().is_empty() {
        let store = HookStore::default();
        if let Ok(mut cache) = hook_store_cache().lock() {
            cache.insert(
                path,
                HookStoreCacheEntry {
                    modified_at_unix_ms,
                    file_len,
                    store: store.clone(),
                    checked_at: Instant::now(),
                },
            );
        }
        return Ok(store);
    }

    let store: HookStore = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse {}: {}", path.display(), e))?;

    if let Ok(mut cache) = hook_store_cache().lock() {
        cache.insert(
            path,
            HookStoreCacheEntry {
                modified_at_unix_ms,
                file_len,
                store: store.clone(),
                checked_at: Instant::now(),
            },
        );
    }

    Ok(store)
}

pub async fn save_store(project_root: &Path, store: &HookStore) -> Result<(), String> {
    let path = hooks_file_path(project_root);
    ensure_parent(&path).await?;

    let text = serde_json::to_string_pretty(store)
        .map_err(|e| format!("failed to serialize hook store: {}", e))?;

    tokio::fs::write(&path, text)
        .await
        .map_err(|e| format!("failed to write {}: {}", path.display(), e))?;

    match tokio::fs::metadata(&path).await {
        Ok(metadata) => {
            let (modified_at_unix_ms, file_len) = file_fingerprint(&metadata);
            if let Ok(mut cache) = hook_store_cache().lock() {
                cache.insert(
                    path,
                    HookStoreCacheEntry {
                        modified_at_unix_ms,
                        file_len,
                        store: store.clone(),
                        checked_at: Instant::now(),
                    },
                );
            }
        }
        Err(_) => {
            if let Ok(mut cache) = hook_store_cache().lock() {
                cache.remove(&path);
            }
        }
    }

    Ok(())
}

pub async fn list_hooks(project_root: &Path) -> Result<Vec<ManagedHook>, String> {
    let store = load_store(project_root).await?;
    let mut hooks = store.hooks;

    // Claude Code 风格的 hooks：写在 settings.json 的 `hooks` 段里。
    // 只读，不进 `store`，所以 `/hooks add|remove` 动不了它们，用户自己维护。
    hooks.extend(super::settings_hooks::load_settings_hooks(project_root));

    for plugin_hook in crate::core::plugins::discover_plugin_hooks(project_root).await? {
        let Some(event) = ManagedHookEvent::parse(&plugin_hook.event) else {
            continue;
        };

        hooks.push(ManagedHook {
            name: plugin_hook.name,
            event,
            command: plugin_hook.command,
            timeout_secs: plugin_hook.timeout_secs,
            blocking: plugin_hook.blocking,
            enabled: true,
            created_at: 0,
            updated_at: 0,
            source: Some(plugin_hook.source),
            working_dir: Some(plugin_hook.working_dir),
            matcher: None,
        });
    }

    Ok(hooks)
}

pub async fn has_enabled_hooks_for_events(
    project_root: &Path,
    events: &[ManagedHookEvent],
) -> Result<bool, String> {
    let hooks = list_hooks(project_root).await?;
    Ok(hooks
        .iter()
        .any(|hook| hook.enabled && events.contains(&hook.event)))
}

pub async fn add_hook(
    project_root: &Path,
    name: String,
    event: ManagedHookEvent,
    command: String,
    timeout_secs: u64,
    blocking: bool,
) -> Result<ManagedHook, String> {
    if name.trim().is_empty() {
        return Err("hook name cannot be empty".to_string());
    }
    if command.trim().is_empty() {
        return Err("hook command cannot be empty".to_string());
    }

    let mut store = load_store(project_root).await?;
    if store.hooks.iter().any(|h| h.name == name) {
        return Err(format!("hook '{}' already exists", name));
    }

    let now = Utc::now().timestamp();
    let hook = ManagedHook {
        name,
        event,
        command,
        timeout_secs: timeout_secs.max(1),
        blocking,
        enabled: true,
        created_at: now,
        updated_at: now,
        source: None,
        working_dir: None,
        matcher: None,
    };

    store.hooks.push(hook.clone());
    save_store(project_root, &store).await?;

    Ok(hook)
}

pub async fn remove_hook(project_root: &Path, name: &str) -> Result<bool, String> {
    let mut store = load_store(project_root).await?;
    let before = store.hooks.len();
    store.hooks.retain(|h| h.name != name);

    if store.hooks.len() == before {
        return Ok(false);
    }

    save_store(project_root, &store).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::ManagedHookEvent;

    /// 事件名是对外契约 —— 外部 harness 的 settings.json 按这些名字挂 hook。
    /// 改名会让已配置的 hook 静默失效。
    #[test]
    fn event_names_are_stable() {
        for (event, expected) in [
            (ManagedHookEvent::PermissionRequest, "PermissionRequest"),
            (ManagedHookEvent::PermissionDenied, "PermissionDenied"),
            (ManagedHookEvent::PostToolUseFailure, "PostToolUseFailure"),
            (ManagedHookEvent::StopFailure, "StopFailure"),
            (ManagedHookEvent::SubagentStart, "SubagentStart"),
            (ManagedHookEvent::PostCompact, "PostCompact"),
        ] {
            assert_eq!(event.as_str(), expected);
        }
    }

    /// Claude Code 兼容的事件名必须能被 `parse` 认出来 ——
    /// 否则用户在 settings.json 里写 "PermissionRequest" 会被静默忽略。
    #[test]
    fn claude_code_event_names_parse() {
        for name in [
            "PermissionRequest",
            "permission_request",
            "PermissionDenied",
            "PostToolUseFailure",
            "StopFailure",
            "SubagentStart",
            "PostCompact",
        ] {
            assert!(
                ManagedHookEvent::parse(name).is_some(),
                "事件名 {name} 解析不出来"
            );
        }
    }

    /// 反序列化也要认（settings.json 里的事件名走这条路）。
    #[test]
    fn claude_code_event_names_deserialize() {
        for name in [
            "PermissionRequest",
            "PermissionDenied",
            "PostToolUseFailure",
            "StopFailure",
            "SubagentStart",
            "PostCompact",
        ] {
            let parsed: Result<ManagedHookEvent, _> =
                serde_json::from_value(serde_json::json!(name));
            assert!(parsed.is_ok(), "事件名 {name} 反序列化失败");
        }
    }

    /// 未知事件名仍然要被拒 —— 不能变成"什么都接受"。
    #[test]
    fn unknown_event_names_are_rejected() {
        assert!(ManagedHookEvent::parse("NoSuchEvent").is_none());
    }
}
