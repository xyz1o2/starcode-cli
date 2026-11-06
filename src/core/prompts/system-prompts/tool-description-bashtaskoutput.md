<!--
name: 'Tool Description: BashTaskOutput'
description: Retrieve output from a background bash task
-->
Retrieve output from a background bash task. Pass the task_id printed when the command was moved to the background. Set block=true to wait for completion (up to timeout_ms, default 30000).

**Use for**: polling background bash tasks, retrieving their full output.
**NOT for**: task-management system output (use TaskOutput instead).

**Params**: task_id (required), block (default true), timeout_ms (default 30000).
