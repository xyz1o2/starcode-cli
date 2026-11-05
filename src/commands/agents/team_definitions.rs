use super::*;

/// 团队成员定义。
///
/// ── 2026-11-05：成员来源从「内置清单」改为「用户自定义 agent」 ──────────
///
/// 原来这是一个 `&'static` 常量数组（`TEAM_AGENT_CATALOG`），逐条写死
/// 5 个内置子代理（analyzer / editor / explore / navigator / auto_fix）。
/// 那 5 个内置子代理的 Rust 实现已从 `src/agent/skills/` 删除，于是这份
/// 静态清单必须跟着改成**动态**的：成员来自用户在 `.star/agents/` 或
/// `~/.starcode/agents/` 里写的 agent 定义。
///
/// 字段因此从 `&'static str` 变成 `String`（名字与别名不再可能是静态的）。
#[derive(Clone, Debug)]
pub(super) struct TeamAgentDef {
    pub(super) cli_name: String,
    pub(super) internal_id: String,
    pub(super) task_type: String,
    pub(super) aliases: Vec<String>,
    pub(super) description: String,
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

/// 团队成员清单 —— **动态**构建，来源是用户自定义 agent。
///
/// ── 2026-11-05：内置成员下线 ───────────────────────────────────────────
///
/// 原来这里是 `&'static` 常量数组，逐条写死 5 个内置子代理。那 5 个内置
/// 子代理的 Rust 实现已从 `src/agent/skills/` 删除，清单随之改为动态：
/// 成员 = 用户在 `.star/agents/` / `~/.starcode/agents/` 里定义的 agent。
///
/// 清单为空是**正常状态**（用户还没定义 agent）—— `/agents team list`
/// 会如实显示"没有可用成员"，而不是列出一堆点不动的东西。
pub(super) fn team_agent_catalog(project_root: &std::path::Path) -> Vec<TeamAgentDef> {
    let defs = crate::agent::skills::custom::load_custom_subagent_definitions(project_root);
    defs.into_iter()
        .map(|def| {
            // 别名一律小写并去重：`normalize_agent_name` 会把输入小写化
            // 后再比对，混进大写别名就永远命中不了。
            let mut aliases: Vec<String> = def
                .aliases
                .iter()
                .map(|alias| normalize_agent_name(alias))
                .filter(|alias| !alias.is_empty())
                .collect();
            aliases.sort();
            aliases.dedup();

            // `task_type` 取第一个 capability（自定义 agent 用 capability
            // 声明自己能干什么）；没有就退回 id 本身。
            let task_type = def
                .capabilities
                .first()
                .cloned()
                .unwrap_or_else(|| def.id.clone());

            TeamAgentDef {
                cli_name: def.name.clone(),
                internal_id: def.id.clone(),
                task_type,
                aliases,
                description: def.description.clone(),
            }
        })
        .collect()
}

pub(super) fn normalize_agent_name(input: &str) -> String {
    input.trim().to_lowercase().replace('-', "_")
}

pub(super) fn find_team_agent<'a>(
    input: &str,
    catalog: &'a [TeamAgentDef],
) -> Option<&'a TeamAgentDef> {
    let n = normalize_agent_name(input);
    catalog.iter().find(|def| {
        normalize_agent_name(&def.cli_name) == n
            || normalize_agent_name(&def.internal_id) == n
            || def.aliases.iter().any(|alias| alias == &n)
    })
}

pub(super) fn resolve_team_agents<'a>(
    raw_agents: Vec<String>,
    catalog: &'a [TeamAgentDef],
) -> Result<Vec<&'a TeamAgentDef>, String> {
    let requested: Vec<String> = raw_agents
        .into_iter()
        .map(|s| normalize_agent_name(&s))
        .filter(|s| !s.is_empty())
        .collect();

    let select_all = requested.iter().any(|name| name == "all" || name == "*");

    if select_all {
        return Ok(catalog.iter().collect());
    }

    let mut unknown: Vec<String> = Vec::new();
    let mut selected: Vec<&'a TeamAgentDef> = Vec::new();
    let mut seen = HashSet::new();

    for name in requested {
        if let Some(def) = find_team_agent(&name, catalog) {
            if seen.insert(def.cli_name.clone()) {
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
        return Err(
            "no valid team agents selected. use --agents <name> (see `/agents team list`)".to_string(),
        );
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

    /// 团队成员来自**用户自定义 agent**，所以清单是动态的。
    ///
    /// 这条测试守住两件事：
    /// 1. 没有用户 agent 时清单为空（内置子代理下线后不应再有"幽灵成员"）；
    /// 2. 用户 agent 一旦存在，就出现在清单里，别名一律小写。
    #[test]
    fn the_catalog_comes_from_user_defined_agents() {
        let dir = tempfile::tempdir().expect("tmp");
        // 空目录 → 没有用户 agent → 清单为空。
        assert!(
            team_agent_catalog(dir.path()).is_empty(),
            "没有用户自定义 agent 时，团队清单必须为空（内置成员已下线）"
        );

        // 写一个用户 agent → 出现在清单里。
        let agents_dir = dir.path().join(".star").join("agents");
        std::fs::create_dir_all(&agents_dir).expect("mkdir");
        std::fs::write(
            agents_dir.join("reviewer.md"),
            "---\nid: reviewer\nname: reviewer\ndescription: 代码审查\naliases: [Review, CR]\n---\n\n审查代码。\n",
        )
        .expect("write");

        let catalog = team_agent_catalog(dir.path());
        assert_eq!(catalog.len(), 1, "用户 agent 应出现在清单里：{catalog:?}");
        let def = &catalog[0];
        assert_eq!(def.internal_id, "reviewer");
        assert_eq!(def.cli_name, "reviewer");
        assert_eq!(def.description, "代码审查");
        // 别名一律小写：`normalize_agent_name` 会把输入小写化后再比对。
        assert!(
            def.aliases.iter().all(|a| a == &a.to_lowercase()),
            "别名必须小写，否则永远命中不了：{:?}",
            def.aliases
        );
    }

    #[test]
    fn catalog_lookup_accepts_cli_name_internal_id_and_aliases() {
        let catalog = vec![
            TeamAgentDef {
                cli_name: "explore".to_string(),
                internal_id: "explore".to_string(),
                task_type: "Grep".to_string(),
                aliases: vec![
                    "search".to_string(),
                    "finder".to_string(),
                    "query".to_string(),
                    "grep".to_string(),
                ],
                description: "代码检索与上下文召回".to_string(),
            },
            TeamAgentDef {
                cli_name: "auto_fix".to_string(),
                internal_id: "auto_fix_agent".to_string(),
                task_type: "auto_fix".to_string(),
                aliases: vec!["autofix".to_string()],
                description: "测试失败分析与自动修复循环".to_string(),
            },
        ];

        let by_name = find_team_agent("explore", &catalog).expect("cli name");
        assert_eq!(by_name.internal_id, "explore");

        // 旧名 `Grep` 仍然作为别名可解析，但解析结果是真正的 explore
        let legacy = find_team_agent("Grep", &catalog).expect("legacy alias");
        assert_eq!(legacy.cli_name, "explore");

        let by_alias = find_team_agent("autofix", &catalog).expect("alias");
        assert_eq!(by_alias.cli_name, "auto_fix");
    }

    #[test]
    fn unknown_names_resolve_to_nothing() {
        let catalog = Vec::new();
        assert!(find_team_agent("gpt", &catalog).is_none());
        assert!(find_team_agent("", &catalog).is_none());
    }

    // ── 成员选择 ────────────────────────────────────────────────

    #[test]
    fn selecting_all_returns_the_whole_catalog() {
        let catalog = vec![
            TeamAgentDef {
                cli_name: "a".to_string(),
                internal_id: "a".to_string(),
                task_type: "t".to_string(),
                aliases: vec![],
                description: "d".to_string(),
            },
            TeamAgentDef {
                cli_name: "b".to_string(),
                internal_id: "b".to_string(),
                task_type: "t".to_string(),
                aliases: vec![],
                description: "d".to_string(),
            },
        ];
        for selector in ["all", "*"] {
            let agents = resolve_team_agents(vec![selector.to_string()], &catalog).unwrap();
            assert_eq!(agents.len(), catalog.len());
        }
    }

    /// 同一个成员用不同写法（名字 + 别名）重复出现时只保留一份 ——
    /// 否则团队会派两个相同的成员去改同一批文件。
    #[test]
    fn duplicate_selections_collapse_to_one_member() {
        let catalog = vec![TeamAgentDef {
            cli_name: "analyzer".to_string(),
            internal_id: "analyzer".to_string(),
            task_type: "analyze".to_string(),
            aliases: vec!["analysis".to_string()],
            description: "结构分析与问题研判".to_string(),
        }];
        let agents = resolve_team_agents(
            vec![
                "analyzer".to_string(),
                "analysis".to_string(),
                "ANALYZER".to_string(),
            ],
            &catalog,
        )
        .unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].cli_name, "analyzer");
    }

    #[test]
    fn selection_preserves_the_requested_order() {
        let catalog = vec![
            TeamAgentDef {
                cli_name: "editor".to_string(),
                internal_id: "editor".to_string(),
                task_type: "edit".to_string(),
                aliases: vec![],
                description: "d".to_string(),
            },
            TeamAgentDef {
                cli_name: "explore".to_string(),
                internal_id: "explore".to_string(),
                task_type: "Grep".to_string(),
                aliases: vec![],
                description: "d".to_string(),
            },
            TeamAgentDef {
                cli_name: "analyzer".to_string(),
                internal_id: "analyzer".to_string(),
                task_type: "analyze".to_string(),
                aliases: vec![],
                description: "d".to_string(),
            },
        ];
        let agents = resolve_team_agents(
            vec![
                "editor".to_string(),
                "explore".to_string(),
                "analyzer".to_string(),
            ],
            &catalog,
        )
        .unwrap();
        let names: Vec<&str> = agents.iter().map(|d| d.cli_name.as_str()).collect();
        assert_eq!(names, ["editor", "explore", "analyzer"]);
    }

    /// 拼错一个成员名时必须整体失败并列出拼错的那个 —— 静默丢掉它会让
    /// 用户以为团队跑齐了。
    #[test]
    fn an_unknown_member_fails_the_whole_selection() {
        let catalog = vec![TeamAgentDef {
            cli_name: "analyzer".to_string(),
            internal_id: "analyzer".to_string(),
            task_type: "analyze".to_string(),
            aliases: vec![],
            description: "d".to_string(),
        }];
        let err = resolve_team_agents(
            vec!["analyzer".to_string(), "gpt5".to_string()],
            &catalog,
        )
        .unwrap_err();
        assert!(err.contains("unknown team agents: gpt5"), "{err}");
        assert!(err.contains("/agents team list"), "{err}");
    }

    #[test]
    fn an_empty_selection_explains_the_flag() {
        let catalog = Vec::new();
        let err = resolve_team_agents(vec!["   ".to_string()], &catalog).unwrap_err();
        assert!(err.contains("no valid team agents selected"), "{err}");
        assert!(err.contains("--agents"), "{err}");
        assert!(resolve_team_agents(Vec::new(), &catalog).is_err());
    }
}
