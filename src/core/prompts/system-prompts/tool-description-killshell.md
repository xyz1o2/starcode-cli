<!--
name: 'Tool Description: KillShell'
description: Stop a running background bash task
-->
Stop a running background bash task. Kills the process group: SIGTERM first, escalating to SIGKILL after a grace period.

**Use for**: terminating background commands by task id (bash-xxxxxxxx).
**NOT for**: cancelling agent turns or tool confirmations (the UI handles those).

**Params**: `task_id` (required).
