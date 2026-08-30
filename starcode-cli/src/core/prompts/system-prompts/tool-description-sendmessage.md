<!--
name: 'Tool Description: SendMessage'
description: Send message to another agent
-->
Send a message to another agent.

**Use for**: continuing a previously spawned agent, multi-agent coordination, inter-agent communication.
**NOT for**: single-agent environments, regular user-facing replies (use normal text output).

**Key usage**:
- To continue a previously spawned agent, use SendMessage with the agent's ID or name as the `to` field. The agent resumes with its full context preserved.
- Each Agent invocation starts fresh — provide a complete task description when spawning new agents.
- `summary` is REQUIRED (5-10 word short description shown as preview in the UI)
- Messages from teammates are delivered automatically; you don't check an inbox.
- Refer to teammates by name, never by UUID.

**Protocol responses**:
- If you receive a JSON message with `type: "shutdown_request"` or `type: "plan_approval_request"`, respond with the matching `_response` type.
- Do not originate `shutdown_request` unless asked.
