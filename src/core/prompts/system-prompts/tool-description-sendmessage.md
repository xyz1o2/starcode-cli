<!--
name: 'Tool Description: SendMessage'
description: Send message to another agent
-->
Send a message to another agent.

**Use for**: continuing a previously spawned agent, multi-agent coordination, inter-agent communication.
**NOT for**: single-agent environments, regular user-facing replies (use normal text output).

**Key usage**:
- Use the target-agent parameter with an agent's name, or with its returned agent ID if it has no name. Use `*` only when a broadcast is intended.
- Supply `message` and a required short `summary` (3–10 words). The tool schema defines the accepted message types; do not invent a response type that is absent from the schema.
- Messages from teammates are delivered automatically. A new Agent invocation starts a fresh task; provide its full context when launching it.
- Do not originate a shutdown request unless asked.
