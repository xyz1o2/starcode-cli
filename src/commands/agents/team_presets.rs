use super::team_definitions::{
    find_team_agent, normalize_agent_name, parse_team_run_mode, resolve_team_agents,
    team_run_mode_label,
};
use super::*;

pub(super) async fn save_team_preset(
    ctx: CommandContext<'_>,
    args: AgentTeamSaveArgs,
) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    save_team_preset_in(ctx, args, &cwd).await
}

/// `cwd` 走参数：单测在临时目录里存取预设，不碰开发者自己的
/// `.star/agent-teams.json`。
pub(super) async fn save_team_preset_in(
    ctx: CommandContext<'_>,
    args: AgentTeamSaveArgs,
    cwd: &Path,
) -> CommandResult {
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

    let storage = crate::core::config::storage::Storage::new(cwd.to_path_buf());
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
    show_team_preset_in(ctx, name, &cwd).await
}

pub(super) async fn show_team_preset_in(
    ctx: CommandContext<'_>,
    name: String,
    cwd: &Path,
) -> CommandResult {
    let storage = crate::core::config::storage::Storage::new(cwd.to_path_buf());
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

    ctx.state.chat_history.push(
        ChatEntry::assistant(lines.join(
            "
",
        ))
        .with_streaming(false),
    );
    Ok(())
}

pub(super) async fn remove_team_preset(
    ctx: CommandContext<'_>,
    name: String,
    user: bool,
) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    remove_team_preset_in(ctx, name, user, &cwd).await
}

pub(super) async fn remove_team_preset_in(
    ctx: CommandContext<'_>,
    name: String,
    user: bool,
    cwd: &Path,
) -> CommandResult {
    let preset_name = sanitize_preset_name(&name);
    if preset_name.is_empty() {
        return Err("invalid preset name".to_string());
    }

    let storage = crate::core::config::storage::Storage::new(cwd.to_path_buf());
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_parts() -> (
        crate::ui::state::ChatState,
        tokio::sync::mpsc::Sender<crate::runtime::messages::AgentRequest>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        // rx 必须活着，否则命令里的 send 会立刻失败
        std::mem::forget(rx);
        (crate::ui::state::ChatState::test_fixture(), tx)
    }

    fn last(state: &crate::ui::state::ChatState) -> String {
        state
            .chat_history
            .last()
            .map(|e| e.content.clone())
            .unwrap_or_default()
    }

    fn save_args(name: &str) -> AgentTeamSaveArgs {
        AgentTeamSaveArgs {
            name: name.to_string(),
            agents: vec!["search".to_string(), "editor".to_string()],
            target: None,
            max_steps: None,
            parallelism: None,
            mode: None,
            rounds: None,
            timeout_secs: None,
            dry_run: false,
            description: None,
            objective: None,
            user: false,
        }
    }

    fn preset_path(dir: &Path) -> PathBuf {
        let storage = crate::core::config::storage::Storage::new(dir.to_path_buf());
        team_preset_file_path(TeamPresetScope::Project, &storage)
    }

    async fn stored_names(dir: &Path) -> Vec<String> {
        let store = load_team_preset_store(&preset_path(dir)).await.unwrap();
        store.teams.into_iter().map(|t| t.name).collect()
    }

    // ── save ────────────────────────────────────────────────────

    /// 保存要真的落盘：只往 chat_history 推一条「已保存」而没写文件，
    /// 是最难被发现的一类回归 —— 界面上一切正常，下次启动就没了。
    #[tokio::test]
    async fn save_writes_the_preset_to_the_project_store() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = ctx_parts();
        let mut args = save_args("rust-refactor");
        args.description = Some("  refactor helpers  ".to_string());
        args.target = Some("src".to_string());
        args.max_steps = Some(6);
        args.parallelism = Some(3);
        args.mode = Some(TeamRunMode::Pipeline);
        args.rounds = Some(2);
        args.timeout_secs = Some(120);
        args.dry_run = true;
        args.objective = Some("split the module".to_string());

        save_team_preset_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            args,
            dir.path(),
        )
        .await
        .unwrap();

        let store = load_team_preset_store(&preset_path(dir.path()))
            .await
            .unwrap();
        assert_eq!(store.teams.len(), 1);
        let saved = &store.teams[0];
        assert_eq!(saved.name, "rust-refactor");
        assert_eq!(saved.description.as_deref(), Some("  refactor helpers  "));
        assert_eq!(saved.target.as_deref(), Some("src"));
        assert_eq!(saved.max_steps, Some(6));
        assert_eq!(saved.parallelism, Some(3));
        assert_eq!(saved.mode.as_deref(), Some("pipeline"));
        assert_eq!(saved.rounds, Some(2));
        assert_eq!(saved.timeout_secs, Some(120));
        assert_eq!(saved.dry_run, Some(true));
        assert_eq!(saved.objective.as_deref(), Some("split the module"));
        // `--agents search,editor` 存的是规范化后的 cli_name —— `search` 是
        // `explore` 的别名，落盘时写回规范名。
        assert_eq!(
            saved.agents,
            vec!["explore".to_string(), "editor".to_string()]
        );

        let out = last(&state);
        assert!(out.contains("✅ Team preset 已保存"), "{out}");
        assert!(out.contains("`rust-refactor`"), "{out}");
        assert!(out.contains("scope: `project`"), "{out}");
    }

    #[tokio::test]
    async fn saving_the_same_name_overwrites_instead_of_duplicating() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = ctx_parts();
        let mut first = save_args("review");
        first.description = Some("first".to_string());
        save_team_preset_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            first,
            dir.path(),
        )
        .await
        .unwrap();

        let mut second = save_args("review");
        second.description = Some("second".to_string());
        save_team_preset_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            second,
            dir.path(),
        )
        .await
        .unwrap();

        let store = load_team_preset_store(&preset_path(dir.path()))
            .await
            .unwrap();
        assert_eq!(store.teams.len(), 1, "同名预设必须覆盖而不是追加");
        assert_eq!(store.teams[0].description.as_deref(), Some("second"));
    }

    #[tokio::test]
    async fn the_store_stays_sorted_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = ctx_parts();
        for name in ["zeta", "alpha", "mike"] {
            save_team_preset_in(
                CommandContext {
                    state: &mut state,
                    agent_tx: &tx,
                },
                save_args(name),
                dir.path(),
            )
            .await
            .unwrap();
        }

        assert_eq!(
            stored_names(dir.path()).await,
            vec!["alpha".to_string(), "mike".to_string(), "zeta".to_string()]
        );
    }

    /// 空描述/空目标按「没填」处理，避免 `show` 里渲染出 `description: `。
    #[tokio::test]
    async fn blank_optional_text_is_stored_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = ctx_parts();
        let mut args = save_args("blank");
        args.description = Some("   ".to_string());
        args.target = Some("".to_string());
        args.objective = Some("\n".to_string());

        save_team_preset_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            args,
            dir.path(),
        )
        .await
        .unwrap();

        let store = load_team_preset_store(&preset_path(dir.path()))
            .await
            .unwrap();
        let saved = &store.teams[0];
        assert!(saved.description.is_none());
        assert!(saved.target.is_none());
        assert!(saved.objective.is_none());
    }

    #[tokio::test]
    async fn an_unsanitizable_preset_name_is_rejected_before_any_write() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = ctx_parts();
        let err = save_team_preset_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            save_args("!!!"),
            dir.path(),
        )
        .await
        .unwrap_err();

        assert_eq!(err, "invalid preset name");
        assert!(!preset_path(dir.path()).exists(), "校验失败不该写文件");
    }

    #[tokio::test]
    async fn unknown_agents_are_rejected_before_any_write() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = ctx_parts();
        let mut args = save_args("broken");
        args.agents = vec!["search".to_string(), "nonexistent".to_string()];
        let err = save_team_preset_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            args,
            dir.path(),
        )
        .await
        .unwrap_err();

        assert!(err.contains("unknown team agents: nonexistent"), "{err}");
        assert!(!preset_path(dir.path()).exists());
    }

    /// 三个上界都是 CLI 的护栏，越界必须报错而不是静默截断。
    #[tokio::test]
    async fn numeric_bounds_are_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = ctx_parts();

        type Mutation = fn(&mut AgentTeamSaveArgs);
        let cases: [(Mutation, &str); 6] = [
            (
                |a| a.max_steps = Some(0),
                "max_steps must be between 1 and 30",
            ),
            (
                |a| a.max_steps = Some(31),
                "max_steps must be between 1 and 30",
            ),
            (
                |a| a.parallelism = Some(0),
                "parallelism must be between 1 and 16",
            ),
            (
                |a| a.parallelism = Some(17),
                "parallelism must be between 1 and 16",
            ),
            (|a| a.rounds = Some(0), "rounds must be between 1 and 8"),
            (|a| a.rounds = Some(9), "rounds must be between 1 and 8"),
        ];

        for (mutate, expected) in cases {
            let mut args = save_args("bounds");
            mutate(&mut args);
            let err = save_team_preset_in(
                CommandContext {
                    state: &mut state,
                    agent_tx: &tx,
                },
                args,
                dir.path(),
            )
            .await
            .unwrap_err();
            assert_eq!(err, expected);
        }
        assert!(!preset_path(dir.path()).exists());
    }

    // ── show ────────────────────────────────────────────────────

    #[tokio::test]
    async fn show_renders_every_field_and_the_follow_up_command() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = ctx_parts();
        let mut args = save_args("rust-refactor");
        args.description = Some("refactor helpers".to_string());
        args.target = Some("src".to_string());
        args.max_steps = Some(5);
        args.mode = Some(TeamRunMode::Pipeline);
        args.rounds = Some(2);
        save_team_preset_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            args,
            dir.path(),
        )
        .await
        .unwrap();

        show_team_preset_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            "rust-refactor".to_string(),
            dir.path(),
        )
        .await
        .unwrap();

        let out = last(&state);
        assert!(out.starts_with("# Team Preset"), "{out}");
        assert!(out.contains("- name: `rust-refactor`"), "{out}");
        assert!(out.contains("- scope: `project`"), "{out}");
        assert!(out.contains("- agents: explore,editor"), "{out}");
        assert!(out.contains("- target: src"), "{out}");
        assert!(out.contains("- max_steps: 5"), "{out}");
        // 未设置的字段渲染占位符而不是空串
        assert!(out.contains("- parallelism: (none)"), "{out}");
        assert!(out.contains("- mode: pipeline"), "{out}");
        assert!(out.contains("- rounds: 2"), "{out}");
        assert!(out.contains("- description: refactor helpers"), "{out}");
        assert!(
            out.contains("- `/agents team run --team rust-refactor <objective...>`"),
            "{out}"
        );
    }

    /// 没设置 mode/rounds 时展示的是运行时默认值（parallel / 1），
    /// 否则用户会以为团队会跑 0 轮。
    #[tokio::test]
    async fn show_falls_back_to_the_runtime_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = ctx_parts();
        save_team_preset_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            save_args("bare"),
            dir.path(),
        )
        .await
        .unwrap();

        show_team_preset_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            "bare".to_string(),
            dir.path(),
        )
        .await
        .unwrap();

        let out = last(&state);
        assert!(out.contains("- mode: parallel"), "{out}");
        assert!(out.contains("- rounds: 1"), "{out}");
        assert!(out.contains("- dry_run: false"), "{out}");
    }

    /// 名字匹配不区分大小写，和 `resolve_team_preset` 保持一致。
    #[tokio::test]
    async fn show_matches_the_name_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = ctx_parts();
        save_team_preset_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            save_args("Rust-Refactor"),
            dir.path(),
        )
        .await
        .unwrap();

        show_team_preset_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            "rust-refactor".to_string(),
            dir.path(),
        )
        .await
        .unwrap();

        assert!(last(&state).contains("- name: `Rust-Refactor`"));
    }

    #[tokio::test]
    async fn show_reports_both_scopes_when_the_preset_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = ctx_parts();
        let err = show_team_preset_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            "does-not-exist".to_string(),
            dir.path(),
        )
        .await
        .unwrap_err();

        assert_eq!(
            err,
            "team preset 'does-not-exist' not found (checked project + user scopes)"
        );
    }

    // ── remove ──────────────────────────────────────────────────

    #[tokio::test]
    async fn remove_deletes_only_the_named_preset() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = ctx_parts();
        for name in ["keep", "drop"] {
            save_team_preset_in(
                CommandContext {
                    state: &mut state,
                    agent_tx: &tx,
                },
                save_args(name),
                dir.path(),
            )
            .await
            .unwrap();
        }

        remove_team_preset_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            "drop".to_string(),
            false,
            dir.path(),
        )
        .await
        .unwrap();

        assert_eq!(stored_names(dir.path()).await, vec!["keep".to_string()]);
        assert!(last(&state).contains("✅ 已删除 team preset `drop` from project scope"));
    }

    /// 删不存在的预设不是错误 —— 用户想的是「确保它没了」。
    #[tokio::test]
    async fn removing_a_missing_preset_warns_but_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = ctx_parts();
        remove_team_preset_in(
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

        assert!(last(&state).contains("⚠️ 未找到 team preset `ghost` in project scope"));
    }

    #[tokio::test]
    async fn removing_with_an_unsanitizable_name_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = ctx_parts();
        let err = remove_team_preset_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            "!!!".to_string(),
            false,
            dir.path(),
        )
        .await
        .unwrap_err();

        assert_eq!(err, "invalid preset name");
    }

    /// 预设名会进文件名/命令行示例，必须先把路径分隔符之类剔掉。
    #[tokio::test]
    async fn preset_names_are_sanitized_on_the_way_in() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = ctx_parts();
        save_team_preset_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            save_args("../../etc/passwd"),
            dir.path(),
        )
        .await
        .unwrap();

        assert_eq!(
            stored_names(dir.path()).await,
            vec!["etcpasswd".to_string()]
        );
    }
}
