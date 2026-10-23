/// `/plan [task]` —— 进入 Plan 模式；带任务描述时顺手让 agent 先出一份计划。
///
/// 对标 Claude Code 的 `/plan`：无参数只切换模式（状态栏立即可见），**不**偷偷
/// 消耗一个模型回合 —— 用户只是想"接下来别改文件"，不是让我现在就开始分析。
/// 带参数才额外发一条 planning prompt。
use crate::commands::execution::{CommandContext, CommandResult};
use crate::runtime::messages::AgentRequest;
use crate::types::ApprovalMode;
use crate::types::ChatEntry;

pub async fn run(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let goal = args.join(" ").trim().to_string();

    // 模式先落地：即使下面发消息失败，用户也确实在 Plan 模式里。
    let _ = ctx
        .agent_tx
        .send(AgentRequest::SetApprovalMode(ApprovalMode::Plan))
        .await;
    ctx.state.approval_mode = ApprovalMode::Plan;

    if goal.is_empty() {
        ctx.state.chat_history.push(
            ChatEntry::assistant(
                "Plan mode on. Explore and propose changes; no files will be modified.",
            )
            .with_streaming(false),
        );
        return Ok(());
    }

    let prompt = planning_prompt(&goal);
    let message_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;
    let _ = ctx
        .agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: prompt,
        })
        .await;

    ctx.state
        .chat_history
        .push(ChatEntry::assistant("Entering Plan Mode...").with_streaming(false));

    Ok(())
}

/// prompt 只约束"产出什么"，不复述用户的话——任务本身由 `goal` 带进来。
fn planning_prompt(goal: &str) -> String {
    format!(
        "User has requested a plan for: '{goal}'. \
        You are already in Plan Mode; do NOT call EnterPlanMode. \
        Please assume the role of a Lead Architect. \
        Do NOT write any code or make changes yet. \
        Instead, analyze the request and the current codebase (use tools like Read, Grep if needed), \
        and then output a detailed Markdown plan.\n\n\
        Structure the plan as follows:\n\
        # Goal: <Summary>\n\
        ## Analysis\n\
        ...\n\
        ## Proposed Changes\n\
        ...\n\
        ## Verification Steps\n\
        ...\n\n\
        End the response with 'User confirmation required to proceed.'"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::execution::CommandContext;
    use crate::ui::state::ChatState;

    async fn run_plan(args: &[&str]) -> (ChatState, Vec<AgentRequest>) {
        let mut state = ChatState::test_fixture();
        let (agent_tx, mut agent_rx) = tokio::sync::mpsc::channel(8);
        run(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            args.iter().map(|s| s.to_string()).collect(),
        )
        .await
        .unwrap();
        drop(agent_tx);

        let mut sent = Vec::new();
        while let Ok(request) = agent_rx.try_recv() {
            sent.push(request);
        }
        (state, sent)
    }

    /// 无参数只切模式：不少用户只是想"这一轮别动文件"，多跑一次模型就是
    /// 白花一次请求，还会在对话里留下一条没人要的回答。
    #[tokio::test]
    async fn entering_plan_mode_without_a_task_sends_no_message() {
        let (state, sent) = run_plan(&[]).await;

        assert_eq!(state.approval_mode, ApprovalMode::Plan);
        assert!(
            matches!(
                sent.as_slice(),
                [AgentRequest::SetApprovalMode(ApprovalMode::Plan)]
            ),
            "only the mode switch should be sent, got {sent:?}"
        );
        let reply = &state.chat_history.last().expect("confirmation").content;
        assert!(reply.contains("Plan mode on"), "{reply}");
    }

    #[tokio::test]
    async fn a_task_switches_mode_first_then_asks_for_a_plan() {
        let (state, sent) = run_plan(&["fix", "the", "auth", "bug"]).await;

        assert_eq!(state.approval_mode, ApprovalMode::Plan);
        assert_eq!(sent.len(), 2, "mode switch + planning prompt: {sent:?}");
        assert!(matches!(
            sent[0],
            AgentRequest::SetApprovalMode(ApprovalMode::Plan)
        ));
        match &sent[1] {
            AgentRequest::SendMessage { message, .. } => {
                assert!(message.contains("fix the auth bug"), "{message}");
                assert!(message.contains("Do NOT write any code"), "{message}");
                assert!(
                    message.contains("User confirmation required to proceed."),
                    "{message}"
                );
            }
            other => panic!("expected a planning message, got {other:?}"),
        }
    }

    /// 消息 ID 必须自增后再交给流式层；复用同一 ID 会让下一轮的 `Done`
    /// 被当成重复事件吞掉。
    #[tokio::test]
    async fn the_planning_message_consumes_a_fresh_message_id() {
        let mut state = ChatState::test_fixture();
        state.next_message_id = 7;
        let (agent_tx, mut agent_rx) = tokio::sync::mpsc::channel(4);

        run(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            vec!["plan".to_string(), "this".to_string()],
        )
        .await
        .unwrap();

        let mut ids = Vec::new();
        while let Ok(request) = agent_rx.try_recv() {
            if let AgentRequest::SendMessage { message_id, .. } = request {
                ids.push(message_id);
            }
        }
        assert_eq!(ids, vec![7]);
        assert_eq!(state.next_message_id, 8);
    }

    #[test]
    fn the_prompt_demands_a_structured_plan_without_edits() {
        let prompt = planning_prompt("migrate the config loader");
        assert!(prompt.contains("migrate the config loader"));
        for section in [
            "# Goal:",
            "## Analysis",
            "## Proposed Changes",
            "## Verification Steps",
        ] {
            assert!(prompt.contains(section), "missing {section}: {prompt}");
        }
        assert!(prompt.contains("already in Plan Mode; do NOT call EnterPlanMode"));
    }
}
