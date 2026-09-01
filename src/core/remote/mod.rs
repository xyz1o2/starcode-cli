use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

const REMOTE_PROTOCOL_V1: &str = "starcode.remote.v1";
const ACTION_SEND_MESSAGE: &str = "send_message";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteRequest {
    #[serde(default = "default_protocol")]
    pub protocol: String,
    #[serde(default = "default_action")]
    pub action: String,
    pub message: String,
    #[serde(default)]
    pub source: String,
    #[serde(default = "default_created_at")]
    pub created_at: i64,
}

#[derive(Debug, Clone, Default)]
pub struct RemoteDrainResult {
    pub accepted: Vec<RemoteRequest>,
    pub rejected: Vec<String>,
}

fn default_protocol() -> String {
    REMOTE_PROTOCOL_V1.to_string()
}

fn default_action() -> String {
    ACTION_SEND_MESSAGE.to_string()
}

fn default_created_at() -> i64 {
    Utc::now().timestamp()
}

pub fn remote_dir(project_root: &Path) -> PathBuf {
    project_root.join(".star").join("remote")
}

pub fn inbox_file_path(project_root: &Path) -> PathBuf {
    remote_dir(project_root).join("inbox.jsonl")
}

async fn ensure_remote_dir(project_root: &Path) -> Result<(), String> {
    let dir = remote_dir(project_root);
    if !dir.exists() {
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| format!("failed to create {}: {}", dir.display(), e))?;
    }
    Ok(())
}

pub async fn queue_message(
    project_root: &Path,
    message: String,
    source: Option<String>,
) -> Result<(), String> {
    if message.trim().is_empty() {
        return Err("message cannot be empty".to_string());
    }

    ensure_remote_dir(project_root).await?;
    let path = inbox_file_path(project_root);
    let req = RemoteRequest {
        protocol: default_protocol(),
        action: default_action(),
        message,
        source: source.unwrap_or_default(),
        created_at: Utc::now().timestamp(),
    };

    let line =
        serde_json::to_string(&req).map_err(|e| format!("failed to encode request: {}", e))?;
    let mut f = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await
        .map_err(|e| format!("failed to open {}: {}", path.display(), e))?;

    f.write_all(line.as_bytes())
        .await
        .map_err(|e| format!("failed to write {}: {}", path.display(), e))?;
    f.write_all(b"\n")
        .await
        .map_err(|e| format!("failed to write {}: {}", path.display(), e))?;
    f.flush()
        .await
        .map_err(|e| format!("failed to flush {}: {}", path.display(), e))?;

    Ok(())
}

pub async fn queued_count(project_root: &Path) -> Result<usize, String> {
    let path = inbox_file_path(project_root);
    if !path.exists() {
        return Ok(0);
    }
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("failed to read {}: {}", path.display(), e))?;
    Ok(content.lines().filter(|l| !l.trim().is_empty()).count())
}

pub async fn drain_requests(project_root: &Path) -> Result<RemoteDrainResult, String> {
    let path = inbox_file_path(project_root);
    if !path.exists() {
        return Ok(RemoteDrainResult::default());
    }

    let raw = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("failed to read {}: {}", path.display(), e))?;
    if raw.trim().is_empty() {
        return Ok(RemoteDrainResult::default());
    }

    tokio::fs::write(&path, "")
        .await
        .map_err(|e| format!("failed to truncate {}: {}", path.display(), e))?;

    let mut out = RemoteDrainResult::default();
    for (idx, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parsed = serde_json::from_str::<RemoteRequest>(line);
        match parsed {
            Ok(req) => {
                if req.message.trim().is_empty() {
                    out.rejected
                        .push(format!("line {}: empty message", idx + 1));
                    continue;
                }
                if req.protocol != REMOTE_PROTOCOL_V1 {
                    out.rejected.push(format!(
                        "line {}: unsupported protocol `{}`",
                        idx + 1,
                        req.protocol
                    ));
                    continue;
                }
                if req.action != ACTION_SEND_MESSAGE {
                    out.rejected.push(format!(
                        "line {}: unsupported action `{}`",
                        idx + 1,
                        req.action
                    ));
                    continue;
                }
                out.accepted.push(req);
            }
            Err(err) => out
                .rejected
                .push(format!("line {}: invalid json ({})", idx + 1, err)),
        }
    }

    Ok(out)
}

pub fn protocol_example() -> &'static str {
    r#"{"protocol":"starcode.remote.v1","action":"send_message","source":"ci-bot","message":"请检查 main 分支构建失败原因"}"#
}
