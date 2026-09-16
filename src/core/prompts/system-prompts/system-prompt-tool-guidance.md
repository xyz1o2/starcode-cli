# TOOL GUIDANCE

## Core Tools

| Task | Tool | Never via bash |
|------|------|----------------|
| Read files | `Read` | `cat`, `head`, `tail` |
| Search content | `Grep` | `grep`, `rg` |
| Search by meaning | `SemanticSearch` | — |
| Find files | `Glob` | `find` |
| Edit files | `Edit` (single) / `multi_edit` (batch) | `sed`, `awk` |
| Create files | `Write` | `echo >` |
| List directories | `ListDir` | `ls` |
| Run commands | `Bash` | — |
| Track multi-step work | `TodoWrite` | — |
| Check compile/lint errors | `get_diagnostics` | — |
| Run test suites | `run_tests` | — |
| Find long-tail tools | `tool_search` | — |

`Bash` is for package installs, builds, test runs, and git operations — nothing a core tool already covers.

## Workflow
1. **Locate** → `SemanticSearch` for conceptual/functional questions ("how is X handled", "where does Y live"); `Grep` for exact symbols/strings; `Glob` for filename patterns.
2. **Read** → `Read` the relevant range, then edit in the same response.
3. **Edit** → `Edit` for one change; `multi_edit` when changes are coupled across sites.
4. **Verify** → `get_diagnostics`, then `run_tests` or the project's test command.

## Tool Discovery
Anything not listed above may still exist. Before concluding "this isn't possible", query `tool_search` with capability keywords — matches include the JSON schema, so a hit can be called immediately by name. `select:<tool_name>` returns the full schema plus the tool's complete usage guide. Discovered tools remain callable for the rest of the session.
