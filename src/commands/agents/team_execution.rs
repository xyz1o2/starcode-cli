use super::team_definitions::{generate_team_run_id, resolve_team_agents, team_run_mode_label};
use super::team_definitions::{
    normalize_agent_name, parse_team_run_mode, TeamAgentDef, TeamMemberWorkspace,
};
use super::team_presets::TeamRuntimeInfo;
use super::*;

fn map_ui_approval_mode(
    mode: &crate::types::ApprovalMode,
) -> crate::core::policy::types::ApprovalMode {
    match mode {
        crate::types::ApprovalMode::Default => crate::core::policy::types::ApprovalMode::Default,
        crate::types::ApprovalMode::Plan => crate::core::policy::types::ApprovalMode::Plan,
        crate::types::ApprovalMode::Yolo => crate::core::policy::types::ApprovalMode::Yolo,
    }
}

async fn resolve_team_runtime(
    current_model: Option<String>,
    approval_mode: crate::types::ApprovalMode,
    target_dir: PathBuf,
) -> Result<(StarClient, Arc<Config>, TeamRuntimeInfo), String> {
    let settings_manager = crate::core::config::settings_manager::get_settings_manager()
        .await
        .map_err(|e| e.to_string())?;
    let settings = settings_manager
        .load_user_settings()
        .await
        .map_err(|e| e.to_string())?;

    let provider_store = ProviderStore::new();
    let provider_config = provider_store.load().await.unwrap_or_default();
    let provider_resolution = resolve_effective_provider_settings(
        ProviderResolutionInputs {
            session_model: current_model,
            ..Default::default()
        },
        &provider_config,
        &settings,
    );

    let model = provider_resolution
        .model
        .value
        .clone()
        .unwrap_or_else(|| "star-code-fast-1".to_string());
    let base_url = provider_resolution.base_url.value.clone().ok_or_else(|| {
        "missing base_url: configure STAR_BASE_URL or run `/provider select ...` first".to_string()
    })?;
    let api_key = provider_resolution.api_key.value.clone().ok_or_else(|| {
        "missing api_key: configure STAR_API_KEY or run `/provider select ... <API_KEY>` first"
            .to_string()
    })?;
    let is_openai_compatible = provider_resolution.openai_compatible;

    let mut params = ConfigParameters::default();
    params.session_id = uuid::Uuid::new_v4().to_string();
    params.target_dir = target_dir.clone();
    params.cwd = target_dir;
    params.model = model.clone();
    params.approval_mode = Some(map_ui_approval_mode(&approval_mode));

    let mut config = Config::new(params);
    config.initialize().await.map_err(|e| e.to_string())?;
    let config = Arc::new(config);

    let client = StarClient::new(
        &api_key,
        Some(model.clone()),
        Some(base_url.clone()),
        Some(is_openai_compatible),
        None,
    );

    Ok((
        client,
        config,
        TeamRuntimeInfo {
            model,
            base_url,
            active_provider: provider_resolution.provider_id,
        },
    ))
}

fn build_team_manager(client: StarClient, config: Arc<Config>) -> SubAgentManager {
    let mut manager = SubAgentManager::new();
    manager.register(Box::new(AnalyzerAgent::new(client.clone(), config.clone())));
    manager.register(Box::new(EditorAgent::new(client.clone(), config.clone())));
    manager.register(Box::new(SearchAgent::new(client.clone(), config.clone())));
    manager.register(Box::new(NavigatorAgent::new(
        client.clone(),
        config.clone(),
    )));
    let _ = register_custom_subagents(&mut manager, client.clone(), config.clone());
    manager.register(Box::new(AutoFixAgent::new(client, config)));
    manager
}

pub(super) fn truncate_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let mut truncated: String = input.chars().take(max_chars).collect();
    truncated.push_str(
        "
...[truncated]",
    );
    truncated
}

#[derive(Clone)]
pub(super) struct TeamTaskOutcome {
    pub(super) member_name: String,
    pub(super) member_internal_id: String,
    pub(super) round: usize,
    pub(super) duration_ms: u128,
    pub(super) result: SubTaskResult,
    pub(super) work_dir: PathBuf,
    pub(super) target: String,
    pub(super) isolation_mode: String,
    pub(super) patch_path: PathBuf,
    pub(super) has_changes: bool,
    pub(super) changed_files: usize,
}

/// 结构化团队上下文——在轮次内存被截断时保留关键信息
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct StructuredTeamContext {
    /// 累计修改的文件：{文件路径: [修改者]}
    pub files_changed: std::collections::HashMap<String, Vec<String>>,
    /// 每轮关键摘要（保留最近8轮）
    pub round_summaries: Vec<RoundDigest>,
    /// 未解决的持久错误
    pub unresolved_errors: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct RoundDigest {
    pub round: usize,
    pub total_outcomes: usize,
    pub success_count: usize,
    pub changed_files: usize,
    pub summary: String,
}

impl StructuredTeamContext {
    pub fn new() -> Self {
        Self {
            files_changed: std::collections::HashMap::new(),
            round_summaries: Vec::new(),
            unresolved_errors: Vec::new(),
        }
    }

    /// 从轮次结果更新结构化上下文
    pub fn update_from_round(&mut self, outcomes: &[TeamTaskOutcome], round: usize) {
        let mut success_count = 0;
        let mut changed = 0;
        let mut summary_parts: Vec<String> = Vec::new();

        for o in outcomes {
            if o.result.success {
                success_count += 1;
            }
            if o.has_changes {
                changed += o.changed_files;
                for word in o.result.summary.split_whitespace() {
                    if word.contains('.') && (word.contains('/') || word.contains('\\')) {
                        let clean = word.trim_matches(|c: char| {
                            !c.is_alphanumeric() && c != '.' && c != '/' && c != '_' && c != '-'
                        });
                        if clean.len() > 3 && clean.len() < 200 {
                            self.files_changed
                                .entry(clean.to_string())
                                .or_default()
                                .push(o.member_name.clone());
                        }
                    }
                }
            }
            if !o.result.success {
                if let Some(ref err) = o.result.error {
                    let short: String = err.chars().take(120).collect();
                    self.unresolved_errors.push(if err.len() > 120 {
                        format!("{}...", short)
                    } else {
                        short
                    });
                }
            }
            summary_parts.push(format!(
                "{}:{}",
                o.member_name,
                if o.result.success { "ok" } else { "fail" }
            ));
        }

        self.round_summaries.push(RoundDigest {
            round,
            total_outcomes: outcomes.len(),
            success_count,
            changed_files: changed,
            summary: summary_parts.join(", "),
        });
        if self.round_summaries.len() > 8 {
            self.round_summaries.remove(0);
        }

        self.unresolved_errors.sort();
        self.unresolved_errors.dedup();
        if self.unresolved_errors.len() > 10 {
            self.unresolved_errors = self
                .unresolved_errors
                .split_off(self.unresolved_errors.len() - 10);
        }
    }

    /// 渲染为注入到轮次目标的 JSON 文本
    pub fn render(&self) -> Option<String> {
        let json = serde_json::to_string(self).ok()?;
        if json.len() < 20 {
            return None;
        }
        let template = crate::core::prompts::loader::load_prompt("team-context-injection.md");
        Some(crate::core::prompts::loader::render_template(
            &template,
            &[("json", &json)],
        ))
    }
}

pub(super) fn build_round_objective(
    base_objective: &str,
    round: usize,
    total_rounds: usize,
    shared_context: &[String],
    structured_context: Option<&StructuredTeamContext>,
) -> String {
    if total_rounds == 1 && round == 1 && shared_context.is_empty() {
        return base_objective.to_string();
    }

    let mut lines = vec![
        base_objective.to_string(),
        "".to_string(),
        format!("[Team Collaboration] round {}/{}", round, total_rounds),
    ];

    // 注入结构化上下文（持久关键信息，不随文本内存截断丢失）
    if let Some(ctx) = structured_context {
        if let Some(rendered) = ctx.render() {
            lines.push(rendered);
        }
    }

    if !shared_context.is_empty() {
        lines.push(format!(
            "Team shared memory (latest {} items):",
            shared_context.len()
        ));
        for item in shared_context {
            lines.push(format!("- {}", item));
        }
    }
    lines.join("\n")
}

pub(super) fn summarize_round_context(
    outcomes: &[TeamTaskOutcome],
    max_items: usize,
) -> Vec<String> {
    outcomes
        .iter()
        .take(max_items)
        .map(|o| {
            let status = if o.result.success { "ok" } else { "fail" };
            format!("{} [{}]: {}", o.member_name, status, o.result.summary)
        })
        .collect()
}

pub(super) fn normalize_memory_entry(input: &str, max_chars: usize) -> Option<String> {
    let compact = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        return None;
    }
    if compact.chars().count() <= max_chars {
        return Some(compact);
    }
    let mut out: String = compact.chars().take(max_chars).collect();
    out.push_str("...");
    Some(out)
}

pub(super) fn summarize_round_memory(
    outcomes: &[TeamTaskOutcome],
    round: usize,
    max_items: usize,
) -> Vec<String> {
    let mut memory = Vec::new();
    if max_items == 0 {
        return memory;
    }

    for outcome in outcomes {
        let status = if outcome.result.success { "ok" } else { "fail" };
        let prefix = format!("[r{}:{}:{}]", round, outcome.member_name, status);

        if let Some(summary) = normalize_memory_entry(&outcome.result.summary, 220) {
            let mut line = format!("{} {}", prefix, summary);
            if outcome.has_changes {
                line.push_str(&format!(" (changed_files={})", outcome.changed_files));
            }
            memory.push(line);
        }
        if memory.len() >= max_items {
            break;
        }

        if let Some(next) = outcome
            .result
            .next_action
            .as_ref()
            .and_then(|s| normalize_memory_entry(s, 180))
        {
            memory.push(format!("{} next: {}", prefix, next));
        }
        if memory.len() >= max_items {
            break;
        }

        if !outcome.result.success {
            if let Some(err) = outcome
                .result
                .error
                .as_ref()
                .and_then(|s| normalize_memory_entry(s, 180))
            {
                memory.push(format!("{} error: {}", prefix, err));
            }
        }
        if memory.len() >= max_items {
            break;
        }
    }

    memory
}

pub(super) fn append_shared_memory(
    shared_memory: &mut Vec<String>,
    additions: Vec<String>,
    max_items: usize,
) {
    if max_items == 0 {
        shared_memory.clear();
        return;
    }
    for item in additions {
        if !item.trim().is_empty() {
            shared_memory.push(item);
        }
    }
    if shared_memory.len() > max_items {
        let overflow = shared_memory.len() - max_items;
        shared_memory.drain(0..overflow);
    }
}

pub(super) fn sort_outcomes_by_member_order(
    outcomes: &mut Vec<TeamTaskOutcome>,
    selected: &[&TeamAgentDef],
) {
    let order_map: HashMap<&str, usize> = selected
        .iter()
        .enumerate()
        .map(|(i, def)| (def.cli_name, i))
        .collect();
    outcomes.sort_by_key(|item| {
        order_map
            .get(item.member_name.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });
}

async fn execute_team_workspace_task(
    workspace: TeamMemberWorkspace,
    idx: usize,
    round: usize,
    objective: String,
    max_steps: usize,
    timeout: Option<Duration>,
    dry_run: bool,
    current_model: Option<String>,
    approval_mode: crate::types::ApprovalMode,
) -> TeamTaskOutcome {
    let exec_started = Instant::now();
    let task_id = format!(
        "team-r{:02}-{:02}-{}",
        round,
        idx + 1,
        workspace.member_name
    );
    let mut task = SubTask::new(
        task_id.clone(),
        objective,
        workspace.task_type.clone(),
        workspace.target.clone(),
    )
    .with_max_steps(max_steps)
    .with_param(
        "team_member".to_string(),
        json!(workspace.member_name.clone()),
    )
    .with_param("team_index".to_string(), json!(idx + 1))
    .with_param("team_round".to_string(), json!(round));

    if dry_run && workspace.member_name == "editor" {
        task = task.with_param("dry_run".to_string(), json!(true));
    }
    let task_user_message = task.objective.clone();
    let task_params_json = serde_json::to_string(&task.params).ok();

    let mut result = match resolve_team_runtime(
        current_model.clone(),
        approval_mode.clone(),
        workspace.work_dir.clone(),
    )
    .await
    {
        Ok((client, config, _)) => {
            let manager = build_team_manager(client, config);
            if let Some(agent) = manager.get_agent(&workspace.member_internal_id) {
                if let Some(limit) = timeout {
                    match tokio::time::timeout(limit, agent.execute(task)).await {
                        Ok(exec_result) => match exec_result {
                            Ok(r) => r,
                            Err(e) => SubTaskResult::failure(task_id.clone(), e.to_string()),
                        },
                        Err(_) => SubTaskResult::failure(
                            task_id.clone(),
                            format!("timeout after {}s", limit.as_secs()),
                        ),
                    }
                } else {
                    match agent.execute(task).await {
                        Ok(r) => r,
                        Err(e) => SubTaskResult::failure(task_id.clone(), e.to_string()),
                    }
                }
            } else {
                SubTaskResult::failure(
                    task_id.clone(),
                    format!(
                        "team agent '{}' is not available",
                        workspace.member_internal_id
                    ),
                )
            }
        }
        Err(e) => SubTaskResult::failure(
            task_id.clone(),
            format!("failed to build runtime for member: {}", e),
        ),
    };

    let (mut has_changes, mut changed_files) = (false, 0usize);
    if workspace.isolation_mode == "git-worktree" {
        match git_service::collect_member_patch(&workspace.work_dir, &workspace.patch_path).await {
            Ok((hc, cf)) => {
                has_changes = hc;
                changed_files = cf;
            }
            Err(e) => {
                result.success = false;
                result.error = Some(match &result.error {
                    Some(existing) if !existing.trim().is_empty() => {
                        format!("{}; patch collection failed: {}", existing, e)
                    }
                    _ => format!("patch collection failed: {}", e),
                });
            }
        }
    }

    if !result.success {
        if let Ok(cwd) = std::env::current_dir() {
            let _ = crate::core::hooks::runner::run_hooks(
                &cwd,
                crate::core::hooks::store::ManagedHookEvent::SubagentStop,
                &crate::core::hooks::runner::HookRunContext {
                    user_message: task_user_message.clone(),
                    status: "subagent_stop".to_string(),
                    tool_name: Some(workspace.member_name.clone()),
                    tool_arguments: task_params_json.clone(),
                    tool_success: Some(false),
                    stop_reason: result.error.clone(),
                    stop_hook_active: false,
                },
            )
            .await;
        }
    }

    TeamTaskOutcome {
        member_name: workspace.member_name,
        member_internal_id: workspace.member_internal_id,
        round,
        duration_ms: exec_started.elapsed().as_millis(),
        result,
        work_dir: workspace.work_dir,
        target: workspace.target,
        isolation_mode: workspace.isolation_mode,
        patch_path: workspace.patch_path,
        has_changes,
        changed_files,
    }
}

async fn execute_round_parallel(
    workspaces: &[TeamMemberWorkspace],
    round: usize,
    objective: String,
    max_steps: usize,
    parallelism: usize,
    timeout: Option<Duration>,
    dry_run: bool,
    current_model: Option<String>,
    approval_mode: crate::types::ApprovalMode,
) -> Result<Vec<TeamTaskOutcome>, String> {
    let semaphore = Arc::new(tokio::sync::Semaphore::new(parallelism));
    let mut joinset: tokio::task::JoinSet<TeamTaskOutcome> = tokio::task::JoinSet::new();

    for (idx, workspace) in workspaces.iter().cloned().enumerate() {
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| format!("failed to acquire team semaphore: {}", e))?;
        let objective = objective.clone();
        let current_model = current_model.clone();
        let approval_mode = approval_mode.clone();

        joinset.spawn(async move {
            let _permit = permit;
            execute_team_workspace_task(
                workspace,
                idx,
                round,
                objective,
                max_steps,
                timeout,
                dry_run,
                current_model,
                approval_mode,
            )
            .await
        });
    }

    let mut outcomes: Vec<TeamTaskOutcome> = Vec::new();
    while let Some(joined) = joinset.join_next().await {
        match joined {
            Ok(item) => outcomes.push(item),
            Err(e) => outcomes.push(TeamTaskOutcome {
                member_name: "unknown".to_string(),
                member_internal_id: "unknown".to_string(),
                round,
                duration_ms: 0,
                result: SubTaskResult::failure(
                    format!("panic-r{}-{}", round, outcomes.len() + 1),
                    format!("join error: {}", e),
                ),
                work_dir: PathBuf::new(),
                target: String::new(),
                isolation_mode: "unknown".to_string(),
                patch_path: PathBuf::new(),
                has_changes: false,
                changed_files: 0,
            }),
        }
    }

    Ok(outcomes)
}

async fn execute_round_pipeline(
    workspaces: &[TeamMemberWorkspace],
    round: usize,
    objective: String,
    max_steps: usize,
    timeout: Option<Duration>,
    dry_run: bool,
    current_model: Option<String>,
    approval_mode: crate::types::ApprovalMode,
) -> Vec<TeamTaskOutcome> {
    let mut outcomes = Vec::new();
    let mut step_context: Vec<String> = Vec::new();

    for (idx, workspace) in workspaces.iter().cloned().enumerate() {
        let mut step_objective = objective.clone();
        if !step_context.is_empty() {
            step_objective.push_str(
                "

[Pipeline handoff context]
",
            );
            for item in &step_context {
                step_objective.push_str("- ");
                step_objective.push_str(item);
                step_objective.push('\n');
            }
        }

        let outcome = execute_team_workspace_task(
            workspace,
            idx,
            round,
            step_objective,
            max_steps,
            timeout,
            dry_run,
            current_model.clone(),
            approval_mode.clone(),
        )
        .await;

        let status = if outcome.result.success { "ok" } else { "fail" };
        let mut handoff = format!(
            "{} [{}]: {}",
            outcome.member_name, status, outcome.result.summary
        );
        if outcome.has_changes {
            handoff.push_str(&format!(" (changed_files={})", outcome.changed_files));
        }
        if let Some(next) = outcome
            .result
            .next_action
            .as_ref()
            .and_then(|s| normalize_memory_entry(s, 120))
        {
            handoff.push_str(&format!(" | next: {}", next));
        }
        step_context.push(handoff);
        outcomes.push(outcome);
    }

    outcomes
}

pub(super) async fn run_agent_team(
    ctx: CommandContext<'_>,
    args: AgentTeamRunArgs,
) -> CommandResult {
    let command_cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let storage = crate::core::config::storage::Storage::new(command_cwd.clone());

    let preset = if let Some(team_name) = args.team.as_ref() {
        resolve_team_preset(&storage, team_name)
            .await?
            .ok_or_else(|| format!("team preset '{}' not found", team_name))?
            .0
    } else {
        TeamPreset {
            name: "inline".to_string(),
            description: None,
            agents: vec![
                "Grep".to_string(),
                "analyzer".to_string(),
                "editor".to_string(),
            ],
            target: Some(".".to_string()),
            max_steps: Some(5),
            parallelism: Some(3),
            mode: Some("parallel".to_string()),
            rounds: Some(1),
            timeout_secs: None,
            dry_run: Some(false),
            objective: None,
            updated_at: 0,
        }
    };

    let objective = {
        let inline = args.objective.join(" ").trim().to_string();
        if !inline.is_empty() {
            inline
        } else if let Some(obj) = preset.objective.clone().filter(|s| !s.trim().is_empty()) {
            obj
        } else {
            return Err("objective is required. pass it at run time or set preset objective via `/agents team save --objective ...`".to_string());
        }
    };

    let agents_input = args.agents.clone().unwrap_or_else(|| preset.agents.clone());
    let selected = resolve_team_agents(agents_input)?;

    let max_steps = args.max_steps.or(preset.max_steps).unwrap_or(5);
    if max_steps == 0 || max_steps > 30 {
        return Err("max_steps must be between 1 and 30".to_string());
    }

    let run_mode = args
        .mode
        .clone()
        .or_else(|| preset.mode.as_deref().and_then(parse_team_run_mode))
        .unwrap_or(TeamRunMode::Parallel);
    let rounds = args.rounds.or(preset.rounds).unwrap_or(1);
    if rounds == 0 || rounds > 8 {
        return Err("rounds must be between 1 and 8".to_string());
    }

    let raw_parallelism = args.parallelism.or(preset.parallelism).unwrap_or(3);
    if raw_parallelism == 0 {
        return Err("parallelism must be >= 1".to_string());
    }
    let parallelism = raw_parallelism.min(selected.len()).max(1);
    let effective_parallelism = if matches!(run_mode, TeamRunMode::Pipeline) {
        1
    } else {
        parallelism
    };

    let timeout_secs = args.timeout_secs.or(preset.timeout_secs);
    let timeout = timeout_secs.map(Duration::from_secs);
    let target = args
        .target
        .clone()
        .or(preset.target.clone())
        .unwrap_or_else(|| ".".to_string());
    let dry_run = args.dry_run || preset.dry_run.unwrap_or(false);

    let current_model = Some(ctx.state.current_model.clone()).filter(|m| !m.trim().is_empty());
    let approval_mode = ctx.state.approval_mode.clone();

    let (_, _, runtime_info) = resolve_team_runtime(
        current_model.clone(),
        approval_mode.clone(),
        command_cwd.clone(),
    )
    .await?;

    let run_id = generate_team_run_id();
    let run_dir = team_run_dir(&storage, &run_id);
    tokio::fs::create_dir_all(&run_dir)
        .await
        .map_err(|e| format!("failed to create run dir {}: {}", run_dir.display(), e))?;

    let repo_root_opt = git_service::detect_repo_root(&command_cwd).await;
    let git_mode = repo_root_opt.is_some();
    let base_head_opt = if let Some(repo_root) = repo_root_opt.as_ref() {
        Some(git_service::rev_parse(repo_root, "HEAD").await?)
    } else {
        None
    };

    let mut workspaces: Vec<TeamMemberWorkspace> = Vec::new();
    for def in &selected {
        let patch_path = run_dir.join(format!("{}.patch", def.cli_name));
        if let (Some(repo_root), Some(base_head)) = (repo_root_opt.as_ref(), base_head_opt.as_ref())
        {
            let work_dir = storage
                .project_temp_dir()
                .join("agent-teams")
                .join(&run_id)
                .join(def.cli_name);
            git_service::worktree_remove_force(repo_root, &work_dir).await;
            if work_dir.exists() {
                if work_dir.is_dir() {
                    let _ = tokio::fs::remove_dir_all(&work_dir).await;
                } else {
                    let _ = tokio::fs::remove_file(&work_dir).await;
                }
            }
            git_service::worktree_add(repo_root, &work_dir, base_head).await?;

            workspaces.push(TeamMemberWorkspace {
                member_name: def.cli_name.to_string(),
                member_internal_id: def.internal_id.to_string(),
                task_type: def.task_type.to_string(),
                target: map_target_for_worktree(&target, &command_cwd, repo_root, &work_dir),
                work_dir,
                isolation_mode: "git-worktree".to_string(),
                patch_path,
            });
        } else {
            workspaces.push(TeamMemberWorkspace {
                member_name: def.cli_name.to_string(),
                member_internal_id: def.internal_id.to_string(),
                task_type: def.task_type.to_string(),
                target: target.clone(),
                work_dir: command_cwd.clone(),
                isolation_mode: "shared".to_string(),
                patch_path,
            });
        }
    }

    let started_at = Instant::now();
    let mut all_outcomes: Vec<TeamTaskOutcome> = Vec::new();
    let mut final_round_outcomes: Vec<TeamTaskOutcome> = Vec::new();
    let mut round_traces: Vec<TeamRunRoundRecord> = Vec::new();
    let mut shared_context: Vec<String> = Vec::new();
    let mut structured_context = StructuredTeamContext::new();

    for round in 1..=rounds {
        let round_objective = build_round_objective(
            &objective,
            round,
            rounds,
            &shared_context,
            Some(&structured_context),
        );
        let round_started = Instant::now();
        let mut round_outcomes = match run_mode {
            TeamRunMode::Parallel => {
                execute_round_parallel(
                    &workspaces,
                    round,
                    round_objective.clone(),
                    max_steps,
                    effective_parallelism,
                    timeout,
                    dry_run,
                    current_model.clone(),
                    approval_mode.clone(),
                )
                .await?
            }
            TeamRunMode::Pipeline => {
                execute_round_pipeline(
                    &workspaces,
                    round,
                    round_objective.clone(),
                    max_steps,
                    timeout,
                    dry_run,
                    current_model.clone(),
                    approval_mode.clone(),
                )
                .await
            }
        };
        sort_outcomes_by_member_order(&mut round_outcomes, &selected);

        let success_count = round_outcomes.iter().filter(|o| o.result.success).count();
        let failed_count = round_outcomes.len().saturating_sub(success_count);
        let changed_members = round_outcomes.iter().filter(|o| o.has_changes).count();
        let round_context = summarize_round_context(&round_outcomes, 24);

        round_traces.push(TeamRunRoundRecord {
            round,
            objective: round_objective,
            success_count,
            failed_count,
            changed_members,
            duration_ms: round_started.elapsed().as_millis() as u64,
            member_summaries: round_context.clone(),
        });
        let memory_updates = summarize_round_memory(&round_outcomes, round, 24);
        append_shared_memory(&mut shared_context, memory_updates, 72);
        structured_context.update_from_round(&round_outcomes, round);
        all_outcomes.extend(round_outcomes.clone());
        final_round_outcomes = round_outcomes;
    }

    let total_ms = started_at.elapsed().as_millis();
    let success_count = final_round_outcomes
        .iter()
        .filter(|o| o.result.success)
        .count();
    let failed_count = final_round_outcomes.len().saturating_sub(success_count);
    let changed_members = final_round_outcomes
        .iter()
        .filter(|o| o.has_changes)
        .count();

    let run_record = TeamRunRecord {
        run_id: run_id.clone(),
        created_at: Utc::now().timestamp(),
        command_cwd: command_cwd.display().to_string(),
        source_target: target.clone(),
        git_mode,
        repo_root: repo_root_opt.as_ref().map(|p| p.display().to_string()),
        base_head: base_head_opt.clone(),
        rounds,
        mode: team_run_mode_label(&run_mode).to_string(),
        round_traces: round_traces.clone(),
        shared_memory: shared_context.clone(),
        members: final_round_outcomes
            .iter()
            .map(|o| TeamRunMemberRecord {
                name: o.member_name.clone(),
                internal_id: o.member_internal_id.clone(),
                work_dir: o.work_dir.display().to_string(),
                target: o.target.clone(),
                isolation_mode: o.isolation_mode.clone(),
                patch_path: o.patch_path.display().to_string(),
                has_changes: o.has_changes,
                changed_files: o.changed_files,
                success: o.result.success,
                summary: o.result.summary.clone(),
                error: o.result.error.clone(),
                duration_ms: o.duration_ms as u64,
                round: o.round,
            })
            .collect(),
    };
    save_team_run_record(&storage, &run_record).await?;

    let mut lines = vec![
        "# Agent Team Run".to_string(),
        "".to_string(),
        format!("- run_id: `{}`", run_id),
        format!(
            "- run_record: `{}`",
            team_run_record_path(&storage, &run_id).display()
        ),
        format!(
            "- preset: {}",
            args.team
                .as_ref()
                .map(|s| format!("`{}`", s))
                .unwrap_or_else(|| "(inline)".to_string())
        ),
        format!("- objective: {}", objective),
        format!("- target: `{}`", target),
        format!(
            "- agents: {}",
            selected
                .iter()
                .map(|d| d.cli_name)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        format!(
            "- runtime: model=`{}` provider=`{}`",
            runtime_info.model,
            runtime_info
                .active_provider
                .unwrap_or_else(|| "n/a".to_string())
        ),
        format!("- base_url: `{}`", runtime_info.base_url),
        format!(
            "- controls: mode={} rounds={} parallelism={} max_steps={} timeout={} dry_run={}",
            team_run_mode_label(&run_mode),
            rounds,
            effective_parallelism,
            max_steps,
            timeout_secs
                .map(|v| format!("{}s", v))
                .unwrap_or_else(|| "none".to_string()),
            dry_run
        ),
        format!(
            "- isolation: {}",
            if git_mode {
                "git-worktree"
            } else {
                "shared(non-git)"
            }
        ),
        format!(
            "- result(final_round): {} success / {} failed / {} changed",
            success_count, failed_count, changed_members
        ),
        format!("- total_member_runs: {}", all_outcomes.len()),
        format!("- wall_time_ms: {}", total_ms),
    ];

    if let Some(repo_root) = repo_root_opt.as_ref() {
        lines.push(format!("- repo_root: `{}`", repo_root.display()));
    }
    if let Some(base_head) = base_head_opt {
        lines.push(format!("- base_head: `{}`", base_head));
    }
    if !git_mode {
        lines.push("- note: not a git repository. team used shared workspace; no per-member patch artifacts were generated.".to_string());
    } else {
        lines.push(format!(
            "- apply_hint: `/agents team apply {} --strategy manual`",
            run_id
        ));
        lines.push(format!(
            "- apply_check_hint: `/agents team apply {} --strategy ours --dry-run`",
            run_id
        ));
    }

    lines.push("".to_string());
    lines.push("## Round Traces".to_string());
    for trace in &round_traces {
        lines.push(format!(
            "### Round {} [{}ms] success={} failed={} changed={}",
            trace.round,
            trace.duration_ms,
            trace.success_count,
            trace.failed_count,
            trace.changed_members
        ));
        for summary in &trace.member_summaries {
            lines.push(format!("- {}", summary));
        }
    }

    if !shared_context.is_empty() {
        lines.push("".to_string());
        lines.push("## Shared Memory".to_string());
        for item in &shared_context {
            lines.push(format!("- {}", item));
        }
    }

    lines.push("".to_string());
    lines.push("## Members".to_string());

    for item in &final_round_outcomes {
        let status = if item.result.success { "✅" } else { "❌" };
        lines.push(format!(
            "### {} `{}` (`{}`) [round={} {}ms]",
            status, item.member_name, item.member_internal_id, item.round, item.duration_ms
        ));
        lines.push(format!("- summary: {}", item.result.summary));
        lines.push(format!("- work_dir: `{}`", item.work_dir.display()));
        lines.push(format!("- target: `{}`", item.target));
        lines.push(format!("- isolation_mode: `{}`", item.isolation_mode));
        if item.isolation_mode == "git-worktree" {
            lines.push(format!("- changed_files: {}", item.changed_files));
            lines.push(format!("- patch: `{}`", item.patch_path.display()));
        }

        if let Some(error) = &item.result.error {
            if !error.trim().is_empty() {
                lines.push(format!("- error: {}", error));
            }
        }
        if let Some(next_action) = &item.result.next_action {
            if !next_action.trim().is_empty() {
                lines.push(format!("- next_action: {}", next_action));
            }
        }
        if let Some(details) = &item.result.details {
            let body = truncate_chars(details, 1800);
            lines.push("```text".to_string());
            lines.push(body);
            lines.push("```".to_string());
        }
    }

    ctx.state.chat_history.push(
        ChatEntry::assistant(lines.join(
            "
",
        ))
        .with_streaming(false),
    );

    Ok(())
}
