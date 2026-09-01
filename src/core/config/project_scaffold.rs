use crate::core::config::storage::Storage;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct ProjectScaffoldSummary {
    pub created_dirs: Vec<PathBuf>,
    pub created_files: Vec<PathBuf>,
}

impl ProjectScaffoldSummary {
    pub fn has_changes(&self) -> bool {
        !self.created_dirs.is_empty() || !self.created_files.is_empty()
    }
}

pub fn scaffold_project_star(project_root: &Path) -> Result<ProjectScaffoldSummary, String> {
    let storage = Storage::new(project_root.to_path_buf());
    let star_dir = storage.star_dir();
    let agents_dir = storage.project_agents_dir();
    let agents_examples_dir = agents_dir.join("_examples");
    let skills_dir = storage.project_skills_dir();
    let skills_examples_dir = skills_dir.join("_examples").join("code_review");
    let extensions_dir = storage.extensions_dir();

    let mut summary = ProjectScaffoldSummary::default();
    for dir in [
        star_dir.clone(),
        agents_dir.clone(),
        agents_examples_dir.clone(),
        skills_dir.clone(),
        skills_examples_dir.clone(),
        extensions_dir.clone(),
    ] {
        ensure_dir(&dir, &mut summary)?;
    }

    write_if_missing(
        &star_dir.join("README.md"),
        star_readme_template(),
        &mut summary,
    )?;
    write_if_missing(
        &storage.workspace_settings_path(),
        settings_template(),
        &mut summary,
    )?;
    write_if_missing(
        &storage.project_mcp_config_path(),
        mcp_template(),
        &mut summary,
    )?;
    write_if_missing(
        &star_dir.join("provider.example.jsonc"),
        provider_template(),
        &mut summary,
    )?;
    write_if_missing(
        &agents_dir.join("README.md"),
        agents_readme_template(),
        &mut summary,
    )?;
    write_if_missing(
        &agents_examples_dir.join("reviewer.md.example"),
        agent_example_template(),
        &mut summary,
    )?;
    write_if_missing(
        &skills_dir.join("README.md"),
        skills_readme_template(),
        &mut summary,
    )?;
    write_if_missing(
        &skills_examples_dir.join("SKILL.md.example"),
        skill_example_template(),
        &mut summary,
    )?;
    write_if_missing(
        &extensions_dir.join("README.md"),
        extensions_readme_template(),
        &mut summary,
    )?;
    write_if_missing(
        &extensions_dir.join("star-extension.example.jsonc"),
        extension_manifest_template(),
        &mut summary,
    )?;

    Ok(summary)
}

fn ensure_dir(path: &Path, summary: &mut ProjectScaffoldSummary) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    fs::create_dir_all(path).map_err(|e| format!("failed to create {}: {}", path.display(), e))?;
    summary.created_dirs.push(path.to_path_buf());
    Ok(())
}

fn write_if_missing(
    path: &Path,
    content: &'static str,
    summary: &mut ProjectScaffoldSummary,
) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {}", parent.display(), e))?;
    }
    fs::write(path, content).map_err(|e| format!("failed to write {}: {}", path.display(), e))?;
    summary.created_files.push(path.to_path_buf());
    Ok(())
}

fn star_readme_template() -> &'static str {
    r#"# Project .star

This directory stores project-local StarCode configuration and generated project state.

Files and directories you will most likely edit:

- `settings.json`: project-local model override and future workspace settings.
- `mcp.json`: MCP servers for this repository only.
- `provider.example.jsonc`: optional provider/model override example for this repository.
- `agents/`: custom project agents.
- `skills/`: local skill folders or skill examples.
- `extensions/`: installed plugins and plugin manifest examples.

To activate provider overrides, copy `provider.example.jsonc` to `provider.json`.

Suggested prompts for StarCode:

- `Help me configure Playwright MCP for this project.`
- `Create a reviewer agent under .star/agents for Rust code reviews.`
- `Install a plugin into the current project and explain the generated files.`

Project scope is preferred here. Use `~/.star` only for truly global user settings.
"#
}

fn settings_template() -> &'static str {
    r#"{
  // Project-local model override. Set to null to follow your global/default model.
  "model": null
}
"#
}

fn mcp_template() -> &'static str {
    r#"{
  // MCP servers for this repository only.
  "mcpServers": {
    // Example stdio server:
    // "filesystem": {
    //   "command": "npx",
    //   "args": ["-y", "@modelcontextprotocol/server-filesystem", "."]
    // },

    // Example HTTP / streamable HTTP server:
    // "internal-api": {
    //   "type": "streamable_http",
    //   "url": "http://127.0.0.1:3000/mcp"
    // },

    // Example with environment variables:
    // "github": {
    //   "command": "npx",
    //   "args": ["-y", "@modelcontextprotocol/server-github"],
    //   "env": {
    //     "GITHUB_TOKEN": "${GITHUB_TOKEN}"
    //   }
    // }
  }
}
"#
}

fn agents_readme_template() -> &'static str {
    r#"# Project Agents

Create project-local custom agents here.

- Active agent files live directly in this folder, for example: `reviewer.md`
- Example files live under `_examples/`
- Project agents override agents with the same id from `~/.star/agents`

Example workflow:

1. Copy `_examples/reviewer.md.example` to `reviewer.md`
2. Adjust frontmatter fields (`id`, `name`, `description`, `tools`, `model`)
3. Update the prompt body with project-specific guidance

You can also ask StarCode directly: `Create a reviewer agent for this repository under .star/agents`.
"#
}

fn provider_template() -> &'static str {
    r#"{
  // Optional project-local provider override example.
  "active_provider_id": "openai",
  "providers": {
    "openai": {
      "base_url": "https://api.openai.com/v1",
      "selected_model": "gpt-5"
    }
  }
}
"#
}

fn agent_example_template() -> &'static str {
    r#"---
id: reviewer
name: Code Reviewer
description: Review diffs for correctness, risk, and missing tests
tools: view_file, search, semantic_search
aliases: review, qa
model: gpt-5
---

You are the project reviewer agent.

Focus on:
- behavioral regressions
- risky edge cases
- missing or weak test coverage
- migration and configuration mistakes

Keep findings concrete and reference files or functions when possible.
"#
}

fn skills_readme_template() -> &'static str {
    r#"# Project Skills

Store project-local reusable skills here.

Recommended structure:

- one skill per folder
- each skill folder contains a `SKILL.md`
- keep examples under `_examples/` so they are not loaded as active skills by accident

Example prompt to StarCode:

- `Create a deployment skill under .star/skills for our release workflow.`
"#
}

fn skill_example_template() -> &'static str {
    r#"---
name: code-review
description: Review code changes with project-specific risk checks
version: 1.0.0
---

When invoked:

1. Inspect the changed files first.
2. Prioritize correctness, migrations, and missing tests.
3. Keep the final output concise and actionable.
"#
}

fn extensions_readme_template() -> &'static str {
    r#"# Project Plugins

Installed project plugins live in this directory.

- Plugin code is stored in subdirectories under `extensions/`
- The machine-managed manifest is `star-extension.json`
- Use `star-extension.example.jsonc` as a commented reference

Prefer using `/plugin install ...` and `/plugin remove ...` instead of editing the manifest by hand.
"#
}

fn extension_manifest_template() -> &'static str {
    r#"{
  // Example plugin manifest. The real file is .star/extensions/star-extension.json
  // and is usually managed by `/plugin install` and `/plugin remove`.
  "plugins": [
    {
      "name": "example-plugin",
      "source": "https://github.com/owner/repo",
      "install_type": "git",
      "installed_at": 0,
      "enabled": true
    }
  ]
}
"#
}
 