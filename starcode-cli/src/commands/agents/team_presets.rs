use super::team_definitions::{normalize_agent_name, find_team_agent, resolve_team_agents, team_run_mode_label, parse_team_run_mode};
use super::*;

pub(super) async fn save_team_preset(ctx: CommandContext<'_>, args: AgentTeamSaveArgs) -> CommandResult {
    let preset_name = sanitize_preset_name(&args.name);
    if preset_name.is_empty() {
        return Err("invalid preset name".to_string());
    }

    let resolved_agents = resolve_team_agents(args.agents.clone())?;
    let agents: Vec<String> = resolved_agents
        .iter()
        .map(|a| a.cli_name.to_string())
        .collect();

    if let Some(ms) = args.max_steps {
        if ms == 0 || ms > 30 {
            return Err("max_steps must be between 1 and 30".to_string());
        }
    }
    if let Some(p) = args.parallelism {
        if p == 0 || p > 16 {
            return Err("parallelism must be between 1 and 16".to_string());
        }
    }
    if let Some(r) = args.rounds {
        if r == 0 || r > 8 {
            return Err("rounds must be between 1 and 8".to_string());
        }
    }

    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let storage = crate::core::config::storage::Storage::new(cwd);
    let scope = if args.user {
        TeamPresetScope::User
    } else {
        TeamPresetScope::Project
    };
    let path = team_preset_file_path(scope, &storage);

    let mut store = load_team_preset_store(&path).await?;
    let now = Utc::now().timestamp();
    let new_preset = TeamPreset {
        name: preset_name.clone(),
        description: args.description.clone().filter(|s| !s.trim().is_empty()),
        agents,
        target: args.target.clone().filter(|s| !s.trim().is_empty()),
        max_steps: args.max_steps,
        parallelism: args.parallelism,
        mode: args
            .mode
            .as_ref()
            .map(|m| team_run_mode_label(m).to_string()),
        rounds: args.rounds,
        timeout_secs: args.timeout_secs,
        dry_run: Some(args.dry_run),
        objective: args.objective.clone().filter(|s| !s.trim().is_empty()),
        updated_at: now,
    };

    if let Some(existing) = store.teams.iter_mut().find(|t| t.name == preset_name) {
        *existing = new_preset;
    } else {
        store.teams.push(new_preset);
    }
    store.teams.sort_by(|a, b| a.name.cmp(&b.name));
    save_team_preset_store(&path, &store).await?;

    let msg = format!(
        "✅ Team preset 已保存

- name: `{}`
- scope: `{}`
- path: `{}`",
        preset_name,
        scope_label(scope),
        path.display()
    );
    ctx.state
        .chat_history
        .push(ChatEntry::assistant(msg).with_streaming(false));
    Ok(())
}

pub(super) async fn show_team_preset(ctx: CommandContext<'_>, name: String) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let storage = crate::core::config::storage::Storage::new(cwd);
    let Some((preset, scope)) = resolve_team_preset(&storage, &name).await? else {
        return Err(format!(
            "team preset '{}' not found (checked project + user scopes)",
            name
        ));
    };

    let mut lines = vec![
        "# Team Preset".to_string(),
        format!("- name: `{}`", preset.name),
        format!("- scope: `{}`", scope_label(scope)),
        format!("- agents: {}", preset.agents.join(",")),
        format!(
            "- target: {}",
            preset.target.unwrap_or_else(|| "(none)".to_string())
        ),
        format!(
            "- max_steps: {}",
            preset
                .max_steps
                .map(|v| v.to_string())
                .unwrap_or_else(|| "(none)".to_string())
        ),
        format!(
            "- parallelism: {}",
            preset
                .parallelism
                .map(|v| v.to_string())
                .unwrap_or_else(|| "(none)".to_string())
        ),
        format!(
            "- mode: {}",
            preset.mode.unwrap_or_else(|| "parallel".to_string())
        ),
        format!(
            "- rounds: {}",
            preset
                .rounds
                .map(|v| v.to_string())
                .unwrap_or_else(|| "1".to_string())
        ),
        format!(
            "- timeout_secs: {}",
            preset
                .timeout_secs
                .map(|v| v.to_string())
                .unwrap_or_else(|| "(none)".to_string())
        ),
        format!("- dry_run: {}", preset.dry_run.unwrap_or(false)),
        format!(
            "- description: {}",
            preset.description.unwrap_or_else(|| "(none)".to_string())
        ),
        format!(
            "- objective: {}",
            preset.objective.unwrap_or_else(|| "(none)".to_string())
        ),
    ];

    lines.push("".to_string());
    lines.push("Examples:".to_string());
    lines.push(format!(
        "- `/agents team run --team {} <objective...>`",
        name
    ));

    ctx.state
        .chat_history
        .push(ChatEntry::assistant(lines.join("
")).with_streaming(false));
    Ok(())
}

pub(super) async fn remove_team_preset(ctx: CommandContext<'_>, name: String, user: bool) -> CommandResult {
    let preset_name = sanitize_preset_name(&name);
    if preset_name.is_empty() {
        return Err("invalid preset name".to_string());
    }

    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let storage = crate::core::config::storage::Storage::new(cwd);
    let scope = if user {
        TeamPresetScope::User
    } else {
        TeamPresetScope::Project
    };
    let path = team_preset_file_path(scope, &storage);

    let mut store = load_team_preset_store(&path).await?;
    let before = store.teams.len();
    store.teams.retain(|t| t.name != preset_name);

    if store.teams.len() == before {
        let msg = format!(
            "⚠️ 未找到 team preset `{}` in {} scope",
            preset_name,
            scope_label(scope)
        );
        ctx.state
            .chat_history
            .push(ChatEntry::assistant(msg).with_streaming(false));
        return Ok(());
    }

    save_team_preset_store(&path, &store).await?;
    let msg = format!(
        "✅ 已删除 team preset `{}` from {} scope",
        preset_name,
        scope_label(scope)
    );
    ctx.state
        .chat_history
        .push(ChatEntry::assistant(msg).with_streaming(false));
    Ok(())
}

#[derive(Clone)]
pub(super) struct TeamRuntimeInfo {
    pub(super) model: String,
    pub(super) base_url: String,
    pub(super) active_provider: Option<String>,
}
