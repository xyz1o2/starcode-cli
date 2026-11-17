//! 子代理终态的有效原生上下文。快照不包含权限、凭据、端点或可执行 cwd。
//! 所有文件操作由调用方等待，唯一临时文件与目标同目录；不持有运行注册表锁。
//! 仅正常退出、失败和协作取消可发布；进程崩溃只能恢复最后已发布的快照。

use crate::core::agents::SubagentType;
use crate::core::config::storage::Storage;
use crate::types::StarMessage;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentSession {
    version: u32,
    task_id: String,
    project: std::path::PathBuf,
    pub subagent_type: SubagentType,
    pub messages: Vec<StarMessage>,
    pub pending_context: Vec<String>,
}

pub(crate) fn valid_task_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl AgentSession {
    pub fn new(
        storage: &Storage,
        task_id: String,
        subagent_type: SubagentType,
        messages: Vec<StarMessage>,
        pending_context: Vec<String>,
    ) -> Result<Self, String> {
        if !valid_task_id(&task_id) {
            return Err("Invalid agent session ID".into());
        }
        Ok(Self {
            version: 1,
            task_id,
            project: project_identity(storage)?,
            subagent_type,
            messages,
            pending_context,
        })
    }

    pub async fn save(&self, storage: &Storage) -> Result<(), String> {
        let directory = storage.project_subagent_sessions_dir();
        tokio::fs::create_dir_all(&directory)
            .await
            .map_err(|e| format!("Failed to create agent session directory: {e}"))?;
        let destination = directory.join(format!("{}.json", self.task_id));
        let temporary = directory.join(format!("{}.{}.tmp", self.task_id, uuid::Uuid::new_v4()));
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        let result = async {
            use tokio::io::AsyncWriteExt;
            let mut file = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .await?;
            file.write_all(&bytes).await?;
            file.sync_all().await?;
            drop(file);
            // 快照只发布一次；硬链接以完整文件原子创建目标，既有来源绝不覆盖。
            tokio::fs::hard_link(&temporary, &destination).await?;
            // 目标已完整发布；清理失败不得谎报为快照保存失败。
            if let Err(error) = tokio::fs::remove_file(&temporary).await {
                crate::utils::logging::append_agent_log_line(&format!(
                    "[SubagentSession] snapshot published, temporary cleanup failed: {error}"
                ));
            }
            Ok::<(), std::io::Error>(())
        }
        .await;
        if let Err(error) = result {
            let _ = tokio::fs::remove_file(temporary).await;
            return Err(format!(
                "Failed to save agent session {}: {error}",
                self.task_id
            ));
        }
        Ok(())
    }

    pub async fn load(storage: &Storage, task_id: &str) -> Result<Self, String> {
        if !valid_task_id(task_id) {
            return Err("Invalid agent session ID".into());
        }
        let bytes = tokio::fs::read(
            storage
                .project_subagent_sessions_dir()
                .join(format!("{task_id}.json")),
        )
        .await
        .map_err(|e| format!("Cannot resume agent {task_id}: {e}"))?;
        let session: Self = serde_json::from_slice(&bytes)
            .map_err(|e| format!("Invalid agent session {task_id}: {e}"))?;
        if session.version != 1
            || session.task_id != task_id
            || session.project != project_identity(storage)?
        {
            return Err(format!(
                "Invalid agent session {task_id}: version, ID or project mismatch"
            ));
        }
        Ok(session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn native_snapshot_keeps_empty_context_and_rejects_invalid_sources() {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::new(root.path().to_owned());
        let mut message = StarMessage::assistant("answer").with_reasoning("reasoning");
        message.cache_control = Some(serde_json::json!({"type":"ephemeral"}));
        let session = AgentSession::new(
            &storage,
            "agent-old".into(),
            SubagentType::Explorer,
            vec![StarMessage::user("question"), message],
            vec!["pending output".into()],
        )
        .unwrap();
        session.save(&storage).await.unwrap();
        let source = tokio::fs::read(
            storage
                .project_subagent_sessions_dir()
                .join("agent-old.json"),
        )
        .await
        .unwrap();
        assert!(session.save(&storage).await.is_err());
        assert_eq!(
            source,
            tokio::fs::read(
                storage
                    .project_subagent_sessions_dir()
                    .join("agent-old.json")
            )
            .await
            .unwrap()
        );
        let loaded = AgentSession::load(&storage, "agent-old").await.unwrap();
        assert_eq!(
            serde_json::to_value(&loaded.messages).unwrap(),
            serde_json::to_value(&session.messages).unwrap()
        );
        assert_eq!(loaded.pending_context, session.pending_context);
        assert_eq!(loaded.subagent_type, SubagentType::Explorer);
        assert!(!storage.star_dir().join(".latest").exists());
        AgentSession::new(
            &storage,
            "empty".into(),
            SubagentType::GeneralPurpose,
            vec![],
            vec![],
        )
        .unwrap()
        .save(&storage)
        .await
        .unwrap();
        assert!(AgentSession::load(&storage, "empty")
            .await
            .unwrap()
            .messages
            .is_empty());
        for id in ["../escape", "", "a/b", "a\\b", "missing"] {
            assert!(AgentSession::load(&storage, id).await.is_err());
        }
        let other_root = tempfile::tempdir().unwrap();
        let other = Storage::new(other_root.path().to_owned());
        tokio::fs::create_dir_all(other.project_subagent_sessions_dir())
            .await
            .unwrap();
        let path = storage
            .project_subagent_sessions_dir()
            .join("agent-old.json");
        let original = tokio::fs::read(&path).await.unwrap();
        tokio::fs::write(
            other.project_subagent_sessions_dir().join("agent-old.json"),
            &original,
        )
        .await
        .unwrap();
        assert!(AgentSession::load(&other, "agent-old")
            .await
            .err()
            .unwrap()
            .contains("project mismatch"));
        for field in ["version", "task_id"] {
            let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
            value[field] = if field == "version" {
                serde_json::json!(99)
            } else {
                serde_json::json!("other")
            };
            tokio::fs::write(&path, serde_json::to_vec(&value).unwrap())
                .await
                .unwrap();
            assert!(AgentSession::load(&storage, "agent-old").await.is_err());
        }
        tokio::fs::write(&path, "{broken").await.unwrap();
        assert!(AgentSession::load(&storage, "agent-old").await.is_err());
        let blocked_root = tempfile::tempdir().unwrap();
        let bad_storage = Storage::new(blocked_root.path().to_owned());
        tokio::fs::write(bad_storage.star_dir(), "not a directory")
            .await
            .unwrap();
        assert!(session.save(&bad_storage).await.is_err());
    }
}

fn project_identity(storage: &Storage) -> Result<std::path::PathBuf, String> {
    let star = storage.star_dir();
    let root = star.parent().ok_or("Invalid project storage root")?;
    std::fs::canonicalize(root).map_err(|e| format!("Cannot identify agent project: {e}"))
}
