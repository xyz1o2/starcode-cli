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
    /// 解析项目状态目录。
    ///
    /// 从工作目录上溯找项目自己的 `.star`（子目录要能继承仓库根的状态），
    /// 但**跳过全局的 `~/.star`** —— 它是全局配置目录，不是某个项目的状态目录。
    /// 认错了它，项目的「总是允许」授权就会写进 home，在别的项目里一起放行。
    pub fn new(target_dir: PathBuf) -> Self {
        let target_dir = crate::core::utils::paths::find_nearest_project_star_dir(&target_dir)
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

    /// 全局「设置」文件：`~/.star/settings.json`
    ///
    /// 这里只放 settings 语义的键：`permissions`（见
    /// `policy::settings_rules`）、`hooks`（见 `hooks::settings_hooks`）、
    /// 以及后续的 model / effort 之类。
    ///
    /// **注意与 `user-settings.json` 的分工**：后者存 provider 列表、API key 和模型
    /// 缓存，由 `SettingsManager` 与 `ProviderStore` 共用（两个结构体反序列化同一
    /// 文件、彼此忽略未知字段），且自带一条 `providers.json` 的迁移链。两者键集
    /// 不重叠，所以**不是**重复，而是两种关注点。合并它们需要同时处理 provider 凭据
    /// 的持久化、`/config` 与 `doctor` 的读点，风险远大于「名字看着乱」的收益。
    ///
    /// 曾经这个函数没有任何调用方，`settings_rules` 内联拼了一遍同样的路径字符串 ——
    /// 那是真正会腐烂的地方。现在两边都走这里。
    pub fn global_settings_path() -> PathBuf {
        Self::global_star_dir().join("settings.json")
    }

    pub fn installation_id_path() -> PathBuf {
        Self::global_star_dir().join("installation_id")
    }

    /// 模型列表的磁盘缓存（按 provider 分桶），见 `agent::model_cache`。
    pub fn model_cache_path() -> PathBuf {
        Self::global_star_dir().join("model-cache.json")
    }

    /// 模型自调覆盖层的持久化落点，见 `core::self_tuning`。
    ///
    /// 刻意与 `settings.json` 分开：覆盖层是**运行中**生效的临时覆盖，
    /// 而 `settings.json` 是用户的正式配置。让模型自调写进正式配置文件，
    /// 会把一次会话内的临时决策变成用户的长期设置。
    pub fn self_tuning_path() -> PathBuf {
        Self::global_star_dir().join("self-tuning.json")
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

    /// StarLens 认知层目录 `.star/lens/`。
    pub fn lens_dir(&self) -> PathBuf {
        self.star_dir().join("lens")
    }

    /// Whole-Index 根文件（开工前注入模型的那份）。
    pub fn lens_index_path(&self) -> PathBuf {
        self.lens_dir().join("lens.txt")
    }

    /// 漂移检测的权威基线：object → source_sha256。
    pub fn lens_baseline_path(&self) -> PathBuf {
        self.lens_dir().join("baseline.json")
    }

    /// Managed Scope 规则快照。
    pub fn lens_scope_path(&self) -> PathBuf {
        self.lens_dir().join("scope.json")
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

    /// 手写权限规则文件（`{"rules":[...]}`），由 `PermissionRuleEngine` 读取。
    pub fn project_permissions_path(&self) -> PathBuf {
        self.star_dir().join("permissions.json")
    }

    /// "总是允许"授权签名（JSON 字符串数组）。
    ///
    /// 必须与 [`Self::project_permissions_path`] 分开：两者曾共用
    /// `permissions.json`，但格式互不兼容（数组 vs 对象），一次"总是允许"
    /// 就会把手写规则整份覆盖掉，且解析错误被吞掉、用户毫无察觉。
    /// 拆成独立文件后，写入方与读取方各自拥有自己的格式。
    pub fn project_permission_grants_path(&self) -> PathBuf {
        self.star_dir().join("permission_grants.json")
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

    /// 回归：项目状态目录不能是全局的 `~/.star`。
    ///
    /// `Storage` 会从工作目录上溯找 `.star`，好让子目录继承仓库根的状态。但
    /// 项目建在 home 之下时（Linux 的 `~/code/foo` 尤其常见），上溯会直接撞上
    /// `~/.star` —— 那是全局配置目录。于是：
    ///
    /// - 「总是允许」的授权写进 home，在**别的**项目里同样生效；
    /// - 反过来，home 下任意一个项目点一次「总是允许」，全部一起放行。
    ///
    /// 授权的爆炸半径必须等于当前项目。
    #[test]
    fn project_storage_never_adopts_the_global_star_dir() {
        let Some(home) = dirs::home_dir() else {
            return;
        };
        // 模拟 home 下没有项目自己的 `.star`：只有全局那个。
        let nested = home.join(".star-storage-probe").join("project");
        std::fs::create_dir_all(&nested).expect("create probe project dir");

        let storage = Storage::new(nested.clone());
        assert_ne!(
            storage.target_dir, home,
            "项目状态不能落在 home 上，否则授权会跨项目泄漏"
        );
        assert_eq!(
            storage.star_dir(),
            nested.join(".star"),
            "找不到项目自己的 `.star` 时，应当就在项目目录下建一个"
        );

        std::fs::remove_dir_all(nested.parent().expect("probe parent")).ok();
    }

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

    /// "总是允许"的授权签名必须写在自己的文件里。
    ///
    /// 回归：两种格式曾共用 `permissions.json` —— 授权签名写 JSON 数组，
    /// 手写规则读 `{"rules":[...]}` 对象。用户点一次"总是允许"，
    /// 手写的 deny 规则就被整份覆盖且无任何报错。
    #[test]
    fn permission_grants_do_not_share_a_file_with_hand_written_rules() {
        let storage = Storage::new(std::env::temp_dir());
        assert_ne!(
            storage.project_permissions_path(),
            storage.project_permission_grants_path(),
            "grant 存储与手写规则文件必须分离，否则一次写入就会清空另一方"
        );
    }
}
