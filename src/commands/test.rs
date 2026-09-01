use crate::commands::execution::{CommandContext, CommandResult};
use std::path::Path;

pub async fn run(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    // 1. Detect project type
    let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());

    let test_cmd = if !args.is_empty() {
        // User provided custom test args
        Some(args.join(" "))
    } else {
        // Auto-detect
        if cwd.join("Cargo.toml").exists() {
            Some("cargo test".to_string())
        } else if cwd.join("package.json").exists() {
            // Check if 'test' script exists could be better, but simple default is npm test
            Some("npm test".to_string())
        } else if cwd.join("pyproject.toml").exists() || cwd.join("requirements.txt").exists() {
            Some("pytest".to_string())
        } else if cwd.join("go.mod").exists() {
            Some("go test ./...".to_string())
        } else {
            None
        }
    };

    let cmd_str = match test_cmd {
        Some(c) => c,
        None => {
            return Err("Could not detect project type. Please provide a test command, e.g., /test 'cargo test'".to_string());
        }
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
    let prompt = format!(
        "Please run the test command: `{}`. \
        If the tests fail, analyze the failure output and attempt to fix the code. \
        If they pass, just report the success.",
        cmd_str
    );

    let _ = ctx
        .agent_tx
        .send(AgentRequest::SendMessage {
            message_id: ctx.state.next_message_id,
            message: prompt,
        })
        .await;

    ctx.state.next_message_id += 1;

    Ok(())
}
