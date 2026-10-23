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
                Ok(()) => push_msg(
                    &mut ctx,
                    "✅ Deep link handler registered for cc:// protocol",
                ),
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
            let name = rest
                .get(1)
                .cloned()
                .unwrap_or_else(|| "default".to_string());
            if endpoint.is_empty() {
                push_msg(&mut ctx, "Usage: /teleport connect <endpoint> [name]");
                return Ok(());
            }
            let mut manager = crate::core::teleport::TeleportManager::new();
            match manager.connect(&endpoint, &name).await {
                Ok(id) => push_msg(
                    &mut ctx,
                    format!("✅ Connected to {}\nSession ID: {}", endpoint, id),
                ),
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
                    output.push_str(&format!(
                        "  {} - {} ({:?})\n",
                        session.id, session.name, session.status
                    ));
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

pub async fn wiki(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    wiki_in(ctx, args, &cwd).await
}

/// `cwd` 显式传入：`/wiki create` 会写 `.star/wiki/*.md`，单测必须在临时
/// 目录里做。
pub(crate) async fn wiki_in(
    mut ctx: CommandContext<'_>,
    args: Vec<String>,
    cwd: &std::path::Path,
) -> CommandResult {
    let mut iter = args.into_iter();
    let sub = iter.next().unwrap_or_else(|| "list".to_string());
    let rest: Vec<String> = iter.collect();
    let wiki_path = cwd.join(".star").join("wiki");

    match sub.as_str() {
        // 分支名曾经写成 "Grep"，而帮助文案写的是 `search` —— 用户照着提示敲
        // `/wiki search foo` 会落到 list 分支。两个名字都留，`search` 是正名。
        "search" | "Grep" => {
            let query = rest.join(" ");
            if query.is_empty() {
                push_msg(&mut ctx, "Usage: /wiki search <query>");
                return Ok(());
            }
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
            let title = rest.join(" ").trim().to_string();
            if title.is_empty() {
                // 空标题会生成一个找不回来的 `page-<ts>`，不如直接报用法。
                push_msg(&mut ctx, "Usage: /wiki create <title>");
                return Ok(());
            }
            let _ = std::fs::create_dir_all(&wiki_path);
            let mut manager = crate::core::wiki::WikiManager::new(&wiki_path.to_string_lossy());
            let page = manager.create_page(&title, "", vec![]);
            push_msg(
                &mut ctx,
                format!("✅ Created wiki page: {} ({})", page.title, page.id),
            );
        }
        _ => {
            let manager = crate::core::wiki::WikiManager::new(&wiki_path.to_string_lossy());
            if manager.pages.is_empty() {
                push_msg(
                    &mut ctx,
                    "📚 No wiki pages found\n\nCreate one with: /wiki create <title>",
                );
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
                push_msg(
                    &mut ctx,
                    format!(
                        "🎉 Incredible work on {}! You should be proud!",
                        achievement
                    ),
                );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::execution::CommandContext;
    use crate::ui::state::ChatState;
    use tokio::sync::mpsc;

    /// 测试用的 `ChatState` + 一个活的 agent 通道。
    ///
    /// 接收端必须一起持有：发送端在接收端 drop 之后仍能构造，但 `.send()`
    /// 会返回错误 —— 留着 rx 才能让"命令没有惊动 worker"这类断言有意义。
    struct Harness {
        state: ChatState,
        agent_tx: mpsc::Sender<crate::runtime::messages::AgentRequest>,
        _agent_rx: mpsc::Receiver<crate::runtime::messages::AgentRequest>,
    }

    impl Harness {
        fn new() -> Self {
            let (agent_tx, _agent_rx) = mpsc::channel(4);
            Self {
                state: ChatState::test_fixture(),
                agent_tx,
                _agent_rx,
            }
        }

        fn ctx(&mut self) -> CommandContext<'_> {
            CommandContext {
                state: &mut self.state,
                agent_tx: &self.agent_tx,
            }
        }

        fn last(&self) -> String {
            self.state
                .chat_history
                .last()
                .map(|entry| entry.content.clone())
                .unwrap_or_default()
        }
    }

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    async fn run_wiki(cwd: &std::path::Path, items: &[&str]) -> String {
        let mut h = Harness::new();
        let before = h.state.chat_history.len();
        wiki_in(h.ctx(), args(items), cwd).await.unwrap();
        assert_eq!(
            h.state.chat_history.len(),
            before + 1,
            "每个子命令都必须恰好回复一条"
        );
        h.last()
    }

    // ── /wiki ────────────────────────────────────────────────────

    #[tokio::test]
    async fn an_empty_wiki_explains_how_to_create_a_page() {
        let dir = tempfile::tempdir().unwrap();
        let out = run_wiki(dir.path(), &[]).await;
        assert!(out.contains("No wiki pages found"), "{out}");
        assert!(out.contains("/wiki create <title>"), "{out}");
    }

    /// `create` 必须真的落盘 —— 只往内存里 push 的话，"已创建"是一句谎话，
    /// 重启后页面就没了。
    #[tokio::test]
    async fn create_writes_a_markdown_file_that_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let created = run_wiki(dir.path(), &["create", "Build", "Guide"]).await;
        assert!(
            created.contains("✅ Created wiki page: Build Guide"),
            "{created}"
        );

        let wiki_dir = dir.path().join(".star").join("wiki");
        let files: Vec<String> = std::fs::read_dir(&wiki_dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(files.len(), 1, "{files:?}");
        assert!(files[0].starts_with("build-guide-"), "{files:?}");
        assert!(files[0].ends_with(".md"), "{files:?}");

        // 重新构造 manager == 重启进程：页面还在，且 id 与文件名一致
        let reloaded = crate::core::wiki::WikiManager::new(&wiki_dir.to_string_lossy());
        assert_eq!(reloaded.pages.len(), 1);
        assert_eq!(reloaded.pages[0].title, "Build Guide");
        assert_eq!(files[0].trim_end_matches(".md"), reloaded.pages[0].id);
    }

    #[tokio::test]
    async fn create_without_a_title_states_the_syntax_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let out = run_wiki(dir.path(), &["create"]).await;
        assert!(out.contains("Usage: /wiki create <title>"), "{out}");
        assert!(!dir.path().join(".star").join("wiki").exists());
    }

    /// 帮助文案让用户敲 `search`；分支名一度写成 `Grep`，于是照着提示敲的
    /// 命令根本走不到搜索逻辑，直接落到 list 分支。
    #[tokio::test]
    async fn search_finds_pages_by_title_and_content() {
        let dir = tempfile::tempdir().unwrap();
        run_wiki(dir.path(), &["create", "Deploy Notes"]).await;
        run_wiki(dir.path(), &["create", "Onboarding"]).await;

        let hit = run_wiki(dir.path(), &["search", "Deploy"]).await;
        assert!(hit.contains("Wiki search results for 'Deploy'"), "{hit}");
        assert!(hit.contains("Deploy Notes"), "{hit}");
        assert!(!hit.contains("Onboarding"), "{hit}");

        let miss = run_wiki(dir.path(), &["search", "nonexistent-topic"]).await;
        assert!(
            miss.contains("No wiki pages found for: nonexistent-topic"),
            "{miss}"
        );
    }

    #[tokio::test]
    async fn bare_search_states_the_syntax() {
        let dir = tempfile::tempdir().unwrap();
        let out = run_wiki(dir.path(), &["search"]).await;
        assert!(out.contains("Usage: /wiki search <query>"), "{out}");
    }

    #[tokio::test]
    async fn list_renders_every_page_in_a_stable_order() {
        let dir = tempfile::tempdir().unwrap();
        run_wiki(dir.path(), &["create", "Zebra"]).await;
        run_wiki(dir.path(), &["create", "Alpha"]).await;

        let out = run_wiki(dir.path(), &["list"]).await;
        assert!(out.starts_with("📚 Wiki Pages:"), "{out}");
        let alpha = out.find("Alpha").expect("Alpha listed");
        let zebra = out.find("Zebra").expect("Zebra listed");
        assert!(alpha < zebra, "pages are sorted by title:\n{out}");
    }

    // ── /deep-link ───────────────────────────────────────────────

    #[tokio::test]
    async fn deep_link_without_a_subcommand_prints_the_status_and_syntax() {
        let mut h = Harness::new();
        let before = h.state.chat_history.len();
        deep_link(h.ctx(), Vec::new()).await.unwrap();
        let out = &h.state.chat_history[before].content;
        assert!(out.contains("Deep Link Status"), "{out}");
        assert!(out.contains("/deep-link register"), "{out}");
        assert!(out.contains("/deep-link parse"), "{out}");
    }

    #[tokio::test]
    async fn deep_link_parse_recognizes_each_action_kind() {
        let cases = [
            ("cc://open/src/main.rs", "📁 Open File: src/main.rs"),
            ("cc://session/abc123", "🔄 Resume Session: abc123"),
            ("cc://run/cargo", "▶️ Run Command: cargo"),
        ];
        for (url, expected) in cases {
            let mut h = Harness::new();
            deep_link(h.ctx(), args(&["parse", url])).await.unwrap();
            assert!(h.last().contains(expected), "{url} → {}", h.last());
        }
    }

    #[tokio::test]
    async fn deep_link_parse_rejects_garbage_without_panicking() {
        let mut h = Harness::new();
        deep_link(h.ctx(), args(&["parse", "not-a-url"]))
            .await
            .unwrap();
        assert!(
            h.last().contains("Invalid deep link: not-a-url"),
            "{}",
            h.last()
        );
    }

    #[tokio::test]
    async fn deep_link_test_round_trips_the_three_sample_urls() {
        let mut h = Harness::new();
        deep_link(h.ctx(), args(&["test"])).await.unwrap();
        let out = h.last();
        assert!(out.contains("Deep Link Test Results"), "{out}");
        assert!(out.contains("cc://open/src/main.rs → OpenFile"), "{out}");
        assert!(out.contains("cc://session/abc123 → ResumeSession"), "{out}");
        assert!(!out.contains("Invalid"), "{out}");
    }

    // ── /teleport ────────────────────────────────────────────────

    #[tokio::test]
    async fn teleport_without_a_subcommand_lists_its_commands() {
        let mut h = Harness::new();
        teleport(h.ctx(), Vec::new()).await.unwrap();
        let out = h.last();
        assert!(out.contains("Teleport - Remote Session Manager"), "{out}");
        assert!(out.contains("/teleport connect <endpoint>"), "{out}");
    }

    /// 缺 endpoint 时不能真的去建连接 —— 先报用法。
    #[tokio::test]
    async fn teleport_connect_without_an_endpoint_states_the_syntax() {
        let mut h = Harness::new();
        teleport(h.ctx(), args(&["connect"])).await.unwrap();
        assert!(
            h.last()
                .contains("Usage: /teleport connect <endpoint> [name]"),
            "{}",
            h.last()
        );
    }

    #[tokio::test]
    async fn teleport_list_is_empty_on_a_fresh_manager() {
        let mut h = Harness::new();
        teleport(h.ctx(), args(&["list"])).await.unwrap();
        assert!(
            h.last().contains("No active teleport sessions"),
            "{}",
            h.last()
        );
    }

    // ── /buddy ───────────────────────────────────────────────────

    #[tokio::test]
    async fn buddy_enable_and_disable_are_distinct_replies() {
        let mut h = Harness::new();
        buddy(h.ctx(), args(&["enable"])).await.unwrap();
        assert!(h.last().contains("enabled"), "{}", h.last());

        buddy(h.ctx(), args(&["off"])).await.unwrap();
        assert!(h.last().contains("disabled"), "{}", h.last());
    }

    #[tokio::test]
    async fn buddy_encourage_always_says_something() {
        let mut h = Harness::new();
        buddy(h.ctx(), args(&["encourage"])).await.unwrap();
        let out = h.last();
        assert!(out.starts_with("🤖 "), "{out}");
        assert!(out.len() > "🤖 ".len(), "{out}");
    }

    #[tokio::test]
    async fn buddy_celebrate_requires_an_achievement() {
        let mut h = Harness::new();
        buddy(h.ctx(), args(&["celebrate"])).await.unwrap();
        assert!(
            h.last().contains("Usage: /buddy celebrate <achievement>"),
            "{}",
            h.last()
        );

        buddy(h.ctx(), args(&["celebrate", "the", "release"]))
            .await
            .unwrap();
        assert!(
            h.last().contains("Incredible work on the release"),
            "{}",
            h.last()
        );
    }
}
