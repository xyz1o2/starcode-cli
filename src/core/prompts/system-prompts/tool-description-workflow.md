<!--
name: 'Tool Description: Workflow'
description: Execute predefined workflows
-->
Run approved workflows from `.star/workflows/`: JSON, `.sh`, or JavaScript (`.js`). JavaScript requires Node.js and provides `agent`, `parallel`, `pipeline`, `phase`, `log`, and input `args`. Agent calls use the normal Agent tool permission gate; no model credentials enter the Node process.

**Use for**: standardized automation, repeatable processes.
**NOT for**: workflow doesn't exist, simple commands more efficient.

**Params**: exactly one of `workflow` or inline JavaScript `script`; `args` (JSON input), `resume_from_run_id` (replay successful matching calls in a new run), `max_concurrency` (1–16), and `timeout_secs` (1–1800, default 300). Legacy `.sh`/JSON workflows use `params` as `WF_` env vars.

Resolves workflow files at the inherited project root. JavaScript runs from that root and stores each immutable run under `.star/workflows/runs/<run-id>/` with source, args, and an atomically replaced journal. Resume creates a new run and reuses successful calls only when script, model context, prompt, options, label, and args still match; changed calls run again. Journal entries contain outputs and project paths, so treat the run directory as project data. JSON takes precedence over `.sh`. JSON must contain a non-empty "steps" array whose entries have non-empty "command" strings and optional `name` fields. All steps are validated before execution. The first failing step stops the workflow and reports its name, exit code, and stderr.

Cancellation and the total execution deadline stop the current command or JavaScript runtime; dispatched Agent calls receive cancellation and are drained before the workflow reports its terminal result. On Unix, commands run in their own process group and that group is terminated on cancellation, timeout, or dropped execution. A successful step is waited for and its output drained; separately detached processes are outside this guarantee. Windows currently guarantees cleanup of the direct child only. Requesting execution is not proof of success: report the actual tool result.