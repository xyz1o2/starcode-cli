<!--
name: 'Tool Description: BackgroundTask'
description: Submit async background task
-->
Submit a task for async background execution. Does not block the conversation, but it does **not** run anything by itself: the message is queued in `.star/remote/inbox.jsonl` and picked up by the background loop, which re-injects it as a new agent message. There is no completion notification, and the payload is a prompt — not a shell command.

**Use for**: deferring a natural-language task into the background loop.
**NOT for**: running a shell command in the background (unsupported), tasks needing immediate results, synchronous completion.

**Params**: `task_description`, `task_prompt`