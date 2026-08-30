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

    pub fn session_messages_path(&self) -> PathBuf {
        self.star_dir().join("session_messages.json")
    }
}
 