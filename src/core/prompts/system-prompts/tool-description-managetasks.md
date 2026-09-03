<!--
name: 'Tool Description: TodoWrite'
description: Update the todo list for the current session. To be used proactively and often to track progress and pending tasks. Make sure that at least one task is in_progress at all times. Always provide both content (imperative) and activeForm (present continuous) for each task.
-->
Use this tool to create and manage a structured task list for the current coding session. This helps track progress, organize complex tasks, and demonstrate thoroughness to the user.

## When to Use This Tool

1. Complex multi-step tasks — 3 or more distinct steps or actions
2. Non-trivial and complex tasks requiring careful planning or multiple operations
3. User explicitly requests a todo list
4. User provides multiple tasks (numbered or comma-separated)
5. After receiving new instructions — immediately capture user requirements as todos
6. When starting a task — mark it in_progress BEFORE beginning work
7. After completing a task — mark it completed and add any new follow-up tasks

## When NOT to Use This Tool

Skip it when the work is a single straightforward task, is trivial, takes fewer than 3 steps, or is purely conversational/informational. If there is only one trivial task, just do the task directly.

## Input Format (strict)

The input is exactly `{"todos": [...]}` where every item has all three fields; unknown fields are rejected:

- `content` — imperative form describing what needs to be done (e.g., "Run tests")
- `status` — one of `pending`, `in_progress`, `completed`
- `activeForm` — present continuous form shown while executing (e.g., "Running tests")

Each call REPLACES the whole list with the one provided. When every item is completed the list is cleared. Do not send JSON-encoded objects as strings for any field.

## Task Management

- Update status in real time as you work; mark tasks complete IMMEDIATELY after finishing (don't batch completions)
- Exactly ONE task must be in_progress at any time (not less, not more)
- Complete current tasks before starting new ones; remove tasks that are no longer relevant
- ONLY mark a task completed when it is FULLY accomplished — if tests are failing, the implementation is partial, or errors remain, keep it in_progress
- Break complex tasks into specific, actionable items; always provide both content and activeForm

When in doubt, use this tool. Being proactive with task management demonstrates attentiveness and ensures all requirements are completed successfully.
