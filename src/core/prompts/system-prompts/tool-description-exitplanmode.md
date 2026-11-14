<!--
name: 'Tool Description: ExitPlanMode'
description: Exit planning mode after plan approval
-->
Present the finished plan for user approval. If approved, exit Plan Mode and seed tasks from the plan; the tool itself does not execute the steps.

**Use for**: requesting approval to transition from planning to execution.
**NOT for**: when no plan exists, or retrying a plan the user declined.

**Rules**:
- Call while Plan Mode is active. The tool requests approval; do not claim approval before the user responds.
- Treat task creation as preparation, not proof that any plan step was executed.
- After approval, continue the plan through normal tool calls and report only verified progress. If declined, stop rather than retrying the same request.