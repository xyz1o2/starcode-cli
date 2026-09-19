use crate::commands::execution::{CommandContext, CommandResult};
use crate::core::config::project_scaffold::scaffold_project_star;

pub async fn run(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
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
        let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
        let _ = scaffold_project_star(&cwd)?;
        let config_path = crate::core::config::storage::Storage::new(cwd).project_mcp_config_path();

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
