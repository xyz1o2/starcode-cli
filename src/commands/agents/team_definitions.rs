use super::*;

#[derive(Clone, Copy, Debug)]
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
    // `internal_id` 必须等于 `SubAgentManager` 里注册的 id（见
    // `ExploreAgent::new` 的注释：id 是 `explore`）。这里曾经写成工具名
    // `Grep`，于是默认团队的检索成员每次都以
    // "team agent 'Grep' is not available" 失败 —— 而 `/agents team list`
    // 还把它列得好好的。`Grep` 保留为别名，用户照着旧文档敲仍能命中。
    TeamAgentDef {
        cli_name: "explore",
        internal_id: "explore",
        task_type: "Grep",
        // 别名一律小写：`normalize_agent_name` 会把输入小写化后再比对，
        // 这里写 `Grep` 就永远命中不了。
        aliases: &["search", "finder", "query", "grep"],
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── run mode ────────────────────────────────────────────────

    #[test]
    fn run_modes_round_trip_and_parse_case_insensitively() {
        for mode in [TeamRunMode::Parallel, TeamRunMode::Pipeline] {
            let label = team_run_mode_label(&mode);
            let parsed = parse_team_run_mode(label).expect("label must parse back");
            assert_eq!(team_run_mode_label(&parsed), label);
        }
        assert!(matches!(
            parse_team_run_mode("  PIPELINE "),
            Some(TeamRunMode::Pipeline)
        ));
        assert!(parse_team_run_mode("sequential").is_none());
    }

    // ── run id ──────────────────────────────────────────────────

    /// run_id 会成为目录名（`.star/agent-teams/runs/<id>/`），所以必须
    /// 在落盘前挡掉路径穿越。
    #[test]
    fn run_ids_reject_path_separators_and_empty_values() {
        assert!(validate_team_run_id("").unwrap_err().contains("required"));
        assert!(validate_team_run_id("   ")
            .unwrap_err()
            .contains("required"));
        let err = validate_team_run_id("../../etc").unwrap_err();
        assert!(err.contains("invalid run_id"), "{err}");
        assert!(validate_team_run_id("team 1").is_err());
    }

    #[test]
    fn run_ids_accept_alphanumerics_dash_and_underscore() {
        assert_eq!(
            validate_team_run_id(" team-20240101_abc ").unwrap(),
            "team-20240101_abc"
        );
    }

    #[test]
    fn generated_run_ids_are_valid_and_unique() {
        let a = generate_team_run_id();
        let b = generate_team_run_id();
        assert!(a.starts_with("team-"), "{a}");
        assert!(validate_team_run_id(&a).is_ok());
        assert_ne!(a, b);
    }

    // ── 名称归一 ────────────────────────────────────────────────

    #[test]
    fn agent_names_are_trimmed_lowercased_and_dash_normalized() {
        assert_eq!(normalize_agent_name("  Auto-Fix  "), "auto_fix");
        assert_eq!(normalize_agent_name("ANALYZER"), "analyzer");
    }

    // ── 目录查找 ────────────────────────────────────────────────

    /// 每个成员的 `internal_id` 必须是 `SubAgentManager` 里真实注册的 id，
    /// 否则 `manager.get_agent(...)` 返回 None，成员直接失败。
    #[test]
    fn every_catalog_member_points_at_a_registered_subagent_id() {
        let registered = [
            "analyzer",
            "editor",
            "explore",
            "navigator",
            "auto_fix_agent",
        ];
        for def in TEAM_AGENT_CATALOG {
            assert!(
                registered.contains(&def.internal_id),
                "`{}` points at `{}`, which no subagent registers (known: {registered:?})",
                def.cli_name,
                def.internal_id
            );
        }
    }

    #[test]
    fn catalog_lookup_accepts_cli_name_internal_id_and_aliases() {
        let by_name = find_team_agent("explore").expect("cli name");
        assert_eq!(by_name.internal_id, "explore");

        // 旧名 `Grep` 仍然作为别名可解析，但解析结果是真正的 explore
        let legacy = find_team_agent("Grep").expect("legacy alias");
        assert_eq!(legacy.cli_name, "explore");

        let by_alias = find_team_agent("autofix").expect("alias");
        assert_eq!(by_alias.cli_name, "auto_fix");
    }

    #[test]
    fn unknown_names_resolve_to_nothing() {
        assert!(find_team_agent("gpt").is_none());
        assert!(find_team_agent("").is_none());
    }

    // ── 成员选择 ────────────────────────────────────────────────

    #[test]
    fn selecting_all_returns_the_whole_catalog() {
        for selector in ["all", "*"] {
            let agents = resolve_team_agents(vec![selector.to_string()]).unwrap();
            assert_eq!(agents.len(), TEAM_AGENT_CATALOG.len());
        }
    }

    /// 同一个成员用不同写法（名字 + 别名）重复出现时只保留一份 ——
    /// 否则团队会派两个相同的成员去改同一批文件。
    #[test]
    fn duplicate_selections_collapse_to_one_member() {
        let agents = resolve_team_agents(vec![
            "analyzer".to_string(),
            "analysis".to_string(),
            "ANALYZER".to_string(),
        ])
        .unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].cli_name, "analyzer");
    }

    #[test]
    fn selection_preserves_the_requested_order() {
        let agents = resolve_team_agents(vec![
            "editor".to_string(),
            "explore".to_string(),
            "analyzer".to_string(),
        ])
        .unwrap();
        let names: Vec<&str> = agents.iter().map(|d| d.cli_name).collect();
        assert_eq!(names, ["editor", "explore", "analyzer"]);
    }

    /// 拼错一个成员名时必须整体失败并列出拼错的那个 —— 静默丢掉它会让
    /// 用户以为团队跑齐了。
    #[test]
    fn an_unknown_member_fails_the_whole_selection() {
        let err =
            resolve_team_agents(vec!["analyzer".to_string(), "gpt5".to_string()]).unwrap_err();
        assert!(err.contains("unknown team agents: gpt5"), "{err}");
        assert!(err.contains("/agents team list"), "{err}");
    }

    #[test]
    fn an_empty_selection_explains_the_flag() {
        let err = resolve_team_agents(vec!["   ".to_string()]).unwrap_err();
        assert!(err.contains("no valid team agents selected"), "{err}");
        assert!(err.contains("--agents"), "{err}");
        assert!(resolve_team_agents(Vec::new()).is_err());
    }
}
