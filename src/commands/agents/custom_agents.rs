use super::*;

pub(super) async fn list_agents(ctx: CommandContext<'_>) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let storage = crate::core::config::storage::Storage::new(cwd);

    let project_dir = storage.project_agents_dir();
    let user_dir = crate::core::config::storage::Storage::user_agents_dir();
    let effective_defs = load_custom_subagent_definitions(&storage.project_root());

    let mut lines = vec!["# Agents
"
    .to_string()];

    let project_agents = list_agent_files(&project_dir).await?;
    lines.push(format!("## Project ({})", project_dir.to_string_lossy()));
    if project_agents.is_empty() {
        lines.push("- (empty)".to_string());
    } else {
        for name in project_agents {
            lines.push(format!("- {}", name));
        }
    }

    let user_agents = list_agent_files(&user_dir).await?;
    lines.push(format!(
        "
## User ({})",
        user_dir.to_string_lossy()
    ));
    if user_agents.is_empty() {
        lines.push("- (empty)".to_string());
    } else {
        for name in user_agents {
            lines.push(format!("- {}", name));
        }
    }

    lines.push(
        "
## Resolved Registry (project overrides user)"
            .to_string(),
    );
    if effective_defs.is_empty() {
        lines.push("- (empty)".to_string());
    } else {
        for def in effective_defs {
            let scope = if def.source_path.starts_with(&project_dir) {
                "project"
            } else if def.source_path.starts_with(&user_dir) {
                "user"
            } else {
                "unknown"
            };
            lines.push(format!(
                "- `{}` ({}) [{}] -> {}",
                def.id, def.name, scope, def.description
            ));
        }
    }

    lines.push("".to_string());
    lines.push("Examples:".to_string());
    lines.push("- `/agents create reviewer --description \"PR review quality gate\" --tools Read,Grep --aliases review,qa`".to_string());
    lines.push(
        "- `/agents edit reviewer --description \"Rust + tests review\" --model gpt-5`".to_string(),
    );
    lines.push("- `/agents delete reviewer`".to_string());

    ctx.state.chat_history.push(
        crate::types::ChatEntry::assistant(lines.join(
            "
",
        ))
        .with_streaming(false),
    );

    Ok(())
}

#[derive(Clone, Copy)]
enum AgentScope {
    Project,
    User,
}

fn agent_scope_label(scope: AgentScope) -> &'static str {
    match scope {
        AgentScope::Project => "project",
        AgentScope::User => "user",
    }
}

fn agent_scope_dir(scope: AgentScope, storage: &crate::core::config::storage::Storage) -> PathBuf {
    match scope {
        AgentScope::Project => storage.project_agents_dir(),
        AgentScope::User => crate::core::config::storage::Storage::user_agents_dir(),
    }
}

fn normalize_name_for_lookup(name: &str) -> String {
    let raw = name
        .trim()
        .trim_end_matches(".markdown")
        .trim_end_matches(".md");
    normalize_custom_agent_id(raw)
}

fn list_agent_paths_in_dir(dir: &Path) -> Vec<PathBuf> {
    if !dir.exists() {
        return Vec::new();
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.ends_with(".md") || name.ends_with(".markdown") {
            out.push(path);
        }
    }
    out.sort();
    out
}

fn find_agent_path_in_dir(dir: &Path, name: &str) -> Option<PathBuf> {
    let normalized = normalize_name_for_lookup(name);
    if normalized.is_empty() {
        return None;
    }

    let direct_md = dir.join(format!("{}.md", normalized));
    if direct_md.exists() {
        return Some(direct_md);
    }
    let direct_markdown = dir.join(format!("{}.markdown", normalized));
    if direct_markdown.exists() {
        return Some(direct_markdown);
    }

    list_agent_paths_in_dir(dir).into_iter().find(|path| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(normalize_custom_agent_id)
            .map(|stem| stem == normalized)
            .unwrap_or(false)
    })
}

fn resolve_agent_path(
    storage: &crate::core::config::storage::Storage,
    name: &str,
    user_only: bool,
) -> Option<(PathBuf, AgentScope)> {
    if user_only {
        let user_dir = agent_scope_dir(AgentScope::User, storage);
        return find_agent_path_in_dir(&user_dir, name).map(|p| (p, AgentScope::User));
    }

    let project_dir = agent_scope_dir(AgentScope::Project, storage);
    if let Some(path) = find_agent_path_in_dir(&project_dir, name) {
        return Some((path, AgentScope::Project));
    }
    let user_dir = agent_scope_dir(AgentScope::User, storage);
    find_agent_path_in_dir(&user_dir, name).map(|p| (p, AgentScope::User))
}

fn normalize_string_list(raw: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for item in raw {
        let normalized = item.trim().to_string();
        if normalized.is_empty() {
            continue;
        }
        if seen.insert(normalized.clone()) {
            out.push(normalized);
        }
    }
    out
}

fn normalize_alias_list(raw: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for item in raw {
        let normalized = normalize_custom_agent_id(&item);
        if normalized.is_empty() {
            continue;
        }
        if seen.insert(normalized.clone()) {
            out.push(normalized);
        }
    }
    out
}

fn default_agent_prompt(id: &str, description: &str, tools: &[String]) -> String {
    let mut lines = vec![
        format!("You are custom subagent `{}`.", id),
        format!("Primary objective: {}.", description),
    ];
    if !tools.is_empty() {
        lines.push(format!(
            "Preferred tools: {}. Use only necessary tools.",
            tools.join(", ")
        ));
    }
    lines.push("Keep responses concise and execution-oriented.".to_string());
    lines.join(
        "
",
    )
}

async fn resolve_prompt_input(
    prompt: Option<String>,
    prompt_file: Option<String>,
) -> Result<Option<String>, String> {
    match (prompt, prompt_file) {
        (Some(_), Some(_)) => Err("use either `--prompt` or `--prompt-file`, not both".to_string()),
        (Some(p), None) => Ok(Some(p)),
        (None, Some(file)) => {
            let path = PathBuf::from(file);
            let text = tokio::fs::read_to_string(&path)
                .await
                .map_err(|e| format!("failed to read prompt file {}: {}", path.display(), e))?;
            Ok(Some(text))
        }
        (None, None) => Ok(None),
    }
}

pub(super) async fn create_agent(ctx: CommandContext<'_>, args: AgentCreateArgs) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let storage = crate::core::config::storage::Storage::new(cwd);
    let scope = if args.user {
        AgentScope::User
    } else {
        AgentScope::Project
    };
    let dir = agent_scope_dir(scope, &storage);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("failed to create agents dir {}: {}", dir.display(), e))?;

    let id = normalize_name_for_lookup(&args.name);
    if id.is_empty() {
        return Err("invalid agent name".to_string());
    }
    let path = dir.join(format!("{}.md", id));
    if path.exists() {
        return Err(format!(
            "agent `{}` already exists at `{}`. use `/agents edit {}`",
            id,
            path.display(),
            id
        ));
    }

    let tools = normalize_string_list(args.tools);
    let aliases = normalize_alias_list(args.aliases);
    let description = args
        .description
        .unwrap_or_else(|| format!("Custom subagent `{}`", id));
    let display_name = args
        .display_name
        .unwrap_or_else(|| id.replace('_', " ").replace('-', " "));
    let prompt = resolve_prompt_input(args.prompt, args.prompt_file)
        .await?
        .unwrap_or_else(|| default_agent_prompt(&id, &description, &tools));

    let model_owned = args.model.and_then(|s| {
        let trimmed = s.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });
    let markdown = render_custom_subagent_markdown(
        &id,
        &display_name,
        &description,
        &tools,
        &aliases,
        model_owned.as_deref(),
        &prompt,
    );
    tokio::fs::write(&path, markdown)
        .await
        .map_err(|e| format!("failed to write {}: {}", path.display(), e))?;

    let msg = format!(
        "✅ Agent 已创建

- id: `{}`
- scope: `{}`
- path: `{}`",
        id,
        agent_scope_label(scope),
        path.display()
    );
    ctx.state
        .chat_history
        .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
    Ok(())
}

pub(super) async fn edit_agent(ctx: CommandContext<'_>, args: AgentEditArgs) -> CommandResult {
    if args.clear_tools && args.tools.is_some() {
        return Err("cannot use `--tools` with `--clear-tools`".to_string());
    }
    if args.clear_aliases && args.aliases.is_some() {
        return Err("cannot use `--aliases` with `--clear-aliases`".to_string());
    }
    if args.clear_model && args.model.is_some() {
        return Err("cannot use `--model` with `--clear-model`".to_string());
    }

    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let storage = crate::core::config::storage::Storage::new(cwd);
    let (source_path, source_scope) = resolve_agent_path(&storage, &args.name, args.user)
        .ok_or_else(|| format!("agent `{}` not found", args.name))?;
    let existing = load_custom_subagent_from_file(&source_path)
        .ok_or_else(|| format!("failed to parse agent file `{}`", source_path.display()))?;

    let new_id = args
        .new_name
        .as_ref()
        .map(|s| normalize_name_for_lookup(s))
        .unwrap_or_else(|| existing.id.clone());
    if new_id.is_empty() {
        return Err("invalid new agent name".to_string());
    }

    let display_name = args.display_name.unwrap_or(existing.name.clone());
    let description = args.description.unwrap_or(existing.description.clone());
    let tools = if args.clear_tools {
        Vec::new()
    } else if let Some(t) = args.tools {
        normalize_string_list(t)
    } else {
        existing.tools.clone()
    };
    let aliases = if args.clear_aliases {
        Vec::new()
    } else if let Some(a) = args.aliases {
        normalize_alias_list(a)
    } else {
        existing.aliases.clone()
    };
    let model = if args.clear_model {
        None
    } else if let Some(model) = args.model {
        let trimmed = model.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    } else {
        existing.model.clone()
    };
    let prompt = resolve_prompt_input(args.prompt, args.prompt_file)
        .await?
        .unwrap_or(existing.prompt.clone());
    let prompt = if prompt.trim().is_empty() {
        default_agent_prompt(&new_id, &description, &tools)
    } else {
        prompt
    };

    let target_scope = if args.user {
        AgentScope::User
    } else {
        source_scope
    };
    let target_dir = agent_scope_dir(target_scope, &storage);
    tokio::fs::create_dir_all(&target_dir).await.map_err(|e| {
        format!(
            "failed to create agents dir {}: {}",
            target_dir.display(),
            e
        )
    })?;

    let target_path = target_dir.join(format!("{}.md", new_id));
    if target_path != source_path && target_path.exists() {
        return Err(format!(
            "target file already exists: `{}`. choose another `--new-name`.",
            target_path.display()
        ));
    }

    let markdown = render_custom_subagent_markdown(
        &new_id,
        &display_name,
        &description,
        &tools,
        &aliases,
        model.as_deref(),
        &prompt,
    );
    tokio::fs::write(&target_path, markdown)
        .await
        .map_err(|e| format!("failed to write {}: {}", target_path.display(), e))?;
    if target_path != source_path {
        tokio::fs::remove_file(&source_path)
            .await
            .map_err(|e| format!("failed to remove old file {}: {}", source_path.display(), e))?;
    }

    let msg = format!(
        "✅ Agent 已更新

- id: `{}`
- scope: `{}`
- path: `{}`",
        new_id,
        agent_scope_label(target_scope),
        target_path.display()
    );
    ctx.state
        .chat_history
        .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
    Ok(())
}

pub(super) async fn delete_agent(
    ctx: CommandContext<'_>,
    name: String,
    user: bool,
) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let storage = crate::core::config::storage::Storage::new(cwd);
    let Some((path, scope)) = resolve_agent_path(&storage, &name, user) else {
        let msg = if user {
            format!("⚠️ 未找到 user scope agent `{}`", name)
        } else {
            format!("⚠️ 未找到 agent `{}` (checked project + user scopes)", name)
        };
        ctx.state
            .chat_history
            .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
        return Ok(());
    };

    tokio::fs::remove_file(&path)
        .await
        .map_err(|e| format!("failed to remove {}: {}", path.display(), e))?;

    let msg = format!(
        "✅ 已删除 agent `{}` from {} scope
- path: `{}`",
        name,
        agent_scope_label(scope),
        path.display()
    );
    ctx.state
        .chat_history
        .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
    Ok(())
}

pub(super) async fn add_agent(
    ctx: CommandContext<'_>,
    source: String,
    name: Option<String>,
) -> CommandResult {
    let source_path = PathBuf::from(&source);
    if !source_path.exists() {
        return Err(format!("source file not found: {}", source));
    }

    let content = tokio::fs::read_to_string(&source_path)
        .await
        .map_err(|e| format!("failed to read source file: {}", e))?;

    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let storage = crate::core::config::storage::Storage::new(cwd);
    let project_dir = storage.project_agents_dir();
    if !project_dir.exists() {
        tokio::fs::create_dir_all(&project_dir)
            .await
            .map_err(|e| format!("failed to create agents dir: {}", e))?;
    }

    let dst_name = match name {
        Some(v) if !v.trim().is_empty() => sanitize_name(&v),
        _ => {
            let stem = source_path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or("invalid source filename")?;
            sanitize_name(stem)
        }
    };

    if dst_name.is_empty() {
        return Err("invalid agent name".to_string());
    }

    let dst_path = project_dir.join(format!("{}.md", dst_name));
    tokio::fs::write(&dst_path, content)
        .await
        .map_err(|e| format!("failed to write agent file: {}", e))?;

    let msg = format!(
        "✅ Agent 已添加

- name: `{}`
- path: `{}`",
        dst_name,
        dst_path.display()
    );
    ctx.state
        .chat_history
        .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));

    Ok(())
}

pub(super) async fn remove_agent(ctx: CommandContext<'_>, name: String) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let storage = crate::core::config::storage::Storage::new(cwd);
    let project_dir = storage.project_agents_dir();

    let mut removed = false;
    if let Some(path) = find_agent_path_in_dir(&project_dir, &name) {
        tokio::fs::remove_file(&path)
            .await
            .map_err(|e| format!("failed to remove {}: {}", path.display(), e))?;
        removed = true;
    }

    let msg = if removed {
        format!("✅ 已删除 agent `{}`", name)
    } else {
        format!("⚠️ 未找到 agent `{}` (仅检查项目级 .star/agents)", name)
    };

    ctx.state
        .chat_history
        .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));

    Ok(())
}

async fn list_agent_files(dir: &Path) -> Result<Vec<String>, String> {
    if !dir.exists() {
        return Ok(vec![]);
    }

    let mut rd = tokio::fs::read_dir(dir)
        .await
        .map_err(|e| format!("failed to read dir {}: {}", dir.display(), e))?;

    let mut out = Vec::new();
    while let Some(entry) = rd
        .next_entry()
        .await
        .map_err(|e| format!("failed to iterate dir {}: {}", dir.display(), e))?
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if file_name.ends_with(".md") || file_name.ends_with(".markdown") {
            out.push(file_name.to_string());
        }
    }

    out.sort();
    Ok(out)
}

fn sanitize_name(name: &str) -> String {
    normalize_custom_agent_id(name)
}
