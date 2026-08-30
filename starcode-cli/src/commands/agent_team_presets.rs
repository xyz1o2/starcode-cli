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
