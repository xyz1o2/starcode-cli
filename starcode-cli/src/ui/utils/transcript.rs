use crate::types::ApprovalMode;
use crate::ui::state::store::ChatState;
use serde_json::{json, Map, Value};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, OnceLock};

pub const TRANSCRIPT_SCHEMA_VERSION: u32 = 2;
pub const DEFAULT_TRANSCRIPT_FILE: &str = "transcript.jsonl";

pub fn transcript_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn transcript_enabled_from_env() -> bool {
    static TRANSCRIPT_ENABLED: OnceLock<bool> = OnceLock::new();
    *TRANSCRIPT_ENABLED.get_or_init(|| {
        std::env::var("STAR_TRANSCRIPT")
            .ok()
            .map(|value| {
                let value = value.to_lowercase();
                !(value == "0" || value == "false" || value == "off")
            })
            .unwrap_or(true)
    })
}

pub fn default_transcript_path(project_root: &Path) -> PathBuf {
    project_root.join(".star").join(DEFAULT_TRANSCRIPT_FILE)
}

pub fn resolve_transcript_path() -> Option<PathBuf> {
    static TRANSCRIPT_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
    TRANSCRIPT_PATH
        .get_or_init(|| {
            if let Ok(path) = std::env::var("STAR_TRANSCRIPT_PATH") {
                let trimmed = path.trim();
                if !trimmed.is_empty() {
                    return Some(PathBuf::from(trimmed));
                }
            }

            std::env::current_dir()
                .ok()
                .map(|cwd| default_transcript_path(&cwd))
        })
        .clone()
}

fn approval_mode_label(mode: &ApprovalMode) -> &'static str {
    match mode {
        ApprovalMode::Default => "default",
        ApprovalMode::Plan => "plan",
        ApprovalMode::Yolo => "yolo",
    }
}

fn current_provider_id(state: &ChatState) -> Option<String> {
    state
        .pending_model_provider
        .clone()
        .or_else(|| state.current_provider_id.clone())
        .or_else(|| state.model_provider_map.get(&state.current_model).cloned())
        .filter(|value| !value.trim().is_empty())
}

fn merge_payload(target: &mut Map<String, Value>, payload: Value) {
    match payload {
        Value::Object(map) => {
            for (key, value) in map {
                target.insert(key, value);
            }
        }
        Value::Null => {}
        other => {
            target.insert("payload".to_string(), other);
        }
    }
}

struct TranscriptWriteRequest {
    path: PathBuf,
    line: Value,
}

fn open_transcript_file(path: &Path) -> Option<File> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    OpenOptions::new().create(true).append(true).open(path).ok()
}

fn write_transcript_value(file: &mut File, line: &Value) -> std::io::Result<()> {
    serde_json::to_writer(&mut *file, line)?;
    file.write_all(b"\n")
}

fn write_transcript_request_sync(path: &Path, line: &Value) {
    let Some(mut file) = open_transcript_file(path) else {
        return;
    };
    let _ = write_transcript_value(&mut file, line);
}

fn write_transcript_request_cached(
    cache: &mut Option<(PathBuf, File)>,
    path: &Path,
    line: &Value,
) -> bool {
    let needs_reopen = cache
        .as_ref()
        .map(|(current_path, _)| current_path != path)
        .unwrap_or(true);

    if needs_reopen {
        let Some(file) = open_transcript_file(path) else {
            *cache = None;
            return false;
        };
        *cache = Some((path.to_path_buf(), file));
    }

    let Some((_, file)) = cache.as_mut() else {
        return false;
    };

    if write_transcript_value(file, line).is_ok() {
        true
    } else {
        *cache = None;
        false
    }
}

fn transcript_sender() -> &'static mpsc::Sender<TranscriptWriteRequest> {
    static SENDER: OnceLock<mpsc::Sender<TranscriptWriteRequest>> = OnceLock::new();
    SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<TranscriptWriteRequest>();
        let _ = std::thread::Builder::new()
            .name("star-transcript-writer".to_string())
            .spawn(move || {
                let mut cache: Option<(PathBuf, File)> = None;
                while let Ok(request) = rx.recv() {
                    if !write_transcript_request_cached(&mut cache, &request.path, &request.line) {
                        write_transcript_request_sync(&request.path, &request.line);
                    }
                }
            });
        tx
    })
}

pub fn build_user_transcript_payload(
    user_content: &str,
    processed_content: &str,
    attached_files: usize,
) -> Value {
    let mut map = Map::new();
    map.insert("content".to_string(), json!(user_content));
    map.insert("attached_files".to_string(), json!(attached_files));

    if processed_content != user_content {
        map.insert("processed_content".to_string(), json!(processed_content));
        map.insert(
            "processed_content_chars".to_string(),
            json!(processed_content.chars().count()),
        );
    }

    Value::Object(map)
}

pub fn build_transcript_event(
    state: &mut ChatState,
    event: &str,
    message_id: Option<u64>,
    payload: Value,
) -> Value {
    state.transcript_seq = state.transcript_seq.saturating_add(1);

    let mut map = Map::new();
    merge_payload(&mut map, payload);

    if let Some(message_id) = message_id {
        map.insert("message_id".to_string(), json!(message_id));
    }

    map.insert(
        "schema_version".to_string(),
        json!(TRANSCRIPT_SCHEMA_VERSION),
    );
    map.insert("run_id".to_string(), json!(state.transcript_run_id.clone()));
    map.insert("seq".to_string(), json!(state.transcript_seq));
    map.insert("ts".to_string(), json!(transcript_now()));
    map.insert("event".to_string(), json!(event));
    map.insert("type".to_string(), json!(event));
    map.insert(
        "approval_mode".to_string(),
        json!(approval_mode_label(&state.approval_mode)),
    );
    map.insert("is_processing".to_string(), json!(state.is_processing));
    map.insert("is_streaming".to_string(), json!(state.is_streaming));
    map.insert("token_count".to_string(), json!(state.token_count));

    if !state.current_model.trim().is_empty() {
        map.insert("model".to_string(), json!(state.current_model.clone()));
    }

    if let Some(provider_id) = current_provider_id(state) {
        map.insert("provider".to_string(), json!(provider_id));
    }

    Value::Object(map)
}

pub fn append_transcript_line(state: &mut ChatState, line: Value) {
    if !state.transcript_enabled {
        return;
    }
    let Some(path) = state.transcript_path.clone() else {
        return;
    };

    let request = TranscriptWriteRequest { path, line };
    if let Err(err) = transcript_sender().send(request) {
        write_transcript_request_sync(&err.0.path, &err.0.line);
    }
}

pub fn append_transcript_event(
    state: &mut ChatState,
    event: &str,
    message_id: Option<u64>,
    payload: Value,
) {
    if !state.transcript_enabled || state.transcript_path.is_none() {
        return;
    }

    let line = build_transcript_event(state, event, message_id, payload);
    append_transcript_line(state, line);
}

 