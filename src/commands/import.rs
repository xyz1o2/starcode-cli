//! `/import` — bring configuration from other terminal coding agents into
//! StarCode.
//!
//! The command follows the same safety model as Claude Code's `/import`:
//! discovery is non-destructive, every write is previewed unless `--yes` is
//! present, and an existing destination is never silently overwritten. The
//! importer intentionally imports *configuration*, not credentials:
//!
//! - MCP servers from Codex, Gemini CLI and Cursor are merged into the project
//!   `.star/mcp.json`; name conflicts keep the existing StarCode entry.
//! - Global instruction files become managed blocks in `~/.star/STAR.md`;
//!   project instruction files become managed blocks in `.star/STAR.md`.
//! - Prompt/command files become `.star/skills/<name>/SKILL.md` so they show
//!   up in `/skills list` and can be loaded through the existing skill
//!   authoring/loading paths. Skill directories keep their `SKILL.md`
//!   frontmatter and copy supporting files (scripts, references, assets).
//! - Agent definitions become `.star/agents/<name>.md`, the format already
//!   loaded by the custom subagent registry.
//!
//! Codex's project `AGENTS.md` is already read natively by StarCode, so it is
//! reported instead of copied. Managed blocks are replaced by source, which
//! makes repeated imports idempotent without touching hand-written text.

use crate::commands::execution::{CommandContext, CommandResult};
use crate::core::mcp::{WindsurfMcpConfig, WindsurfMcpServer};
use serde_json::Value as JsonValue;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const USAGE: &str = "Usage: /import [codex|gemini|cursor] [--dry-run] [--yes]";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ImportSource {
    Codex,
    Gemini,
    Cursor,
}

impl ImportSource {
    fn key(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Cursor => "cursor",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Gemini => "Gemini CLI",
            Self::Cursor => "Cursor",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw.to_ascii_lowercase().as_str() {
            "codex" => Some(Self::Codex),
            "gemini" | "gemini-cli" => Some(Self::Gemini),
            "cursor" => Some(Self::Cursor),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct ImportOptions {
    sources: BTreeSet<ImportSource>,
    dry_run: bool,
    yes: bool,
}

fn parse_options(args: &[String]) -> Result<ImportOptions, String> {
    let mut options = ImportOptions::default();
    for arg in args {
        match arg.as_str() {
            "--dry-run" | "-n" => options.dry_run = true,
            "--yes" | "-y" => options.yes = true,
            value if value.starts_with('-') => {
                return Err(format!("Unknown option `{value}`.\n{USAGE}"));
            }
            value => {
                let source = ImportSource::parse(value)
                    .ok_or_else(|| format!("Unknown import source `{value}`.\n{USAGE}"))?;
                options.sources.insert(source);
            }
        }
    }
    if options.sources.is_empty() {
        options.sources.extend([
            ImportSource::Codex,
            ImportSource::Gemini,
            ImportSource::Cursor,
        ]);
    }
    Ok(options)
}

#[derive(Debug, Clone)]
struct ImportEnv {
    home: PathBuf,
    project_root: PathBuf,
    codex_home: PathBuf,
}

impl ImportEnv {
    fn from_process() -> Result<Self, String> {
        let home = dirs::home_dir().ok_or("Could not resolve the home directory.")?;
        let project_root = std::env::current_dir()
            .map_err(|error| format!("Could not resolve the working directory: {error}"))?;
        let codex_home = std::env::var_os("CODEX_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"));
        Ok(Self {
            home,
            project_root,
            codex_home,
        })
    }

    fn user_star_path(&self) -> PathBuf {
        self.home.join(".star").join("STAR.md")
    }

    fn project_star_path(&self) -> PathBuf {
        self.project_root.join(".star").join("STAR.md")
    }

    fn mcp_path(&self) -> PathBuf {
        self.project_root.join(".star").join("mcp.json")
    }

    fn user_skills_dir(&self) -> PathBuf {
        self.home.join(".star").join("skills")
    }

    fn project_skills_dir(&self) -> PathBuf {
        self.project_root.join(".star").join("skills")
    }

    fn user_agents_dir(&self) -> PathBuf {
        self.home.join(".star").join("agents")
    }

    fn project_agents_dir(&self) -> PathBuf {
        self.project_root.join(".star").join("agents")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ImportScope {
    User,
    Project,
}

#[derive(Debug, Clone)]
struct InstructionDraft {
    source: ImportSource,
    origin: PathBuf,
    destination: PathBuf,
    body: String,
}

#[derive(Debug, Clone)]
struct SkillDraft {
    source: ImportSource,
    origin: PathBuf,
    destination: PathBuf,
    content: String,
    assets: Vec<SkillAsset>,
}

#[derive(Debug, Clone)]
struct SkillAsset {
    relative_path: PathBuf,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
struct AgentDraft {
    source: ImportSource,
    origin: PathBuf,
    destination: PathBuf,
    content: String,
}

#[derive(Debug, Clone)]
struct McpDraft {
    source: ImportSource,
    origin: PathBuf,
    name: String,
    server: WindsurfMcpServer,
}

#[derive(Debug, Default)]
struct ImportPlan {
    instructions: Vec<InstructionDraft>,
    skills: Vec<SkillDraft>,
    agents: Vec<AgentDraft>,
    mcp: Vec<McpDraft>,
    warnings: Vec<String>,
}

impl ImportPlan {
    fn is_empty(&self) -> bool {
        self.instructions.is_empty()
            && self.skills.is_empty()
            && self.agents.is_empty()
            && self.mcp.is_empty()
    }

    fn push_instruction(&mut self, draft: InstructionDraft) {
        if let Some(existing) = self
            .instructions
            .iter_mut()
            .find(|item| item.destination == draft.destination && item.source == draft.source)
        {
            if !existing.body.ends_with('\n') {
                existing.body.push('\n');
            }
            existing.body.push('\n');
            existing.body.push_str(draft.body.trim());
            existing.body.push('\n');
        } else {
            self.instructions.push(draft);
        }
    }

    fn push_skill(&mut self, draft: SkillDraft) {
        if let Some(existing) = self
            .skills
            .iter()
            .find(|item| item.destination == draft.destination)
        {
            self.warnings.push(format!(
                "Skipped duplicate skill `{}` from `{}`; `{}` already provides it.",
                draft.destination.display(),
                draft.origin.display(),
                existing.origin.display()
            ));
        } else {
            self.skills.push(draft);
        }
    }

    fn push_agent(&mut self, draft: AgentDraft) {
        if let Some(existing) = self
            .agents
            .iter()
            .find(|item| item.destination == draft.destination)
        {
            self.warnings.push(format!(
                "Skipped duplicate agent `{}` from `{}`; `{}` already provides it.",
                draft.destination.display(),
                draft.origin.display(),
                existing.origin.display()
            ));
        } else {
            self.agents.push(draft);
        }
    }
}

fn read_text(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)
        .map_err(|error| format!("Could not read `{}`: {error}", path.display()))?;
    if content.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(content))
    }
}

fn read_optional(path: &Path, plan: &mut ImportPlan) -> Option<String> {
    match read_text(path) {
        Ok(text) => text,
        Err(error) => {
            plan.warnings.push(error);
            None
        }
    }
}

fn sorted_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            files.push(path);
        }
    }
    files.sort();
    files
}

fn build_plan(env: &ImportEnv, sources: &BTreeSet<ImportSource>) -> ImportPlan {
    let mut plan = ImportPlan::default();
    for source in sources {
        match source {
            ImportSource::Codex => collect_codex(env, &mut plan),
            ImportSource::Gemini => collect_gemini(env, &mut plan),
            ImportSource::Cursor => collect_cursor(env, &mut plan),
        }
    }
    plan.skills
        .sort_by(|a, b| a.destination.cmp(&b.destination));
    plan.agents
        .sort_by(|a, b| a.destination.cmp(&b.destination));
    plan.instructions
        .sort_by(|a, b| a.destination.cmp(&b.destination));
    plan
}

fn collect_codex(env: &ImportEnv, plan: &mut ImportPlan) {
    let config = env.codex_home.join("config.toml");
    if let Some(content) = read_optional(&config, plan) {
        match parse_codex_mcp(&content) {
            Ok(servers) => {
                for (name, server) in servers {
                    plan.mcp.push(McpDraft {
                        source: ImportSource::Codex,
                        origin: config.clone(),
                        name,
                        server,
                    });
                }
            }
            Err(error) => plan.warnings.push(format!(
                "Could not parse Codex MCP config `{}`: {error}",
                config.display()
            )),
        }
    }

    let global_agents = env.codex_home.join("AGENTS.md");
    if let Some(content) = read_optional(&global_agents, plan) {
        plan.push_instruction(InstructionDraft {
            source: ImportSource::Codex,
            origin: global_agents.clone(),
            destination: env.user_star_path(),
            body: format!(
                "## Imported from Codex\n\nSource: `{}`\n\n{}",
                global_agents.display(),
                content.trim()
            ),
        });
    }

    let project_agents = env.project_root.join("AGENTS.md");
    if project_agents.exists() {
        plan.warnings.push(
            "Codex project `AGENTS.md` is already read natively by StarCode; left in place."
                .to_string(),
        );
    }

    collect_markdown_skills(
        &env.codex_home.join("prompts"),
        ImportSource::Codex,
        ImportScope::User,
        env,
        plan,
    );
    collect_markdown_skills(
        &env.project_root.join(".codex").join("prompts"),
        ImportSource::Codex,
        ImportScope::Project,
        env,
        plan,
    );
    collect_agent_files(
        &env.codex_home.join("agents"),
        ImportSource::Codex,
        ImportScope::User,
        env,
        plan,
    );
    collect_agent_files(
        &env.project_root.join(".codex").join("agents"),
        ImportSource::Codex,
        ImportScope::Project,
        env,
        plan,
    );
    collect_skill_directories(
        &env.codex_home.join("skills"),
        ImportSource::Codex,
        ImportScope::User,
        env,
        plan,
    );
    collect_skill_directories(
        &env.project_root.join(".codex").join("skills"),
        ImportSource::Codex,
        ImportScope::Project,
        env,
        plan,
    );
}

fn collect_gemini(env: &ImportEnv, plan: &mut ImportPlan) {
    let global_home = env.home.join(".gemini");
    for settings in [
        global_home.join("settings.json"),
        env.project_root.join(".gemini").join("settings.json"),
    ] {
        if let Some(content) = read_optional(&settings, plan) {
            match parse_json_mcp(&content) {
                Ok(servers) => {
                    for (name, server) in servers {
                        plan.mcp.push(McpDraft {
                            source: ImportSource::Gemini,
                            origin: settings.clone(),
                            name,
                            server,
                        });
                    }
                }
                Err(error) => plan.warnings.push(format!(
                    "Could not parse Gemini MCP config `{}`: {error}",
                    settings.display()
                )),
            }
        }
    }

    for (path, destination) in [
        (global_home.join("GEMINI.md"), env.user_star_path()),
        (env.project_root.join("GEMINI.md"), env.project_star_path()),
        (
            env.project_root.join(".gemini").join("GEMINI.md"),
            env.project_star_path(),
        ),
    ] {
        if let Some(content) = read_optional(&path, plan) {
            plan.push_instruction(InstructionDraft {
                source: ImportSource::Gemini,
                origin: path.clone(),
                destination,
                body: format!(
                    "## Imported from Gemini CLI\n\nSource: `{}`\n\n{}",
                    path.display(),
                    content.trim()
                ),
            });
        }
    }

    collect_gemini_commands(&global_home.join("commands"), ImportScope::User, env, plan);
    collect_gemini_commands(
        &env.project_root.join(".gemini").join("commands"),
        ImportScope::Project,
        env,
        plan,
    );
    for (dir, scope) in [
        (global_home.join("agents"), ImportScope::User),
        (
            env.project_root.join(".gemini").join("agents"),
            ImportScope::Project,
        ),
    ] {
        collect_agent_files(&dir, ImportSource::Gemini, scope, env, plan);
    }
    for (dir, scope) in [
        (global_home.join("skills"), ImportScope::User),
        (
            env.project_root.join(".gemini").join("skills"),
            ImportScope::Project,
        ),
    ] {
        collect_skill_directories(&dir, ImportSource::Gemini, scope, env, plan);
    }
}

fn collect_cursor(env: &ImportEnv, plan: &mut ImportPlan) {
    for path in [
        env.home.join(".cursor").join("mcp.json"),
        env.project_root.join(".cursor").join("mcp.json"),
        env.project_root.join(".mcp.json"),
    ] {
        if let Some(content) = read_optional(&path, plan) {
            match parse_json_mcp(&content) {
                Ok(servers) => {
                    for (name, server) in servers {
                        plan.mcp.push(McpDraft {
                            source: ImportSource::Cursor,
                            origin: path.clone(),
                            name,
                            server,
                        });
                    }
                }
                Err(error) => plan.warnings.push(format!(
                    "Could not parse Cursor MCP config `{}`: {error}",
                    path.display()
                )),
            }
        }
    }

    let mut rules = String::new();
    let legacy = env.project_root.join(".cursorrules");
    if let Some(content) = read_optional(&legacy, plan) {
        rules.push_str(&format!(
            "### Legacy `.cursorrules`\n\n{}\n\n",
            strip_frontmatter(&content).trim()
        ));
    }

    let rules_dir = env.project_root.join(".cursor").join("rules");
    let mut rule_files: Vec<PathBuf> = WalkDir::new(&rules_dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| matches!(extension, "md" | "mdc"))
                .unwrap_or(false)
        })
        .collect();
    rule_files.sort();
    for path in rule_files {
        if let Some(content) = read_optional(&path, plan) {
            let relative = path.strip_prefix(&env.project_root).unwrap_or(&path);
            rules.push_str(&format!(
                "### `{}`\n\n{}\n\n",
                relative.display(),
                strip_frontmatter(&content).trim()
            ));
        }
    }
    if !rules.trim().is_empty() {
        plan.push_instruction(InstructionDraft {
            source: ImportSource::Cursor,
            origin: env.project_root.join(".cursor"),
            destination: env.project_star_path(),
            body: format!("## Imported from Cursor\n\n{}", rules.trim()),
        });
    }

    collect_markdown_skills(
        &env.home.join(".cursor").join("commands"),
        ImportSource::Cursor,
        ImportScope::User,
        env,
        plan,
    );
    collect_markdown_skills(
        &env.project_root.join(".cursor").join("commands"),
        ImportSource::Cursor,
        ImportScope::Project,
        env,
        plan,
    );
    for (dir, scope) in [
        (env.home.join(".cursor").join("agents"), ImportScope::User),
        (
            env.project_root.join(".cursor").join("agents"),
            ImportScope::Project,
        ),
    ] {
        collect_agent_files(&dir, ImportSource::Cursor, scope, env, plan);
    }
    for (dir, scope) in [
        (env.home.join(".cursor").join("skills"), ImportScope::User),
        (
            env.project_root.join(".cursor").join("skills"),
            ImportScope::Project,
        ),
    ] {
        collect_skill_directories(&dir, ImportSource::Cursor, scope, env, plan);
    }
}

fn collect_markdown_skills(
    dir: &Path,
    source: ImportSource,
    scope: ImportScope,
    env: &ImportEnv,
    plan: &mut ImportPlan,
) {
    for path in sorted_files(dir) {
        if path.extension().and_then(|value| value.to_str()) != Some("md") {
            continue;
        }
        let Some(content) = read_optional(&path, plan) else {
            continue;
        };
        if let Some(draft) = skill_from_markdown(&path, &content, source, scope, env, plan) {
            plan.push_skill(draft);
        }
    }
}

fn collect_gemini_commands(dir: &Path, scope: ImportScope, env: &ImportEnv, plan: &mut ImportPlan) {
    for path in sorted_files(dir) {
        if path.extension().and_then(|value| value.to_str()) != Some("toml") {
            continue;
        }
        let Some(content) = read_optional(&path, plan) else {
            continue;
        };
        if let Some(draft) = skill_from_gemini_command(&path, &content, scope, env, plan) {
            plan.push_skill(draft);
        }
    }
}

fn sorted_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return dirs;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        }
    }
    dirs.sort();
    dirs
}

fn agent_destination(env: &ImportEnv, scope: ImportScope, name: &str) -> PathBuf {
    let base = match scope {
        ImportScope::User => env.user_agents_dir(),
        ImportScope::Project => env.project_agents_dir(),
    };
    base.join(format!("{name}.md"))
}

fn collect_agent_files(
    dir: &Path,
    source: ImportSource,
    scope: ImportScope,
    env: &ImportEnv,
    plan: &mut ImportPlan,
) {
    for path in sorted_files(dir) {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let Some(content) = read_optional(&path, plan) else {
            continue;
        };
        let draft = match extension {
            "md" | "markdown" => agent_from_markdown(&path, &content, source, scope, env, plan),
            "toml" => agent_from_codex_toml(&path, &content, source, scope, env, plan),
            _ => None,
        };
        if let Some(draft) = draft {
            plan.push_agent(draft);
        }
    }
}

fn agent_from_markdown(
    path: &Path,
    content: &str,
    source: ImportSource,
    scope: ImportScope,
    env: &ImportEnv,
    plan: &mut ImportPlan,
) -> Option<AgentDraft> {
    let (metadata, body) = crate::agent::skills::loader::SkillLoader::parse_frontmatter(content);
    let stem = path.file_stem()?.to_string_lossy().to_string();
    let raw_name = if metadata.name.trim().is_empty() {
        stem
    } else {
        metadata.name
    };
    let name = crate::commands::skills::slugify(&raw_name);
    if name.is_empty() {
        plan.warnings.push(format!(
            "Skipped agent `{}`: it did not produce a safe name.",
            path.display()
        ));
        return None;
    }
    if body.trim().is_empty() {
        plan.warnings.push(format!(
            "Skipped agent `{}`: the prompt body is empty.",
            path.display()
        ));
        return None;
    }
    let description = if metadata.description.trim().is_empty() {
        format!("Imported from {}: {}", source.label(), path.display())
    } else {
        metadata.description
    };
    let mut tools = metadata.allowed_tools;
    for tool in frontmatter_tools(content) {
        if !tools.iter().any(|existing| existing == &tool) {
            tools.push(tool);
        }
    }
    Some(AgentDraft {
        source,
        origin: path.to_path_buf(),
        destination: agent_destination(env, scope, &name),
        content: render_agent(
            &name,
            &description,
            metadata.model.as_deref(),
            &tools,
            &body,
        ),
    })
}

fn frontmatter_tools(content: &str) -> Vec<String> {
    let normalized = content.replace("\r\n", "\n");
    let Some(rest) = normalized.strip_prefix("---\n") else {
        return Vec::new();
    };
    let Some(end) = rest.find("\n---") else {
        return Vec::new();
    };
    let metadata = &rest[..end];
    let mut tools = Vec::new();
    let mut reading_tools = false;
    for line in metadata.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed
            .strip_prefix("tools:")
            .or_else(|| trimmed.strip_prefix("tool:"))
        {
            tools.extend(parse_inline_list(value));
            reading_tools = true;
            continue;
        }
        if matches!(trimmed, "allowed-tools:" | "allowed_tools:") {
            reading_tools = true;
            continue;
        }
        if reading_tools {
            if let Some(item) = trimmed.strip_prefix("- ") {
                let item = item.trim().trim_matches('"').trim_matches('\'');
                if !item.is_empty() {
                    tools.push(item.to_string());
                }
            } else if !line.starts_with(' ') && !line.starts_with('\t') {
                reading_tools = false;
            }
        }
    }
    tools
}

fn parse_inline_list(value: &str) -> Vec<String> {
    value
        .trim()
        .trim_matches('[')
        .trim_matches(']')
        .split(',')
        .map(str::trim)
        .map(|item| item.trim_matches('"').trim_matches('\''))
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

fn agent_from_codex_toml(
    path: &Path,
    content: &str,
    source: ImportSource,
    scope: ImportScope,
    env: &ImportEnv,
    plan: &mut ImportPlan,
) -> Option<AgentDraft> {
    let parsed: toml::Value = match toml::from_str(content) {
        Ok(value) => value,
        Err(error) => {
            plan.warnings.push(format!(
                "Could not parse agent `{}`: {error}",
                path.display()
            ));
            return None;
        }
    };
    let table = parsed.as_table()?;
    let stem = path.file_stem()?.to_string_lossy().to_string();
    let raw_name = table
        .get("name")
        .and_then(toml::Value::as_str)
        .unwrap_or(stem.as_str());
    let name = crate::commands::skills::slugify(raw_name);
    if name.is_empty() {
        plan.warnings.push(format!(
            "Skipped agent `{}`: it did not produce a safe name.",
            path.display()
        ));
        return None;
    }
    let body = ["developer_instructions", "instructions", "prompt"]
        .iter()
        .find_map(|key| table.get(*key).and_then(toml::Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(body) = body else {
        plan.warnings.push(format!(
            "Skipped agent `{}`: no developer instructions were found.",
            path.display()
        ));
        return None;
    };
    let description = table
        .get("description")
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("Imported from {}: {}", source.label(), path.display()));
    let model = table.get("model").and_then(toml::Value::as_str);
    let tools = table
        .get("tools")
        .or_else(|| table.get("allowed_tools"))
        .map(toml_string_list)
        .unwrap_or_default();
    Some(AgentDraft {
        source,
        origin: path.to_path_buf(),
        destination: agent_destination(env, scope, &name),
        content: render_agent(&name, &description, model, &tools, body),
    })
}

fn toml_string_list(value: &toml::Value) -> Vec<String> {
    if let Some(items) = value.as_array() {
        return items
            .iter()
            .filter_map(toml::Value::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect();
    }
    value
        .as_str()
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn render_agent(
    name: &str,
    description: &str,
    model: Option<&str>,
    tools: &[String],
    body: &str,
) -> String {
    let mut output = format!(
        "---\nname: {name}\ndescription: {}\n",
        single_line(description)
    );
    if let Some(model) = model.map(single_line).filter(|value| !value.is_empty()) {
        output.push_str(&format!("model: {model}\n"));
    }
    let tools = tools
        .iter()
        .map(|tool| single_line(tool))
        .filter(|tool| !tool.is_empty())
        .collect::<Vec<_>>();
    if !tools.is_empty() {
        output.push_str(&format!("tools: {}\n", tools.join(", ")));
    }
    output.push_str("---\n\n");
    output.push_str(body.trim());
    output.push('\n');
    output
}

fn collect_skill_directories(
    dir: &Path,
    source: ImportSource,
    scope: ImportScope,
    env: &ImportEnv,
    plan: &mut ImportPlan,
) {
    for skill_dir in sorted_dirs(dir) {
        let path = skill_dir.join("SKILL.md");
        if !path.is_file() {
            continue;
        }
        let Some(content) = read_optional(&path, plan) else {
            continue;
        };
        if let Some(draft) =
            skill_from_skill_directory(&skill_dir, &path, &content, source, scope, env, plan)
        {
            plan.push_skill(draft);
        }
    }
    for path in sorted_files(dir) {
        if !path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.ends_with(".skill.md"))
            .unwrap_or(false)
        {
            continue;
        }
        let Some(content) = read_optional(&path, plan) else {
            continue;
        };
        if let Some(draft) = skill_from_skill_file(&path, &content, source, scope, env, plan) {
            plan.push_skill(draft);
        }
    }
}

fn skill_from_skill_directory(
    skill_dir: &Path,
    path: &Path,
    content: &str,
    source: ImportSource,
    scope: ImportScope,
    env: &ImportEnv,
    plan: &mut ImportPlan,
) -> Option<SkillDraft> {
    let (metadata, body) = crate::agent::skills::loader::SkillLoader::parse_frontmatter(content);
    let raw_name = if metadata.name.trim().is_empty() {
        skill_dir.file_name()?.to_string_lossy().to_string()
    } else {
        metadata.name
    };
    let name = crate::commands::skills::slugify(&raw_name);
    if name.is_empty() {
        plan.warnings.push(format!(
            "Skipped skill `{}`: it did not produce a safe name.",
            path.display()
        ));
        return None;
    }
    let description = if metadata.description.trim().is_empty() {
        infer_description(&body, &name, source, path)
    } else {
        metadata.description
    };
    Some(SkillDraft {
        source,
        origin: path.to_path_buf(),
        destination: skill_destination(env, scope, &name),
        content: ensure_skill_metadata(content, &name, &description),
        assets: collect_skill_assets(skill_dir, path, plan),
    })
}

fn skill_from_skill_file(
    path: &Path,
    content: &str,
    source: ImportSource,
    scope: ImportScope,
    env: &ImportEnv,
    plan: &mut ImportPlan,
) -> Option<SkillDraft> {
    let (metadata, body) = crate::agent::skills::loader::SkillLoader::parse_frontmatter(content);
    let stem = path
        .file_name()?
        .to_string_lossy()
        .strip_suffix(".skill.md")
        .unwrap_or_else(|| {
            path.file_stem()
                .unwrap_or_default()
                .to_str()
                .unwrap_or_default()
        })
        .to_string();
    let raw_name = if metadata.name.trim().is_empty() {
        stem
    } else {
        metadata.name
    };
    let name = crate::commands::skills::slugify(&raw_name);
    if name.is_empty() {
        plan.warnings.push(format!(
            "Skipped skill `{}`: it did not produce a safe name.",
            path.display()
        ));
        return None;
    }
    let description = if metadata.description.trim().is_empty() {
        infer_description(&body, &name, source, path)
    } else {
        metadata.description
    };
    Some(SkillDraft {
        source,
        origin: path.to_path_buf(),
        destination: skill_destination(env, scope, &name),
        content: ensure_skill_metadata(content, &name, &description),
        assets: Vec::new(),
    })
}

fn infer_description(body: &str, name: &str, source: ImportSource, path: &Path) -> String {
    body.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.trim_start_matches('#').trim().to_string())
        .filter(|line| !line.is_empty())
        .unwrap_or_else(|| {
            format!(
                "Imported {name} from {}: {}",
                source.label(),
                path.display()
            )
        })
}

fn ensure_skill_metadata(content: &str, name: &str, description: &str) -> String {
    let normalized = content.replace("\r\n", "\n");
    let description = single_line(description);
    if let Some(rest) = normalized.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---") {
            let metadata = &rest[..end];
            let body = rest[end + 4..]
                .strip_prefix('\n')
                .unwrap_or(&rest[end + 4..]);
            let mut lines = metadata.lines().map(str::to_string).collect::<Vec<_>>();
            set_metadata_line(&mut lines, "name", name);
            set_metadata_line(&mut lines, "description", &description);
            return format!(
                "---\n{}\n---\n\n{}\n",
                lines.join("\n"),
                body.trim_matches('\n')
            );
        }
    }
    format!(
        "---\nname: {name}\ndescription: {description}\n---\n\n{}\n",
        normalized.trim()
    )
}

fn set_metadata_line(lines: &mut Vec<String>, key: &str, value: &str) {
    let prefix = format!("{key}:");
    let mut replaced = false;
    let mut output = Vec::with_capacity(lines.len() + 1);
    for line in lines.drain(..) {
        if line.trim_start().starts_with(&prefix) {
            if !replaced {
                output.push(format!("{key}: {value}"));
                replaced = true;
            }
        } else {
            output.push(line);
        }
    }
    if !replaced {
        output.push(format!("{key}: {value}"));
    }
    *lines = output;
}

fn collect_skill_assets(
    skill_dir: &Path,
    skill_file: &Path,
    plan: &mut ImportPlan,
) -> Vec<SkillAsset> {
    const MAX_TOTAL_BYTES: u64 = 10 * 1024 * 1024;
    let mut assets = Vec::new();
    let mut total = 0_u64;
    for entry in WalkDir::new(skill_dir).into_iter().filter_map(Result::ok) {
        let path = entry.path();
        if path == skill_file || !entry.file_type().is_file() {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                plan.warnings.push(format!(
                    "Could not inspect skill asset `{}`: {error}",
                    path.display()
                ));
                continue;
            }
        };
        if total.saturating_add(metadata.len()) > MAX_TOTAL_BYTES {
            plan.warnings.push(format!(
                "Skipped the remaining assets in `{}`: the skill exceeds the {} MiB import limit.",
                skill_dir.display(),
                MAX_TOTAL_BYTES / (1024 * 1024)
            ));
            break;
        }
        let relative_path = match path.strip_prefix(skill_dir) {
            Ok(relative)
                if relative
                    .components()
                    .all(|component| matches!(component, std::path::Component::Normal(_))) =>
            {
                relative.to_path_buf()
            }
            _ => continue,
        };
        match std::fs::read(path) {
            Ok(bytes) => {
                total = total.saturating_add(metadata.len());
                assets.push(SkillAsset {
                    relative_path,
                    bytes,
                });
            }
            Err(error) => plan.warnings.push(format!(
                "Could not read skill asset `{}`: {error}",
                path.display()
            )),
        }
    }
    assets.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    assets
}

fn single_line(value: &str) -> String {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn skill_destination(env: &ImportEnv, scope: ImportScope, name: &str) -> PathBuf {
    let base = match scope {
        ImportScope::User => env.user_skills_dir(),
        ImportScope::Project => env.project_skills_dir(),
    };
    base.join(name).join("SKILL.md")
}

fn skill_from_markdown(
    path: &Path,
    content: &str,
    source: ImportSource,
    scope: ImportScope,
    env: &ImportEnv,
    plan: &mut ImportPlan,
) -> Option<SkillDraft> {
    let (metadata, body) = crate::agent::skills::loader::SkillLoader::parse_frontmatter(content);
    let stem = path.file_stem()?.to_string_lossy().to_string();
    let raw_name = if metadata.name.trim().is_empty() {
        stem
    } else {
        metadata.name
    };
    let name = crate::commands::skills::slugify(&raw_name);
    if name.is_empty() {
        plan.warnings.push(format!(
            "Skipped `{}`: it did not produce a safe skill name.",
            path.display()
        ));
        return None;
    }
    let description = if metadata.description.trim().is_empty() {
        format!("Imported from {}: {}", source.label(), path.display())
    } else {
        metadata.description
    };
    Some(SkillDraft {
        source,
        origin: path.to_path_buf(),
        destination: skill_destination(env, scope, &name),
        content: render_skill(&name, &description, &body),
        assets: Vec::new(),
    })
}

fn skill_from_gemini_command(
    path: &Path,
    content: &str,
    scope: ImportScope,
    env: &ImportEnv,
    plan: &mut ImportPlan,
) -> Option<SkillDraft> {
    let value: toml::Value = match toml::from_str(content) {
        Ok(value) => value,
        Err(error) => {
            plan.warnings.push(format!(
                "Could not parse Gemini command `{}`: {error}",
                path.display()
            ));
            return None;
        }
    };
    let prompt = value.get("prompt").and_then(toml::Value::as_str)?;
    let description = value
        .get("description")
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!(
                "Imported from Gemini CLI: {}",
                path.file_stem()
                    .map(|stem| stem.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string())
            )
        });
    let raw_name = path.file_stem()?.to_string_lossy().to_string();
    let name = crate::commands::skills::slugify(&raw_name);
    if name.is_empty() {
        plan.warnings.push(format!(
            "Skipped `{}`: it did not produce a safe skill name.",
            path.display()
        ));
        return None;
    }
    Some(SkillDraft {
        source: ImportSource::Gemini,
        origin: path.to_path_buf(),
        destination: skill_destination(env, scope, &name),
        content: render_skill(&name, &description, prompt),
        assets: Vec::new(),
    })
}

fn render_skill(name: &str, description: &str, body: &str) -> String {
    let description = single_line(description);
    let body = body.replace("$ARGUMENTS", "$1").replace("{{args}}", "$1");
    format!(
        "---\nname: {name}\ndescription: {}\nversion: 1.0.0\n---\n\n{}\n",
        if description.is_empty() {
            "Imported skill"
        } else {
            description.as_str()
        },
        body.trim()
    )
}

fn strip_frontmatter(content: &str) -> String {
    let normalized = content.replace("\r\n", "\n");
    let Some(rest) = normalized.strip_prefix("---\n") else {
        return normalized;
    };
    let Some(end) = rest.find("\n---") else {
        return normalized;
    };
    rest[end + 4..]
        .strip_prefix('\n')
        .unwrap_or(&rest[end + 4..])
        .to_string()
}

fn parse_codex_mcp(content: &str) -> Result<Vec<(String, WindsurfMcpServer)>, String> {
    let parsed: toml::Value = toml::from_str(content).map_err(|error| error.to_string())?;
    let Some(servers) = parsed.get("mcp_servers").and_then(toml::Value::as_table) else {
        return Ok(Vec::new());
    };
    let mut result = Vec::new();
    for (name, value) in servers {
        let Some(table) = value.as_table() else {
            continue;
        };
        let command = table
            .get("command")
            .and_then(toml::Value::as_str)
            .map(str::to_string);
        let args = table
            .get("args")
            .and_then(toml::Value::as_array)
            .map(|args| {
                args.iter()
                    .filter_map(toml::Value::as_str)
                    .map(str::to_string)
                    .collect()
            });
        let env = table
            .get("env")
            .and_then(toml::Value::as_table)
            .map(toml_string_map);
        let url = table
            .get("url")
            .and_then(toml::Value::as_str)
            .map(str::to_string);
        let disabled = table
            .get("disabled")
            .and_then(toml::Value::as_bool)
            .or_else(|| {
                table
                    .get("enabled")
                    .and_then(toml::Value::as_bool)
                    .map(|enabled| !enabled)
            });
        let transport_type = table
            .get("type")
            .or_else(|| table.get("transport"))
            .and_then(toml::Value::as_str)
            .map(str::to_string);
        if command.is_none() && url.is_none() {
            continue;
        }
        result.push((
            name.clone(),
            WindsurfMcpServer {
                command,
                args,
                env,
                disabled,
                transport_type,
                url,
            },
        ));
    }
    result.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(result)
}

fn toml_string_map(table: &toml::map::Map<String, toml::Value>) -> HashMap<String, String> {
    table
        .iter()
        .filter_map(|(key, value)| {
            let value = match value {
                toml::Value::String(value) => value.clone(),
                toml::Value::Integer(value) => value.to_string(),
                toml::Value::Float(value) => value.to_string(),
                toml::Value::Boolean(value) => value.to_string(),
                _ => return None,
            };
            Some((key.clone(), value))
        })
        .collect()
}

fn parse_json_mcp(content: &str) -> Result<Vec<(String, WindsurfMcpServer)>, String> {
    let parsed: JsonValue = serde_json::from_str(content).map_err(|error| error.to_string())?;
    let Some(root) = parsed.as_object() else {
        return Err("the top-level JSON value must be an object".to_string());
    };
    let servers = root
        .get("mcpServers")
        .or_else(|| root.get("mcp_servers"))
        .and_then(JsonValue::as_object);
    let Some(servers) = servers else {
        return Ok(Vec::new());
    };

    let mut result = Vec::new();
    for (name, value) in servers {
        let Some(server) = value.as_object() else {
            continue;
        };
        let command = server
            .get("command")
            .and_then(JsonValue::as_str)
            .map(str::to_string);
        let args = server
            .get("args")
            .and_then(JsonValue::as_array)
            .map(|args| {
                args.iter()
                    .filter_map(JsonValue::as_str)
                    .map(str::to_string)
                    .collect()
            });
        let env = server.get("env").and_then(JsonValue::as_object).map(|env| {
            env.iter()
                .filter_map(|(key, value)| {
                    let value = match value {
                        JsonValue::String(value) => value.clone(),
                        JsonValue::Number(value) => value.to_string(),
                        JsonValue::Bool(value) => value.to_string(),
                        _ => return None,
                    };
                    Some((key.clone(), value))
                })
                .collect()
        });
        let url = server
            .get("url")
            .or_else(|| server.get("httpUrl"))
            .and_then(JsonValue::as_str)
            .map(str::to_string);
        let disabled = server
            .get("disabled")
            .and_then(JsonValue::as_bool)
            .or_else(|| {
                server
                    .get("enabled")
                    .and_then(JsonValue::as_bool)
                    .map(|enabled| !enabled)
            });
        let transport_type = server
            .get("type")
            .or_else(|| server.get("transport"))
            .or_else(|| server.get("transport_type"))
            .and_then(JsonValue::as_str)
            .map(str::to_string);
        if command.is_none() && url.is_none() {
            continue;
        }
        result.push((
            name.clone(),
            WindsurfMcpServer {
                command,
                args,
                env,
                disabled,
                transport_type,
                url,
            },
        ));
    }
    result.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(result)
}

#[derive(Debug, Default)]
struct ApplyReport {
    instructions_written: Vec<PathBuf>,
    instructions_unchanged: usize,
    skills_written: Vec<PathBuf>,
    skill_assets_written: usize,
    skills_unchanged: usize,
    skills_skipped: Vec<String>,
    agents_written: Vec<PathBuf>,
    agents_unchanged: usize,
    agents_skipped: Vec<String>,
    mcp_added: Vec<String>,
    mcp_unchanged: usize,
    mcp_conflicts: Vec<String>,
    warnings: Vec<String>,
}

fn managed_begin(source: ImportSource) -> String {
    format!("<!-- starcode-import:begin source={} -->", source.key())
}

fn managed_end(source: ImportSource) -> String {
    format!("<!-- starcode-import:end source={} -->", source.key())
}

fn upsert_managed_block(existing: &str, source: ImportSource, body: &str) -> String {
    let begin = managed_begin(source);
    let end = managed_end(source);
    let sanitized = body.replace("<!-- starcode-import:", "<!-- starcode-import-escaped:");
    let block = format!("{begin}\n{}\n{end}", sanitized.trim());

    if let Some(start) = existing.find(&begin) {
        if let Some(relative_end) = existing[start..].find(&end) {
            let end_index = start + relative_end + end.len();
            let mut output = String::with_capacity(existing.len() + block.len());
            output.push_str(&existing[..start]);
            output.push_str(&block);
            output.push_str(&existing[end_index..]);
            return output;
        }
    }

    let mut output = existing.trim_end().to_string();
    if !output.is_empty() {
        output.push_str("\n\n");
    }
    output.push_str(&block);
    output.push('\n');
    output
}

fn apply_plan(plan: &ImportPlan, env: &ImportEnv) -> Result<ApplyReport, String> {
    let mut report = ApplyReport::default();
    report.warnings.extend(plan.warnings.iter().cloned());
    apply_instructions(plan, &mut report)?;
    apply_skills(plan, &mut report)?;
    apply_agents(plan, &mut report)?;
    apply_mcp(plan, env, &mut report)?;
    Ok(report)
}

fn apply_instructions(plan: &ImportPlan, report: &mut ApplyReport) -> Result<(), String> {
    let mut by_destination: BTreeMap<&Path, Vec<&InstructionDraft>> = BTreeMap::new();
    for draft in &plan.instructions {
        by_destination
            .entry(draft.destination.as_path())
            .or_default()
            .push(draft);
    }

    for (destination, drafts) in by_destination {
        let existing = read_text(destination)?.unwrap_or_default();
        let mut updated = existing.clone();
        for draft in drafts {
            updated = upsert_managed_block(&updated, draft.source, &draft.body);
        }
        if updated == existing {
            report.instructions_unchanged += 1;
            continue;
        }
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Could not create `{}` for imported instructions: {error}",
                    parent.display()
                )
            })?;
        }
        std::fs::write(destination, &updated).map_err(|error| {
            format!(
                "Could not write imported instructions to `{}`: {error}",
                destination.display()
            )
        })?;
        report.instructions_written.push(destination.to_path_buf());
    }
    Ok(())
}

fn apply_skills(plan: &ImportPlan, report: &mut ApplyReport) -> Result<(), String> {
    for draft in &plan.skills {
        let current = read_text(&draft.destination)?;
        if let Some(current) = current {
            if current == draft.content {
                report.skills_unchanged += 1;
            } else {
                report.skills_skipped.push(format!(
                    "`{}` already exists from `{}`; kept the existing file.",
                    draft.destination.display(),
                    draft.origin.display()
                ));
            }
            continue;
        }
        if let Some(parent) = draft.destination.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Could not create `{}` for imported skill: {error}",
                    parent.display()
                )
            })?;
        }
        std::fs::write(&draft.destination, &draft.content).map_err(|error| {
            format!(
                "Could not write imported skill `{}`: {error}",
                draft.destination.display()
            )
        })?;
        report.skills_written.push(draft.destination.clone());
        copy_skill_assets(draft, report)?;
    }
    Ok(())
}

fn copy_skill_assets(draft: &SkillDraft, report: &mut ApplyReport) -> Result<(), String> {
    let Some(skill_dir) = draft.destination.parent() else {
        return Ok(());
    };
    for asset in &draft.assets {
        let path = skill_dir.join(&asset.relative_path);
        if path.exists() {
            report.warnings.push(format!(
                "Kept existing skill asset `{}` from `{}`.",
                path.display(),
                draft.origin.display()
            ));
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Could not create `{}` for an imported skill asset: {error}",
                    parent.display()
                )
            })?;
        }
        std::fs::write(&path, &asset.bytes).map_err(|error| {
            format!(
                "Could not write imported skill asset `{}`: {error}",
                path.display()
            )
        })?;
        report.skill_assets_written += 1;
    }
    Ok(())
}

fn apply_agents(plan: &ImportPlan, report: &mut ApplyReport) -> Result<(), String> {
    for draft in &plan.agents {
        let current = read_text(&draft.destination)?;
        if let Some(current) = current {
            if current == draft.content {
                report.agents_unchanged += 1;
            } else {
                report.agents_skipped.push(format!(
                    "`{}` already exists from `{}`; kept the existing file.",
                    draft.destination.display(),
                    draft.origin.display()
                ));
            }
            continue;
        }
        if let Some(parent) = draft.destination.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Could not create `{}` for imported agent: {error}",
                    parent.display()
                )
            })?;
        }
        std::fs::write(&draft.destination, &draft.content).map_err(|error| {
            format!(
                "Could not write imported agent `{}`: {error}",
                draft.destination.display()
            )
        })?;
        report.agents_written.push(draft.destination.clone());
    }
    Ok(())
}

fn load_mcp_config(path: &Path) -> Result<WindsurfMcpConfig, String> {
    let Some(content) = read_text(path)? else {
        return Ok(WindsurfMcpConfig::default());
    };
    crate::core::config::json_with_comments::parse_json_with_comments(&content)
        .map_err(|error| format!("Could not parse `{}`: {error}", path.display()))
}

fn apply_mcp(plan: &ImportPlan, env: &ImportEnv, report: &mut ApplyReport) -> Result<(), String> {
    if plan.mcp.is_empty() {
        return Ok(());
    }
    let path = env.mcp_path();
    let mut config = load_mcp_config(&path)?;
    let mut changed = false;
    for draft in &plan.mcp {
        if let Some(existing) = config.mcp_servers.get(&draft.name) {
            if serde_json::to_value(existing).ok() == serde_json::to_value(&draft.server).ok() {
                report.mcp_unchanged += 1;
            } else {
                report.mcp_conflicts.push(format!(
                    "MCP server `{}` from {} was not imported because StarCode already has a different definition.",
                    draft.name,
                    draft.source.label()
                ));
            }
            continue;
        }
        config
            .mcp_servers
            .insert(draft.name.clone(), draft.server.clone());
        report
            .mcp_added
            .push(format!("{} ({})", draft.name, draft.source.label()));
        changed = true;
    }

    if changed {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Could not create `{}` for imported MCP servers: {error}",
                    parent.display()
                )
            })?;
        }
        let content = serde_json::to_string_pretty(&config)
            .map_err(|error| format!("Could not serialize imported MCP config: {error}"))?;
        std::fs::write(&path, content).map_err(|error| {
            format!(
                "Could not write imported MCP config to `{}`: {error}",
                path.display()
            )
        })?;
    }
    Ok(())
}

fn render_plan(plan: &ImportPlan, env: &ImportEnv) -> String {
    if plan.is_empty() {
        let mut out =
            String::from("No importable Codex, Gemini CLI, or Cursor configuration was found.\n");
        out.push_str(&format!(
            "Searched from project `{}` and home `{}`.\n",
            env.project_root.display(),
            env.home.display()
        ));
        if !plan.warnings.is_empty() {
            out.push_str("\nWarnings:\n");
            for warning in &plan.warnings {
                out.push_str(&format!("- {warning}\n"));
            }
        }
        return out;
    }

    let mut out = String::from("## Import preview\n\n");
    out.push_str(&format!(
        "Found {} MCP server(s), {} instruction source(s), {} skill(s), and {} agent(s).\n",
        plan.mcp.len(),
        plan.instructions.len(),
        plan.skills.len(),
        plan.agents.len()
    ));

    if !plan.mcp.is_empty() {
        out.push_str("\n**MCP servers**\n");
        for draft in &plan.mcp {
            out.push_str(&format!(
                "- `{}` from {} (`{}`)\n",
                draft.name,
                draft.source.label(),
                draft.origin.display()
            ));
        }
    }
    if !plan.instructions.is_empty() {
        out.push_str("\n**Instructions**\n");
        for draft in &plan.instructions {
            out.push_str(&format!(
                "- {}: `{}` -> `{}`\n",
                draft.source.label(),
                draft.origin.display(),
                draft.destination.display()
            ));
        }
    }
    if !plan.skills.is_empty() {
        out.push_str("\n**Skills**\n");
        for draft in &plan.skills {
            let assets = if draft.assets.is_empty() {
                String::new()
            } else {
                format!(" ({} supporting file(s))", draft.assets.len())
            };
            out.push_str(&format!(
                "- `{}` from {} -> `{}`{}\n",
                draft.source.label(),
                draft.origin.display(),
                draft.destination.display(),
                assets
            ));
        }
    }
    if !plan.agents.is_empty() {
        out.push_str("\n**Agents**\n");
        for draft in &plan.agents {
            out.push_str(&format!(
                "- `{}` from {} -> `{}`\n",
                draft.source.label(),
                draft.origin.display(),
                draft.destination.display()
            ));
        }
    }
    if !plan.warnings.is_empty() {
        out.push_str("\n**Warnings**\n");
        for warning in &plan.warnings {
            out.push_str(&format!("- {warning}\n"));
        }
    }
    out
}

fn render_report(report: &ApplyReport) -> String {
    let mut out = String::from("## Import complete\n\n");
    out.push_str(&format!(
        "- MCP servers added: {}\n- MCP servers already present: {}\n- Instruction files updated: {}\n- Instruction files unchanged: {}\n- Skills written: {}\n- Skill assets written: {}\n- Skills unchanged: {}\n- Agents written: {}\n- Agents unchanged: {}\n",
        report.mcp_added.len(),
        report.mcp_unchanged,
        report.instructions_written.len(),
        report.instructions_unchanged,
        report.skills_written.len(),
        report.skill_assets_written,
        report.skills_unchanged,
        report.agents_written.len(),
        report.agents_unchanged
    ));
    if !report.mcp_added.is_empty() {
        out.push_str("\n**Added MCP servers**\n");
        for server in &report.mcp_added {
            out.push_str(&format!("- {server}\n"));
        }
    }
    if !report.skills_written.is_empty() {
        out.push_str("\n**Added skills**\n");
        for path in &report.skills_written {
            out.push_str(&format!("- `{}`\n", path.display()));
        }
    }
    if !report.agents_written.is_empty() {
        out.push_str("\n**Added agents**\n");
        for path in &report.agents_written {
            out.push_str(&format!("- `{}`\n", path.display()));
        }
    }
    if !report.instructions_written.is_empty() {
        out.push_str("\n**Updated instruction files**\n");
        for path in &report.instructions_written {
            out.push_str(&format!("- `{}`\n", path.display()));
        }
    }
    if !report.mcp_conflicts.is_empty()
        || !report.skills_skipped.is_empty()
        || !report.agents_skipped.is_empty()
    {
        out.push_str("\n**Not overwritten**\n");
        for warning in report
            .mcp_conflicts
            .iter()
            .chain(&report.skills_skipped)
            .chain(&report.agents_skipped)
        {
            out.push_str(&format!("- {warning}\n"));
        }
    }
    if !report.warnings.is_empty() {
        out.push_str("\n**Warnings**\n");
        for warning in &report.warnings {
            out.push_str(&format!("- {warning}\n"));
        }
    }
    out
}

fn execute_with_env(env: &ImportEnv, options: &ImportOptions) -> Result<String, String> {
    let plan = build_plan(env, &options.sources);
    if plan.is_empty() {
        return Ok(render_plan(&plan, env));
    }
    if options.dry_run || !options.yes {
        let mut output = render_plan(&plan, env);
        if options.dry_run {
            output.push_str(
                "\nDry run: no files were changed. Re-run with `--yes` (without `--dry-run`) to apply.\n",
            );
        } else {
            output.push_str(&format!(
                "\nPreview only. Re-run `/import {} --yes` to apply these changes.\n",
                options
                    .sources
                    .iter()
                    .map(|source| source.key())
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        return Ok(output);
    }

    let report = apply_plan(&plan, env)?;
    Ok(format!(
        "{}\n{}",
        render_plan(&plan, env),
        render_report(&report)
    ))
}

pub async fn run(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let options = match parse_options(&args) {
        Ok(options) => options,
        Err(error) => {
            ctx.state.chat_history.push(
                crate::types::ChatEntry::assistant(format!("❌ {error}")).with_streaming(false),
            );
            return Ok(());
        }
    };
    let env = match ImportEnv::from_process() {
        Ok(env) => env,
        Err(error) => {
            ctx.state.chat_history.push(
                crate::types::ChatEntry::assistant(format!("❌ {error}")).with_streaming(false),
            );
            return Ok(());
        }
    };
    match execute_with_env(&env, &options) {
        Ok(output) => {
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(output).with_streaming(false));
        }
        Err(error) => {
            ctx.state.chat_history.push(
                crate::types::ChatEntry::assistant(format!("❌ Import failed: {error}"))
                    .with_streaming(false),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_env(root: &TempDir) -> ImportEnv {
        let home = root.path().join("home");
        let project = root.path().join("project");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        ImportEnv {
            codex_home: home.join(".codex"),
            home,
            project_root: project,
        }
    }

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    fn options(sources: &[ImportSource]) -> ImportOptions {
        ImportOptions {
            sources: sources.iter().copied().collect(),
            ..Default::default()
        }
    }

    #[test]
    fn options_default_to_all_sources_and_parse_flags() {
        let options = parse_options(&[]).unwrap();
        assert_eq!(options.sources.len(), 3);

        let options = parse_options(&[
            "codex".to_string(),
            "--dry-run".to_string(),
            "--yes".to_string(),
        ])
        .unwrap();
        assert_eq!(options.sources, [ImportSource::Codex].into_iter().collect());
        assert!(options.dry_run);
        assert!(options.yes);
    }

    #[test]
    fn options_reject_an_unknown_source() {
        let error = parse_options(&["windsurf".to_string()]).unwrap_err();
        assert!(error.contains("Unknown import source"));
    }

    #[test]
    fn codex_import_writes_mcp_prompts_and_global_instructions() {
        let root = TempDir::new().unwrap();
        let env = test_env(&root);
        write(
            &env.codex_home.join("config.toml"),
            r#"
[mcp_servers.docs]
command = "npx"
args = ["-y", "@example/docs"]
enabled = true
[mcp_servers.docs.env]
TOKEN = "secret-value"
"#,
        );
        write(&env.codex_home.join("AGENTS.md"), "Prefer small commits.\n");
        write(
            &env.codex_home.join("prompts").join("review.md"),
            "Review this diff.",
        );
        write(&env.project_root.join("AGENTS.md"), "Project instructions.");

        let plan = build_plan(&env, &options(&[ImportSource::Codex]).sources);
        assert_eq!(plan.mcp.len(), 1);
        assert_eq!(plan.skills.len(), 1);
        assert_eq!(plan.instructions.len(), 1);
        assert!(plan
            .warnings
            .iter()
            .any(|warning| warning.contains("already read natively")));

        let mut options = options(&[ImportSource::Codex]);
        options.yes = true;
        let output = execute_with_env(&env, &options).unwrap();
        assert!(output.contains("Import complete"));

        let mcp = std::fs::read_to_string(env.mcp_path()).unwrap();
        assert!(mcp.contains("\"docs\""));
        assert!(mcp.contains("secret-value"));

        let skill = std::fs::read_to_string(env.user_skills_dir().join("review/SKILL.md")).unwrap();
        assert!(skill.contains("name: review"));
        assert!(skill.contains("Review this diff."));

        let user_star = std::fs::read_to_string(env.user_star_path()).unwrap();
        assert!(user_star.contains("starcode-import:begin source=codex"));
        assert!(user_star.contains("Prefer small commits."));
        assert!(!env.project_star_path().exists());
    }

    #[test]
    fn gemini_and_cursor_import_instructions_commands_and_mcp() {
        let root = TempDir::new().unwrap();
        let env = test_env(&root);
        write(
            &env.home.join(".gemini").join("settings.json"),
            r#"{"mcpServers":{"search":{"command":"gemini-mcp","args":["--stdio"]}}}"#,
        );
        write(&env.project_root.join("GEMINI.md"), "Gemini project rule.");
        write(
            &env.home
                .join(".gemini")
                .join("commands")
                .join("explain.toml"),
            "description = \"Explain a file\"\nprompt = \"Explain $1\"\n",
        );
        write(
            &env.project_root
                .join(".cursor")
                .join("rules")
                .join("style.mdc"),
            "---\ndescription: style\nalwaysApply: true\n---\nUse tabs.\n",
        );
        write(
            &env.project_root.join(".cursor").join("mcp.json"),
            r#"{"mcpServers":{"browser":{"url":"http://localhost:1234/mcp","type":"http"}}}"#,
        );

        let plan = build_plan(
            &env,
            &options(&[ImportSource::Gemini, ImportSource::Cursor]).sources,
        );
        assert_eq!(plan.mcp.len(), 2);
        assert_eq!(plan.instructions.len(), 2);
        assert_eq!(plan.skills.len(), 1);

        let mut options = options(&[ImportSource::Gemini, ImportSource::Cursor]);
        options.yes = true;
        execute_with_env(&env, &options).unwrap();

        let mcp = std::fs::read_to_string(env.mcp_path()).unwrap();
        assert!(mcp.contains("\"search\""));
        assert!(mcp.contains("\"browser\""));
        assert!(mcp.contains("http://localhost:1234/mcp"));

        let project_star = std::fs::read_to_string(env.project_star_path()).unwrap();
        assert!(project_star.contains("Gemini project rule."));
        assert!(project_star.contains("Use tabs."));
        assert!(!project_star.contains("alwaysApply"));

        let skill =
            std::fs::read_to_string(env.user_skills_dir().join("explain/SKILL.md")).unwrap();
        assert!(skill.contains("Explain $1"));
        assert!(skill.contains("description: Explain a file"));
    }

    #[test]
    fn agents_and_skill_directories_keep_metadata_and_supporting_files() {
        let root = TempDir::new().unwrap();
        let env = test_env(&root);
        write(
            &env.codex_home.join("agents").join("reviewer.toml"),
            r#"
name = "reviewer"
description = "Reviews code changes"
model = "gpt-5"
tools = ["Read", "Grep"]
developer_instructions = "Review carefully and report risks."
"#,
        );
        write(
            &env.project_root
                .join(".gemini")
                .join("agents")
                .join("helper.md"),
            "---\nname: helper\ndescription: Helps with tests\ntools: Read, Bash\n---\nRun the focused tests.\n",
        );
        write(
            &env.project_root
                .join(".cursor")
                .join("skills")
                .join("deploy")
                .join("SKILL.md"),
            "---\nname: deploy\ndescription: Deploy safely\nallowed-tools: Read, Bash\npaths: src/**\n---\n\nFollow the checklist.\n",
        );
        write(
            &env.project_root
                .join(".cursor")
                .join("skills")
                .join("deploy")
                .join("references")
                .join("checklist.md"),
            "- confirm the environment\n",
        );
        write(
            &env.codex_home.join("skills").join("audit").join("SKILL.md"),
            "---\nname: audit\ndescription: Audit dependencies\n---\n\nCheck the lockfile.\n",
        );
        write(
            &env.codex_home
                .join("skills")
                .join("audit")
                .join("scripts")
                .join("audit.sh"),
            "#!/bin/sh\n",
        );

        let mut options = options(&[
            ImportSource::Codex,
            ImportSource::Gemini,
            ImportSource::Cursor,
        ]);
        options.yes = true;
        let output = execute_with_env(&env, &options).unwrap();
        assert!(output.contains("Agents written: 2"));
        assert!(output.contains("Skill assets written: 2"));

        let codex_agent =
            std::fs::read_to_string(env.user_agents_dir().join("reviewer.md")).unwrap();
        assert!(codex_agent.contains("name: reviewer"));
        assert!(codex_agent.contains("description: Reviews code changes"));
        assert!(codex_agent.contains("model: gpt-5"));
        assert!(codex_agent.contains("tools: Read, Grep"));
        assert!(codex_agent.contains("Review carefully"));

        let gemini_agent =
            std::fs::read_to_string(env.project_agents_dir().join("helper.md")).unwrap();
        assert!(gemini_agent.contains("description: Helps with tests"));
        assert!(gemini_agent.contains("tools: Read, Bash"));
        assert!(gemini_agent.contains("Run the focused tests."));

        let deploy =
            std::fs::read_to_string(env.project_skills_dir().join("deploy/SKILL.md")).unwrap();
        assert!(deploy.contains("allowed-tools: Read, Bash"));
        assert!(deploy.contains("paths: src/**"));
        assert_eq!(
            std::fs::read_to_string(
                env.project_skills_dir()
                    .join("deploy/references/checklist.md")
            )
            .unwrap(),
            "- confirm the environment\n"
        );

        let audit = std::fs::read_to_string(env.user_skills_dir().join("audit/SKILL.md")).unwrap();
        assert!(audit.contains("description: Audit dependencies"));
        assert_eq!(
            std::fs::read_to_string(env.user_skills_dir().join("audit/scripts/audit.sh")).unwrap(),
            "#!/bin/sh\n"
        );

        let second = execute_with_env(&env, &options).unwrap();
        assert!(second.contains("Agents written: 0"));
        assert!(second.contains("Agents unchanged: 2"));
        assert!(second.contains("Skills written: 0"));
        assert!(second.contains("Skills unchanged: 2"));
    }

    #[test]
    fn dry_run_reports_work_without_writing_any_files() {
        let root = TempDir::new().unwrap();
        let env = test_env(&root);
        write(
            &env.codex_home.join("prompts").join("review.md"),
            "Review this diff.",
        );
        let options = ImportOptions {
            sources: [ImportSource::Codex].into_iter().collect(),
            dry_run: true,
            yes: true,
        };

        let output = execute_with_env(&env, &options).unwrap();
        assert!(output.contains("Dry run"));
        assert!(!env.user_skills_dir().exists());
        assert!(!env.mcp_path().exists());
    }

    #[test]
    fn preview_never_prints_mcp_environment_values() {
        let root = TempDir::new().unwrap();
        let env = test_env(&root);
        write(
            &env.codex_home.join("config.toml"),
            "[mcp_servers.docs]\ncommand = \"x\"\n[mcp_servers.docs.env]\nTOKEN = \"top-secret\"\n",
        );
        let plan = build_plan(&env, &options(&[ImportSource::Codex]).sources);
        let preview = render_plan(&plan, &env);
        assert!(preview.contains("docs"));
        assert!(!preview.contains("top-secret"));
    }

    #[test]
    fn applying_twice_is_idempotent_and_preserves_skill_edits() {
        let root = TempDir::new().unwrap();
        let env = test_env(&root);
        write(
            &env.codex_home.join("prompts").join("review.md"),
            "Review this diff.",
        );
        let mut options = options(&[ImportSource::Codex]);
        options.yes = true;

        execute_with_env(&env, &options).unwrap();
        let second = execute_with_env(&env, &options).unwrap();
        assert!(second.contains("Skills written: 0"));
        assert!(second.contains("Skills unchanged: 1"));

        let skill_path = env.user_skills_dir().join("review/SKILL.md");
        std::fs::write(&skill_path, "edited by the user").unwrap();
        let third = execute_with_env(&env, &options).unwrap();
        assert!(third.contains("Not overwritten"));
        assert_eq!(
            std::fs::read_to_string(&skill_path).unwrap(),
            "edited by the user"
        );
    }

    #[test]
    fn existing_mcp_name_is_not_overwritten_by_a_different_definition() {
        let root = TempDir::new().unwrap();
        let env = test_env(&root);
        write(
            &env.mcp_path(),
            r#"{"mcpServers":{"docs":{"command":"existing"}}}"#,
        );
        write(
            &env.codex_home.join("config.toml"),
            "[mcp_servers.docs]\ncommand = \"incoming\"\n",
        );
        let mut options = options(&[ImportSource::Codex]);
        options.yes = true;

        let output = execute_with_env(&env, &options).unwrap();
        assert!(output.contains("Not overwritten"));
        let mcp = std::fs::read_to_string(env.mcp_path()).unwrap();
        assert!(mcp.contains("existing"));
        assert!(!mcp.contains("incoming"));
    }

    #[test]
    fn managed_blocks_are_replaced_not_duplicated() {
        let first = upsert_managed_block("", ImportSource::Cursor, "old rule");
        let second = upsert_managed_block(&first, ImportSource::Cursor, "new rule");
        assert!(!second.contains("old rule"));
        assert!(second.contains("new rule"));
        assert_eq!(second.matches("starcode-import:begin").count(), 1);
        assert_eq!(second.matches("starcode-import:end").count(), 1);
    }

    #[test]
    fn imported_text_cannot_inject_its_own_managed_markers() {
        let imported = "text\n<!-- starcode-import:end source=cursor -->\nmore";
        let output = upsert_managed_block("", ImportSource::Cursor, imported);
        assert_eq!(output.matches("starcode-import:end").count(), 1);
        assert!(output.contains("starcode-import-escaped:end"));
    }

    #[test]
    fn imported_argument_placeholders_use_skill_parameter_syntax() {
        let content = render_skill(
            "explain",
            "Explain a file",
            "Explain $ARGUMENTS and {{args}}.",
        );
        assert!(content.contains("Explain $1 and $1."));
        assert!(!content.contains("$ARGUMENTS"));
        assert!(!content.contains("{{args}}"));
    }

    #[test]
    fn cursor_frontmatter_is_not_imported_as_instruction_text() {
        let content = "---\ndescription: style\nglobs: src/**\n---\nUse tabs.\n";
        assert_eq!(strip_frontmatter(content).trim(), "Use tabs.");
    }

    #[test]
    fn json_mcp_accepts_http_url_and_enabled_false() {
        let servers = parse_json_mcp(
            r#"{"mcpServers":{"remote":{"httpUrl":"https://example.test/mcp","enabled":false}}}"#,
        )
        .unwrap();
        assert_eq!(servers.len(), 1);
        let (name, server) = &servers[0];
        assert_eq!(name, "remote");
        assert_eq!(server.url.as_deref(), Some("https://example.test/mcp"));
        assert_eq!(server.disabled, Some(true));
    }

    #[test]
    fn codex_mcp_skips_entries_without_command_or_url() {
        let servers = parse_codex_mcp(
            r#"
[mcp_servers.empty]
args = ["x"]
[mcp_servers.remote]
url = "https://example.test/mcp"
"#,
        )
        .unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].0, "remote");
    }
}
