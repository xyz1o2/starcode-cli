<!--
name: 'Tool Description: Cross Agent'
description: Send messages or files to other agents or to the user's device (push notification, assistant mode).
-->
Cross-agent messaging and device delivery.

**Use for**: sending structured messages to other agents, push notifications, delivering files to the user's device.
**NOT for**: regular user-facing replies (use normal text output).

**Rules**:
- `send_message` supports `target_agent='*'` for broadcast
- `summary` is REQUIRED (3-10 word short description)
- `send_user_file` requires assistant mode
