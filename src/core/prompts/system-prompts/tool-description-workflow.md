<!--
name: 'Tool Description: Workflow'
description: Execute predefined workflows
-->
Run workflow scripts from `.star/workflows/`. JSON (multi-step) or `.sh`.

**Use for**: standardized automation, repeatable processes.
**NOT for**: workflow doesn't exist, simple commands more efficient.

**Params**: `workflow` (a single file stem, without directories), `params` (key-value, injected as `WF_` env vars), "timeout_secs" (1–1800, defaults to 300 for the entire workflow).

Resolves `.star/workflows/` at the inherited project root and runs commands from that root. JSON takes precedence over `.sh`. JSON must contain a non-empty "steps" array whose entries have non-empty "command" strings and optional `name` fields. All steps are validated before execution. The first failing step stops the workflow and reports its name, exit code, and stderr.

Cancellation and the total execution deadline stop the current command. On Unix, commands run in their own process group and that group is terminated on cancellation, timeout, or dropped execution. A successful step is waited for and its output drained; separately detached processes are outside this guarantee. Windows currently guarantees cleanup of the direct child only. Requesting execution is not proof of success: report the actual tool result.