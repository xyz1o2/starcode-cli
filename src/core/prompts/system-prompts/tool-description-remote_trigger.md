<!--
name: 'Tool Description: Remote Trigger'
description: Preview remote trigger requests; no remote scheduling or execution is performed.
-->
This tool currently returns local placeholder responses. It does not create, update, list, or run real remote triggers.

**Use for**: previewing the request/response shape during development.
**NOT for**: scheduling or checking real remote execution. Use `CronCreate` for supported local cron tasks.

**Rules**:
- Do not report a trigger as created or executed based on this tool's placeholder output.
