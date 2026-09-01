<!--
name: 'Tool Description: Remote Trigger'
description: Manage remote agent triggers - list, get, create, update, run.
-->
Manage remote agent scheduled triggers.

**Use for**: scheduling remote agent execution, listing/updating existing triggers.
**NOT for**: local cron tasks (use `cron_create`).

**Rules**:
- List before creating to avoid duplicates
