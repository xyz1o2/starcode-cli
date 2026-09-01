use crate::commands::execution::{CommandContext, CommandResult};
use clap::Subcommand;

#[derive(Subcommand)]
pub enum SkillsCommand {
    /// List all custom skills (project + user)
    List,
    /// Show a skill's content and metadata
    #[command(arg_required_else_help = true)]
    Show {
        /// Skill name
        name: String,
    },
    /// Create a new custom skill
    #[command(arg_required_else_help = true)]
    New {
        /// Skill name (used as directory name and default skill name)
        name: String,
        /// Short description for the skill
        #[arg(short, long)]
        description: Option<String>,
        /// Create in user skill directory (~/.star/skills) instead of project
        #[arg(long)]
        user: bool,
    },
    /// Delete a custom skill
    #[command(arg_required_else_help = true)]
    Delete {
        /// Skill name
        name: String,
        /// Delete from user directory instead of project
        #[arg(long)]
        user: bool,
    },
    /// List available built-in sub-agents and custom agents
    Agents,
}

pub async fn execute_skills_command(ctx: CommandContext<'_>, cmd: SkillsCommand) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let storage = crate::core::config::storage::Storage::new(cwd.clone());
    let project_skills_dir = storage.project_skills_dir();
    let user_skills_dir = crate::core::config::storage::Storage::user_skills_dir();

    match cmd {
        SkillsCommand::List => {
            let mut project_skills =
                crate::agent::skills::loader::SkillLoader::load_skills_from_dir(
                    &project_skills_dir,
                )
                .await;
            let user_skills =
                crate::agent::skills::loader::SkillLoader::load_skills_from_dir(&user_skills_dir)
                    .await;

            project_skills.sort_by(|a, b| a.name.cmp(&b.name));
            let mut user_skills_sorted = user_skills;
            user_skills_sorted.sort_by(|a, b| a.name.cmp(&b.name));

            if project_skills.is_empty() && user_skills_sorted.is_empty() {
                ctx.state.chat_history.push(
                    crate::types::ChatEntry::assistant(
                        "No custom skills found.\n\nCreate one with `/skills new <name>` or place a `SKILL.md` under `.star/skills/<name>/`.".to_string()
                    ).with_streaming(false),
                );
                return Ok(());
            }

            let mut lines = vec!["# Custom Skills\n".to_string()];

            if !project_skills.is_empty() {
                lines.push("## Project Skills\n".to_string());
                for s in &project_skills {
                    let desc = if s.description.is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", s.description)
                    };
                    let ver = s
                        .version
                        .as_deref()
                        .map(|v| format!(" `v{}`", v))
                        .unwrap_or_default();
                    lines.push(format!("- **{}**{}{}", s.name, ver, desc));
                    lines.push(format!("  - location: `{}`", s.location));
                }
            }

            if !user_skills_sorted.is_empty() {
                if !project_skills.is_empty() {
                    lines.push(String::new());
                }
                lines.push("## User Skills\n".to_string());
                for s in &user_skills_sorted {
                    let desc = if s.description.is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", s.description)
                    };
                    let ver = s
                        .version
                        .as_deref()
                        .map(|v| format!(" `v{}`", v))
                        .unwrap_or_default();
                    lines.push(format!("- **{}**{}{}", s.name, ver, desc));
                    lines.push(format!("  - location: `{}`", s.location));
                }
            }

            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(lines.join("\n")).with_streaming(false));
            Ok(())
        }

        SkillsCommand::Show { name } => {
            let all_skills = {
                let mut v = crate::agent::skills::loader::SkillLoader::load_skills_from_dir(
                    &project_skills_dir,
                )
                .await;
                v.extend(
                    crate::agent::skills::loader::SkillLoader::load_skills_from_dir(
                        &user_skills_dir,
                    )
                    .await,
                );
                v
            };

            let skill = all_skills.into_iter().find(|s| s.name == name);

            let msg = match skill {
                None => format!(
                    "⚠️ Skill `{}` not found.\n\nUse `/skills list` to see available skills.",
                    name
                ),
                Some(s) => {
                    let mut lines = vec![format!("# Skill: `{}`\n", s.name)];
                    if !s.description.is_empty() {
                        lines.push(format!("**Description:** {}\n", s.description));
                    }
                    if let Some(ver) = s.version.as_deref() {
                        lines.push(format!("**Version:** {}\n", ver));
                    }
                    lines.push(format!("**Location:** `{}`\n", s.location));
                    lines.push("---\n".to_string());
                    lines.push(s.body.clone());
                    lines.join("\n")
                }
            };

            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
            Ok(())
        }

        SkillsCommand::New {
            name,
            description,
            user,
        } => {
            let safe_name = slugify(&name);
            if safe_name.is_empty() {
                return Err("invalid skill name".to_string());
            }

            let base_dir = if user {
                user_skills_dir.clone()
            } else {
                project_skills_dir.clone()
            };

            let skill_dir = base_dir.join(&safe_name);
            let skill_file = skill_dir.join("SKILL.md");

            if skill_file.exists() {
                return Err(format!(
                    "skill `{}` already exists at {}",
                    safe_name,
                    skill_file.display()
                ));
            }

            tokio::fs::create_dir_all(&skill_dir)
                .await
                .map_err(|e| format!("failed to create skill directory: {}", e))?;

            let desc_str = description
                .as_deref()
                .unwrap_or("Describe what this skill does");
            let content = format!(
                "---\nname: {}\ndescription: {}\nversion: 1.0.0\n---\n\nDescribe the steps or instructions for this skill here.\n\nExample:\n\n1. First step...\n2. Second step...\n",
                safe_name, desc_str
            );

            tokio::fs::write(&skill_file, &content)
                .await
                .map_err(|e| format!("failed to write skill file: {}", e))?;

            let scope = if user { "user" } else { "project" };
            let msg = format!(
                "✅ Created skill `{}`\n\n- scope: {}\n- location: `{}`\n\nEdit the file to add instructions, then invoke with `skill(\"{}\")` or ask the agent to use it.",
                safe_name,
                scope,
                skill_file.display(),
                safe_name
            );

            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
            Ok(())
        }

        SkillsCommand::Delete { name, user } => {
            let base_dir = if user {
                user_skills_dir.clone()
            } else {
                project_skills_dir.clone()
            };

            let skill_dir = base_dir.join(&name);
            let skill_file_in_dir = skill_dir.join("SKILL.md");
            let skill_file_flat = base_dir.join(format!("{}.skill.md", name));

            if skill_file_in_dir.exists() || skill_dir.exists() {
                if skill_dir.exists() {
                    tokio::fs::remove_dir_all(&skill_dir)
                        .await
                        .map_err(|e| format!("failed to remove skill directory: {}", e))?;
                }
                ctx.state.chat_history.push(
                    crate::types::ChatEntry::assistant(format!(
                        "✅ Deleted skill `{}` (removed `{}`)",
                        name,
                        skill_dir.display()
                    ))
                    .with_streaming(false),
                );
            } else if skill_file_flat.exists() {
                tokio::fs::remove_file(&skill_file_flat)
                    .await
                    .map_err(|e| format!("failed to remove skill file: {}", e))?;
                ctx.state.chat_history.push(
                    crate::types::ChatEntry::assistant(format!(
                        "✅ Deleted skill `{}` (removed `{}`)",
                        name,
                        skill_file_flat.display()
                    ))
                    .with_streaming(false),
                );
            } else {
                ctx.state.chat_history.push(
                    crate::types::ChatEntry::assistant(format!(
                        "⚠️ Skill `{}` not found in {} skills directory.\n\nUse `/skills list` to see available skills.",
                        name,
                        if user { "user" } else { "project" }
                    ))
                    .with_streaming(false),
                );
            }
            Ok(())
        }

        SkillsCommand::Agents => {
            let builtin = vec![
                (
                    "analyzer",
                    "Analyzes code structure, architecture, and patterns",
                ),
                ("editor", "Makes targeted code edits and refactors"),
                (
                    "Grep",
                    "Searches for code, symbols, and content across the project",
                ),
                (
                    "navigator",
                    "Navigates complex codebases to understand structure and flow",
                ),
                (
                    "auto_fix",
                    "Automatically diagnoses and fixes common code issues",
                ),
            ];

            let custom_defs = crate::agent::skills::custom::load_custom_subagent_definitions(&cwd);

            let mut lines = vec!["# Available Skills / Sub-Agents\n".to_string()];

            lines.push("## Built-in Sub-Agents\n".to_string());
            for (id, desc) in &builtin {
                lines.push(format!("- **{}** — {}", id, desc));
            }

            if !custom_defs.is_empty() {
                lines.push(String::new());
                lines.push("## Custom Sub-Agents\n".to_string());
                for def in &custom_defs {
                    let desc = if def.description.is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", def.description)
                    };
                    let aliases = if def.aliases.is_empty() {
                        String::new()
                    } else {
                        format!(" _(aliases: {})_", def.aliases.join(", "))
                    };
                    lines.push(format!("- **{}**{}{}", def.id, desc, aliases));
                    lines.push(format!("  - source: `{}`", def.source_path.display()));
                }
            }

            lines.push(String::new());
            lines.push("**Usage:** ask the agent to use a skill, e.g. _\"use the analyzer skill to audit src/auth.rs\"_".to_string());

            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(lines.join("\n")).with_streaming(false));
            Ok(())
        }
    }
}

fn slugify(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else if c == ' ' {
                '-'
            } else {
                '\0'
            }
        })
        .filter(|&c| c != '\0')
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}
