<!--
name: 'Tool Description: Workflow'
description: Execute predefined workflows
-->
Run workflow scripts from `.star/workflows/`. JSON (multi-step) or `.sh`.

**Use for**: standardized automation, repeatable processes.
**NOT for**: workflow doesn't exist, simple commands more efficient.

**Params**: `workflow` (name), `params` (key-value, injected as `WF_` env vars)