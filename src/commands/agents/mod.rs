mod custom_agents;
mod team_apply;
mod team_definitions;
mod team_execution;
mod team_presets;
mod team_runs;

use crate::commands::execution::{CommandContext, CommandResult};
use crate::commands::agent_team_support::{
    cleanup_team_run_artifacts, collect_apply_conflict_files, load_team_run_record,
    map_target_for_worktree, save_team_run_record, scan_team_run_records, summarize_output,
    team_run_dir, team_run_record_path, team_runs_root, TeamRunMemberRecord, TeamRunRecord,
    TeamRunRoundRecord,
};
use crate::commands::agent_team_presets::{
    list_team_presets, load_team_preset_store, resolve_team_preset, sanitize_preset_name,
    save_team_preset_store, scope_label, team_preset_file_path, TeamPreset, TeamPresetScope,
};
use crate::commands::agent_team_render::{render_team_run_details, render_team_runs_list};
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
    register_custom_subagents, AnalyzerAgent, AutoFixAgent, EditorAgent, NavigatorAgent,
    SearchAgent, SubAgentManager, SubTask, SubTaskResult,
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

pub(crate) async fn execute_agents_command(ctx: CommandContext<'_>, cmd: AgentsCommand) -> CommandResult {
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
        AgentTeamCommand::Remove { name, user } => team_presets::remove_team_preset(ctx, name, user).await,
        AgentTeamCommand::Apply(args) => team_apply::apply_team_run(ctx, args).await,
        AgentTeamCommand::Clean(args) => team_runs::clean_team_runs(ctx, args).await,
    }
}
