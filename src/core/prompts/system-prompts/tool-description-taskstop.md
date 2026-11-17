<!--
name: 'Tool Description: TaskStop'
description: Request cancellation of a running background agent
-->
Request cooperative cancellation of a background agent using its exact task ID.

**Params**: `task_id` (required).

This signals the actual agent loop, including model streaming and pending tool approval. A successful request means cancellation was requested; the task remains running until execution exits and cleanup finishes. Use `TaskOutput` to check its eventual status and stored result. Repeated requests are safe. Completed tasks keep their existing terminal status.

This tool currently handles background agents in the current process. Use `KillShell` for background Bash tasks. It does not resume agents or restore tasks from a previous process.
