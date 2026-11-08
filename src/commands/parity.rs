//! 对标 Claude Code 的补齐命令实现。
//!
//! 这些命令此前只在 [`crate::commands::system::ALL_COMMANDS`] 里声明
//! （category = "Pending"），执行时返回占位提示。本模块把它们逐个接到
//! 仓库里已有的 `core/*` 管理器与 UI 状态上。
//!
//! 约定：
//! - 只读命令直接把 markdown 结果 push 进 `chat_history`（不进 LLM 上下文）；
//! - 需要模型参与的命令（/brief、/ultrareview…）走 [`ask_agent`]；
//! - 需要旁路生成、不污染主上下文的走 `AgentRequest::GenerateNote`。

use crate::commands::execution::{CommandContext, CommandResult};
use crate::runtime::messages::AgentRequest;
use crate::types::ChatEntry;
use std::path::PathBuf;

mod core;
mod diag;
mod git_github;
mod goal_job;
mod memory;
mod remote;
mod skills;
mod team_mail;
mod trigger;
mod ultra;
mod web;

pub use core::*;
pub use diag::*;
pub use git_github::*;
pub use goal_job::*;
pub use memory::*;
pub use remote::*;
pub use skills::*;
pub use team_mail::*;
pub use trigger::*;
pub use ultra::*;
pub use web::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autocompact_tokens_accept_compact_units_and_separators() {
        assert_eq!(parse_autocompact_tokens("200k").unwrap(), 200_000);
        assert_eq!(parse_autocompact_tokens("200K tokens").unwrap(), 200_000);
        assert_eq!(parse_autocompact_tokens("1m").unwrap(), 1_000_000);
        assert_eq!(parse_autocompact_tokens("500000").unwrap(), 500_000);
        assert_eq!(parse_autocompact_tokens("50,000").unwrap(), 50_000);
        assert_eq!(parse_autocompact_tokens("50_000").unwrap(), 50_000);
        assert_eq!(parse_autocompact_tokens("12.5k").unwrap(), 12_500);
    }

    #[test]
    fn autocompact_tokens_reject_small_or_invalid_values() {
        assert!(parse_autocompact_tokens("9k").is_err());
        assert!(parse_autocompact_tokens("0").is_err());
        assert!(parse_autocompact_tokens("-5").is_err());
        assert!(parse_autocompact_tokens("lots").is_err());
        assert!(parse_autocompact_tokens("11m").is_err());
        assert!(parse_autocompact_tokens("").is_err());
    }

    #[test]
    fn autocompact_status_formats_thousands_periods() {
        assert_eq!(format_token_count(999), "999");
        assert_eq!(format_token_count(1_000), "1,000");
        assert_eq!(format_token_count(200_000), "200,000");
        assert_eq!(format_token_count(1_000_000), "1,000,000");
    }

    #[tokio::test]
    async fn autocompact_without_arguments_reports_the_live_policy() {
        let mut state = crate::ui::state::ChatState::new();
        let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(1);
        autocompact(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            Vec::new(),
        )
        .await
        .unwrap();

        let output = &state.chat_history.last().expect("status message").content;
        assert!(output.contains("Auto-compact"), "{output}");
        assert!(output.contains("Threshold"), "{output}");
        assert!(output.contains("/autocompact auto|on|off"), "{output}");
    }

    #[tokio::test]
    async fn autocompact_reports_an_invalid_size() {
        let mut state = crate::ui::state::ChatState::new();
        let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(1);
        autocompact(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            vec!["banana".to_string()],
        )
        .await
        .unwrap();

        let output = &state.chat_history.last().expect("error message").content;
        assert!(output.contains("Expected a token count"), "{output}");
    }

    #[test]
    fn detects_cargo_verifiers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        let found = detect_verifiers(dir.path());
        let commands: Vec<&str> = found.iter().map(|v| v.command.as_str()).collect();
        assert!(commands.contains(&"cargo check --all-targets"));
        assert!(commands.contains(&"cargo test"));
        assert!(found.iter().all(|v| v.source == "Cargo.toml"));
    }

    #[test]
    fn detects_package_json_scripts_only_when_declared() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"build":"tsc","lint":"eslint ."}}"#,
        )
        .unwrap();
        let commands: Vec<String> = detect_verifiers(dir.path())
            .into_iter()
            .map(|v| v.command)
            .collect();
        assert!(commands.contains(&"npm run build".to_string()));
        assert!(commands.contains(&"npm run lint".to_string()));
        assert!(!commands.iter().any(|c| c.contains("test")));
    }

    #[test]
    fn verification_section_is_appended_then_replaced_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("STAR.md");
        std::fs::write(&path, "# Project\n\nintro\n\n## Style\n\nkeep it short\n").unwrap();

        let first = verification_section(&[Verifier {
            kind: "test",
            source: "Cargo.toml".into(),
            command: "cargo test".into(),
        }]);
        assert!(!write_verification_section(&path, &first).unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("## Verification"));
        assert!(text.contains("`cargo test`"));

        let second = verification_section(&[Verifier {
            kind: "lint",
            source: "Cargo.toml".into(),
            command: "cargo clippy".into(),
        }]);
        assert!(write_verification_section(&path, &second).unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("## Verification").count(), 1);
        assert!(text.contains("`cargo clippy`"));
        assert!(!text.contains("`cargo test`"));
        assert!(text.contains("## Style"), "later sections must survive");
    }

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace().map(|w| w.to_string()).collect()
    }

    #[test]
    fn schedule_args_split_message_from_time_modifier() {
        // `in <secs>` / `at <HH:MM>` 是修饰，不能留在消息正文里
        let (msg, delay) = split_schedule_args(&words("run the tests in 30"));
        assert_eq!(msg, "run the tests");
        assert_eq!(delay.unwrap(), 30);

        let (msg, delay) = split_schedule_args(&words("check CI AT 23:59"));
        assert_eq!(msg, "check CI");
        assert!(delay.is_ok(), "`at` must be case-insensitive");

        // 没有修饰时整串都是消息，用默认延迟
        let (msg, delay) = split_schedule_args(&words("ping me"));
        assert_eq!(msg, "ping me");
        assert_eq!(delay.unwrap(), DEFAULT_SCHEDULE_DELAY_SECS);
    }

    #[test]
    fn schedule_delay_rejects_nonsense() {
        assert!(
            parse_schedule_delay(&words("in 0")).is_err(),
            "in 0 is not a delay"
        );
        assert!(parse_schedule_delay(&words("in soon")).is_err());
        assert!(
            parse_schedule_delay(&words("in")).is_err(),
            "`in` needs a value"
        );
        assert!(parse_schedule_delay(&words("at 25:00")).is_err());
        assert!(
            parse_schedule_delay(&words("every 5m")).is_err(),
            "only in/at are supported"
        );
    }

    #[test]
    fn schedule_when_reads_naturally() {
        assert!(format_schedule_when(45).contains("45"));
        assert!(
            format_schedule_when(3_600).contains("60"),
            "minutes for long delays"
        );
    }
}
