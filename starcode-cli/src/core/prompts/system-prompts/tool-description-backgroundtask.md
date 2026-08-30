<!--
name: 'Tool Description: BackgroundTask'
description: Submit async background task
-->
Submit task for async background execution. Does not block conversation.

**Use for**: long-running, low-priority work.
**NOT for**: tasks needing immediate results, synchronous completion.

**Params**: `task_description`, `task_prompt`