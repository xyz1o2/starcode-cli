/// 独立循环命令：解析先于存储访问；只在持久化成功后反馈或清理待发任务。
use crate::commands::execution::{CommandContext, CommandResult};
use crate::core::i18n;
use chrono::{Local, TimeZone, Utc};
use clap::{FromArgMatches, Subcommand};

#[derive(Debug, PartialEq, Eq, Subcommand)]
pub enum LoopCommand {
    /// 创建定时提示词任务。
    #[command(arg_required_else_help = true)]
    Add {
        /// 项目内唯一的任务名。
        name: String,
        /// 正整数分钟间隔。
        every_minutes: u64,
        /// 后续参数均作为提示词，保留命令行选项。
        #[arg(required = true, num_args = 1.., trailing_var_arg = true, allow_hyphen_values = true)]
        prompt: Vec<String>,
    },
    /// 列出任务。
    List,
    /// 删除任务。
    #[command(arg_required_else_help = true)]
    Remove { name: String },
    /// 暂停后续调度，不中断正在运行的一轮。
    #[command(arg_required_else_help = true)]
    Pause { name: String },
    /// 恢复调度，从现在开始等待一个完整间隔。
    #[command(arg_required_else_help = true)]
    Resume { name: String },
}

const DEFAULT_INTERVAL_MINUTES: u64 = 10;

/// 管理入口保留严格解析；其余文本直接创建任务，避免把自然语言误当子命令。
fn parse_command(args: Vec<String>) -> Result<Option<LoopCommand>, String> {
    let Some(first) = args.first() else {
        return Ok(Some(LoopCommand::List));
    };
    if first == "--" {
        return parse_prompt(args.into_iter().skip(1).collect()).map(Some);
    }
    if !matches!(
        first.as_str(),
        "add" | "list" | "pause" | "resume" | "remove" | "help"
    ) && !first.starts_with('-')
    {
        return parse_prompt(args).map(Some);
    }
    let command = LoopCommand::augment_subcommands(clap::Command::new("loop"));
    match command.try_get_matches_from(std::iter::once("loop".to_string()).chain(args)) {
        Ok(matches) => LoopCommand::from_arg_matches(&matches)
            .map(Some)
            .map_err(|error| error.to_string()),
        Err(error) if matches!(error.kind(), clap::error::ErrorKind::DisplayHelp) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

/// 简写复用显式 add 的持久化与反馈路径，名称不依赖提示词内容。
fn parse_prompt(prompt: Vec<String>) -> Result<LoopCommand, String> {
    if prompt.join(" ").trim().is_empty() {
        return Err(i18n::t1("cmd.loop.empty_prompt"));
    }
    Ok(LoopCommand::Add {
        name: format!("loop-{}", uuid::Uuid::new_v4().simple()),
        every_minutes: DEFAULT_INTERVAL_MINUTES,
        prompt,
    })
}

pub async fn run(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    match parse_command(args) {
        Ok(Some(command)) => execute_loop_command(ctx, command).await,
        result => {
            let help = i18n::t1("cmd.loop.help");
            let text = match result {
                Err(error) => format!("{error}\n{help}"),
                _ => help,
            };
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(text).with_streaming(false));
            Ok(())
        }
    }
}

pub async fn execute_loop_command(ctx: CommandContext<'_>, cmd: LoopCommand) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    execute_loop_command_in(ctx, cmd, &cwd).await
}

/// `cwd` 显式传入：单测要在临时目录里建/删任务，不能写进开发者自己的
/// `.star/loops.json`。
pub(crate) async fn execute_loop_command_in(
    ctx: CommandContext<'_>,
    cmd: LoopCommand,
    cwd: &std::path::Path,
) -> CommandResult {
    match cmd {
        LoopCommand::Add {
            name,
            every_minutes,
            prompt,
        } => {
            let prompt_text = prompt.join(" ").trim().to_string();
            let task = crate::core::loops::add_task(cwd, name.clone(), every_minutes, prompt_text)
                .await
                .map_err(|e| i18n::t1("cmd.loop.add_failed").replace("{error}", &e))?;

            crate::runtime::background::allow_loop_task(ctx.state, cwd, &task.name);
            let msg = i18n::t1("cmd.loop.created")
                .replace("{minutes}", &task.interval_minutes.to_string())
                .replace("{next}", &format_ts(task.next_run_at))
                .replace("{name}", &task.name)
                .replace("{prompt}", &task.prompt);

            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
            Ok(())
        }
        LoopCommand::List => {
            let tasks = crate::core::loops::list_tasks(cwd)
                .await
                .map_err(|e| i18n::t1("cmd.loop.list_failed").replace("{error}", &e))?;

            if tasks.is_empty() {
                ctx.state.chat_history.push(
                    crate::types::ChatEntry::assistant(i18n::t1("cmd.loop.empty"))
                        .with_streaming(false),
                );
                return Ok(());
            }

            let now = Utc::now().timestamp();
            let mut lines = vec![i18n::t1("cmd.loop.list_title")];
            for t in tasks {
                let status = if t.enabled {
                    i18n::t1("cmd.loop.enabled")
                } else {
                    i18n::t1("cmd.loop.disabled")
                };
                let next = if t.enabled {
                    let seconds = t.next_run_at.saturating_sub(now).max(0);
                    let minutes = seconds / 60 + i64::from(seconds % 60 != 0);
                    i18n::t1("cmd.loop.next_run")
                        .replace("{next}", &format_ts(t.next_run_at))
                        .replace("{minutes}", &minutes.to_string())
                } else {
                    i18n::t1("cmd.loop.not_scheduled")
                };
                lines.push(
                    i18n::t1("cmd.loop.list_entry")
                        .replace("{status}", &status)
                        .replace("{minutes}", &t.interval_minutes.to_string())
                        .replace("{next}", &next)
                        .replace("{name}", &t.name)
                        .replace("{prompt}", &t.prompt),
                );
            }
            lines.push(i18n::t1("cmd.loop.list_hint"));

            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(lines.join("\n")).with_streaming(false));
            Ok(())
        }
        command @ (LoopCommand::Pause { .. } | LoopCommand::Resume { .. }) => {
            let (name, enabled) = match command {
                LoopCommand::Pause { name } => (name, false),
                LoopCommand::Resume { name } => (name, true),
                _ => unreachable!(),
            };
            let task = crate::core::loops::set_task_enabled(cwd, &name, enabled)
                .await
                .map_err(|error| i18n::t1("cmd.loop.change_failed").replace("{error}", &error))?;
            let msg = if let Some(task) = task {
                if enabled {
                    crate::runtime::background::allow_loop_task(ctx.state, cwd, &task.name);
                    i18n::t1("cmd.loop.resumed")
                        .replace("{next}", &format_ts(task.next_run_at))
                        .replace("{name}", &task.name)
                } else {
                    crate::runtime::background::suppress_loop_task(ctx.state, cwd, &task.name);
                    i18n::t1("cmd.loop.paused").replace("{name}", &task.name)
                }
            } else {
                i18n::t1("cmd.loop.not_found").replace("{name}", name.trim())
            };
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
            Ok(())
        }
        LoopCommand::Remove { name } => {
            let removed = crate::core::loops::remove_task(cwd, &name)
                .await
                .map_err(|e| i18n::t1("cmd.loop.remove_failed").replace("{error}", &e))?;

            crate::runtime::background::suppress_loop_task(ctx.state, cwd, name.trim());
            let msg = if removed {
                i18n::t1("cmd.loop.removed").replace("{name}", name.trim())
            } else {
                i18n::t1("cmd.loop.not_found").replace("{name}", name.trim())
            };

            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
            Ok(())
        }
    }
}

pub(crate) fn format_ts(ts: i64) -> String {
    match Local.timestamp_opt(ts, 0).single() {
        Some(dt) => dt.format("%Y-%m-%d %H:%M:%S %Z").to_string(),
        None => ts.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::execution::CommandContext;
    use crate::ui::state::ChatState;

    async fn run_in(cwd: &std::path::Path, cmd: LoopCommand) -> Result<String, String> {
        let mut state = ChatState::test_fixture();
        let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(1);
        execute_loop_command_in(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            cmd,
            cwd,
        )
        .await?;
        Ok(state
            .chat_history
            .last()
            .map(|entry| entry.content.clone())
            .unwrap_or_default())
    }

    fn add(name: &str, minutes: u64, prompt: &str) -> LoopCommand {
        LoopCommand::Add {
            name: name.to_string(),
            every_minutes: minutes,
            prompt: prompt.split_whitespace().map(str::to_string).collect(),
        }
    }

    #[tokio::test]
    async fn an_empty_list_explains_how_to_create_one() {
        let dir = tempfile::tempdir().unwrap();
        let out = run_in(dir.path(), LoopCommand::List).await.unwrap();
        assert!(out.contains("No /loop tasks"), "{out}");
        assert!(out.contains("/loop <prompt...>"), "{out}");
        assert!(
            out.contains("/loop add <name> <minutes> <prompt...>"),
            "{out}"
        );
    }

    #[tokio::test]
    async fn adding_a_task_persists_it_and_the_list_shows_it() {
        let dir = tempfile::tempdir().unwrap();
        let out = run_in(dir.path(), add("standup", 30, "summarize the diff"))
            .await
            .unwrap();
        assert!(out.contains("Created /loop task `standup`"), "{out}");
        assert!(out.contains("- Interval: every 30 minutes"), "{out}");
        assert!(out.contains("- Prompt: summarize the diff"), "{out}");

        assert!(crate::core::loops::loops_file_path(dir.path()).exists());

        let listed = run_in(dir.path(), LoopCommand::List).await.unwrap();
        assert!(listed.starts_with("# /loop tasks"), "{listed}");
        assert!(listed.contains("- `standup` [enabled]"), "{listed}");
        assert!(listed.contains("- Interval: every 30 minutes"), "{listed}");
        assert!(listed.contains("- Prompt: summarize the diff"), "{listed}");
    }

    /// 间隔 0 会让 next_run_at 停在过去，调度器每轮都会重新触发 ——
    /// 必须在落盘前拒绝。
    #[tokio::test]
    async fn a_zero_interval_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let err = run_in(dir.path(), add("bad", 0, "loop forever"))
            .await
            .unwrap_err();
        assert!(err.contains("Failed to add /loop task"), "{err}");
        assert!(!crate::core::loops::loops_file_path(dir.path()).exists());
    }

    #[tokio::test]
    async fn prompt_arguments_are_joined_with_spaces() {
        let dir = tempfile::tempdir().unwrap();
        run_in(dir.path(), add("multi", 5, "check   the   build"))
            .await
            .unwrap();
        let tasks = crate::core::loops::list_tasks(dir.path()).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].prompt, "check the build");
    }

    #[tokio::test]
    async fn removing_reports_whether_the_task_existed() {
        let dir = tempfile::tempdir().unwrap();
        run_in(dir.path(), add("standup", 30, "summarize"))
            .await
            .unwrap();

        let removed = run_in(
            dir.path(),
            LoopCommand::Remove {
                name: "standup".to_string(),
            },
        )
        .await
        .unwrap();
        assert!(
            removed.contains("Removed /loop task `standup`"),
            "{removed}"
        );

        let again = run_in(
            dir.path(),
            LoopCommand::Remove {
                name: "standup".to_string(),
            },
        )
        .await
        .unwrap();
        assert!(
            again.contains("Could not find /loop task `standup`"),
            "{again}"
        );
    }

    fn parse(args: &[&str]) -> Result<Option<LoopCommand>, String> {
        parse_command(args.iter().map(|arg| (*arg).to_owned()).collect())
    }

    #[test]
    fn parser_defaults_to_list_and_accepts_help_without_a_task() {
        assert_eq!(parse(&[]).unwrap(), Some(LoopCommand::List));
        assert_eq!(parse(&["--help"]).unwrap(), None);
        assert_eq!(parse(&["add", "--help"]).unwrap(), None);
        assert_eq!(
            parse(&["pause", "work"]).unwrap(),
            Some(LoopCommand::Pause {
                name: "work".into()
            })
        );
        assert_eq!(
            parse(&["resume", "work"]).unwrap(),
            Some(LoopCommand::Resume {
                name: "work".into()
            })
        );
    }

    #[test]
    fn parser_rejects_missing_extra_and_invalid_arguments() {
        for args in [
            vec!["--unknown"],
            vec!["--"],
            vec!["--", "  "],
            vec!["add"],
            vec!["add", "name", "1"],
            vec!["add", "name", "-1", "prompt"],
            vec!["add", "name", "x", "prompt"],
            vec!["list", "extra"],
            vec!["pause"],
            vec!["resume"],
            vec!["remove"],
            vec!["remove", "name", "extra"],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
    }

    #[tokio::test]
    async fn plain_prompt_creates_an_automatically_named_task_without_selecting_a_subcommand() {
        let dir = tempfile::tempdir().unwrap();
        let command = parse(&["分析项目"]).unwrap().unwrap();
        let out = run_in(dir.path(), command).await.unwrap();
        let tasks = crate::core::loops::list_tasks(dir.path()).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].prompt, "分析项目");
        assert_eq!(tasks[0].interval_minutes, DEFAULT_INTERVAL_MINUTES);
        assert!(tasks[0].name.starts_with("loop-"));
        assert!(out.contains(&tasks[0].name), "{out}");
        assert!(out.contains("every 10 minutes"), "{out}");
        assert!(tasks[0].next_run_at >= tasks[0].created_at + 600);

        run_in(dir.path(), parse(&["分析项目"]).unwrap().unwrap())
            .await
            .unwrap();
        let tasks = crate::core::loops::list_tasks(dir.path()).await.unwrap();
        assert_eq!(tasks.len(), 2);
        assert_ne!(tasks[0].name, tasks[1].name);
    }

    #[test]
    fn plain_prompt_preserves_flags_and_explicit_literal_reserved_words() {
        for (args, expected) in [
            (
                vec!["run", "cargo", "test", "--lib"],
                "run cargo test --lib",
            ),
            (vec!["--", "list", "the", "changes"], "list the changes"),
            (vec!["--", "--help"], "--help"),
        ] {
            let command = parse(&args).unwrap().unwrap();
            let LoopCommand::Add {
                every_minutes,
                prompt,
                ..
            } = command
            else {
                panic!("expected a task for {args:?}");
            };
            assert_eq!(every_minutes, DEFAULT_INTERVAL_MINUTES);
            assert_eq!(prompt.join(" "), expected);
        }
    }

    #[test]
    fn parser_keeps_flags_in_the_prompt() {
        assert_eq!(
            parse(&[
                "add",
                "tests",
                "5",
                "run",
                "cargo",
                "test",
                "--lib",
                "--",
                "--nocapture"
            ])
            .unwrap(),
            Some(LoopCommand::Add {
                name: "tests".into(),
                every_minutes: 5,
                prompt: ["run", "cargo", "test", "--lib", "--", "--nocapture"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            })
        );
    }

    #[tokio::test]
    async fn pause_resume_and_missing_task_feedback_match_persisted_state() {
        let dir = tempfile::tempdir().unwrap();
        run_in(dir.path(), add("work", 1, "prompt")).await.unwrap();
        let paused = run_in(
            dir.path(),
            LoopCommand::Pause {
                name: "work".into(),
            },
        )
        .await
        .unwrap();
        assert!(paused.contains("Paused /loop task `work`"), "{paused}");
        let listed = run_in(dir.path(), LoopCommand::List).await.unwrap();
        assert!(listed.contains("[paused]"), "{listed}");
        assert!(listed.contains("Not scheduled while paused"), "{listed}");
        assert!(!listed.contains("Next run:"), "{listed}");
        assert!(!crate::core::loops::list_tasks(dir.path()).await.unwrap()[0].enabled);
        let resumed = run_in(
            dir.path(),
            LoopCommand::Resume {
                name: "work".into(),
            },
        )
        .await
        .unwrap();
        assert!(resumed.contains("Resumed /loop task `work`"), "{resumed}");
        assert!(crate::core::loops::list_tasks(dir.path()).await.unwrap()[0].enabled);
        for cmd in [
            LoopCommand::Pause {
                name: "missing".into(),
            },
            LoopCommand::Resume {
                name: "missing".into(),
            },
        ] {
            let missing = run_in(dir.path(), cmd).await.unwrap();
            assert!(
                missing.contains("Could not find /loop task `missing`"),
                "{missing}"
            );
        }
    }

    #[tokio::test]
    async fn failed_storage_does_not_emit_success_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = crate::core::loops::loops_file_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "invalid JSON").unwrap();
        let mut state = ChatState::test_fixture();
        let history_len = state.chat_history.len();
        let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(1);
        for cmd in [
            add("work", 1, "prompt"),
            LoopCommand::Pause {
                name: "work".into(),
            },
            LoopCommand::Resume {
                name: "work".into(),
            },
            LoopCommand::Remove {
                name: "work".into(),
            },
        ] {
            assert!(execute_loop_command_in(
                CommandContext {
                    state: &mut state,
                    agent_tx: &agent_tx
                },
                cmd,
                dir.path()
            )
            .await
            .is_err());
            assert_eq!(state.chat_history.len(), history_len);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "invalid JSON");
        }
    }

    #[test]
    fn timestamps_fall_back_to_the_raw_value_when_out_of_range() {
        assert_eq!(format_ts(i64::MAX), i64::MAX.to_string());
        // 合理的时间戳至少带上年份，用户才能判断"下次触发"是不是马上
        assert!(format_ts(1_700_000_000).contains("2023"));
    }
}
