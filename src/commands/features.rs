use crate::commands::execution::{CommandContext, CommandResult};
use crate::types::ChatEntry;

fn push_msg(ctx: &mut CommandContext<'_>, content: impl Into<String>) {
    ctx.state
        .chat_history
        .push(ChatEntry::assistant(content).with_streaming(false));
}

// ── Deep Link Commands ──────────────────────────────────────────

pub async fn deep_link(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let mut iter = args.into_iter();
    let sub = iter.next().unwrap_or_else(|| "status".to_string());
    let rest: Vec<String> = iter.collect();

    match sub.as_str() {
        "register" => {
            let mut handler = crate::core::deep_link::DeepLinkHandler::new();
            match handler.register() {
                Ok(()) => push_msg(&mut ctx, "✅ Deep link handler registered for cc:// protocol"),
                Err(e) => push_msg(&mut ctx, format!("❌ Failed to register: {}", e)),
            }
        }
        "unregister" => {
            let mut handler = crate::core::deep_link::DeepLinkHandler::new();
            match handler.unregister() {
                Ok(()) => push_msg(&mut ctx, "✅ Deep link handler unregistered"),
                Err(e) => push_msg(&mut ctx, format!("❌ Failed to unregister: {}", e)),
            }
        }
        "parse" => {
            let url = rest.join(" ");
            let handler = crate::core::deep_link::DeepLinkHandler::new();
            match handler.parse_url(&url) {
                Some(action) => {
                    let msg = match &action {
                        crate::core::deep_link::DeepLinkAction::OpenFile(f) => {
                            format!("📁 Open File: {}", f)
                        }
                        crate::core::deep_link::DeepLinkAction::ResumeSession(id) => {
                            format!("🔄 Resume Session: {}", id)
                        }
                        crate::core::deep_link::DeepLinkAction::RunCommand(cmd) => {
                            format!("▶️ Run Command: {}", cmd)
                        }
                    };
                    push_msg(&mut ctx, msg);
                }
                None => push_msg(&mut ctx, format!("❌ Invalid deep link: {}", url)),
            }
        }
        "test" => {
            let handler = crate::core::deep_link::DeepLinkHandler::new();
            let test_urls = vec![
                "cc://open/src/main.rs",
                "cc://session/abc123",
                "cc://run/cargo test",
            ];
            let mut output = String::from("🔗 Deep Link Test Results:\n\n");
            for url in test_urls {
                let result = handler.parse_url(url);
                output.push_str(&format!(
                    "  {} → {}\n",
                    url,
                    match result {
                        Some(a) => format!("{:?}", a),
                        None => "Invalid".to_string(),
                    }
                ));
            }
            push_msg(&mut ctx, output);
        }
        _ => {
            let handler = crate::core::deep_link::DeepLinkHandler::new();
            push_msg(
                &mut ctx,
                format!(
                    "🔗 Deep Link Status\n\nProtocol: {}://\nRegistered: {}\n\nCommands:\n  /deep-link register   - Register protocol handler\n  /deep-link unregister - Unregister protocol handler\n  /deep-link parse <url> - Parse a deep link URL\n  /deep-link test        - Test deep link parsing",
                    handler.protocol, handler.registered
                ),
            );
        }
    }
    Ok(())
}

// ── Teleport Commands ──────────────────────────────────────────

pub async fn teleport(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let mut iter = args.into_iter();
    let sub = iter.next().unwrap_or_else(|| "status".to_string());
    let rest: Vec<String> = iter.collect();

    match sub.as_str() {
        "connect" => {
            let endpoint = rest.first().cloned().unwrap_or_default();
            let name = rest.get(1).cloned().unwrap_or_else(|| "default".to_string());
            if endpoint.is_empty() {
                push_msg(&mut ctx, "Usage: /teleport connect <endpoint> [name]");
                return Ok(());
            }
            let mut manager = crate::core::teleport::TeleportManager::new();
            match manager.connect(&endpoint, &name).await {
                Ok(id) => push_msg(&mut ctx, format!("✅ Connected to {}\nSession ID: {}", endpoint, id)),
                Err(e) => push_msg(&mut ctx, format!("❌ Connection failed: {}", e)),
            }
        }
        "list" => {
            let manager = crate::core::teleport::TeleportManager::new();
            let sessions = manager.list_sessions();
            if sessions.is_empty() {
                push_msg(&mut ctx, "📡 No active teleport sessions");
            } else {
                let mut output = String::from("📡 Teleport Sessions:\n\n");
                for session in sessions {
                    output.push_str(&format!("  {} - {} ({:?})\n", session.id, session.name, session.status));
                }
                push_msg(&mut ctx, output);
            }
        }
        _ => {
            push_msg(
                &mut ctx,
                "📡 Teleport - Remote Session Manager\n\nCommands:\n  /teleport connect <endpoint> [name] - Connect to remote\n  /teleport list                       - List sessions\n  /teleport disconnect <id>            - Disconnect session\n  /teleport send <id> <message>        - Send message",
            );
        }
    }
    Ok(())
}

// ── Wiki Commands ──────────────────────────────────────────────

pub async fn wiki(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let mut iter = args.into_iter();
    let sub = iter.next().unwrap_or_else(|| "list".to_string());
    let rest: Vec<String> = iter.collect();

    match sub.as_str() {
        "Grep" => {
            let query = rest.join(" ");
            if query.is_empty() {
                push_msg(&mut ctx, "Usage: /wiki search <query>");
                return Ok(());
            }
            let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
            let wiki_path = cwd.join(".star").join("wiki");
            let manager = crate::core::wiki::WikiManager::new(&wiki_path.to_string_lossy());
            let results = manager.search(&query);
            if results.is_empty() {
                push_msg(&mut ctx, format!("📚 No wiki pages found for: {}", query));
            } else {
                let mut output = format!("📚 Wiki search results for '{}':\n\n", query);
                for page in results {
                    output.push_str(&format!("  📄 {} ({})\n", page.title, page.id));
                }
                push_msg(&mut ctx, output);
            }
        }
        "create" => {
            let title = rest.first().cloned().unwrap_or_else(|| "Untitled".to_string());
            let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
            let wiki_path = cwd.join(".star").join("wiki");
            let _ = std::fs::create_dir_all(&wiki_path);
            let mut manager = crate::core::wiki::WikiManager::new(&wiki_path.to_string_lossy());
            let page = manager.create_page(&title, "", vec![]);
            push_msg(&mut ctx, format!("✅ Created wiki page: {} ({})", page.title, page.id));
        }
        _ => {
            let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
            let wiki_path = cwd.join(".star").join("wiki");
            let manager = crate::core::wiki::WikiManager::new(&wiki_path.to_string_lossy());
            if manager.pages.is_empty() {
                push_msg(&mut ctx, "📚 No wiki pages found\n\nCreate one with: /wiki create <title>");
            } else {
                let mut output = String::from("📚 Wiki Pages:\n\n");
                for page in &manager.pages {
                    output.push_str(&format!("  📄 {} ({})\n", page.title, page.id));
                }
                push_msg(&mut ctx, output);
            }
        }
    }
    Ok(())
}

// ── Buddy Commands ─────────────────────────────────────────────

pub async fn buddy(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let mut iter = args.into_iter();
    let sub = iter.next().unwrap_or_else(|| "status".to_string());
    let rest: Vec<String> = iter.collect();

    match sub.as_str() {
        "enable" | "on" => {
            push_msg(&mut ctx, "🤖 Buddy mode enabled!\n\nHello! I'm your coding buddy. Let's build something great together!");
        }
        "disable" | "off" => {
            push_msg(&mut ctx, "🤖 Buddy mode disabled");
        }
        "encourage" => {
            let messages = vec![
                "You're doing amazing! Every line of code is a step forward. 💪",
                "Keep going! You're making great progress! 🚀",
                "That's the spirit! You've got this! ⭐",
                "Don't give up! The best code comes from persistence! 🎯",
            ];
            let idx = (chrono::Utc::now().timestamp() as usize) % messages.len();
            push_msg(&mut ctx, format!("🤖 {}", messages[idx]));
        }
        "celebrate" => {
            let achievement = rest.join(" ");
            if achievement.is_empty() {
                push_msg(&mut ctx, "Usage: /buddy celebrate <achievement>");
            } else {
                push_msg(&mut ctx, format!("🎉 Incredible work on {}! You should be proud!", achievement));
            }
        }
        _ => {
            push_msg(
                &mut ctx,
                "🤖 Buddy Mode - Your Coding Companion\n\nCommands:\n  /buddy enable     - Enable buddy mode\n  /buddy disable    - Disable buddy mode\n  /buddy encourage  - Get encouragement\n  /buddy celebrate <achievement> - Celebrate an achievement",
            );
        }
    }
    Ok(())
}
