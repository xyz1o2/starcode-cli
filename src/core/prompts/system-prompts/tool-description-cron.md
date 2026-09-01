<!--
name: 'Tool Description: Cron'
description: Manage scheduled recurring tasks
-->
Create, list, delete scheduled tasks that run at intervals.

**Use for**: periodic monitoring, automated checks, scheduled operations.
**NOT for**: one-time tasks, very short intervals.

**Sub-tools**:
- `cron_create`: `name`, `prompt`, `interval_minutes`
- `cron_list`: no params
- `cron_delete`: `name`