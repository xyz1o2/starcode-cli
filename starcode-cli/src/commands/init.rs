use crate::commands::execution::{CommandContext, CommandResult};
use crate::core::config::project_scaffold::{scaffold_project_star, ProjectScaffoldSummary};
use crate::utils::environment_context;
use std::path::{Path, PathBuf};

/// 核心初始化逻辑：检测技术栈并生成 STARCODE.md
pub fn generate_initial_context(cwd: &Path) -> Result<PathBuf, String> {
    let target_path = cwd.join("STARCODE.md");

    if target_path.exists() {
        return Err(format!(
            "STARCODE.md already exists at {}",
            target_path.display()
        ));
    }

    // 智能分析技术栈
    let mut build_cmd = "cargo build";
    let mut test_cmd = "cargo test";
    let mut lint_cmd = "cargo clippy";
    let mut tech_stack = "Rust";

    if cwd.join("package.json").exists() {
        tech_stack = "Node.js/JavaScript";
        build_cmd = "npm run build";
        test_cmd = "npm test";
        lint_cmd = "npm run lint";
    } else if cwd.join("pyproject.toml").exists() || cwd.join("requirements.txt").exists() {
        tech_stack = "Python";
        build_cmd = "pip install .";
        test_cmd = "pytest";
        lint_cmd = "flake8";
    } else if cwd.join("go.mod").exists() {
        tech_stack = "Go";
        build_cmd = "go build ./...";
        test_cmd = "go test ./...";
        lint_cmd = "go vet ./...";
    } else if cwd.join("Makefile").exists() {
        tech_stack = "C/C++ (Make)";
        build_cmd = "make";
        test_cmd = "make test";
        lint_cmd = "make lint";
    }

    let structure = environment_context::get_directory_context_string(&cwd);

    let template = format!(
        r#"# STARCODE.md - Project Context

## Project Overview
- Tech Stack: {}
- Initialization Date: {}

## Commands
- Build: `{}`
- Test: `{}`
- Lint: `{}`

## Code Style
- Follow project conventions
- Maintain clean and documented code

## Project Structure
{}
"#,
        tech_stack,
        chrono::Local::now().format("%Y-%m-%d"),
        build_cmd,
        test_cmd,
        lint_cmd,
        structure
    );

    if let Err(e) = std::fs::write(&target_path, template) {
        return Err(format!("Failed to write STARCODE.md: {}", e));
    }

    Ok(target_path)
}

fn render_scaffold_note(summary: &ProjectScaffoldSummary) -> Option<String> {
    if !summary.has_changes() {
        return None;
    }

    Some(format!(
        "📁 Initialized project `.star` scaffold (created {} dirs, {} files).",
        summary.created_dirs.len(),
        summary.created_files.len()
    ))
}

pub async fn run(ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    let scaffold_summary = scaffold_project_star(&cwd)?;
    let scaffold_note = render_scaffold_note(&scaffold_summary);

    let target_path_res = generate_initial_context(&cwd);

    match target_path_res {
        Err(msg) => {
            // 如果只是文件已存在，我们仍然可以继续进行智能分析，但要通知用户
            let content = if msg.contains("already exists") {
                let mut content = format!("⚠️ {}", msg);
                if let Some(note) = &scaffold_note {
                    content.push_str(&format!("\n\n{}", note));
                }
                content.push_str("\n\n🔍 Proceeding with intelligent analysis to update it...");
                content
            } else {
                return Err(msg);
            };

            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(content).with_streaming(false));
        }
        Ok(path) => {
            let mut content = format!(
                "✅ Successfully created basic STARCODE.md at {}",
                path.display()
            );
            if let Some(note) = &scaffold_note {
                content.push_str(&format!("\n\n{}", note));
            }
            content.push_str("\n\n🔍 Starting intelligent analysis to enhance it...");
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(content).with_streaming(false));
        }
    }

    // 触发智能分析
    let prompt = format!(
        "Please analyze this codebase and create/update the STARCODE.md file, which will be given to future instances of StarCode to operate in this repository.\n\n\
        What to add:\n\
        1. Commands that will be commonly used, such as how to build, lint, and run tests. Include the necessary commands to develop in this codebase, such as how to run a single test.\n\n\
        2. High-level code architecture and structure so that future instances can be productive more quickly. Focus on the \"big picture\" architecture that requires reading multiple files to understand.\n\n\
        Usage notes:\n\
        - If there's already a STARCODE.md, suggest improvements to it.\n\
        - Do not repeat yourself and do not include obvious instructions like \"Provide helpful error messages\".\n\
        - Avoid listing every component or file structure that can be easily discovered.\n\
        - Don't include generic development practices.\n\
        - Be sure to prefix the file with the following text EXACTLY:\n\
        ```\n\
        # STARCODE.md\n\n\
        This file provides guidance to StarCode when working with code in this repository.\n\
        ```\n\n\
        Current CWD: {}\n\n\
        Please use `read_many_files` to inspect relevant config files first, then `edit_file` or `write_file` to update `STARCODE.md`.",
        cwd.display()
    );

    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    let _ = ctx
        .agent_tx
        .send(crate::runtime::messages::AgentRequest::SendMessage {
            message_id,
            message: prompt,
        })
        .await;

    Ok(())
}
