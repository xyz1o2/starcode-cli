<!--
name: 'Tool Description: TaskList'
description: List tasks with optional filter
-->
List tasks with optional status filter.

**Use for**: overview of tasks, filtering by state.
**NOT for**: full task details (use `task_get`).

**Params**: `status` (pending/in_progress/completed/blocked/skipped), `limit` (default: 20)