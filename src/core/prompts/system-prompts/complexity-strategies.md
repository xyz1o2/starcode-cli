# Task Complexity Strategy

## COMPLEX

4+ files or unclear scope. List every symbol you plan to change with its usages (`Grep`) before editing. Apply changes in dependency order; verify each phase with `get_diagnostics`. Use `multi_edit` for coordinated cross-file changes and `TodoWrite` for multi-milestone work. Enter plan mode only when the user asks or the change is high risk.

## MEDIUM

2-3 files. `Grep` for exact symbols, read the relevant ranges, check call sites before changing signatures. `Edit` for single-file changes, `multi_edit` for cross-file. `get_diagnostics` after edits.

## SIMPLE

One file, one change. Locate with `Grep`, read the surrounding lines, fix with `Edit`, confirm with `get_diagnostics`. Skip ceremony for trivial edits.
