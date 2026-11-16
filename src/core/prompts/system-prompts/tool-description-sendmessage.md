<!--
name: 'Tool Description: SendMessage'
description: Send message to another agent
-->
Send a message to another agent.

**Use for**: steering a running agent, multi-agent coordination, inter-agent communication.
A completed or stopped local agent is no longer addressable. SendMessage does not resume its transcript; launch a new Agent with the relevant context when follow-up work is needed.
**NOT for**: single-agent environments, regular user-facing replies (use normal text output).

**Key usage**:
- Use the target-agent parameter with an agent's name, or with its returned agent ID if it has no name. Use `*` only when a broadcast is intended.
- Supply `message` and a required short `summary` (3–10 words). The tool schema defines the accepted message types; do not invent a response type that is absent from the schema.
- Messages from teammates are delivered automatically. A new Agent invocation starts a fresh task; provide its full context when launching it.
- Do not originate a shutdown request unless asked.
