use crate::commands::execution::{CommandContext, CommandResult};
use crate::types::ChatEntry;

pub async fn stats(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("session");

    if sub != "session" {
        ctx.state
            .chat_history
            .push(ChatEntry::assistant(format!(
                "Unknown stats category '{}'. Available: session (default). Try /cost for token/cost details.",
                sub
            )).with_streaming(false));
        return Ok(());
    }

    // 与 /cost 同口径：优先用最终 usage（prompt/completion/total），
    // 流式期间才回退到实时计数的 token_count
    let (prompt, completion, total, source) = match &ctx.state.token_usage {
        Some(u) if u.total_tokens > 0 || u.prompt_tokens > 0 => (
            u.prompt_tokens,
            u.completion_tokens,
            u.total_tokens.max(u.prompt_tokens + u.completion_tokens),
            "final usage",
        ),
        _ => (0, 0, ctx.state.token_count, "streaming estimate"),
    };

    let content = format!(
        "Session Stats:\n- Messages: {}\n- Tokens: {} ({})\n  - Prompt: {}\n  - Completion: {}\n- Model: {}\n- Processing Time: {}s",
        ctx.state.chat_history.len(),
        total,
        source,
        prompt,
        completion,
        if ctx.state.current_model.is_empty() { "-" } else { &ctx.state.current_model },
        ctx.state.processing_time_secs,
    );

    ctx.state
        .chat_history
        .push(ChatEntry::assistant(content).with_streaming(false));
    Ok(())
}

/// `/tools` 的展示顺序。分类名来自 [`ToolName::category`]，这里只决定先后。
const TOOL_GROUPS: &[&str] = &[
    "Files",
    "Search",
    "Execution",
    "Agents & tasks",
    "Plan & worktree",
    "Web",
    "Git",
    "Scheduling",
    "MCP",
    "Other",
];

pub async fn tools(ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    // 对标 Claude Code：内置工具名即唯一名，不再维护别名表。
    // 清单来自注册表而不是手写常量 —— 手写的那份漏过 LSP、CodeGraph 等工具，
    // `/tools` 因此成了一个会撒谎的命令。新增工具必须先在 `ToolName::category`
    // 里归类，否则编译不过。
    use crate::core::tools::constants::ToolName;

    let mut grouped: Vec<(&'static str, Vec<&'static str>)> = TOOL_GROUPS
        .iter()
        .map(|group| (*group, Vec::new()))
        .collect();
    for tool in ToolName::all_builtin() {
        let category = tool.category();
        let entry = grouped.iter_mut().find(|(group, _)| *group == category);
        match entry {
            Some((_, names)) => names.push(tool.as_str()),
            // 分组表漏了某个分类：不要悄悄丢工具，落到末尾让它显形。
            None => {
                ctx.state.chat_history.push(
                    ChatEntry::assistant(format!(
                        "Internal: tool `{}` has category `{}` which /tools does not know about.",
                        tool.as_str(),
                        category
                    ))
                    .with_streaming(false),
                );
                return Ok(());
            }
        }
    }

    let mut lines = vec!["Available tools (registered names):".to_string()];
    for (group, names) in grouped {
        if names.is_empty() {
            continue;
        }
        lines.push(format!("- {}: {}", group, names.join(", ")));
    }
    lines.push(String::new());
    lines.push("Use `ToolSearch` to find tools by capability.".to_string());

    ctx.state
        .chat_history
        .push(ChatEntry::assistant(lines.join("\n")).with_streaming(false));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::execution::CommandContext;
    use crate::core::tools::constants::ToolName;
    use crate::ui::state::ChatState;

    async fn render_tools() -> String {
        let mut state = ChatState::test_fixture();
        let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(1);
        tools(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            Vec::new(),
        )
        .await
        .unwrap();
        state
            .chat_history
            .last()
            .map(|entry| entry.content.clone())
            .unwrap_or_default()
    }

    /// `/tools` 唯一的价值就是"我到底能用哪些工具" —— 漏一个就是把发现工具
    /// 的唯一途径堵死一半。
    #[tokio::test]
    async fn every_registered_tool_is_listed() {
        let output = render_tools().await;
        for tool in ToolName::all_builtin() {
            assert!(
                output.contains(tool.as_str()),
                "`{}` is registered but /tools does not list it:\n{output}",
                tool.as_str()
            );
        }
    }

    #[tokio::test]
    async fn groups_render_in_a_stable_order_and_skip_empty_ones() {
        let output = render_tools().await;
        let positions: Vec<usize> = TOOL_GROUPS
            .iter()
            .filter_map(|group| output.find(&format!("- {}: ", group)))
            .collect();
        assert!(!positions.is_empty());
        let mut sorted = positions.clone();
        sorted.sort_unstable();
        assert_eq!(positions, sorted, "groups must keep TOOL_GROUPS order");

        for group in TOOL_GROUPS {
            assert!(
                output.contains(&format!("- {}: ", group)),
                "every known group has at least one tool; missing {group}:\n{output}"
            );
        }
    }

    #[test]
    fn every_tool_category_is_renderable() {
        for tool in ToolName::all_builtin() {
            assert!(
                TOOL_GROUPS.contains(&tool.category()),
                "`{}` has category `{}` that /tools does not render",
                tool.as_str(),
                tool.category()
            );
        }
    }

    #[tokio::test]
    async fn stats_reports_session_totals() {
        let mut state = ChatState::test_fixture();
        state.current_model = "claude-sonnet-5-5".to_string();
        state.token_usage = Some(crate::types::StarUsage {
            prompt_tokens: 120,
            completion_tokens: 30,
            total_tokens: 150,
            ..Default::default()
        });
        let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(1);

        stats(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            Vec::new(),
        )
        .await
        .unwrap();

        let output = &state.chat_history.last().expect("stats").content;
        assert!(output.contains("Tokens: 150 (final usage)"), "{output}");
        assert!(output.contains("Prompt: 120"), "{output}");
        assert!(output.contains("Completion: 30"), "{output}");
        assert!(output.contains("claude-sonnet-5-5"), "{output}");
    }

    #[tokio::test]
    async fn stats_falls_back_to_the_streaming_estimate() {
        let mut state = ChatState::test_fixture();
        state.token_usage = None;
        state.token_count = 42;
        let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(1);

        stats(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            Vec::new(),
        )
        .await
        .unwrap();

        let output = &state.chat_history.last().expect("stats").content;
        assert!(
            output.contains("Tokens: 42 (streaming estimate)"),
            "{output}"
        );
    }

    #[tokio::test]
    async fn unknown_stats_categories_point_at_cost() {
        let mut state = ChatState::test_fixture();
        let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(1);

        stats(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            vec!["weekly".to_string()],
        )
        .await
        .unwrap();

        let output = &state.chat_history.last().expect("stats").content;
        assert!(
            output.contains("Unknown stats category 'weekly'"),
            "{output}"
        );
        assert!(output.contains("/cost"), "{output}");
    }
}
