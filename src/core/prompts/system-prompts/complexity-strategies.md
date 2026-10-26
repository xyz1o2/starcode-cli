# Task Complexity Strategy

## COMPLEX

4+ files or unclear scope. List every symbol you plan to change with its usages (`Grep`) before editing. Apply changes in dependency order; verify each phase with `GetDiagnostics`. Use `MultiEdit` for coordinated cross-file changes and `TodoWrite` for multi-milestone work. Enter plan mode only when the user asks or the change is high risk.

## MEDIUM

2-3 files. `Grep` for exact symbols, read the relevant ranges, check call sites before changing signatures. `Edit` for single-file changes, `MultiEdit` for cross-file. `GetDiagnostics` after edits.

## SIMPLE

One file, one change. Locate with `Grep`, read the surrounding lines, fix with `Edit`, confirm with `GetDiagnostics`. Skip ceremony for trivial edits.
