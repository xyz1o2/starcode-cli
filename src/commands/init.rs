use crate::commands::execution::{CommandContext, CommandResult};
use crate::core::config::project_scaffold::{scaffold_project_star, ProjectScaffoldSummary};
use std::path::{Path, PathBuf};

/// 上下文文件的解析口径与 `utils::project_context` 的加载顺序保持一致：
/// 加载器只注入第一个命中的候选文件。
const CONTEXT_FILE_CANDIDATES: &[&str] = &[
    "AGENTS.override.md",
    "STAR.md",
    "STARCODE.md",
    "CLAUDE.md",
    "AGENTS.md",
];

/// 解析 /init 的目标文件：返回 (路径, 是否已存在)。
///
/// 已有上下文文件时必须改进现有文件，而不是新建一个把旧的遮蔽掉 ——
/// 加载器只读第一个命中的候选，比如仓库里已有 CLAUDE.md 时再新建
/// STARCODE.md，会让既有的 CLAUDE.md 从此失效（对标 Claude Code 的
/// "If there's already a CLAUDE.md, suggest improvements to it"）。
pub fn resolve_context_target(cwd: &Path) -> (PathBuf, bool) {
    for name in CONTEXT_FILE_CANDIDATES {
        let candidate = cwd.join(name);
        if candidate.is_file() {
            return (candidate, true);
        }
    }
    (cwd.join("STARCODE.md"), false)
}

/// 构建 /init 发给模型的完整提示词。
///
/// 对标 Claude Code 的 init 提示词，全部用法说明逐条保留：README、
/// Cursor/Copilot 规则、禁止编造栏目、明显指令反例、精确前缀块。
pub fn build_init_prompt(cwd: &Path, target: &Path, target_exists: bool) -> String {
    let file_name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("STARCODE.md");

    let exists_note = if target_exists {
        format!(
            "IMPORTANT: {file_name} already exists at this repository's root. Read it first, then suggest and apply specific improvements to it. Do not discard or silently overwrite its existing content.\n\n"
        )
    } else {
        String::new()
    };

    format!(
        r#"Please analyze this codebase and create a {file} file, which will be given to future instances of StarCode to operate in this repository.

{exists_note}What to add:
1. Commands that will be commonly used, such as how to build, lint, and run tests. Include the necessary commands to develop in this codebase, such as how to run a single test.
2. High-level code architecture and structure so that future instances can be productive more quickly. Focus on the "big picture" architecture that requires reading multiple files to understand.

Usage notes:
- If there's already a {file}, suggest improvements to it.
- When you make the initial {file}, do not repeat yourself and do not include obvious instructions like "Provide helpful error messages to users", "Write unit tests for all new utilities", "Never include sensitive information (API keys, tokens) in code or commits".
- Avoid listing every component or file structure that can be easily discovered.
- Don't include generic development practices.
- If there are Cursor rules (in .cursor/rules/ or .cursorrules) or Copilot rules (in .github/copilot-instructions.md), make sure to include the important parts.
- If there is a README.md, make sure to include the important parts.
- Do not make up information such as "Common Development Tasks", "Tips for Development", "Support and Documentation" unless this is expressly included in other files that you read.
- Be sure to prefix the file with the following text:

```
# {file}

This file provides guidance to StarCode when working with code in this repository.
```

Current CWD: {cwd}

Please use `read_many_files` to inspect relevant config files first, then `Write` or `Edit` to update `{file}`."#,
        file = file_name,
        exists_note = exists_note,
        cwd = cwd.display(),
    )
}

/// 离线引导：无模型参与时（`starcode init` 子命令）写一个最小上下文骨架。
///
/// 交互式 `/init` 不走这里 —— 文件由模型分析后直接生成，对标 Claude Code
/// 的行为；占位模板里的"Initialization Date"之类内容正是其提示词明令禁止的。
pub fn generate_initial_context(cwd: &Path) -> Result<PathBuf, String> {
    let (target_path, exists) = resolve_context_target(cwd);
    if exists {
        return Err(format!(
            "{} already exists at {}",
            target_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("context file"),
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

    let structure = crate::utils::environment_context::get_directory_context_string(cwd);

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

    let (target_path, target_exists) = resolve_context_target(&cwd);
    let file_name = target_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("STARCODE.md")
        .to_string();

    let mut content = if target_exists {
        format!(
            "🔍 Analyzing codebase to improve existing {}...",
            target_path.display()
        )
    } else {
        format!(
            "🔍 Analyzing codebase to create {}...",
            target_path.display()
        )
    };
    if let Some(note) = &scaffold_note {
        content.push_str(&format!("\n\n{}", note));
    }
    ctx.state
        .chat_history
        .push(crate::types::ChatEntry::assistant(content).with_streaming(false));

    let prompt = build_init_prompt(&cwd, &target_path, target_exists);

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

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个测试独占一个临时目录，避免相互干扰
    fn temp_repo(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "starcode-init-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 目标解析必须镜像加载器的候选顺序：STAR.md > STARCODE.md > CLAUDE.md
    #[test]
    fn resolve_prefers_existing_context_files_in_loader_order() {
        let dir = temp_repo("order");

        // 都不存在 → 新建 STARCODE.md
        let (path, exists) = resolve_context_target(&dir);
        assert_eq!(path, dir.join("STARCODE.md"));
        assert!(!exists);

        // 只有 CLAUDE.md → 改进它，而不是新建 STARCODE.md 遮蔽
        std::fs::write(dir.join("CLAUDE.md"), "hi").unwrap();
        let (path, exists) = resolve_context_target(&dir);
        assert_eq!(path, dir.join("CLAUDE.md"));
        assert!(exists);

        // STARCODE.md 优先级更高
        std::fs::write(dir.join("STARCODE.md"), "hi").unwrap();
        let (path, exists) = resolve_context_target(&dir);
        assert_eq!(path, dir.join("STARCODE.md"));
        assert!(exists);

        // STAR.md 最优先
        std::fs::write(dir.join("STAR.md"), "hi").unwrap();
        let (path, exists) = resolve_context_target(&dir);
        assert_eq!(path, dir.join("STAR.md"));
        assert!(exists);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 提示词必须包含对标参考实现的全部用法说明（此前缺失的三条 + 反例 + 前缀块）
    #[test]
    fn prompt_carries_all_reference_usage_notes() {
        let dir = temp_repo("prompt");
        let target = dir.join("STARCODE.md");
        let prompt = build_init_prompt(&dir, &target, false);

        for needle in [
            "If there is a README.md, make sure to include the important parts.",
            ".cursorrules",
            ".github/copilot-instructions.md",
            "Do not make up information such as \"Common Development Tasks\"",
            "Write unit tests for all new utilities",
            "Never include sensitive information (API keys, tokens) in code or commits",
            "If there's already a STARCODE.md, suggest improvements to it.",
            "# STARCODE.md",
            "This file provides guidance to StarCode when working with code in this repository.",
        ] {
            assert!(prompt.contains(needle), "prompt missing: {needle}");
        }

        // 已存在时附加"先读后改、不得静默覆盖"的强提醒
        let prompt = build_init_prompt(&dir, &target, true);
        assert!(prompt.contains("already exists"));
        assert!(prompt.contains("Do not discard or silently overwrite"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 离线骨架：已有上下文文件时报错而不是新建遮蔽文件；空目录则写出带前缀的骨架
    #[test]
    fn offline_scaffold_never_shadows_existing_context() {
        let dir = temp_repo("offline");

        std::fs::write(dir.join("CLAUDE.md"), "existing").unwrap();
        let err = generate_initial_context(&dir).unwrap_err();
        assert!(err.contains("already exists"), "err = {err}");
        assert!(!dir.join("STARCODE.md").exists(), "不得新建遮蔽文件");

        // 移除既有文件后再跑，空目录应成功写出骨架
        std::fs::remove_file(dir.join("CLAUDE.md")).unwrap();

        let created = generate_initial_context(&dir).expect("empty repo should scaffold");
        assert!(created.ends_with("STARCODE.md"));
        let content = std::fs::read_to_string(&created).unwrap();
        assert!(content.starts_with("# STARCODE.md"));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
