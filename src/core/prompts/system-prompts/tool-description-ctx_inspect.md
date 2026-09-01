<!--
name: 'Tool Description: Context Inspect'
description: Inspect the current context window contents and token usage.
-->
Inspect the current context window contents and token usage.

**Use for**: debugging context saturation, checking what the model can see, diagnosing truncation.
**NOT for**: routine progress tracking.

**Rules**:
- Call sparingly — it consumes output tokens to report usage
