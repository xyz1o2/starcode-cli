use crate::commands::execution::{CommandContext, CommandResult};
use std::path::Path;

pub async fn run(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());

    let Some(cmd_str) = detect_test_command(&cwd, &args) else {
        return Err(
            "Could not detect project type. Please provide a test command, e.g., /test 'cargo test'"
                .to_string(),
        );
    };

    // 2. Notify user
    ctx.state.chat_history.push(
        crate::types::ChatEntry::assistant(format!("🧪 Running tests: `{}`...", cmd_str))
            .with_streaming(false),
    );

    // 3. Execute command
    // We use the agent to run the command so it can see the output and potential errors
    use crate::runtime::messages::AgentRequest;

    // Construct a prompt that tells the agent to run the test and fix if fails
    let prompt = test_prompt(&cmd_str);

    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    let _ = ctx
        .agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: prompt,
        })
        .await;

    Ok(())
}

/// 显式参数永远优先；否则按项目 marker 猜。`None` 表示"猜不出来"，
/// 由调用方给出可操作的报错，而不是悄悄跑一条错误语言生态的命令。
fn detect_test_command(cwd: &Path, args: &[String]) -> Option<String> {
    if !args.is_empty() {
        let joined = args.join(" ");
        let trimmed = joined.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if cwd.join("Cargo.toml").exists() {
        Some("cargo test".to_string())
    } else if cwd.join("package.json").exists() {
        Some("npm test".to_string())
    } else if cwd.join("pyproject.toml").exists() || cwd.join("requirements.txt").exists() {
        Some("pytest".to_string())
    } else if cwd.join("go.mod").exists() {
        Some("go test ./...".to_string())
    } else {
        None
    }
}

fn test_prompt(cmd_str: &str) -> String {
    format!(
        "Please run the test command: `{}`. \
        If the tests fail, analyze the failure output and attempt to fix the code. \
        If they pass, just report the success.",
        cmd_str
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::execution::CommandContext;
    use crate::ui::state::ChatState;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn an_explicit_command_wins_over_detection() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "").unwrap();
        assert_eq!(
            detect_test_command(dir.path(), &args(&["cargo", "test", "--lib"])),
            Some("cargo test --lib".to_string())
        );
    }

    #[test]
    fn whitespace_only_arguments_fall_back_to_detection() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module x\n").unwrap();
        assert_eq!(
            detect_test_command(dir.path(), &args(&["  "])),
            Some("go test ./...".to_string())
        );
    }

    #[test]
    fn cargo_projects_declare_their_own_test_command() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "").unwrap();
        assert_eq!(
            detect_test_command(dir.path(), &[]),
            Some("cargo test".to_string())
        );
    }

    #[test]
    fn node_python_and_go_markers_are_recognized() {
        let node = tempfile::tempdir().unwrap();
        std::fs::write(node.path().join("package.json"), "{}").unwrap();
        assert_eq!(
            detect_test_command(node.path(), &[]),
            Some("npm test".to_string())
        );

        for marker in ["pyproject.toml", "requirements.txt"] {
            let py = tempfile::tempdir().unwrap();
            std::fs::write(py.path().join(marker), "").unwrap();
            assert_eq!(
                detect_test_command(py.path(), &[]),
                Some("pytest".to_string()),
                "{marker} should select pytest"
            );
        }

        let go = tempfile::tempdir().unwrap();
        std::fs::write(go.path().join("go.mod"), "").unwrap();
        assert_eq!(
            detect_test_command(go.path(), &[]),
            Some("go test ./...".to_string())
        );
    }

    #[test]
    fn an_unknown_project_has_no_guess() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect_test_command(dir.path(), &[]), None);
    }

    #[tokio::test]
    async fn the_prompt_asks_for_failure_analysis() {
        let prompt = test_prompt("cargo test --lib");
        assert!(prompt.contains("cargo test --lib"));
        assert!(prompt.contains("analyze the failure output"));
        assert!(prompt.contains("attempt to fix the code"));

        // handler 侧：消息必须带一个占用的 ID
        let mut state = ChatState::test_fixture();
        state.next_message_id = 3;
        let (agent_tx, mut agent_rx) = tokio::sync::mpsc::channel(4);
        // 显式参数让 cwd 不参与判定，测试与仓库内容无关
        run(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            args(&["cargo", "test"]),
        )
        .await
        .unwrap();

        match agent_rx.try_recv().unwrap() {
            crate::runtime::messages::AgentRequest::SendMessage {
                message_id,
                message,
            } => {
                assert_eq!(message_id, 3);
                assert!(message.contains("cargo test"));
            }
            other => panic!("expected SendMessage, got {other:?}"),
        }
        assert_eq!(state.next_message_id, 4);
    }
}
