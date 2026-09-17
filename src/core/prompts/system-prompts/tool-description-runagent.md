<!--
name: 'Tool Description: RunAgent'
description: Launch autonomous SubAgent
-->
Launch a new agent to handle complex, multi-step tasks autonomously.

The Agent tool launches specialized agents (subprocesses) that autonomously handle complex tasks. Each agent type has specific capabilities and tools available to it.

**Use for**: broad searches, complex refactors, multi-file operations, research tasks.
**NOT for**: simple single-step tasks, precise sequential control needed, reading specific files (use Read), searching for specific classes/functions (use Grep/Glob).

**Available agent types** (`subagent_type`):
- `general_purpose` (default): researching complex questions, searching code, and executing multi-step tasks
- `explorer`: fast codebase exploration — finding files by pattern, searching code for keywords
- `analyzer`: focused analysis of one question or component
- `editor`: targeted code edits
- `code_reviewer`: reviewing changes for correctness and quality

**Usage notes**:
- Always include a short description (3-5 words) summarizing what the agent will do
- When the agent is done, it will return a single message back to you. The result returned by the agent is NOT visible to the user. To show the user the result, you should send a text message back to the user with a concise summary of the result.
- You can optionally run agents in the background using `background: true`. When an agent runs in the background, you will be automatically notified when it completes — do NOT sleep, poll, or proactively check on its progress. Continue with other work or respond to the user instead.
- **Foreground vs background**: Use foreground (default) when you need the agent's results before you can proceed. Use background when you have genuinely independent work to do in parallel.
- To continue a previously spawned agent, use SendMessage with the agent's ID or name as the `to` field. The agent resumes with its full context preserved. Each Agent invocation starts fresh — provide a complete task description.
- Set `name` to give the agent a stable handle so `send_message` can target it later.
- The agent's outputs should generally be trusted
- Clearly tell the agent whether you expect it to write code or just to do research (search, file reads, web fetches, etc.), since it is not aware of the user's intent
- If the user specifies that they want you to run agents "in parallel", you MUST send a single message with multiple Agent tool use content blocks.

**Writing the prompt**:
Brief the agent like a smart colleague who just walked into the room — it hasn't seen this conversation, doesn't know what you've tried, doesn't understand why this task matters.
- Explain what you're trying to accomplish and why, what you've already learned or ruled out, and enough context for the agent to make judgment calls.
- If you need a short response, say so ("report in under 200 words").
- Lookups: hand over the exact command. Investigations: hand over the question — prescribed steps become dead weight when the premise is wrong.
- Terse command-style prompts produce shallow, generic work.
- **Never delegate understanding.** Don't write "based on your findings, fix the bug." Write prompts that prove you understood: include file paths, line numbers, what specifically to change.

**When NOT to use**:
- If you want to read a specific file path, use the Read tool instead
- If you are searching for a specific class definition like "class Foo", use Grep instead
- If you are searching for code within a specific file or set of 2-3 files, use Read instead
- Other tasks that are not related to the agent descriptions above
