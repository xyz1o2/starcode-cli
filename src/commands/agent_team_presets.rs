use crate::core::config::storage::Storage;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TeamPreset {
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) agents: Vec<String>,
    pub(crate) target: Option<String>,
    pub(crate) max_steps: Option<usize>,
    pub(crate) parallelism: Option<usize>,
    pub(crate) mode: Option<String>,
    pub(crate) rounds: Option<usize>,
    pub(crate) timeout_secs: Option<u64>,
    pub(crate) dry_run: Option<bool>,
    pub(crate) objective: Option<String>,
    pub(crate) updated_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct TeamPresetStore {
    pub(crate) teams: Vec<TeamPreset>,
}

#[derive(Clone, Copy)]
pub(crate) enum TeamPresetScope {
    Project,
    User,
}

pub(crate) fn team_preset_file_path(scope: TeamPresetScope, storage: &Storage) -> PathBuf {
    match scope {
        TeamPresetScope::Project => storage.star_dir().join("agent-teams.json"),
        TeamPresetScope::User => Storage::global_star_dir().join("agent-teams.json"),
    }
}

pub(crate) async fn load_team_preset_store(path: &Path) -> Result<TeamPresetStore, String> {
    if !path.exists() {
        return Ok(TeamPresetStore::default());
    }

    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| format!("failed to read {}: {}", path.display(), e))?;
    if content.trim().is_empty() {
        return Ok(TeamPresetStore::default());
    }

    serde_json::from_str(&content).map_err(|e| format!("failed to parse {}: {}", path.display(), e))
}

pub(crate) async fn save_team_preset_store(
    path: &Path,
    store: &TeamPresetStore,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("failed to create directory {}: {}", parent.display(), e))?;
    }
    let content = serde_json::to_string_pretty(store)
        .map_err(|e| format!("failed to serialize team preset store: {}", e))?;
    tokio::fs::write(path, content)
        .await
        .map_err(|e| format!("failed to write {}: {}", path.display(), e))
}

pub(crate) async fn list_team_presets(
    storage: &Storage,
) -> Result<(Vec<TeamPreset>, Vec<TeamPreset>), String> {
    let project_path = team_preset_file_path(TeamPresetScope::Project, storage);
    let user_path = team_preset_file_path(TeamPresetScope::User, storage);

    let mut project = load_team_preset_store(&project_path).await?.teams;
    let mut user = load_team_preset_store(&user_path).await?.teams;

    project.sort_by(|a, b| a.name.cmp(&b.name));
    user.sort_by(|a, b| a.name.cmp(&b.name));
    Ok((project, user))
}

pub(crate) async fn resolve_team_preset(
    storage: &Storage,
    name: &str,
) -> Result<Option<(TeamPreset, TeamPresetScope)>, String> {
    let normalized = name.trim().to_lowercase();
    if normalized.is_empty() {
        return Ok(None);
    }

    let project_path = team_preset_file_path(TeamPresetScope::Project, storage);
    let user_path = team_preset_file_path(TeamPresetScope::User, storage);

    let project = load_team_preset_store(&project_path).await?;
    if let Some(found) = project
        .teams
        .into_iter()
        .find(|t| t.name.to_lowercase() == normalized)
    {
        return Ok(Some((found, TeamPresetScope::Project)));
    }

    let user = load_team_preset_store(&user_path).await?;
    if let Some(found) = user
        .teams
        .into_iter()
        .find(|t| t.name.to_lowercase() == normalized)
    {
        return Ok(Some((found, TeamPresetScope::User)));
    }

    Ok(None)
}

pub(crate) fn scope_label(scope: TeamPresetScope) -> &'static str {
    match scope {
        TeamPresetScope::Project => "project",
        TeamPresetScope::User => "user",
    }
}

pub(crate) fn sanitize_preset_name(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn storage_in(dir: &Path) -> Storage {
        Storage::new(dir.to_path_buf())
    }

    fn preset(name: &str, updated_at: i64) -> TeamPreset {
        TeamPreset {
            name: name.to_string(),
            description: Some(format!("{name} description")),
            agents: vec!["alice".to_string(), "bob".to_string()],
            target: Some("src".to_string()),
            max_steps: Some(3),
            parallelism: Some(2),
            mode: Some("parallel".to_string()),
            rounds: Some(1),
            timeout_secs: Some(60),
            dry_run: Some(false),
            objective: Some("fix the build".to_string()),
            updated_at,
        }
    }

    // ── 名称清洗 ────────────────────────────────────────────────

    /// 预设名会变成 JSON 里的 key，也是 `/agents team show <name>` 的入口；
    /// 允许路径分隔符等于允许把配置文件写到别处。
    #[test]
    fn preset_names_keep_only_safe_characters() {
        assert_eq!(sanitize_preset_name("review-team"), "review-team");
        assert_eq!(sanitize_preset_name("review_team"), "review_team");
        assert_eq!(sanitize_preset_name("Review Team"), "ReviewTeam");
        assert_eq!(sanitize_preset_name("../../etc/passwd"), "etcpasswd");
        assert_eq!(sanitize_preset_name("a/b\\c"), "abc");
        assert_eq!(sanitize_preset_name("团队"), "");
    }

    // ── 存取 ────────────────────────────────────────────────────

    #[tokio::test]
    async fn a_missing_file_loads_as_an_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent-teams.json");
        let store = load_team_preset_store(&path).await.unwrap();
        assert!(store.teams.is_empty());
    }

    /// 空文件（用户 touch 出来的）不该被当成损坏而报错。
    #[tokio::test]
    async fn an_empty_file_loads_as_an_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent-teams.json");
        std::fs::write(&path, "   \n").unwrap();
        assert!(load_team_preset_store(&path)
            .await
            .unwrap()
            .teams
            .is_empty());
    }

    #[tokio::test]
    async fn a_corrupt_file_names_itself_in_the_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent-teams.json");
        std::fs::write(&path, "{not json").unwrap();
        let err = load_team_preset_store(&path).await.unwrap_err();
        assert!(err.contains("failed to parse"), "{err}");
        assert!(err.contains("agent-teams.json"), "{err}");
    }

    #[tokio::test]
    async fn saving_creates_parent_directories_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".star").join("agent-teams.json");
        let store = TeamPresetStore {
            teams: vec![preset("review", 100)],
        };
        save_team_preset_store(&path, &store).await.unwrap();

        let loaded = load_team_preset_store(&path).await.unwrap();
        assert_eq!(loaded.teams.len(), 1);
        assert_eq!(loaded.teams[0].name, "review");
        assert_eq!(loaded.teams[0].agents, ["alice", "bob"]);
        assert_eq!(loaded.teams[0].objective.as_deref(), Some("fix the build"));
    }

    // ── 列表 ────────────────────────────────────────────────────

    /// 列表按名字排序：`/agents team list` 的输出必须是可复现的，
    /// 不能取决于文件里写入的先后。
    #[tokio::test]
    async fn listing_sorts_both_scopes_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let storage = storage_in(dir.path());
        let path = team_preset_file_path(TeamPresetScope::Project, &storage);
        save_team_preset_store(
            &path,
            &TeamPresetStore {
                teams: vec![preset("zebra", 1), preset("alpha", 2), preset("mango", 3)],
            },
        )
        .await
        .unwrap();

        let (project, _user) = list_team_presets(&storage).await.unwrap();
        let names: Vec<&str> = project.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["alpha", "mango", "zebra"]);
    }

    #[tokio::test]
    async fn the_project_file_lives_under_the_project_star_dir() {
        let dir = tempfile::tempdir().unwrap();
        let storage = storage_in(dir.path());
        let path = team_preset_file_path(TeamPresetScope::Project, &storage);
        assert_eq!(path, storage.star_dir().join("agent-teams.json"));
    }

    // ── 解析 ────────────────────────────────────────────────────

    /// 解析先查项目再查用户：项目级的同名预设必须能覆盖全局的那份。
    #[tokio::test]
    async fn resolution_prefers_the_project_scope() {
        let dir = tempfile::tempdir().unwrap();
        let storage = storage_in(dir.path());
        let path = team_preset_file_path(TeamPresetScope::Project, &storage);
        save_team_preset_store(
            &path,
            &TeamPresetStore {
                teams: vec![preset("review", 1)],
            },
        )
        .await
        .unwrap();

        let (found, scope) = resolve_team_preset(&storage, "review")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.name, "review");
        assert!(matches!(scope, TeamPresetScope::Project));
    }

    /// 用户习惯随手敲大小写；名字匹配必须大小写不敏感，否则"明明存过"
    /// 却报 not found。
    #[tokio::test]
    async fn resolution_is_case_insensitive_and_trims_whitespace() {
        let dir = tempfile::tempdir().unwrap();
        let storage = storage_in(dir.path());
        let path = team_preset_file_path(TeamPresetScope::Project, &storage);
        save_team_preset_store(
            &path,
            &TeamPresetStore {
                teams: vec![preset("review-team", 1)],
            },
        )
        .await
        .unwrap();

        assert!(resolve_team_preset(&storage, "  REVIEW-TEAM  ")
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn an_unknown_or_empty_name_resolves_to_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let storage = storage_in(dir.path());
        assert!(resolve_team_preset(&storage, "nope")
            .await
            .unwrap()
            .is_none());
        assert!(resolve_team_preset(&storage, "   ")
            .await
            .unwrap()
            .is_none());
    }

    #[test]
    fn scope_labels_match_the_cli_flags() {
        assert_eq!(scope_label(TeamPresetScope::Project), "project");
        assert_eq!(scope_label(TeamPresetScope::User), "user");
    }
}
