<!--
name: 'Tool Description: TodoWrite'
description: Update the current session's todo list to track multi-step work and outstanding tasks.
routing: track-work
-->
Use this tool to create and manage a structured task list for the current coding session. This helps track progress, organize complex tasks, and demonstrate thoroughness to the user.

## When to Use This Tool

1. Complex multi-step tasks — 3 or more distinct steps or actions
2. Non-trivial and complex tasks requiring careful planning or multiple operations
3. User explicitly requests a todo list
4. User provides multiple tasks (numbered or comma-separated)
5. When requirements change during multi-step work, update the list so it remains accurate.
6. Mark the task being worked on in_progress and mark it completed when verified.

## When NOT to Use This Tool

Skip it when the work is a single straightforward task, is trivial, takes fewer than 3 steps, or is purely conversational/informational. If there is only one trivial task, just do the task directly.

## Recommended Input Format

Use `{"todos": [...]}` with the following fields. The parser accepts some aliases and defaults; extra fields on individual items are ignored, but unknown top-level keys alongside the list are rejected:

- `content` — imperative form describing what needs to be done (e.g., "Run tests")
- `status` — one of `pending`, `in_progress`, `completed`
- `activeForm` — present continuous form shown while executing (e.g., "Running tests")

Each call REPLACES the whole list with the one provided. When every item is completed the list is cleared. Do not send JSON-encoded objects as strings for any field.

## Task Management

- Update status in real time as you work; mark tasks complete IMMEDIATELY after finishing (don't batch completions)
- Keep the list faithful to actual work: an active task may be in_progress, but do not invent one when waiting or after all tasks are done.
- Remove tasks that are no longer relevant; independent tasks may proceed in parallel.
- ONLY mark a task completed when it is FULLY accomplished — if tests are failing, the implementation is partial, or errors remain, keep it in_progress
- Break complex tasks into specific, actionable items; always provide both content and activeForm

Use the list when it helps coordinate real work; it does not replace execution or verification.
