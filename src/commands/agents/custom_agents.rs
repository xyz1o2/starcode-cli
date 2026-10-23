use super::*;

pub(super) async fn list_agents(ctx: CommandContext<'_>) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    list_agents_in(ctx, &cwd).await
}

/// `cwd` 走参数：单测在临时目录里增删 agent，不能写进开发者自己的
/// `.star/agents`。
pub(super) async fn list_agents_in(ctx: CommandContext<'_>, cwd: &Path) -> CommandResult {
    let storage = crate::core::config::storage::Storage::new(cwd.to_path_buf());

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
    create_agent_in(ctx, args, &cwd).await
}

pub(super) async fn create_agent_in(
    ctx: CommandContext<'_>,
    args: AgentCreateArgs,
    cwd: &Path,
) -> CommandResult {
    let storage = crate::core::config::storage::Storage::new(cwd.to_path_buf());
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
        .unwrap_or_else(|| id.replace(['_', '-'], " "));
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
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    edit_agent_in(ctx, args, &cwd).await
}

pub(super) async fn edit_agent_in(
    ctx: CommandContext<'_>,
    args: AgentEditArgs,
    cwd: &Path,
) -> CommandResult {
    if args.clear_tools && args.tools.is_some() {
        return Err("cannot use `--tools` with `--clear-tools`".to_string());
    }
    if args.clear_aliases && args.aliases.is_some() {
        return Err("cannot use `--aliases` with `--clear-aliases`".to_string());
    }
    if args.clear_model && args.model.is_some() {
        return Err("cannot use `--model` with `--clear-model`".to_string());
    }

    let storage = crate::core::config::storage::Storage::new(cwd.to_path_buf());
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
    let explicit_prompt = resolve_prompt_input(args.prompt, args.prompt_file).await?;
    let prompt = match explicit_prompt {
        Some(p) if !p.trim().is_empty() => p,
        // 提示词还是当初由 description/tools 生成的那份时，跟着一起重生成。
        // 否则 `--description` 改完了，agent 读到的提示词里仍是旧目标。
        _ => {
            let generated_from =
                default_agent_prompt(&existing.id, &existing.description, &existing.tools);
            if existing.prompt.trim() == generated_from.trim() || existing.prompt.trim().is_empty()
            {
                default_agent_prompt(&new_id, &description, &tools)
            } else {
                existing.prompt.clone()
            }
        }
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
    delete_agent_in(ctx, name, user, &cwd).await
}

pub(super) async fn delete_agent_in(
    ctx: CommandContext<'_>,
    name: String,
    user: bool,
    cwd: &Path,
) -> CommandResult {
    let storage = crate::core::config::storage::Storage::new(cwd.to_path_buf());
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
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    add_agent_in(ctx, source, name, &cwd).await
}

pub(super) async fn add_agent_in(
    ctx: CommandContext<'_>,
    source: String,
    name: Option<String>,
    cwd: &Path,
) -> CommandResult {
    let source_path = PathBuf::from(&source);
    if !source_path.exists() {
        return Err(format!("source file not found: {}", source));
    }

    let content = tokio::fs::read_to_string(&source_path)
        .await
        .map_err(|e| format!("failed to read source file: {}", e))?;

    let storage = crate::core::config::storage::Storage::new(cwd.to_path_buf());
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
    remove_agent_in(ctx, name, &cwd).await
}

pub(super) async fn remove_agent_in(
    ctx: CommandContext<'_>,
    name: String,
    cwd: &Path,
) -> CommandResult {
    let storage = crate::core::config::storage::Storage::new(cwd.to_path_buf());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::state::ChatState;

    fn harness() -> (
        ChatState,
        tokio::sync::mpsc::Sender<crate::runtime::messages::AgentRequest>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        std::mem::forget(rx);
        (ChatState::test_fixture(), tx)
    }

    fn last(state: &ChatState) -> String {
        state
            .chat_history
            .last()
            .map(|e| e.content.clone())
            .unwrap_or_default()
    }

    fn create_args(name: &str) -> AgentCreateArgs {
        AgentCreateArgs {
            name: name.to_string(),
            display_name: None,
            description: None,
            tools: Vec::new(),
            aliases: Vec::new(),
            model: None,
            prompt: None,
            prompt_file: None,
            user: false,
        }
    }

    fn edit_args(name: &str) -> AgentEditArgs {
        AgentEditArgs {
            name: name.to_string(),
            new_name: None,
            display_name: None,
            description: None,
            tools: None,
            aliases: None,
            model: None,
            prompt: None,
            prompt_file: None,
            clear_tools: false,
            clear_aliases: false,
            clear_model: false,
            user: false,
        }
    }

    fn agents_dir(dir: &Path) -> PathBuf {
        crate::core::config::storage::Storage::new(dir.to_path_buf()).project_agents_dir()
    }

    // ── 创建 ────────────────────────────────────────────────────

    #[tokio::test]
    async fn creating_an_agent_writes_a_markdown_file_and_reports_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = harness();
        let mut args = create_args("Reviewer");
        args.description = Some("PR review quality gate".to_string());
        args.tools = vec!["Read".to_string(), "Grep".to_string()];

        create_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            args,
            dir.path(),
        )
        .await
        .unwrap();

        // 名字归一成 snake_case 并成为文件名
        let path = agents_dir(dir.path()).join("reviewer.md");
        assert!(path.exists(), "{}", path.display());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("PR review quality gate"), "{content}");

        let out = last(&state);
        assert!(out.contains("✅ Agent 已创建"), "{out}");
        assert!(out.contains("- id: `reviewer`"), "{out}");
        assert!(out.contains("- scope: `project`"), "{out}");
        assert!(out.contains(&path.display().to_string()), "{out}");
    }

    /// 同名 agent 已存在时必须指路 `/agents edit`，而不是静默覆盖。
    #[tokio::test]
    async fn creating_a_duplicate_points_at_the_edit_command() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = harness();
        create_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            create_args("reviewer"),
            dir.path(),
        )
        .await
        .unwrap();

        let err = create_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            create_args("reviewer"),
            dir.path(),
        )
        .await
        .unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        assert!(err.contains("/agents edit reviewer"), "{err}");
    }

    #[tokio::test]
    async fn an_agent_name_with_nothing_usable_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = harness();
        let err = create_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            create_args("///"),
            dir.path(),
        )
        .await
        .unwrap_err();
        assert_eq!(err, "invalid agent name");
    }

    /// `--prompt` 和 `--prompt-file` 同时给会让人不知道以哪个为准。
    #[tokio::test]
    async fn specifying_both_prompt_sources_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = harness();
        let mut args = create_args("reviewer");
        args.prompt = Some("inline".to_string());
        args.prompt_file = Some("/tmp/whatever.md".to_string());

        let err = create_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            args,
            dir.path(),
        )
        .await
        .unwrap_err();
        assert!(
            err.contains("use either `--prompt` or `--prompt-file`"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_prompt_file_is_read_and_must_exist() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = harness();
        let prompt_path = dir.path().join("prompt.md");
        std::fs::write(&prompt_path, "You review Rust code.").unwrap();

        let mut args = create_args("rust-reviewer");
        args.prompt_file = Some(prompt_path.display().to_string());
        create_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            args,
            dir.path(),
        )
        .await
        .unwrap();
        let content =
            std::fs::read_to_string(agents_dir(dir.path()).join("rust-reviewer.md")).unwrap();
        assert!(content.contains("You review Rust code."), "{content}");

        let mut args = create_args("missing-prompt");
        args.prompt_file = Some(dir.path().join("nope.md").display().to_string());
        let err = create_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            args,
            dir.path(),
        )
        .await
        .unwrap_err();
        assert!(err.contains("failed to read prompt file"), "{err}");
    }

    /// 没有描述时也要生成一份能用的默认提示词，否则 agent 建出来是空的。
    #[tokio::test]
    async fn a_created_agent_gets_a_default_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = harness();
        create_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            create_args("helper"),
            dir.path(),
        )
        .await
        .unwrap();

        let content = std::fs::read_to_string(agents_dir(dir.path()).join("helper.md")).unwrap();
        assert!(content.contains("custom subagent `helper`"), "{content}");
        assert!(content.contains("Keep responses concise"), "{content}");
    }

    // ── 列表 ────────────────────────────────────────────────────

    #[tokio::test]
    async fn listing_shows_the_project_scope_and_the_examples() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = harness();
        create_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            create_args("reviewer"),
            dir.path(),
        )
        .await
        .unwrap();

        list_agents_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            dir.path(),
        )
        .await
        .unwrap();

        let out = last(&state);
        assert!(out.starts_with("# Agents"), "{out}");
        assert!(out.contains("## Project ("), "{out}");
        assert!(out.contains("- reviewer.md"), "{out}");
        assert!(out.contains("## Resolved Registry"), "{out}");
        assert!(out.contains("/agents create reviewer"), "{out}");
    }

    #[tokio::test]
    async fn an_empty_project_scope_says_empty() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = harness();
        list_agents_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            dir.path(),
        )
        .await
        .unwrap();

        let out = last(&state);
        assert!(out.contains("- (empty)"), "{out}");
    }

    // ── 编辑 ────────────────────────────────────────────────────

    #[tokio::test]
    async fn editing_updates_fields_and_keeps_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = harness();
        let mut create = create_args("reviewer");
        create.description = Some("old description".to_string());
        create.tools = vec!["Read".to_string()];
        create_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            create,
            dir.path(),
        )
        .await
        .unwrap();

        let mut edit = edit_args("reviewer");
        edit.description = Some("new description".to_string());
        edit.model = Some("claude-sonnet-5-5".to_string());
        edit_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            edit,
            dir.path(),
        )
        .await
        .unwrap();

        let content = std::fs::read_to_string(agents_dir(dir.path()).join("reviewer.md")).unwrap();
        assert!(content.contains("new description"), "{content}");
        assert!(!content.contains("old description"), "{content}");
        assert!(content.contains("claude-sonnet-5-5"), "{content}");
        assert!(content.contains("Read"), "tools must survive: {content}");
        assert!(last(&state).contains("✅ Agent 已更新"));
    }

    #[tokio::test]
    async fn editing_an_unknown_agent_reports_the_missing_name() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = harness();
        let err = edit_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            edit_args("ghost"),
            dir.path(),
        )
        .await
        .unwrap_err();
        assert!(err.contains("agent `ghost` not found"), "{err}");
    }

    /// 互相矛盾的开关必须直接报错，否则结果取决于代码里先判断哪一个。
    #[tokio::test]
    async fn contradictory_clear_flags_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = harness();
        create_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            create_args("reviewer"),
            dir.path(),
        )
        .await
        .unwrap();

        let mut edit = edit_args("reviewer");
        edit.clear_tools = true;
        edit.tools = Some(vec!["Read".to_string()]);
        let err = edit_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            edit,
            dir.path(),
        )
        .await
        .unwrap_err();
        assert!(
            err.contains("cannot use `--tools` with `--clear-tools`"),
            "{err}"
        );

        let mut edit = edit_args("reviewer");
        edit.clear_aliases = true;
        edit.aliases = Some(vec!["r".to_string()]);
        let err = edit_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            edit,
            dir.path(),
        )
        .await
        .unwrap_err();
        assert!(
            err.contains("cannot use `--aliases` with `--clear-aliases`"),
            "{err}"
        );

        let mut edit = edit_args("reviewer");
        edit.clear_model = true;
        edit.model = Some("m".to_string());
        let err = edit_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            edit,
            dir.path(),
        )
        .await
        .unwrap_err();
        assert!(
            err.contains("cannot use `--model` with `--clear-model`"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn clearing_tools_removes_them_from_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = harness();
        let mut create = create_args("reviewer");
        create.tools = vec!["Read".to_string(), "Grep".to_string()];
        create_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            create,
            dir.path(),
        )
        .await
        .unwrap();

        let mut edit = edit_args("reviewer");
        edit.clear_tools = true;
        edit_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            edit,
            dir.path(),
        )
        .await
        .unwrap();

        let content = std::fs::read_to_string(agents_dir(dir.path()).join("reviewer.md")).unwrap();
        assert!(!content.contains("Grep"), "{content}");
    }

    /// 改名后旧文件必须消失 —— 留着的话注册表里会有两个同义 agent。
    #[tokio::test]
    async fn renaming_moves_the_file_and_removes_the_old_one() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = harness();
        create_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            create_args("reviewer"),
            dir.path(),
        )
        .await
        .unwrap();

        let mut edit = edit_args("reviewer");
        edit.new_name = Some("pr-reviewer".to_string());
        edit_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            edit,
            dir.path(),
        )
        .await
        .unwrap();

        assert!(agents_dir(dir.path()).join("pr-reviewer.md").exists());
        assert!(!agents_dir(dir.path()).join("reviewer.md").exists());
    }

    #[tokio::test]
    async fn renaming_onto_an_existing_agent_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = harness();
        for name in ["alpha", "beta"] {
            create_agent_in(
                CommandContext {
                    state: &mut state,
                    agent_tx: &tx,
                },
                create_args(name),
                dir.path(),
            )
            .await
            .unwrap();
        }

        let mut edit = edit_args("alpha");
        edit.new_name = Some("beta".to_string());
        let err = edit_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            edit,
            dir.path(),
        )
        .await
        .unwrap_err();
        assert!(err.contains("target file already exists"), "{err}");
        assert!(agents_dir(dir.path()).join("alpha.md").exists());
    }

    // ── 删除 ────────────────────────────────────────────────────

    #[tokio::test]
    async fn deleting_removes_the_file_and_names_the_scope() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = harness();
        create_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            create_args("reviewer"),
            dir.path(),
        )
        .await
        .unwrap();

        delete_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            "reviewer".to_string(),
            false,
            dir.path(),
        )
        .await
        .unwrap();

        assert!(!agents_dir(dir.path()).join("reviewer.md").exists());
        let out = last(&state);
        assert!(out.contains("✅ 已删除 agent `reviewer`"), "{out}");
        assert!(out.contains("from project scope"), "{out}");
    }

    #[tokio::test]
    async fn deleting_a_missing_agent_says_which_scopes_were_checked() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = harness();
        delete_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            "ghost".to_string(),
            false,
            dir.path(),
        )
        .await
        .unwrap();
        assert!(last(&state).contains("checked project + user scopes"));

        delete_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            "ghost".to_string(),
            true,
            dir.path(),
        )
        .await
        .unwrap();
        assert!(last(&state).contains("未找到 user scope agent `ghost`"));
    }

    // ── 导入 / 移除 ─────────────────────────────────────────────

    #[tokio::test]
    async fn adding_copies_a_file_into_the_project_scope() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = harness();
        let src = dir.path().join("imported.md");
        std::fs::write(&src, "---\nname: imported\n---\nbody").unwrap();

        add_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            src.display().to_string(),
            None,
            dir.path(),
        )
        .await
        .unwrap();

        let dst = agents_dir(dir.path()).join("imported.md");
        assert!(dst.exists());
        assert_eq!(
            std::fs::read_to_string(&dst).unwrap(),
            "---\nname: imported\n---\nbody"
        );
        assert!(last(&state).contains("- name: `imported`"));
    }

    #[tokio::test]
    async fn adding_with_an_explicit_name_overrides_the_filename() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = harness();
        let src = dir.path().join("whatever.md");
        std::fs::write(&src, "body").unwrap();

        add_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            src.display().to_string(),
            Some("Custom Name".to_string()),
            dir.path(),
        )
        .await
        .unwrap();

        assert!(agents_dir(dir.path()).join("custom_name.md").exists());
    }

    #[tokio::test]
    async fn adding_a_missing_source_fails_before_creating_anything() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = harness();
        let err = add_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            dir.path().join("nope.md").display().to_string(),
            None,
            dir.path(),
        )
        .await
        .unwrap_err();
        assert!(err.starts_with("source file not found:"), "{err}");
        assert!(!agents_dir(dir.path()).exists());
    }

    #[tokio::test]
    async fn removing_only_touches_the_project_scope() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = harness();
        create_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            create_args("reviewer"),
            dir.path(),
        )
        .await
        .unwrap();

        remove_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            "reviewer".to_string(),
            dir.path(),
        )
        .await
        .unwrap();
        assert!(last(&state).contains("✅ 已删除 agent `reviewer`"));
        assert!(!agents_dir(dir.path()).join("reviewer.md").exists());

        remove_agent_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            "reviewer".to_string(),
            dir.path(),
        )
        .await
        .unwrap();
        assert!(last(&state).contains("仅检查项目级"));
    }

    // ── 名称工具 ────────────────────────────────────────────────

    #[test]
    fn lookup_names_accept_any_file_extension_and_case() {
        assert_eq!(normalize_name_for_lookup("Reviewer.md"), "reviewer");
        assert_eq!(normalize_name_for_lookup("Reviewer.markdown"), "reviewer");
        // 连字符保留：`review-er` 和 `review_er` 是两个不同的 id
        assert_eq!(normalize_name_for_lookup("  Review-Er  "), "review-er");
    }

    #[test]
    fn string_lists_drop_blanks_and_duplicates_but_keep_order() {
        let out = normalize_string_list(vec![
            " Read ".to_string(),
            "".to_string(),
            "Grep".to_string(),
            "Read".to_string(),
        ]);
        assert_eq!(out, ["Read", "Grep"]);
    }

    #[test]
    fn alias_lists_are_normalized_like_ids() {
        let out = normalize_alias_list(vec![
            "Review QA".to_string(),
            "review_qa".to_string(),
            " ".to_string(),
        ]);
        assert_eq!(out, ["review_qa"]);
    }

    #[test]
    fn agent_files_are_found_with_or_without_an_extension() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("reviewer.markdown"), "x").unwrap();
        assert!(find_agent_path_in_dir(dir.path(), "reviewer").is_some());
        assert!(find_agent_path_in_dir(dir.path(), "REVIEWER").is_some());
        assert!(find_agent_path_in_dir(dir.path(), "ghost").is_none());
        assert!(find_agent_path_in_dir(dir.path(), "   ").is_none());
    }

    #[test]
    fn listing_agent_files_ignores_other_files_and_sorts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.md"), "x").unwrap();
        std::fs::write(dir.path().join("a.markdown"), "x").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
        std::fs::create_dir_all(dir.path().join("subdir.md")).unwrap();

        let files = list_agent_paths_in_dir(dir.path());
        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, ["a.markdown", "b.md"]);
    }
}
