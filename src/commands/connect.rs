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

    // Assume provider configuration
    // Trigger Input Modal
    ctx.state.show_input_modal = true;
    ctx.state.input_modal_title = format!("Configure {}", target);
    ctx.state.input_modal_prompt = format!("Enter API Key for {}:", target);
    ctx.state.input_modal_value = String::new();
    ctx.state.input_context = Some(crate::ui::state::palette::InputContext::ProviderKey {
        provider_id: target,
    });

    Ok(())
}
