mod custom_agents;
mod team_apply;
mod team_definitions;
mod team_execution;
mod team_presets;
mod team_runs;

use crate::commands::agent_team_presets::{
    list_team_presets, load_team_preset_store, resolve_team_preset, sanitize_preset_name,
    save_team_preset_store, scope_label, team_preset_file_path, TeamPreset, TeamPresetScope,
};
use crate::commands::agent_team_render::{render_team_run_details, render_team_runs_list};
use crate::commands::agent_team_support::{
    cleanup_team_run_artifacts, collect_apply_conflict_files, load_team_run_record,
    map_target_for_worktree, save_team_run_record, scan_team_run_records, summarize_output,
    team_run_dir, team_run_record_path, team_runs_root, TeamRunMemberRecord, TeamRunRecord,
    TeamRunRoundRecord,
};
use crate::commands::execution::{CommandContext, CommandResult};
use crate::core::config::provider_resolution::{
    resolve_effective_provider_settings, ProviderResolutionInputs,
};
use crate::core::config::provider_store::ProviderStore;
use crate::core::config::{Config, ConfigParameters};
use crate::core::services::git_service;
use crate::llm::client::StarClient;
use crate::types::ChatEntry;
use chrono::Utc;
use clap::Args;
use clap::Subcommand;
use clap::ValueEnum;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::agent::skills::custom::{
    load_custom_subagent_definitions, load_custom_subagent_from_file, normalize_custom_agent_id,
    render_custom_subagent_markdown,
};
use crate::agent::skills::{
    register_custom_subagents, AnalyzerAgent, AutoFixAgent, EditorAgent, ExploreAgent,
    NavigatorAgent, SubAgentManager, SubTask, SubTaskResult,
};

#[derive(Subcommand)]
pub(crate) enum AgentsCommand {
    /// List available custom agents
    List,
    /// Create a new custom agent
    Create(AgentCreateArgs),
    /// Edit an existing custom agent
    Edit(AgentEditArgs),
    /// Delete a custom agent
    Delete {
        /// Agent name
        name: String,
        /// Delete from user-level scope instead of project-level
        #[arg(long)]
        user: bool,
    },
    /// Add an agent from a file path
    Add {
        /// Path to the agent definition file
        source: String,
        /// Optional name override
        #[arg(long)]
        name: Option<String>,
    },
    /// Remove an agent definition file
    Remove {
        /// Agent name
        name: String,
    },
    /// Manage agent teams
    Team {
        #[command(subcommand)]
        command: AgentTeamCommand,
    },
}

#[derive(Args)]
pub struct AgentCreateArgs {
    /// Agent ID / name (used as file name)
    pub name: String,
    /// Display name shown in list
    #[arg(long)]
    pub display_name: Option<String>,
    /// Agent short description
    #[arg(long)]
    pub description: Option<String>,
    /// Tool names (comma-separated)
    #[arg(long, value_delimiter = ',')]
    pub tools: Vec<String>,
    /// Alias names (comma-separated)
    #[arg(long, value_delimiter = ',')]
    pub aliases: Vec<String>,
    /// Override model name for this sub-agent
    #[arg(long)]
    pub model: Option<String>,
    /// Prompt text
    #[arg(long)]
    pub prompt: Option<String>,
    /// Load prompt from file path
    #[arg(long)]
    pub prompt_file: Option<String>,
    /// Create under user scope (~/.star/agents)
    #[arg(long, default_value_t = false)]
    pub user: bool,
}

#[derive(Args)]
pub struct AgentEditArgs {
    /// Existing agent name or id
    pub name: String,
    /// Rename to new ID / name
    #[arg(long)]
    pub new_name: Option<String>,
    /// Update display name
    #[arg(long)]
    pub display_name: Option<String>,
    /// Update description
    #[arg(long)]
    pub description: Option<String>,
    /// Replace tools list (comma-separated)
    #[arg(long, value_delimiter = ',')]
    pub tools: Option<Vec<String>>,
    /// Clear tools list
    #[arg(long, default_value_t = false)]
    pub clear_tools: bool,
    /// Replace alias list (comma-separated)
    #[arg(long, value_delimiter = ',')]
    pub aliases: Option<Vec<String>>,
    /// Clear alias list
    #[arg(long, default_value_t = false)]
    pub clear_aliases: bool,
    /// Set/override model
    #[arg(long)]
    pub model: Option<String>,
    /// Clear model setting
    #[arg(long, default_value_t = false)]
    pub clear_model: bool,
    /// Replace prompt text
    #[arg(long)]
    pub prompt: Option<String>,
    /// Replace prompt from file path
    #[arg(long)]
    pub prompt_file: Option<String>,
    /// Edit in user scope (~/.star/agents) only
    #[arg(long, default_value_t = false)]
    pub user: bool,
}

#[derive(Subcommand)]
pub(crate) enum AgentTeamCommand {
    /// List built-in team agents and aliases
    List,
    /// List historical team runs
    Runs(AgentTeamRunsArgs),
    /// Show one team run details
    #[command(arg_required_else_help = true)]
    ShowRun(AgentTeamShowRunArgs),
    /// Run a team objective with selected agents
    Run(AgentTeamRunArgs),
    /// Save a reusable team preset into project/user scope
    #[command(arg_required_else_help = true)]
    Save(AgentTeamSaveArgs),
    /// Show one team preset details
    #[command(arg_required_else_help = true)]
    Show {
        /// Team preset name
        name: String,
    },
    /// Remove a team preset
    #[command(arg_required_else_help = true)]
    Remove {
        /// Team preset name
        name: String,
        /// Remove from user scope (~/.star) instead of project scope
        #[arg(long, default_value_t = false)]
        user: bool,
    },
    /// Apply one team run back to current repository
    #[command(arg_required_else_help = true)]
    Apply(AgentTeamApplyArgs),
    /// Clean run artifacts/worktrees for one run or all runs
    Clean(AgentTeamCleanArgs),
}

#[derive(Args)]
pub struct AgentTeamRunArgs {
    /// Team preset name to load first
    #[arg(long)]
    pub team: Option<String>,
    /// Team members (comma-separated): search,analyzer,editor,navigator,auto_fix or all
    #[arg(long, value_delimiter = ',')]
    pub agents: Option<Vec<String>>,
    /// Target path or module (default: .)
    #[arg(long)]
    pub target: Option<String>,
    /// Max steps for each sub-agent task
    #[arg(long)]
    pub max_steps: Option<usize>,
    /// Team execution parallelism
    #[arg(long)]
    pub parallelism: Option<usize>,
    /// Run mode: parallel | pipeline
    #[arg(long, value_enum)]
    pub mode: Option<TeamRunMode>,
    /// Multi-round collaboration count
    #[arg(long)]
    pub rounds: Option<usize>,
    /// Per-agent timeout (seconds)
    #[arg(long)]
    pub timeout_secs: Option<u64>,
    /// Force editor agent into dry_run=true mode
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
    /// Objective text (put it at the end)
    pub objective: Vec<String>,
}

#[derive(Args)]
pub struct AgentTeamSaveArgs {
    /// Preset name
    pub name: String,
    /// Team members (comma-separated): search,analyzer,editor,navigator,auto_fix or all
    #[arg(long, value_delimiter = ',', default_value = "search,analyzer,editor")]
    pub agents: Vec<String>,
    /// Default target path for this preset
    #[arg(long)]
    pub target: Option<String>,
    /// Default max steps for this preset
    #[arg(long)]
    pub max_steps: Option<usize>,
    /// Default parallelism for this preset
    #[arg(long)]
    pub parallelism: Option<usize>,
    /// Default run mode for this preset
    #[arg(long, value_enum)]
    pub mode: Option<TeamRunMode>,
    /// Default round count for this preset
    #[arg(long)]
    pub rounds: Option<usize>,
    /// Default timeout seconds for this preset
    #[arg(long)]
    pub timeout_secs: Option<u64>,
    /// Force editor agent dry-run by default
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
    /// Optional description
    #[arg(long)]
    pub description: Option<String>,
    /// Optional default objective text
    #[arg(long)]
    pub objective: Option<String>,
    /// Save to user scope (~/.star) instead of project scope
    #[arg(long, default_value_t = false)]
    pub user: bool,
}

#[derive(Debug, Clone, ValueEnum)]
pub enum TeamApplyStrategy {
    Manual,
    Ours,
}

#[derive(Debug, Clone, ValueEnum)]
pub enum TeamRunMode {
    Parallel,
    Pipeline,
}

#[derive(Args)]
pub struct AgentTeamApplyArgs {
    /// Team run id (from /agents team run output)
    pub run_id: String,
    /// Apply strategy: manual | ours
    #[arg(long, value_enum, default_value = "manual")]
    pub strategy: TeamApplyStrategy,
    /// Apply only selected members (comma-separated)
    #[arg(long, value_delimiter = ',')]
    pub members: Option<Vec<String>>,
    /// Dry-run apply (check only, no workspace changes)
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
    /// Require clean git workspace before apply
    #[arg(long, default_value_t = false)]
    pub require_clean: bool,
    /// Require current HEAD to match run base_head
    #[arg(long, default_value_t = false)]
    pub base_head_check: bool,
    /// Auto clean run artifacts after successful apply
    #[arg(long, default_value_t = false)]
    pub auto_clean: bool,
}

#[derive(Args)]
pub struct AgentTeamRunsArgs {
    /// Max run records to show
    #[arg(long, default_value_t = 10)]
    pub limit: usize,
}

#[derive(Args)]
pub struct AgentTeamShowRunArgs {
    /// Team run id
    pub run_id: String,
    /// Output as JSON
    #[arg(long, default_value_t = false)]
    pub json: bool,
    /// Show only selected members (comma-separated)
    #[arg(long, value_delimiter = ',')]
    pub members: Option<Vec<String>>,
}

#[derive(Args)]
pub struct AgentTeamCleanArgs {
    /// Team run id to clean (omit when using --all)
    pub run_id: Option<String>,
    /// Clean all run artifacts for current project
    #[arg(long, default_value_t = false, conflicts_with = "run_id")]
    pub all: bool,
}

pub(crate) async fn execute_agents_command(
    ctx: CommandContext<'_>,
    cmd: AgentsCommand,
) -> CommandResult {
    match cmd {
        AgentsCommand::List => custom_agents::list_agents(ctx).await,
        AgentsCommand::Create(args) => custom_agents::create_agent(ctx, args).await,
        AgentsCommand::Edit(args) => custom_agents::edit_agent(ctx, args).await,
        AgentsCommand::Delete { name, user } => custom_agents::delete_agent(ctx, name, user).await,
        AgentsCommand::Add { source, name } => custom_agents::add_agent(ctx, source, name).await,
        AgentsCommand::Remove { name } => custom_agents::remove_agent(ctx, name).await,
        AgentsCommand::Team { command } => execute_agent_team_command(ctx, command).await,
    }
}

async fn execute_agent_team_command(
    ctx: CommandContext<'_>,
    cmd: AgentTeamCommand,
) -> CommandResult {
    match cmd {
        AgentTeamCommand::List => team_runs::list_agent_team_catalog(ctx).await,
        AgentTeamCommand::Runs(args) => team_runs::list_team_runs(ctx, args).await,
        AgentTeamCommand::ShowRun(args) => team_runs::show_team_run(ctx, args).await,
        AgentTeamCommand::Run(args) => team_execution::run_agent_team(ctx, args).await,
        AgentTeamCommand::Save(args) => team_presets::save_team_preset(ctx, args).await,
        AgentTeamCommand::Show { name } => team_presets::show_team_preset(ctx, name).await,
        AgentTeamCommand::Remove { name, user } => {
            team_presets::remove_team_preset(ctx, name, user).await
        }
        AgentTeamCommand::Apply(args) => team_apply::apply_team_run(ctx, args).await,
        AgentTeamCommand::Clean(args) => team_runs::clean_team_runs(ctx, args).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, FromArgMatches};

    /// 与 `commands/mod.rs` 的 `simple_command_wrapper!` 走同一条 clap 路径：
    /// `augment_subcommands` + `from_arg_matches`。
    fn parse(args: &[&str]) -> Result<AgentsCommand, clap::Error> {
        let cmd = AgentsCommand::augment_subcommands(clap::Command::new("agents"));
        let argv = std::iter::once("agents".to_string())
            .chain(args.iter().map(|s| s.to_string()))
            .collect::<Vec<_>>();
        let matches = cmd.try_get_matches_from(argv)?;
        AgentsCommand::from_arg_matches(&matches)
    }

    fn parse_team_save(args: &[&str]) -> AgentTeamSaveArgs {
        match parse(args).expect("save must parse") {
            AgentsCommand::Team {
                command: AgentTeamCommand::Save(a),
            } => a,
            other => panic!(
                "expected team save, got {:?}",
                std::mem::discriminant(&other)
            ),
        }
    }

    // ── clap 解析 ───────────────────────────────────────────────

    /// `--agents` 的默认值是文档里承诺的三个成员；改了它等于改了
    /// `/agents team save` 的语义。
    #[test]
    fn team_save_defaults_to_the_documented_members() {
        let args = parse_team_save(&["team", "save", "my-team"]);
        assert_eq!(args.name, "my-team");
        assert_eq!(
            args.agents,
            vec![
                "search".to_string(),
                "analyzer".to_string(),
                "editor".to_string()
            ]
        );
        assert!(!args.user);
        assert!(!args.dry_run);
        assert!(args.mode.is_none());
        assert!(args.max_steps.is_none());
    }

    #[test]
    fn team_save_splits_comma_separated_agents_and_parses_the_mode() {
        let args = parse_team_save(&[
            "team",
            "save",
            "rust-refactor",
            "--agents",
            "explore,editor",
            "--mode",
            "pipeline",
            "--max-steps",
            "5",
            "--parallelism",
            "3",
            "--rounds",
            "2",
            "--timeout-secs",
            "90",
            "--dry-run",
            "--user",
            "--target",
            "src",
            "--description",
            "refactor helpers",
        ]);
        assert_eq!(
            args.agents,
            vec!["explore".to_string(), "editor".to_string()]
        );
        assert!(matches!(args.mode, Some(TeamRunMode::Pipeline)));
        assert_eq!(args.max_steps, Some(5));
        assert_eq!(args.parallelism, Some(3));
        assert_eq!(args.rounds, Some(2));
        assert_eq!(args.timeout_secs, Some(90));
        assert!(args.dry_run);
        assert!(args.user);
        assert_eq!(args.target.as_deref(), Some("src"));
        assert_eq!(args.description.as_deref(), Some("refactor helpers"));
    }

    #[test]
    fn team_save_rejects_an_unknown_mode() {
        let err = parse(&["team", "save", "x", "--mode", "shuffle"])
            .err()
            .expect("unknown mode must fail");
        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue);
    }

    /// 目标文本是「剩下的所有词」，不是 `--objective` 选项 —— 用户是
    /// 按自然语言敲进来的。
    #[test]
    fn team_run_keeps_the_objective_as_trailing_words() {
        let parsed = parse(&[
            "team",
            "run",
            "--agents",
            "search,editor",
            "fix",
            "the",
            "build",
        ])
        .expect("run must parse");
        match parsed {
            AgentsCommand::Team {
                command: AgentTeamCommand::Run(args),
            } => {
                assert_eq!(args.agents.unwrap(), vec!["search", "editor"]);
                assert_eq!(args.objective, vec!["fix", "the", "build"]);
            }
            _ => panic!("expected team run"),
        }
    }

    /// `show` / `remove` / `apply` 都标了 `arg_required_else_help`：少了
    /// 名字应该打印用法，而不是跑一个空名字的查询。
    #[test]
    fn commands_that_need_an_identifier_print_help_without_one() {
        for argv in [
            vec!["team", "show"],
            vec!["team", "remove"],
            vec!["team", "apply"],
            vec!["team", "save"],
        ] {
            let err = parse(&argv).err().expect("missing identifier must fail");
            assert_eq!(
                err.kind(),
                clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand,
                "{argv:?}"
            );
        }
    }

    /// `clean` 的两种形式互斥：要么指定 run_id，要么 `--all`。
    #[test]
    fn clean_accepts_a_run_id_or_all_but_not_both() {
        match parse(&["team", "clean", "run-123"]).unwrap() {
            AgentsCommand::Team {
                command: AgentTeamCommand::Clean(args),
            } => {
                assert_eq!(args.run_id.as_deref(), Some("run-123"));
                assert!(!args.all);
            }
            _ => panic!("expected team clean"),
        }

        let err = parse(&["team", "clean", "run-123", "--all"])
            .err()
            .expect("--all conflicts with run_id");
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    // ── 分发覆盖 ────────────────────────────────────────────────

    fn source() -> &'static str {
        include_str!("mod.rs")
    }

    /// 取出某个 match 臂的源码（含续行）。
    ///
    /// 必须做**边界**匹配：`AgentTeamCommand::Run` 是
    /// `AgentTeamCommand::Runs` 的前缀，纯 `find` 会先撞上 `Runs`，于是
    /// 一条本来正确的分发被误判成「接错了处理函数」。
    fn arm_source(body: &str, enum_name: &str, variant: &str) -> Option<String> {
        let marker = format!("{enum_name}::{variant}");
        let lines: Vec<&str> = body.lines().collect();
        for (idx, line) in lines.iter().enumerate() {
            let Some(pos) = line.find(&marker) else {
                continue;
            };
            let after = line[pos + marker.len()..].chars().next();
            let is_boundary = matches!(after, None | Some('(') | Some(' ') | Some('{') | Some(','));
            if !is_boundary {
                continue;
            }
            let tail = lines[idx..lines.len().min(idx + 4)].join("\n");
            return Some(tail);
        }
        None
    }

    /// 枚举变体一旦加进来却没有 match 臂，编译器会报错；但反过来，
    /// 一个变体接错了处理函数（比如 `Show` 接到 `Remove`）编译器不会
    /// 吭声。这里逐条把变体名和它后面紧跟的处理函数钉住。
    #[test]
    fn every_team_subcommand_reaches_its_own_handler() {
        let body = {
            let start = source()
                .find("async fn execute_agent_team_command(")
                .expect("execute_agent_team_command must exist");
            let rest = &source()[start..];
            let end = rest.find("\n}\n").expect("body end");
            &rest[..end]
        };

        let expected = [
            ("List", "team_runs::list_agent_team_catalog"),
            ("Runs", "team_runs::list_team_runs"),
            ("ShowRun", "team_runs::show_team_run"),
            ("Run", "team_execution::run_agent_team"),
            ("Save", "team_presets::save_team_preset"),
            ("Show", "team_presets::show_team_preset"),
            ("Remove", "team_presets::remove_team_preset"),
            ("Apply", "team_apply::apply_team_run"),
            ("Clean", "team_runs::clean_team_runs"),
        ];

        for (variant, handler) in expected {
            let arm = arm_source(body, "AgentTeamCommand", variant).unwrap_or_else(|| {
                panic!("variant {variant} has no dispatch arm in execute_agent_team_command")
            });
            assert!(
                arm.contains(handler),
                "`{variant}` dispatches to the wrong handler: {arm}"
            );
        }
    }

    /// 同上，但针对 `/agents` 这一层。
    #[test]
    fn every_agents_subcommand_reaches_its_own_handler() {
        let body = {
            let start = source()
                .find("pub(crate) async fn execute_agents_command(")
                .expect("execute_agents_command must exist");
            let rest = &source()[start..];
            let end = rest.find("\n}\n").expect("body end");
            &rest[..end]
        };

        let expected = [
            ("List", "custom_agents::list_agents"),
            ("Create", "custom_agents::create_agent"),
            ("Edit", "custom_agents::edit_agent"),
            ("Delete", "custom_agents::delete_agent"),
            ("Add", "custom_agents::add_agent"),
            ("Remove", "custom_agents::remove_agent"),
            ("Team", "execute_agent_team_command"),
        ];

        for (variant, handler) in expected {
            let arm = arm_source(body, "AgentsCommand", variant).unwrap_or_else(|| {
                panic!("variant {variant} has no dispatch arm in execute_agents_command")
            });
            assert!(
                arm.contains(handler),
                "`{variant}` dispatches to the wrong handler: {arm}"
            );
        }
    }
}
