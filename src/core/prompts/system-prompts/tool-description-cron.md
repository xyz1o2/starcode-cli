<!--
name: 'Tool Description: Cron'
description: Manage scheduled recurring tasks
-->
Create, list, delete scheduled tasks that run at intervals.

**Use for**: periodic monitoring, automated checks, scheduled operations.
**NOT for**: one-time tasks, very short intervals.

**Sub-tools**:
- `CronCreate`: `name`, `prompt`, `interval_minutes`
- `CronList`: no params
- `CronDelete`: `name`