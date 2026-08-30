<!--
name: 'Tool Description: Wait'
description: Pause execution for a specified duration. Useful for rate-limiting or waiting for external processes.
-->
Pause execution for a specified duration.

**Use for**: rate-limiting, waiting for external processes, pacing autonomous loops.
**NOT for**: replacing file operations or as a substitute for proper synchronization.

**Rules**:
- Use the shortest duration that achieves the goal
- Prefer smaller sleeps in loops over one long sleep
