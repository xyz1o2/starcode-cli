use crate::core::utils::paths::current_project_star_dir;
use crate::types::{ChatEntry, ChatEntryType};
use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub created_at: i64,
    pub chat_history: Vec<ChatEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub created_at: i64,
}

fn sessions_dir() -> Result<PathBuf, Box<dyn Error + Send + Sync>> {
    Ok(current_project_star_dir().join("sessions"))
}

fn latest_session_marker_path() -> Result<PathBuf, Box<dyn Error + Send + Sync>> {
    Ok(sessions_dir()?.join(".latest"))
}

pub async fn list_sessions() -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
    let dir = sessions_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut out: Vec<String> = Vec::new();
    let mut rd = tokio::fs::read_dir(&dir).await?;
    while let Some(ent) = rd.next_entry().await? {
        let path = ent.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            out.push(stem.to_string());
        }
    }
    out.sort();
    Ok(out)
}

pub async fn list_session_summaries() -> Result<Vec<SessionSummary>, Box<dyn Error + Send + Sync>> {
    let dir = sessions_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let latest_id = read_latest_session_id().await.ok().flatten();
    let mut summaries = Vec::new();
    let mut rd = tokio::fs::read_dir(&dir).await?;
    while let Some(ent) = rd.next_entry().await? {
        let path = ent.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        let content = match tokio::fs::read_to_string(&path).await {
            Ok(content) => content,
            Err(_) => continue,
        };
        let session: Session = match serde_json::from_str(&content) {
            Ok(session) => session,
            Err(_) => continue,
        };

        summaries.push(summarize_session(
            &session,
            latest_id.as_deref() == Some(session.id.as_str()),
        ));
    }

    summaries.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.title.cmp(&right.title))
    });
    Ok(summaries)
}

pub async fn save_session(
    id: &str,
    history: &[ChatEntry],
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let dir = sessions_dir()?;
    tokio::fs::create_dir_all(&dir).await?;
    let p = dir.join(format!("{}.json", id));

    let session = Session {
        id: id.to_string(),
        created_at: chrono::Utc::now().timestamp(),
        chat_history: history.to_vec(),
    };

    let s = serde_json::to_string_pretty(&session)?;
    tokio::fs::write(&p, s).await?;
    tokio::fs::write(latest_session_marker_path()?, id.as_bytes()).await?;
    Ok(())
}

pub async fn load_session(id: &str) -> Result<Session, Box<dyn Error + Send + Sync>> {
    let dir = sessions_dir()?;
    let p = dir.join(format!("{}.json", id));

    if !p.exists() {
        return Err(format!("Session '{}' not found", id).into());
    }

    let content = tokio::fs::read_to_string(&p).await?;
    let session: Session = serde_json::from_str(&content)?;
    Ok(session)
}

pub async fn load_latest_session() -> Result<Session, Box<dyn Error + Send + Sync>> {
    let Some(id) = read_latest_session_id().await? else {
        return Err("No latest session found. Use /chat save first.".into());
    };
    load_session(&id).await
}

pub async fn delete_session(id: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
    let dir = sessions_dir()?;
    let p = dir.join(format!("{}.json", id));

    if p.exists() {
        tokio::fs::remove_file(&p).await?;
    }
    Ok(())
}

pub async fn read_latest_session_id() -> Result<Option<String>, Box<dyn Error + Send + Sync>> {
    let marker_path = latest_session_marker_path()?;
    if !marker_path.exists() {
        return Ok(None);
    }

    let id = tokio::fs::read_to_string(&marker_path).await?;
    let id = id.trim();
    if id.is_empty() {
        return Ok(None);
    }

    Ok(Some(id.to_string()))
}

fn summarize_session(session: &Session, is_latest: bool) -> SessionSummary {
    SessionSummary {
        id: session.id.clone(),
        title: session_title(session),
        subtitle: session_subtitle(session, is_latest),
        created_at: session.created_at,
    }
}

fn session_title(session: &Session) -> String {
    session
        .chat_history
        .iter()
        .find_map(|entry| {
            if entry.entry_type == ChatEntryType::User {
                compact_text(&entry.content)
            } else {
                None
            }
        })
        .or_else(|| {
            session.chat_history.iter().find_map(|entry| {
                if entry.entry_type == ChatEntryType::Assistant {
                    compact_text(&entry.content)
                } else {
                    None
                }
            })
        })
        .map(|text| truncate_text(&text, 42))
        .unwrap_or_else(|| fallback_session_title(&session.id))
}

fn session_subtitle(session: &Session, is_latest: bool) -> String {
    let time_label = Local
        .timestamp_opt(session.created_at, 0)
        .single()
        .map(|dt| dt.format("%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "Unknown time".to_string());
    let latest_label = if is_latest { "Latest · " } else { "" };
    format!(
        "{}{} · {} msgs",
        latest_label,
        time_label,
        session.chat_history.len()
    )
}

fn compact_text(text: &str) -> Option<String> {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        None
    } else {
        Some(compact)
    }
}

fn fallback_session_title(id: &str) -> String {
    if id.starts_with("auto-") {
        "Saved Session".to_string()
    } else {
        truncate_text(id, 24)
    }
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}...", truncated)
    } else {
        truncated
    }
}
