<!--
name: 'Tool Description: GetDiagnostics'
description: Get code diagnostics and errors
-->
Retrieve code diagnostics: errors, warnings, linting issues.

**Use for**: verifying changes, finding issues, checking code health.
**NOT for**: runtime errors (use `Bash` to run), search (use `Grep`).

**Rules**:
- Run after edits to catch introduced errors
- Shows file:line:severity:message format
- Use to verify completion of tasks