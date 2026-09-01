use crate::commands::execution::{CommandContext, CommandResult};
use crate::types::ApprovalMode;

pub async fn run(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let goal = if args.is_empty() {
        "Please provide a plan for the current project state.".to_string()
    } else {
        args.join(" ")
    };

    let prompt = format!(
        "User has requested a plan for: '{}'. \
        You are already in Plan Mode; do NOT call enter_plan_mode. \
        Please assume the role of a Lead Architect. \
        Do NOT write any code or make changes yet. \
        Instead, analyze the request and the current codebase (use tools like ls, read if needed), \
        and then output a detailed Markdown plan. \
        \
        Structure the plan as follows: \
        # Goal: <Summary> \
        ## Analysis \
        ... \
        ## Proposed Changes \
        ... \
        ## Verification Steps \
        ... \
        \
        End the response with 'User confirmation required to proceed.'",
        goal
    );

    // Push the special prompt to chat history as a User message, but effectively it's a system directive wrapper
    // Actually, we should just inject it as a user message to trigger the agent.

    // We want to force the agent to THINK and PLAN, not act.
    // However, the agent's main loop handles the execution.
    // By pushing this message, the main loop (if we were in it) would pick it up.
    // But `run` here is called by `handle_command`, which is called inside `enqueue_user_message`.
    // The `CommandContext` gives us access to `agent_tx` to send messages back to the main loop.

    use crate::runtime::messages::AgentRequest;

    let _ = ctx
        .agent_tx
        .send(AgentRequest::SetApprovalMode(ApprovalMode::Plan))
        .await;
    ctx.state.approval_mode = ApprovalMode::Plan;

    // Send the planning prompt to the agent
    let _ = ctx
        .agent_tx
        .send(AgentRequest::SendMessage {
            message_id: ctx.state.next_message_id, // This might be tricky if we don't own the ID generation
            message: prompt,
        })
        .await;

    // Wait, `CommandContext` has `state` but it's a mutable reference.
    // `next_message_id` needs to be incremented.
    let _msg_id = ctx.state.next_message_id;
    ctx.state.next_message_id += 1;

    // We should also show a system message saying we are entering plan mode
    ctx.state
        .chat_history
        .push(crate::types::ChatEntry::assistant("Entering Plan Mode...").with_streaming(false));

    Ok(())
}
