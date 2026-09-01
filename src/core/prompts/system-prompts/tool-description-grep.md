<!--
name: 'Tool Description: Grep'
description: Content search using ripgrep
-->
A powerful search tool built on ripgrep.

**Use for**: finding code patterns, locating functions, searching text.
**NOT for**: `grep`/`rg` via bash — this tool handles permissions and access.

**Key params**:
- `pattern`: regex or literal string
- `include`/`glob`: file glob filter (e.g., `*.js`, `*.{ts,tsx}`)
- `type`: file type filter (e.g., `js`, `py`, `rust`)
- `output_mode`: `content` | `files_with_matches` | `count`

**Rules**:
- ALWAYS use Grep for search tasks. NEVER invoke `grep` or `rg` as a Bash command. The Grep tool has been optimized for correct permissions and access.
- Supports full regex syntax (e.g., `log.*Error`, `function\s+\w+`)
- Multiline matching: By default patterns match within single lines only. For cross-line patterns, use `multiline: true`.
- After match at file:line, use `Read(offset=line-10, limit=60)` — don't read whole file
- Parallel search: batch multiple candidate patterns in one response
- For open-ended searches requiring multiple rounds, use the Agent tool instead
