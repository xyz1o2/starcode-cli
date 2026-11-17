<!--
name: 'Tool Description: TaskOutput'
description: Get task execution output
-->
Get stored task output. Background agents are looked up by their exact task ID in the shared runtime registry; final results remain readable after their notification is consumed and after a process restart through the project's stored terminal records. Other tasks use the project's `.star/task_outputs/` directory. Reading historical output does not resume an agent or restore its conversation.

**Use for**: viewing task results and output logs, checking whether a background agent has final output.
A running background agent returns its running status without claiming a result exists. Output write or read errors are reported explicitly.

**Params**: `task_id` (required)