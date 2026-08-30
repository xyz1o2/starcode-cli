<!--
name: 'Tool Description: Brief'
description: Generate content summary
-->
Return brief summary. Truncates with ellipsis if exceeds max_length.

**Use for**: compressing long text, reducing context usage.
**NOT for**: when full content must be preserved.

**Params**: `content` (required), `max_length` (default: 200)