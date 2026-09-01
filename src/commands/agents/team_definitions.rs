use super::*;

#[derive(Clone, Copy)]
pub(super) struct TeamAgentDef {
    pub(super) cli_name: &'static str,
    pub(super) internal_id: &'static str,
    pub(super) task_type: &'static str,
    pub(super) aliases: &'static [&'static str],
    pub(super) description: &'static str,
}

pub(super) fn team_run_mode_label(mode: &TeamRunMode) -> &'static str {
    match mode {
        TeamRunMode::Parallel => "parallel",
        TeamRunMode::Pipeline => "pipeline",
    }
}

pub(super) fn parse_team_run_mode(raw: &str) -> Option<TeamRunMode> {
    match raw.trim().to_lowercase().as_str() {
        "parallel" => Some(TeamRunMode::Parallel),
        "pipeline" => Some(TeamRunMode::Pipeline),
        _ => None,
    }
}

#[derive(Clone)]
pub(super) struct TeamMemberWorkspace {
    pub(super) member_name: String,
    pub(super) member_internal_id: String,
    pub(super) task_type: String,
    pub(super) work_dir: PathBuf,
    pub(super) target: String,
    pub(super) isolation_mode: String,
    pub(super) patch_path: PathBuf,
}

pub(super) fn validate_team_run_id(run_id: &str) -> Result<String, String> {
    let trimmed = run_id.trim();
    if trimmed.is_empty() {
        return Err("run_id is required".to_string());
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "invalid run_id `{}`: only [a-zA-Z0-9_-] are allowed",
            run_id
        ));
    }
    Ok(trimmed.to_string())
}

pub(super) fn generate_team_run_id() -> String {
    let ts = Utc::now().format("%Y%m%d%H%M%S");
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    format!("team-{}-{}", ts, &suffix[..8])
}

pub(super) const TEAM_AGENT_CATALOG: &[TeamAgentDef] = &[
    TeamAgentDef {
        cli_name: "Grep",
        internal_id: "Grep",
        task_type: "Grep",
        aliases: &["finder", "query", "Grep"],
        description: "代码检索与上下文召回",
    },
    TeamAgentDef {
        cli_name: "analyzer",
        internal_id: "analyzer",
        task_type: "analyze",
        aliases: &["analysis", "analyze", "review"],
        description: "结构分析与问题研判",
    },
    TeamAgentDef {
        cli_name: "editor",
        internal_id: "editor",
        task_type: "edit",
        aliases: &["edit", "refactor", "modify"],
        description: "代码改动与重构执行",
    },
    TeamAgentDef {
        cli_name: "navigator",
        internal_id: "navigator",
        task_type: "navigate",
        aliases: &["nav", "trace", "dependency"],
        description: "递归追踪依赖与调用链",
    },
    TeamAgentDef {
        cli_name: "auto_fix",
        internal_id: "auto_fix_agent",
        task_type: "auto_fix",
        aliases: &["autofix", "auto-fix", "test"],
        description: "测试失败分析与自动修复循环",
    },
];

pub(super) fn normalize_agent_name(input: &str) -> String {
    input.trim().to_lowercase().replace('-', "_")
}

pub(super) fn find_team_agent(input: &str) -> Option<&'static TeamAgentDef> {
    let n = normalize_agent_name(input);
    TEAM_AGENT_CATALOG.iter().find(|def| {
        def.cli_name == n || def.internal_id == n || def.aliases.iter().any(|alias| *alias == n)
    })
}

pub(super) fn resolve_team_agents(
    raw_agents: Vec<String>,
) -> Result<Vec<&'static TeamAgentDef>, String> {
    let requested: Vec<String> = raw_agents
        .into_iter()
        .map(|s| normalize_agent_name(&s))
        .filter(|s| !s.is_empty())
        .collect();

    let select_all = requested.iter().any(|name| name == "all" || name == "*");

    if select_all {
        return Ok(TEAM_AGENT_CATALOG.iter().collect());
    }

    let mut unknown: Vec<String> = Vec::new();
    let mut selected: Vec<&'static TeamAgentDef> = Vec::new();
    let mut seen = HashSet::new();

    for name in requested {
        if let Some(def) = find_team_agent(&name) {
            if seen.insert(def.cli_name) {
                selected.push(def);
            }
        } else {
            unknown.push(name);
        }
    }

    if !unknown.is_empty() {
        return Err(format!(
            "unknown team agents: {}. use `/agents team list` to inspect available members.",
            unknown.join(", ")
        ));
    }

    if selected.is_empty() {
        return Err("no valid team agents selected. use --agents search,analyzer,...".to_string());
    }

    Ok(selected)
}
