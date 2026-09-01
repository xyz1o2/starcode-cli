use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptEntry {
    pub timestamp: String,
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_results: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<TranscriptMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TranscriptMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<TokenUsageSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenUsageSummary {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub session_id: String,
    pub title: String,
    pub start_time: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<String>,
    pub tools_used: Vec<String>,
    pub files_modified: Vec<String>,
    pub token_usage: TokenUsageSummary,
}

pub struct TranscriptWriter {
    path: PathBuf,
    session_id: String,
    session_start: String,
    title: String,
    tools_used: HashSet<String>,
}

impl TranscriptWriter {
    pub fn new(session_id: &str) -> Self {
        let star_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".star");
        let transcripts_dir = star_dir.join("transcripts");
        let start_time = Utc::now().format("%Y%m%d_%H%M%S").to_string();
        let path = transcripts_dir.join(format!("{}_{}.jsonl", session_id, start_time));
        Self {
            path,
            session_id: session_id.to_string(),
            session_start: Utc::now().to_rfc3339(),
            title: String::new(),
            tools_used: HashSet::new(),
        }
    }

    pub fn transcript_path(&self) -> &PathBuf {
        &self.path
    }

    pub fn set_title_from_message(&mut self, message: &str) {
        if self.title.is_empty() {
            let title = message.lines().next().unwrap_or(message);
            let title: String = title.chars().take(80).collect();
            self.title = title;
        }
    }

    pub async fn append_entry(&mut self, entry: &TranscriptEntry) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create transcript dir: {}", e))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await
            .map_err(|e| format!("Failed to open transcript: {}", e))?;
        let line = serde_json::to_string(entry).map_err(|e| e.to_string())?;
        file.write_all(line.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        file.write_all(b"\n").await.map_err(|e| e.to_string())?;
        file.flush().await.map_err(|e| e.to_string())?;

        if entry.role == "assistant" {
            if let Some(tc) = &entry.tool_calls {
                for call in tc {
                    if let Some(name) = call
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str())
                    {
                        self.tools_used.insert(name.to_string());
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn finalize(
        &self,
        files_modified: &[String],
        token_usage: &TokenUsageSummary,
    ) -> Result<(), String> {
        let metadata = SessionMetadata {
            session_id: self.session_id.clone(),
            title: if self.title.is_empty() {
                "Untitled Session".to_string()
            } else {
                self.title.clone()
            },
            start_time: self.session_start.clone(),
            end_time: Some(Utc::now().to_rfc3339()),
            tools_used: self.tools_used.iter().cloned().collect(),
            files_modified: files_modified.to_vec(),
            token_usage: token_usage.clone(),
        };
        let star_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".star");
        let meta_path = star_dir
            .join("sessions")
            .join(format!("{}.meta.json", self.session_id));
        if let Some(parent) = meta_path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create sessions dir: {}", e))?;
        }
        let json = serde_json::to_string_pretty(&metadata).map_err(|e| e.to_string())?;
        fs::write(&meta_path, json)
            .await
            .map_err(|e| format!("Failed to write session metadata: {}", e))?;
        Ok(())
    }
}

pub async fn list_sessions() -> Result<Vec<SessionMetadata>, String> {
    let sessions_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".star")
        .join("sessions");
    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }
    let mut entries = fs::read_dir(&sessions_dir)
        .await
        .map_err(|e| e.to_string())?;
    let mut sessions = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json")
            && path
                .file_stem()
                .and_then(|s| s.to_str())
                .map_or(false, |s| s.ends_with(".meta"))
        {
            if let Ok(content) = fs::read_to_string(&path).await {
                if let Ok(meta) = serde_json::from_str::<SessionMetadata>(&content) {
                    sessions.push(meta);
                }
            }
        }
    }
    sessions.sort_by(|a, b| b.start_time.cmp(&a.start_time));
    Ok(sessions)
}

pub async fn delete_session(session_id: &str) -> Result<(), String> {
    let sessions_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".star")
        .join("sessions");
    let meta_path = sessions_dir.join(format!("{}.meta.json", session_id));
    if meta_path.exists() {
        fs::remove_file(&meta_path)
            .await
            .map_err(|e| e.to_string())?;
    }
    let transcripts_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".star")
        .join("transcripts");
    if transcripts_dir.exists() {
        let mut entries = fs::read_dir(&transcripts_dir)
            .await
            .map_err(|e| e.to_string())?;
        while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with(session_id) {
                let _ = fs::remove_file(entry.path()).await;
            }
        }
    }
    Ok(())
}

pub async fn export_session(session_id: &str) -> Result<String, String> {
    let transcripts_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".star")
        .join("transcripts");
    if !transcripts_dir.exists() {
        return Err("No transcripts directory found".to_string());
    }
    let mut entries = fs::read_dir(&transcripts_dir)
        .await
        .map_err(|e| e.to_string())?;
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with(session_id) && name_str.ends_with(".jsonl") {
            return fs::read_to_string(entry.path())
                .await
                .map_err(|e| e.to_string());
        }
    }
    Err(format!("Session transcript not found: {}", session_id))
}
