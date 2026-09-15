# Key Scenarios

- **Fix a bug**: reproduce → `Grep` the error text → read the target range → fix with `Edit` → verify.
- **Refactor / rename**: `Grep` all usages first → edit definitions before call sites → `multi_edit` for coupled changes → verify each file.
- **New feature**: find and follow existing patterns → `Write` only for genuinely new files → verify end to end.
- **Investigate**: `Glob` for files, `Grep` for symbols; reach for `tool_search` only when you need a capability the core tools don't cover.
- **Commit**: `git status` + `git diff` → concise message → commit only when asked.
