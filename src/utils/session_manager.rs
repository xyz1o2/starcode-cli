use crate::core::utils::paths::current_project_star_dir;
use crate::types::{ChatEntry, ChatEntryType, StarMessage, StarToolCall, StarUsage};
use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::error::Error;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub created_at: i64,
    pub chat_history: Vec<ChatEntry>,
    /// Agent 的原生上下文。缺失表示旧版仅保存了 UI transcript。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_messages: Option<Vec<StarMessage>>,
    /// 等待下一条用户输入时注入的本地命令上下文。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_local_context: Option<Vec<String>>,
    /// 最近一次真实 provider 响应的用量快照，不是累计会话用量。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_usage: Option<StarUsage>,
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

/// Session IDs become part of a filesystem path, so keep them as portable filename atoms.
fn session_path(id: &str) -> Result<PathBuf, Box<dyn Error + Send + Sync>> {
    session_path_in(&sessions_dir()?, id)
}

fn session_path_in(dir: &Path, id: &str) -> Result<PathBuf, Box<dyn Error + Send + Sync>> {
    let valid = !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|character| character.is_alphanumeric() || matches!(character, '-' | '_'));
    if !valid {
        return Err(format!(
            "Invalid session ID '{}'. Use up to 128 letters, digits, '-' or '_'.",
            id
        )
        .into());
    }

    Ok(dir.join(format!("{}.json", id)))
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
    list_session_summaries_in(&sessions_dir()?).await
}

pub(crate) async fn list_session_summaries_in(
    dir: &Path,
) -> Result<Vec<SessionSummary>, Box<dyn Error + Send + Sync>> {
    let mut rd = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let latest_id = read_latest_session_id_in(dir).await.ok().flatten();
    let mut summaries = Vec::new();
    while let Some(ent) = rd.next_entry().await? {
        let path = ent.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        // 会话文件名是恢复、覆盖与 `.latest` 指针使用的 canonical identity。
        // 不信任 JSON 内嵌 id，避免损坏或手工编辑的文件让选择器加载错误文件。
        let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if session_path_in(dir, id).is_err()
            || !ent
                .file_type()
                .await
                .is_ok_and(|file_type| file_type.is_file())
        {
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
            id,
            &session,
            latest_id.as_deref() == Some(id),
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
    save_session_snapshot(id, history, None, None, None).await
}

/// 保存 UI transcript 和 worker 所有的原生上下文快照。
///
/// `Some(Vec::new())` 是权威的空上下文，和旧版文件缺失该字段不同。
pub async fn save_session_snapshot(
    id: &str,
    history: &[ChatEntry],
    agent_messages: Option<Vec<StarMessage>>,
    pending_local_context: Option<Vec<String>>,
    last_usage: Option<StarUsage>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let session = Session {
        id: id.to_string(),
        created_at: chrono::Utc::now().timestamp(),
        chat_history: history.to_vec(),
        agent_messages,
        pending_local_context,
        last_usage,
    };
    save_session_record(&session).await
}

/// 将完整会话写入磁盘。先成功写入 session，才推进 `.latest` 指针。
pub async fn save_session_record(session: &Session) -> Result<(), Box<dyn Error + Send + Sync>> {
    let dir = sessions_dir()?;
    tokio::fs::create_dir_all(&dir).await?;
    let p = session_path(&session.id)?;

    let serialized = serde_json::to_string_pretty(session)?;
    tokio::fs::write(&p, serialized).await?;
    tokio::fs::write(latest_session_marker_path()?, session.id.as_bytes()).await?;
    Ok(())
}

/// 返回用于恢复 agent 的原生消息。
///
/// 新版快照直接使用原始协议消息；缺失字段说明是旧文件，才从 display transcript
/// 做保守重建。这个兼容路径绝不为无法验证的工具调用伪造 id 或参数。
pub fn agent_messages_for_restore(session: &Session) -> Vec<StarMessage> {
    session
        .agent_messages
        .clone()
        .unwrap_or_else(|| reconstruct_legacy_agent_messages(&session.chat_history))
}

fn reconstruct_legacy_agent_messages(history: &[ChatEntry]) -> Vec<StarMessage> {
    let mut messages = Vec::new();
    let mut known_tool_call_ids = HashSet::new();

    for entry in history {
        if entry.is_welcome || entry.is_streaming == Some(true) {
            continue;
        }

        match entry.entry_type {
            ChatEntryType::User if !entry.content.trim().is_empty() || !entry.images.is_empty() => {
                let mut message = StarMessage::user(entry.content.clone());
                message.images = entry.images.clone();
                messages.push(message);
            }
            ChatEntryType::User => {}
            ChatEntryType::Assistant => {
                let tool_calls = entry
                    .tool_calls
                    .as_ref()
                    .map(|calls| valid_tool_calls(calls))
                    .unwrap_or_default();
                let has_content = !entry.content.trim().is_empty();
                let has_reasoning = entry
                    .reasoning_content
                    .as_ref()
                    .is_some_and(|reasoning| !reasoning.trim().is_empty());

                if has_content || has_reasoning || !tool_calls.is_empty() {
                    let mut message = StarMessage::assistant(entry.content.clone());
                    if has_reasoning {
                        message = message.with_reasoning(
                            entry.reasoning_content.clone().expect("checked above"),
                        );
                    }
                    if !tool_calls.is_empty() {
                        known_tool_call_ids.extend(tool_calls.iter().map(|call| call.id.clone()));
                        message = message.with_tool_calls(tool_calls);
                    }
                    messages.push(message);
                }
            }
            ChatEntryType::ToolCall => {
                let tool_call = entry
                    .tool_call
                    .as_ref()
                    .filter(|call| valid_tool_call(call))
                    .cloned();
                if let Some(tool_call) = tool_call {
                    known_tool_call_ids.insert(tool_call.id.clone());
                    messages.push(StarMessage::assistant_with_tool_calls(vec![tool_call]));
                }
            }
            ChatEntryType::ToolResult => {
                let Some(tool_call_id) = entry
                    .tool_call
                    .as_ref()
                    .map(|call| call.id.as_str())
                    .filter(|id| known_tool_call_ids.contains(*id))
                else {
                    continue;
                };
                let output = entry
                    .tool_result
                    .as_ref()
                    .and_then(|result| result.output.as_deref().or(result.error.as_deref()))
                    .filter(|output| !output.trim().is_empty());
                if let Some(output) = output {
                    messages.push(StarMessage::tool(tool_call_id, output));
                }
            }
            _ => {}
        }
    }

    messages
}

fn valid_tool_calls(calls: &[StarToolCall]) -> Vec<StarToolCall> {
    calls
        .iter()
        .filter(|call| valid_tool_call(call))
        .cloned()
        .collect()
}

fn valid_tool_call(call: &StarToolCall) -> bool {
    !call.id.trim().is_empty()
        && !call.function.name.trim().is_empty()
        && serde_json::from_str::<serde_json::Value>(&call.function.arguments).is_ok()
}

pub async fn load_session(id: &str) -> Result<Session, Box<dyn Error + Send + Sync>> {
    load_session_in(&sessions_dir()?, id).await
}

pub(crate) async fn load_session_in(
    dir: &Path,
    id: &str,
) -> Result<Session, Box<dyn Error + Send + Sync>> {
    let path = session_path_in(dir, id)?;
    match tokio::fs::metadata(&path).await {
        Ok(metadata) if !metadata.is_file() => {
            return Err(format!("Session '{}' is not a regular file", id).into())
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!("Session '{}' not found", id).into())
        }
        Err(error) => return Err(error.into()),
    }
    let content = tokio::fs::read_to_string(path).await?;
    Ok(serde_json::from_str(&content)?)
}

pub async fn load_latest_session() -> Result<Session, Box<dyn Error + Send + Sync>> {
    let Some(id) = read_latest_session_id().await? else {
        return Err("No latest session found. Use /chat save first.".into());
    };
    load_session(&id).await
}

pub async fn delete_session(id: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
    let p = session_path(id)?;

    if p.exists() {
        tokio::fs::remove_file(&p).await?;
    }
    Ok(())
}

pub async fn read_latest_session_id() -> Result<Option<String>, Box<dyn Error + Send + Sync>> {
    read_latest_session_id_in(&sessions_dir()?).await
}

pub(crate) async fn read_latest_session_id_in(
    dir: &Path,
) -> Result<Option<String>, Box<dyn Error + Send + Sync>> {
    let id = match tokio::fs::read_to_string(dir.join(".latest")).await {
        Ok(id) => id,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let id = id.trim();
    if id.is_empty() {
        return Ok(None);
    }

    let target = session_path_in(dir, id)?;
    match tokio::fs::metadata(target).await {
        Ok(_) => return Ok(Some(id.to_string())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    // 删除会话可能留下悬空指针；仅隐式恢复回退，显式 ID 仍由 load_session 报错。
    let mut newest: Option<(i64, String)> = None;
    let mut entries = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let Some(candidate) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if session_path_in(dir, candidate).is_err() || !entry.file_type().await?.is_file() {
            continue;
        }
        let content = match tokio::fs::read_to_string(&path).await {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        let session: Session = serde_json::from_str(&content)?;
        let candidate = (session.created_at, candidate.to_string());
        if newest.as_ref().is_none_or(|current| candidate > *current) {
            newest = Some(candidate);
        }
    }
    Ok(newest.map(|(_, id)| id))
}

fn summarize_session(id: &str, session: &Session, is_latest: bool) -> SessionSummary {
    SessionSummary {
        id: id.to_string(),
        title: session_title(session, id),
        subtitle: session_subtitle(session, is_latest),
        created_at: session.created_at,
    }
}

fn session_title(session: &Session, fallback_id: &str) -> String {
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
        .unwrap_or_else(|| fallback_session_title(fallback_id))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{StarToolCallFunction, ToolResult};

    fn tool_call(id: &str, name: &str, arguments: &str) -> StarToolCall {
        StarToolCall {
            id: id.to_string(),
            call_type: "function".to_string(),
            function: StarToolCallFunction {
                name: name.to_string(),
                arguments: arguments.to_string(),
            },
        }
    }

    struct SessionTestProject {
        original_dir: PathBuf,
        _directory: tempfile::TempDir,
    }

    impl SessionTestProject {
        fn new() -> Self {
            let original_dir = std::env::current_dir().expect("读取原工作目录");
            let directory = tempfile::tempdir().expect("创建隔离会话目录");
            std::fs::create_dir(directory.path().join(".star")).expect("隔离项目存储");
            // 会话存储读取缓存 cwd，必须使用正式切目录入口同步并清理路径缓存。
            crate::core::utils::paths::set_current_dir(directory.path()).expect("进入隔离项目");
            Self {
                original_dir,
                _directory: directory,
            }
        }
    }

    impl Drop for SessionTestProject {
        fn drop(&mut self) {
            crate::core::utils::paths::set_current_dir(&self.original_dir)
                .expect("恢复工作目录及缓存");
        }
    }

    async fn save_fixture_session(id: &str, created_at: i64) {
        save_session_record(&Session {
            id: id.to_string(),
            created_at,
            chat_history: vec![ChatEntry::user(id)],
            agent_messages: None,
            pending_local_context: None,
            last_usage: None,
        })
        .await
        .expect("保存隔离会话");
    }

    #[tokio::test]
    async fn deleting_latest_session_keeps_implicit_resume_available() {
        let _project = SessionTestProject::new();
        save_fixture_session("oldest", 10).await;
        save_fixture_session("remaining", 20).await;
        save_fixture_session("deleted", 30).await;
        delete_session("deleted").await.unwrap();

        assert_eq!(
            read_latest_session_id().await.unwrap().as_deref(),
            Some("remaining")
        );
        assert_eq!(load_latest_session().await.unwrap().id, "remaining");
        assert!(
            load_session("deleted").await.is_err(),
            "显式恢复不能偷换会话"
        );
    }

    #[tokio::test]
    async fn deleting_last_session_leaves_no_implicit_resume_target() {
        let _project = SessionTestProject::new();
        save_fixture_session("only", 10).await;
        delete_session("only").await.unwrap();
        assert_eq!(read_latest_session_id().await.unwrap(), None);
    }

    #[tokio::test]
    async fn latest_session_keeps_existing_marker_authoritative() {
        let _project = SessionTestProject::new();
        save_fixture_session("newer-timestamp", 100).await;
        save_fixture_session("chosen", 1).await;
        delete_session("newer-timestamp").await.unwrap();
        assert_eq!(
            read_latest_session_id().await.unwrap().as_deref(),
            Some("chosen")
        );
        std::fs::write(session_path("chosen").unwrap(), "{broken").unwrap();
        assert_eq!(
            read_latest_session_id().await.unwrap().as_deref(),
            Some("chosen")
        );
        assert!(
            load_latest_session().await.is_err(),
            "原指针损坏必须暴露错误"
        );
    }

    #[tokio::test]
    async fn latest_session_fallback_preserves_errors_and_canonical_filename() {
        let _project = SessionTestProject::new();
        save_fixture_session("canonical", 10).await;
        save_fixture_session("deleted", 20).await;
        delete_session("deleted").await.unwrap();
        let path = session_path("canonical").unwrap();
        let mut session: Session =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        session.id = "untrusted-embedded-id".to_string();
        std::fs::write(&path, serde_json::to_string(&session).unwrap()).unwrap();
        assert_eq!(
            read_latest_session_id().await.unwrap().as_deref(),
            Some("canonical")
        );
        std::fs::write(&path, "{broken").unwrap();
        assert!(
            read_latest_session_id().await.is_err(),
            "回退扫描不得吞掉损坏存档"
        );
    }

    #[tokio::test]
    async fn latest_session_empty_missing_invalid_and_unreadable_markers_stay_distinct() {
        let _project = SessionTestProject::new();
        assert_eq!(read_latest_session_id().await.unwrap(), None);
        save_fixture_session("saved", 1).await;
        let marker = latest_session_marker_path().unwrap();
        std::fs::write(&marker, " \n").unwrap();
        assert_eq!(read_latest_session_id().await.unwrap(), None);
        std::fs::write(&marker, "../outside").unwrap();
        assert!(read_latest_session_id().await.is_err());
        std::fs::remove_file(&marker).unwrap();
        std::fs::create_dir(&marker).unwrap();
        assert!(
            read_latest_session_id().await.is_err(),
            "标记读取 I/O 错误不得隐藏"
        );
    }

    #[test]
    fn session_path_rejects_filesystem_escape_ids() {
        for id in [
            "",
            ".",
            "..",
            "../outside",
            "/tmp/outside",
            "nested/name",
            "name.json",
        ] {
            assert!(
                session_path(id).is_err(),
                "{id:?} must not form a session path"
            );
        }
        assert!(session_path("safe-session_123").is_ok());
    }

    #[test]
    fn session_json_round_trips_native_messages_and_latest_usage() {
        let usage = StarUsage {
            prompt_tokens: 100,
            completion_tokens: 20,
            total_tokens: 120,
            cache_read_tokens: 80,
            cache_creation_tokens: 10,
            cache_telemetry_reported: true,
        };
        let messages = vec![
            StarMessage::user("question"),
            StarMessage::assistant("answer").with_reasoning("because"),
            StarMessage::assistant_with_tool_calls(vec![tool_call("call_1", "Read", "{}")]),
            StarMessage::tool("call_1", "result"),
        ];
        let session = Session {
            id: "snapshot".to_string(),
            created_at: 1,
            chat_history: vec![ChatEntry::user("question")],
            agent_messages: Some(messages),
            pending_local_context: Some(vec!["local test output".to_string()]),
            last_usage: Some(usage),
        };

        let restored: Session =
            serde_json::from_str(&serde_json::to_string(&session).unwrap()).unwrap();
        assert_eq!(restored.agent_messages.as_ref().unwrap().len(), 4);
        assert_eq!(
            restored.pending_local_context,
            Some(vec!["local test output".to_string()])
        );
        assert_eq!(
            restored.agent_messages.as_ref().unwrap()[2]
                .tool_calls
                .as_ref()
                .unwrap()[0]
                .id,
            "call_1"
        );
        let usage = restored.last_usage.unwrap();
        assert_eq!(usage.total_tokens, 120);
        assert_eq!(usage.cache_read_tokens, 80);
        assert!(usage.cache_telemetry_reported);
    }

    #[test]
    fn session_summary_uses_file_stem_as_canonical_identity() {
        let session = Session {
            id: "embedded-id".to_string(),
            created_at: 1,
            chat_history: Vec::new(),
            agent_messages: None,
            pending_local_context: None,
            last_usage: None,
        };

        let summary = summarize_session("file-stem", &session, true);
        assert_eq!(summary.id, "file-stem");
        assert!(summary.subtitle.starts_with("Latest · "));
    }

    #[test]
    fn legacy_session_json_uses_conservative_transcript_reconstruction() {
        let legacy = r#"{
            "id":"legacy",
            "created_at":1,
            "chat_history":[
                {"type":"User","content":"hello","timestamp":"2025-01-01T00:00:00Z"},
                {"type":"SystemMessage","content":"ui only","timestamp":"2025-01-01T00:00:01Z"},
                {"type":"Assistant","content":"hi","timestamp":"2025-01-01T00:00:02Z"}
            ]
        }"#;
        let session: Session = serde_json::from_str(legacy).unwrap();

        assert!(session.agent_messages.is_none());
        assert!(session.last_usage.is_none());
        let messages = agent_messages_for_restore(&session);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].content.as_deref(), Some("hi"));
    }

    #[test]
    fn explicit_empty_native_snapshot_does_not_fall_back_to_display_history() {
        let session = Session {
            id: "empty".to_string(),
            created_at: 1,
            chat_history: vec![ChatEntry::user("must not return")],
            agent_messages: Some(Vec::new()),
            pending_local_context: Some(Vec::new()),
            last_usage: None,
        };

        assert!(agent_messages_for_restore(&session).is_empty());
    }

    #[test]
    fn legacy_reconstruction_preserves_valid_tool_protocol_and_rejects_invalid_rows() {
        let valid = tool_call("call_ok", "Read", r#"{"path":"a.rs"}"#);
        let invalid = tool_call("call_bad", "Read", "not-json");
        let orphan = tool_call("call_orphan", "Bash", "{}");
        let mut welcome = ChatEntry::user("welcome");
        welcome.is_welcome = true;
        let streaming = ChatEntry::assistant("incomplete").with_streaming(true);
        let history = vec![
            welcome,
            ChatEntry::user("question"),
            ChatEntry::assistant("reasoned").with_reasoning("thinking"),
            ChatEntry::tool_call("Read", valid.clone()),
            ChatEntry::tool_result(
                "result",
                valid,
                ToolResult {
                    success: true,
                    output: Some("contents".to_string()),
                    error: None,
                    data: None,
                    llm_content: None,
                },
            ),
            ChatEntry::tool_call("bad", invalid),
            ChatEntry::tool_result(
                "orphan",
                orphan,
                ToolResult {
                    success: false,
                    output: None,
                    error: Some("failed".to_string()),
                    data: None,
                    llm_content: None,
                },
            ),
            streaming,
        ];

        let messages = reconstruct_legacy_agent_messages(&history);
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].reasoning_content.as_deref(), Some("thinking"));
        assert_eq!(messages[2].tool_calls.as_ref().unwrap()[0].id, "call_ok");
        assert_eq!(messages[3].role, "tool");
        assert_eq!(messages[3].tool_call_id.as_deref(), Some("call_ok"));
        assert_eq!(messages[3].content.as_deref(), Some("contents"));
        assert!(messages
            .iter()
            .all(|message| message.content.as_deref() != Some("incomplete")));
    }
}
