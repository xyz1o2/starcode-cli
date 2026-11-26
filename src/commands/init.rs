/// 在线 `/init` 将仓库分析交给现有 worker；离线分支只创建固定的最小指引文件。
/// 参数先校验再产生副作用；帮助和预览不写磁盘、不请求模型。
use crate::commands::execution::{CommandContext, CommandResult};
use crate::core::i18n;
use std::io::Write;
use std::path::{Path, PathBuf};

/// 参数必须在任何写入前完成校验，避免留下半初始化状态。
#[derive(Debug, Default, PartialEq, Eq)]
struct InitOptions {
    help: bool,
    offline: bool,
    dry_run: bool,
    guidance: String,
}
impl InitOptions {
    fn parse(args: Vec<String>) -> Result<Self, String> {
        let mut options = Self::default();
        let mut guidance = Vec::new();
        let mut literal = false;
        for arg in args {
            match arg.as_str() {
                "--" if !literal => literal = true,
                "--help" | "-h" if !literal => options.help = true,
                "--offline" if !literal => options.offline = true,
                "--dry-run" if !literal => options.dry_run = true,
                _ if !literal && arg.starts_with('-') => {
                    return Err(i18n::t1("cmd.init.unknown_option").replace("{option}", &arg));
                }
                _ => guidance.push(arg),
            }
        }
        options.guidance = guidance.join(" ").trim().to_string();
        if options.offline && !options.guidance.is_empty() && !options.help {
            return Err(i18n::t1("cmd.init.offline_guidance"));
        }
        Ok(options)
    }
}

/// 初始化只维护一个目标；其他指引由模型探索仓库时按需读取。
const CONTEXT_FILE: &str = "AGENTS.md";

const MINIMAL_CONTEXT: &str = "# AGENTS.md\n\nThis file provides guidance to coding agents when working with code in this repository.\n";

/// 解析 /init 的目标文件：返回 (路径, 是否已存在)。
///
/// 只选择当前目录的指引，不会因父目录有全局规则就改写父目录文件。
/// 悬空链接也算已占用，避免离线写入意外穿透到链接目标。
pub fn resolve_context_target(cwd: &Path) -> (PathBuf, bool) {
    let target = cwd.join(CONTEXT_FILE);
    let exists = target.exists() || target.is_symlink();
    (target, exists)
}

/// 构建 /init 发给模型的完整提示词。
///
/// 提示词约束证据和产物，不根据文件名预设技术栈或命令。
/// 已有文件只做有依据的增量改进，不强制重写标题和语言。
pub fn build_init_prompt(cwd: &Path, target: &Path, target_exists: bool) -> String {
    let file_name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(CONTEXT_FILE);

    let exists_note = if target_exists {
        format!(
            "IMPORTANT: Existing project guidance is at {target}. Read it first, then suggest and apply specific, evidence-based improvements. Do not discard or silently overwrite its existing content.\n\n",
            target = target.display(),
        )
    } else {
        String::new()
    };

    format!(
        r#"Please analyze this codebase and create or improve project guidance at `{target}`. It will be given to future instances of StarCode to operate in this repository.

{exists_note}What to add:
1. Commands that will be commonly used, such as how to build, lint, and run tests. Include the necessary commands to develop in this codebase, such as how to run a single test.
2. High-level code architecture and structure so that future instances can be productive more quickly. Focus on the "big picture" architecture that requires reading multiple files to understand.

Usage notes:
- If there's already a {file}, suggest improvements to it.
- When you make the initial {file}, do not repeat yourself and do not include obvious instructions like "Provide helpful error messages to users", "Write unit tests for all new utilities", "Never include sensitive information (API keys, tokens) in code or commits".
- Avoid listing every component or file structure that can be easily discovered.
- Don't include generic development practices.
- Discover other repository instruction and editor-rule files as needed. Read relevant guidance and incorporate important repository-specific constraints without duplicating entire documents.
- If there is a README.md, make sure to include the important parts.
- Do not make up information such as "Common Development Tasks", "Tips for Development", "Support and Documentation" unless this is expressly included in other files that you read.
- Only for a new file, use the following prefix. Preserve the language, heading, and useful content of an existing file:

```
# {file}

This file provides guidance to coding agents when working with code in this repository.
```

Evidence and scope:
- Read existing project instructions, README, manifests, package-manager declarations, lockfiles, task scripts, and CI workflows before documenting commands. Use available read/search tools; no particular tool is required.
- Include only commands supported by files you actually read. Verify script/target names, working directories, environment requirements, and test filters. Never infer a build, lint, or test command solely from a manifest filename or assume a package manager or language when evidence is absent.
- For mixed-language repositories and monorepos, document each relevant workspace and its working directory rather than selecting one stack.
- Trace entry points, constructors, and runtime call sites before describing architecture. Distinguish live paths from unconnected implementations.
- Keep guidance concise and durable. Omit initialization dates, exhaustive directory trees, generic advice, invented sections, and unverified claims. If evidence is missing, leave it out and mention the uncertainty in your reply.
- Read other applicable instruction files to avoid conflicts, but modify only the exact target below. Do not create duplicate instruction files, scaffold configuration directories, install dependencies, or run builds/tests merely to discover commands.
- If the existing guidance is already accurate, leave it unchanged. Summarize what changed and which commands were verified from files versus actually executed; never claim execution without running them.

Current CWD: {cwd}
Exact target path: {target}

Analyze this repository, then use available file-editing tools to update the exact target path. Respect the current permission mode."#,
        file = file_name,
        exists_note = exists_note,
        cwd = cwd.display(),
        target = target.display(),
    )
}

/// 离线引导：供 CLI `init` 与 `/init --offline` 创建最小 AGENTS.md。
///
/// 不猜测技术栈、命令或架构；在线分析由模型读取仓库证据后完成。
pub fn generate_initial_context(cwd: &Path) -> Result<PathBuf, String> {
    let (target_path, exists) = resolve_context_target(cwd);
    if exists {
        return Err(
            i18n::t1("cmd.init.exists").replace("{path}", &target_path.display().to_string())
        );
    }

    // 排他创建：检查之后目标仍可能出现，不能用覆盖写入或跟随悬空链接。
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target_path)
        .map_err(|error| {
            i18n::t1("cmd.init.write_failed")
                .replace("{path}", &target_path.display().to_string())
                .replace("{error}", &error.to_string())
        })?;
    file.write_all(MINIMAL_CONTEXT.as_bytes())
        .map_err(|error| {
            i18n::t1("cmd.init.write_failed")
                .replace("{path}", &target_path.display().to_string())
                .replace("{error}", &error.to_string())
        })?;

    Ok(target_path)
}

pub async fn run(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let options = InitOptions::parse(args)?;
    if options.help {
        reply(ctx, i18n::t1("cmd.init.help"));
        return Ok(());
    }
    let cwd = std::env::current_dir()
        .map_err(|error| i18n::t1("cmd.init.cwd_failed").replace("{error}", &error.to_string()))?;
    run_in_dir(ctx, options, &cwd).await
}

/// 显式传目录便于隔离测试，避免修改进程级 CWD。
async fn run_in_dir(ctx: CommandContext<'_>, options: InitOptions, cwd: &Path) -> CommandResult {
    let (target_path, target_exists) = resolve_context_target(cwd);
    if options.offline {
        if target_exists {
            return Err(
                i18n::t1("cmd.init.exists").replace("{path}", &target_path.display().to_string())
            );
        }
        if !options.dry_run {
            if ctx.state.approval_mode == crate::types::ApprovalMode::Plan {
                return Err(i18n::t1("cmd.init.plan_offline"));
            }
            generate_initial_context(cwd)?;
        }
        let content = if options.dry_run {
            format!("{}\n\n{MINIMAL_CONTEXT}", i18n::t1("cmd.init.preview"))
        } else {
            i18n::t1("cmd.init.created")
        };
        reply(
            ctx,
            content.replace("{path}", &target_path.display().to_string()),
        );
        return Ok(());
    }

    let mut prompt = build_init_prompt(cwd, &target_path, target_exists);
    if !options.guidance.is_empty() {
        prompt.push_str("\n\nAdditional user guidance:\n");
        prompt.push_str(&options.guidance);
    }
    if ctx.state.approval_mode == crate::types::ApprovalMode::Plan {
        prompt.push_str("\n\nPlan mode is active: analyze and propose improvements only. Do not write or edit any files.");
    }
    if options.dry_run {
        reply(
            ctx,
            format!(
                "{}\n\n{prompt}",
                i18n::t1("cmd.init.preview").replace("{path}", &target_path.display().to_string())
            ),
        );
        return Ok(());
    }

    let message_id = ctx.state.next_message_id;
    ctx.agent_tx
        .send(crate::runtime::messages::AgentRequest::SendMessage {
            message_id,
            message: prompt,
        })
        .await
        .map_err(|_| i18n::t1("cmd.init.send_failed"))?;
    ctx.state.next_message_id += 1;
    let content = if target_exists {
        i18n::t1("cmd.init.analyzing_existing")
    } else {
        i18n::t1("cmd.init.analyzing_new")
    };
    reply(
        ctx,
        content.replace("{path}", &target_path.display().to_string()),
    );
    Ok(())
}

fn reply(ctx: CommandContext<'_>, content: String) {
    ctx.state
        .chat_history
        .push(crate::types::ChatEntry::assistant(content).with_streaming(false));
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::runtime::messages::AgentRequest;
    use crate::types::ApprovalMode;
    use crate::ui::state::ChatState;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn parses_options_and_literal_guidance() {
        assert_eq!(InitOptions::parse(vec![]).unwrap(), InitOptions::default());
        let options = InitOptions::parse(args(&["--dry-run", "Focus", "on tests"])).unwrap();
        assert!(options.dry_run);
        assert_eq!(options.guidance, "Focus on tests");
        let options = InitOptions::parse(args(&["--", "--help", "--offline"])).unwrap();
        assert!(!options.help);
        assert!(!options.offline);
        assert_eq!(options.guidance, "--help --offline");
        assert!(InitOptions::parse(args(&["-h"])).unwrap().help);
        assert!(InitOptions::parse(args(&["--help"])).unwrap().help);
        let options = InitOptions::parse(args(&["--offline", "--dry-run"])).unwrap();
        assert!(options.offline && options.dry_run);
        assert!(InitOptions::parse(args(&["--unknown"])).is_err());
        assert!(InitOptions::parse(args(&["--offline", "analyze tests"])).is_err());
    }

    /// 其他指引不影响固定目标，也不会被初始化逻辑改写。
    #[test]
    fn target_is_always_local_agents_md() {
        let repo = tempfile::tempdir().unwrap();
        let dir = repo.path();
        for name in ["CLAUDE.md", "STAR.md", "STARCODE.md", "AGENTS.override.md"] {
            std::fs::write(dir.join(name), "existing rules").unwrap();
        }
        let target = dir.join("AGENTS.md");
        assert_eq!(resolve_context_target(dir), (target.clone(), false));
        std::fs::write(&target, "agent rules").unwrap();
        assert_eq!(resolve_context_target(dir), (target, true));
    }

    #[test]
    fn prompt_requires_evidence_and_preserves_existing_guidance() {
        let repo = tempfile::tempdir().unwrap();
        let target = repo.path().join("AGENTS.md");
        let prompt = build_init_prompt(repo.path(), &target, false);
        for needle in [
            "# AGENTS.md",
            "Discover other repository instruction",
            "Never infer a build, lint, or test command solely from a manifest filename",
            "Trace entry points, constructors, and runtime call sites",
            "modify only the exact target below",
            "Only for a new file",
            "Preserve the language, heading, and useful content",
        ] {
            assert!(prompt.contains(needle), "prompt missing: {needle}");
        }
        assert!(prompt.contains(&format!("Exact target path: {}", target.display())));
        let prompt = build_init_prompt(repo.path(), &target, true);
        assert!(prompt.contains("Read it first"));
        assert!(prompt.contains("Do not discard or silently overwrite"));
    }

    #[test]
    fn offline_creates_only_minimal_agents_without_stack_guesses() {
        let repo = tempfile::tempdir().unwrap();
        for name in [
            "Cargo.toml",
            "package.json",
            "go.mod",
            "pyproject.toml",
            "CLAUDE.md",
        ] {
            std::fs::write(repo.path().join(name), "existing").unwrap();
        }
        let target = generate_initial_context(repo.path()).unwrap();
        assert_eq!(target, repo.path().join("AGENTS.md"));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), MINIMAL_CONTEXT);
        assert!(!repo.path().join(".star").exists());
        assert!(!repo.path().join("STARCODE.md").exists());
        assert_eq!(
            std::fs::read_to_string(repo.path().join("CLAUDE.md")).unwrap(),
            "existing"
        );
        assert!(generate_initial_context(repo.path()).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), MINIMAL_CONTEXT);
    }

    #[cfg(unix)]
    #[test]
    fn offline_does_not_follow_symlinks() {
        let repo = tempfile::tempdir().unwrap();
        let target = repo.path().join("AGENTS.md");
        let destination = repo.path().join("missing.md");
        std::os::unix::fs::symlink(&destination, &target).unwrap();
        assert_eq!(resolve_context_target(repo.path()), (target, true));
        assert!(generate_initial_context(repo.path()).is_err());
        assert!(!destination.exists());
    }

    #[tokio::test]
    async fn help_and_invalid_options_send_no_requests() {
        let mut state = ChatState::test_fixture();
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let id = state.next_message_id;
        for values in [&["--help"][..], &["--unknown"][..]] {
            let result = run(
                CommandContext {
                    state: &mut state,
                    agent_tx: &tx,
                },
                args(values),
            )
            .await;
            assert_eq!(result.is_ok(), values == ["--help"]);
        }
        assert_eq!(state.next_message_id, id);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn previews_do_not_write_or_request_a_model() {
        for offline in [false, true] {
            let repo = tempfile::tempdir().unwrap();
            let mut state = ChatState::test_fixture();
            let (tx, mut rx) = tokio::sync::mpsc::channel(4);
            let id = state.next_message_id;
            run_in_dir(
                CommandContext {
                    state: &mut state,
                    agent_tx: &tx,
                },
                InitOptions {
                    offline,
                    dry_run: true,
                    ..Default::default()
                },
                repo.path(),
            )
            .await
            .unwrap();
            assert_eq!(std::fs::read_dir(repo.path()).unwrap().count(), 0);
            assert!(rx.try_recv().is_err());
            assert_eq!(state.next_message_id, id);
            let content = &state.chat_history.last().unwrap().content;
            assert!(content.contains("AGENTS.md"));
            if offline {
                assert!(content.contains(MINIMAL_CONTEXT));
            } else {
                assert!(content.contains("Evidence and scope:"));
            }
        }
    }

    #[tokio::test]
    async fn online_sends_guidance_and_consumes_one_message_id() {
        let repo = tempfile::tempdir().unwrap();
        let mut state = ChatState::test_fixture();
        state.next_message_id = 7;
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        run_in_dir(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            InitOptions {
                guidance: "Focus on workspace tests".to_string(),
                ..Default::default()
            },
            repo.path(),
        )
        .await
        .unwrap();
        match rx.try_recv().unwrap() {
            AgentRequest::SendMessage {
                message_id,
                message,
            } => {
                assert_eq!(message_id, 7);
                assert!(message.contains("Additional user guidance:\nFocus on workspace tests"));
                assert!(message.contains(&repo.path().join("AGENTS.md").display().to_string()));
            }
            other => panic!("unexpected request: {other:?}"),
        }
        assert_eq!(state.next_message_id, 8);
        assert!(rx.try_recv().is_err());
        assert_eq!(std::fs::read_dir(repo.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn closed_worker_reports_failure_without_false_progress() {
        let repo = tempfile::tempdir().unwrap();
        let mut state = ChatState::test_fixture();
        let id = state.next_message_id;
        let history_len = state.chat_history.len();
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        drop(rx);
        let result = run_in_dir(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            InitOptions::default(),
            repo.path(),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(state.next_message_id, id);
        assert_eq!(state.chat_history.len(), history_len);
        assert_eq!(std::fs::read_dir(repo.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn plan_mode_blocks_offline_writes_and_constrains_online_prompt() {
        let repo = tempfile::tempdir().unwrap();
        let mut state = ChatState::test_fixture();
        state.approval_mode = ApprovalMode::Plan;
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        assert!(run_in_dir(
            CommandContext {
                state: &mut state,
                agent_tx: &tx
            },
            InitOptions {
                offline: true,
                ..Default::default()
            },
            repo.path(),
        )
        .await
        .is_err());
        assert!(rx.try_recv().is_err());
        run_in_dir(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            InitOptions::default(),
            repo.path(),
        )
        .await
        .unwrap();
        match rx.try_recv().unwrap() {
            AgentRequest::SendMessage { message, .. } => {
                assert!(message.contains("Do not write or edit any files."));
            }
            other => panic!("unexpected request: {other:?}"),
        }
        assert_eq!(std::fs::read_dir(repo.path()).unwrap().count(), 0);
    }
}
