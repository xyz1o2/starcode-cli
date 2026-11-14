<!--
name: 'Tool Description: RunAgent'
description: Delegate multi-step work to a specialized agent; background results arrive via task notifications
-->
Launch a specialized agent for a distinct, multi-step task. Each agent type has its own capabilities and available tools.

**Use for**: broad cross-file exploration or independent work you can run concurrently.
**NOT for**: reading a known file, a single symbol lookup, or duplicating work you are already doing.

**Available agent types** (`subagent_type`):
- `general_purpose` (default): research and multi-step tasks
- `explorer`: codebase exploration; defaults to background execution
- `analyzer`: focused analysis
- `editor`: targeted code edits
- `code_reviewer`: reviewing code changes

**Launch and completion**:
- Provide a short description (3–5 words) and a detailed prompt. Specify the goal, what you already know, expected output, and whether edits are allowed. A fresh agent does not inherit this conversation.
- `explorer` runs in the background unless you explicitly set `background: false`. Other agent types run in the foreground unless you set `background: true`. Use background for independent work; use foreground if you need the result before continuing.
- A background launch returns an agent ID and output-file path. This only means the task started. Its progress may appear in the UI; neither progress nor the launch result is a finding.
- On completion or failure, a `<task-notification>` is injected into a subsequent turn. Read its `<status>` and `<result>` before reporting findings; relay the actual outcome to the user. Do not sleep, poll, or repeatedly read the output file while waiting. If the result is required for the user's answer, do not finish before it arrives.
- Results are not shown to the user as your answer. Integrate the agent's findings into your response, and verify claims that matter to the decision. If it fails or does not report back, state what is unverified.
- `name` makes a running agent addressable through SendMessage. Do not treat a completed agent as still running. Each new Agent call starts a fresh task.
- If multiple independent agents are genuinely useful, launch them in one response; avoid spawning redundant agents or re-running their investigations yourself.
