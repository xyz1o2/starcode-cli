use dirs::home_dir;
use std::env;
use std::path::PathBuf;

pub const OAUTH_FILE: &str = "oauth_creds.json";
const TMP_DIR_NAME: &str = "tmp";
const BIN_DIR_NAME: &str = "bin";
const STAR_DIR: &str = ".star";

#[derive(Clone)]
pub struct Storage {
    target_dir: PathBuf,
}

impl Storage {
    pub fn new(target_dir: PathBuf) -> Self {
        let target_dir = crate::core::utils::paths::find_nearest_existing_star_dir(&target_dir)
            .and_then(|star_dir| star_dir.parent().map(|parent| parent.to_path_buf()))
            .unwrap_or(target_dir);

        Self { target_dir }
    }

    pub fn global_star_dir() -> PathBuf {
        if let Some(home) = home_dir() {
            home.join(STAR_DIR)
        } else {
            env::temp_dir().join(STAR_DIR)
        }
    }

    pub fn mcp_oauth_tokens_path() -> PathBuf {
        Self::global_star_dir().join("mcp-oauth-tokens.json")
    }

    pub fn global_settings_path() -> PathBuf {
        Self::global_star_dir().join("settings.json")
    }

    pub fn installation_id_path() -> PathBuf {
        Self::global_star_dir().join("installation_id")
    }

    pub fn user_commands_dir() -> PathBuf {
        Self::global_star_dir().join("commands")
    }

    pub fn user_skills_dir() -> PathBuf {
        Self::global_star_dir().join("skills")
    }

    pub fn global_memory_file_path() -> PathBuf {
        Self::global_star_dir().join("memory.md")
    }

    pub fn user_policies_dir() -> PathBuf {
        Self::global_star_dir().join("policies")
    }

    pub fn user_agents_dir() -> PathBuf {
        Self::global_star_dir().join("agents")
    }

    pub fn global_i18n_dir() -> PathBuf {
        Self::global_star_dir().join("i18n")
    }

    pub fn global_temp_dir() -> PathBuf {
        Self::global_star_dir().join(TMP_DIR_NAME)
    }

    pub fn global_bin_dir() -> PathBuf {
        Self::global_temp_dir().join(BIN_DIR_NAME)
    }

    pub fn star_dir(&self) -> PathBuf {
        self.target_dir.join(STAR_DIR)
    }

    pub fn project_temp_dir(&self) -> PathBuf {
        self.star_dir().join(TMP_DIR_NAME)
    }

    pub fn ensure_project_temp_dir_exists(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.project_temp_dir())
    }

    pub fn oauth_creds_path() -> PathBuf {
        Self::global_star_dir().join(OAUTH_FILE)
    }

    pub fn project_root(&self) -> PathBuf {
        self.target_dir.clone()
    }

    pub fn history_dir(&self) -> PathBuf {
        self.star_dir().join("history")
    }

    pub fn workspace_settings_path(&self) -> PathBuf {
        self.star_dir().join("settings.json")
    }

    pub fn project_mcp_config_path(&self) -> PathBuf {
        self.star_dir().join("mcp.json")
    }

    pub fn project_commands_dir(&self) -> PathBuf {
        self.star_dir().join("commands")
    }

    pub fn project_skills_dir(&self) -> PathBuf {
        self.star_dir().join("skills")
    }

    pub fn project_agents_dir(&self) -> PathBuf {
        self.star_dir().join("agents")
    }

    pub fn project_i18n_dir(&self) -> PathBuf {
        self.star_dir().join("i18n")
    }

    pub fn project_temp_checkpoints_dir(&self) -> PathBuf {
        self.project_temp_dir().join("checkpoints")
    }

    pub fn extensions_dir(&self) -> PathBuf {
        self.star_dir().join("extensions")
    }

    pub fn extensions_config_path(&self) -> PathBuf {
        self.extensions_dir().join("star-extension.json")
    }

    pub fn history_file_path(&self) -> PathBuf {
        self.project_temp_dir().join("shell_history")
    }

    pub fn project_permissions_path(&self) -> PathBuf {
        self.star_dir().join("permissions.json")
    }

    /// 会话消息持久化路径，按会话 ID 隔离（对标 Claude Code 的
    /// `<projectDir>/<sessionId>.jsonl` per-session transcript）。
    /// 新会话 = 新 ID = 空上下文；只有 --resume 才会加载对应文件。
    /// 放在 `sessions/<id>/` 子目录而非 `sessions/<id>.json`，
    /// 避免 session_manager 的 `*.json` 扫描把它误当成 UI 会话。
    pub fn session_messages_path(&self, session_id: &str) -> PathBuf {
        self.star_dir()
            .join("sessions")
            .join(session_id)
            .join("messages.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 会话消息路径必须按会话 ID 隔离：不同会话读写不同文件，
    /// 否则新会话会继承上一个会话的完整上下文（对标 Claude Code
    /// 的 per-session transcript）。
    #[test]
    fn session_messages_paths_are_isolated_per_session() {
        let storage = Storage::new(std::env::temp_dir());
        let a = storage.session_messages_path("aaaa-bbbb");
        let b = storage.session_messages_path("cccc-dddd");
        assert_ne!(a, b);
        assert_eq!(
            a.parent().and_then(|p| p.file_name()),
            Some(std::ffi::OsStr::new("aaaa-bbbb"))
        );
        assert_eq!(a.file_name(), Some(std::ffi::OsStr::new("messages.json")));
    }

    /// 会话消息文件位于 `sessions/<id>/` 子目录内而不是 `sessions/<id>.json`，
    /// 这样 session_manager 扫描 `sessions/*.json` 时不会把它误当成 UI 会话。
    #[test]
    fn session_messages_path_lives_inside_session_subdir() {
        let storage = Storage::new(std::env::temp_dir());
        let path = storage.session_messages_path("sess-1");
        let sessions_dir = storage.star_dir().join("sessions");
        assert_eq!(
            path.parent().map(|p| p.to_path_buf()),
            Some(sessions_dir.join("sess-1"))
        );
    }
}
