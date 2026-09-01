use crate::types::StarMessage;
use std::path::PathBuf;
use std::sync::{mpsc, OnceLock};

pub(crate) struct SessionPersistRequest {
    pub(crate) path: PathBuf,
    pub(crate) body: Option<Vec<u8>>,
}

fn write_session_persist_request_sync(request: &SessionPersistRequest) {
    if let Some(body) = request.body.as_ref() {
        if let Some(parent) = request.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&request.path, body);
    } else {
        let _ = std::fs::remove_file(&request.path);
    }
}

pub(crate) fn session_persist_sender() -> &'static mpsc::Sender<SessionPersistRequest> {
    static SENDER: OnceLock<mpsc::Sender<SessionPersistRequest>> = OnceLock::new();
    SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<SessionPersistRequest>();
        let _ = std::thread::Builder::new()
            .name("star-session-persist".to_string())
            .spawn(move || {
                while let Ok(request) = rx.recv() {
                    write_session_persist_request_sync(&request);
                }
            });
        tx
    })
}

pub(crate) fn persist_session_messages(session_messages: &[StarMessage], storage_path: PathBuf) {
    let max_messages = std::env::var("STAR_SESSION_MAX_MESSAGES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(480)
        .max(32);

    if session_messages.len() <= max_messages {
        persist_session_messages_to_disk(session_messages, storage_path);
        return;
    }

    let mut compacted = Vec::with_capacity(max_messages);
    let has_system = session_messages
        .first()
        .map(|m| m.role.as_str() == "system")
        .unwrap_or(false);
    if has_system {
        compacted.push(session_messages[0].clone());
    }
    let tail_keep = max_messages.saturating_sub(compacted.len());
    let start = session_messages.len().saturating_sub(tail_keep);
    compacted.extend(session_messages[start..].iter().cloned());
    persist_session_messages_to_disk(&compacted, storage_path);
}

pub(crate) fn sync_and_persist(
    messages: Vec<StarMessage>,
    storage_path: PathBuf,
) -> Vec<StarMessage> {
    persist_session_messages(&messages, storage_path);
    messages
}

pub(crate) fn persist_session_messages_to_disk(
    session_messages: &[StarMessage],
    storage_path: PathBuf,
) {
    let request = if session_messages.is_empty() {
        SessionPersistRequest {
            path: storage_path,
            body: None,
        }
    } else {
        let Ok(json) = serde_json::to_vec(session_messages) else {
            return;
        };
        SessionPersistRequest {
            path: storage_path,
            body: Some(json),
        }
    };

    if let Err(err) = session_persist_sender().send(request) {
        write_session_persist_request_sync(&err.0);
    }
}

pub(crate) fn load_persisted_session_messages(storage_path: &PathBuf) -> Vec<StarMessage> {
    let content = match std::fs::read_to_string(storage_path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    serde_json::from_str::<Vec<StarMessage>>(&content).unwrap_or_default()
}
