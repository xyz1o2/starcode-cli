use crate::commands::execution::{CommandContext, CommandResult};
use crate::core::config::project_scaffold::scaffold_project_star;

pub async fn run(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    run_in(ctx, args, &cwd).await
}

/// `cwd` 显式传入：`/connect mcp` 会 scaffold 项目 `.star/` 目录，单测必须在
/// 临时目录里做，不能写进开发者自己的仓库。
pub(crate) async fn run_in(
    ctx: CommandContext<'_>,
    args: Vec<String>,
    cwd: &std::path::Path,
) -> CommandResult {
    // Check if we have args. If not, show help/status
    if args.is_empty() {
        ctx.state.chat_history.push(crate::types::ChatEntry::assistant(
            "🔗 **Connect Command**\n\nUsage:\n- `/connect <provider>` (e.g., openai, anthropic) - Configure API Key interactively\n- `/connect mcp <server-name>` - Show MCP config instructions"
        ).with_streaming(false));
        return Ok(());
    }

    let target = args[0].to_lowercase();

    if target == "mcp" {
        if args.len() < 2 {
            ctx.state.chat_history.push(
                crate::types::ChatEntry::assistant("Usage: `/connect mcp <server-name>`")
                    .with_streaming(false),
            );
            return Ok(());
        }
        let server_name = &args[1];
        let _ = scaffold_project_star(cwd)?;
        let config_path =
            crate::core::config::storage::Storage::new(cwd.to_path_buf()).project_mcp_config_path();

        ctx.state.chat_history.push(crate::types::ChatEntry::assistant(
            format!("📝 Project MCP config has been prepared at:\n{}\n\nThe file supports inline comments, so you can keep the generated examples.\n\nStarter configuration:\n```jsonc\n{{\n  \"mcpServers\": {{\n    \"{}\": {{\n      \"command\": \"npx\",\n      \"args\": [\"-y\", \"@modelcontextprotocol/server-{}\"]\n    }}\n  }}\n}}\n```\n\nYou can also ask StarCode directly: `Configure {} MCP for this project and update .star/mcp.json`.", config_path.display(), server_name, server_name, server_name)
        ).with_streaming(false));
        return Ok(());
    }

    // 走统一的 provider 表单：预填该 provider 的现值，Base URL / API Key /
    // Model 一次改完（原来只弹一个 API Key 输入框的那套已经下线）
    crate::ui::events::input::open_provider_form(ctx.state, Some(target)).await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::execution::CommandContext;
    use crate::ui::state::ChatState;

    async fn run_in_dir(cwd: &std::path::Path, args: &[&str]) -> (ChatState, String) {
        let mut state = ChatState::test_fixture();
        let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(1);
        run_in(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            args.iter().map(|s| s.to_string()).collect(),
            cwd,
        )
        .await
        .unwrap();
        let last = state
            .chat_history
            .last()
            .map(|entry| entry.content.clone())
            .unwrap_or_default();
        (state, last)
    }

    #[tokio::test]
    async fn bare_connect_explains_the_two_forms() {
        let dir = tempfile::tempdir().unwrap();
        let (_, out) = run_in_dir(dir.path(), &[]).await;

        assert!(out.contains("/connect <provider>"), "{out}");
        assert!(out.contains("/connect mcp <server-name>"), "{out}");
    }

    #[tokio::test]
    async fn mcp_without_a_server_name_states_the_syntax() {
        let dir = tempfile::tempdir().unwrap();
        let (_, out) = run_in_dir(dir.path(), &["mcp"]).await;

        assert!(out.contains("Usage: `/connect mcp <server-name>`"), "{out}");
        // 语法错误不该顺手 scaffold 项目
        assert!(!dir.path().join(".star").join("mcp.json").exists());
    }

    /// `/connect mcp <name>` 的产出是一份**能直接改**的起步配置，而不是口头
    /// 指导 —— 所以必须落盘到项目 `.star/` 并把真实路径告诉用户。
    #[tokio::test]
    async fn mcp_scaffolds_a_project_config_and_reports_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let (_, out) = run_in_dir(dir.path(), &["mcp", "github"]).await;

        let config_path = crate::core::config::storage::Storage::new(dir.path().to_path_buf())
            .project_mcp_config_path();
        assert!(
            config_path.exists(),
            "{} should exist",
            config_path.display()
        );
        assert!(out.contains(&config_path.display().to_string()), "{out}");
        assert!(out.contains("@modelcontextprotocol/server-github"), "{out}");
        assert!(out.contains("\"github\""), "{out}");
    }

    /// provider 分支不进聊天记录，而是打开预填了该 provider 的表单。
    /// 名字大小写不敏感（`/connect OpenAI` 一样能用）。
    #[tokio::test]
    async fn a_provider_opens_the_form_with_the_id_lowercased() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = ChatState::test_fixture();
        let before = state.chat_history.len();
        let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(1);

        run_in(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            vec!["OpenAI".to_string()],
            dir.path(),
        )
        .await
        .unwrap();

        assert_eq!(
            state.chat_history.len(),
            before,
            "provider setup is a form, not a chat message"
        );
        assert_eq!(state.provider_form.editing_id.as_deref(), Some("openai"));
        assert_eq!(
            state.quick_menu_back,
            Some(crate::ui::state::QuickMenuKind::Provider)
        );
    }
}
