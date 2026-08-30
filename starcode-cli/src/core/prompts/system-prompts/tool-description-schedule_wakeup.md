<!--
name: 'Tool Description: Schedule Wakeup'
description: Schedule a one-shot wakeup event. Used by autonomous loop mode for self-paced work intervals.
-->
Schedule a one-shot wakeup event.

**Use for**: autonomous loop mode, self-paced work intervals.
**NOT for**: user-interactive sessions.

**Rules**:
- The runtime clamps delay to [60, 3600] seconds
- Used by autonomous loop mode to pace work
