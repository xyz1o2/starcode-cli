<!--
name: 'Tool Description: RunAgent'
description: Delegate multi-step work to a specialized agent; background results arrive via task notifications
-->
Launch a specialized agent for a distinct, multi-step task. Each agent type has its own capabilities and available tools.

**Use for**: broad cross-file exploration or independent work you can run concurrently.
**NOT for**: reading a known file, a single symbol lookup, or duplicating work you are already doing.

**Available agent types** (`subagent_type`):
- `general_purpose` (default): research and multi-step tasks
- `explorer`: read-only code search and explanation; defaults to background execution. Specify search breadth ("quick", "medium", or "very thorough") and the paths or relationships to investigate.
- `analyzer`: focused analysis
- `editor`: targeted code edits
- `code_reviewer`: reviewing code changes

**Launch and completion**:
- Provide a short description (3–5 words) and a detailed prompt. Specify the goal, what you already know, paths already ruled out, expected output, and whether edits are allowed. Ordinary agents start without this conversation; provide the background needed to do the work.
- Fork is opt-in: when STAR_FORK_AGENT_ENABLED is enabled, omit `subagent_type` and include "fork" in the description to start a background branch with the caller's completed conversation history. The current unfinished tool batch is excluded. Fork does not promise an identical system prompt or tool pool. `/subtask` is the explicit command for branching the current session.
- `explorer` runs in the background unless you explicitly set `background: false`. Other agent types run in the foreground unless you set `background: true`. Use background for independent work; use foreground if you need the result before continuing.
- Optionally set "model" to a model ID available on the current provider. Omit it or use "inherit" to keep the parent model; it does not select another provider or endpoint.
- A background launch returns an agent ID and output-file path. This only means the task started. Its progress may appear in the UI; neither progress nor the launch result is a finding.
- On completion or failure, a `<task-notification>` is injected into a subsequent turn. Read its `<status>` and `<result>` before reporting findings; relay the actual outcome to the user. Do not sleep, poll, or repeatedly read the output file while waiting. If the result is required for the user's answer, do not finish before it arrives.
- Results are not shown to the user as your answer. Integrate the agent's findings into your response, and verify claims that matter to the decision. If it fails or does not report back, state what is unverified.
- `name` makes a running agent addressable through SendMessage. Do not treat a completed agent as still running.
- Set "resume" to an ended agent ID to continue from its last saved native context in this project. Supply a new prompt; omit the specialized type to inherit it, or specify the original type explicitly. The attempt gets a new task ID; current provider, model selection, permissions and working directory apply. Missing or invalid snapshots are errors. This does not restore history removed by earlier compaction, run old tools again, or recover an active task after a process crash. Reading TaskOutput alone does not continue a conversation.
- If multiple independent agents are genuinely useful, launch them in one response; avoid spawning redundant agents or re-running their investigations yourself.
