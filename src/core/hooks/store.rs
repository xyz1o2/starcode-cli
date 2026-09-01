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
