<!--
name: 'Tool Description: Grep'
description: Content search built on ripgrep. Searches file contents by regex or literal, with glob include/exclude filters. Respects .gitignore/.starignore; dotfiles are searched, VCS metadata directories are not.
-->
A powerful search tool built on ripgrep.

**Use for**: finding code patterns, locating functions, searching text.
**NOT for**: `grep`/`rg` via bash — this tool handles permissions and access.

**Key params**:
- `pattern` (alias `query`): the search string. Regex by default for content search; set `regex: false` for a literal match.
- `path` (alias `dir_path`): directory to search
- `output_mode`: `content` (matching lines, default) | `files_with_matches` (unique paths only)
- `search_type`: `text` (file contents) | `files` (file names) | `both`
- `include_pattern` / `exclude_pattern`: file globs, e.g. `*.rs`, `src/**/*.ts`. A bare pattern matches at any depth; a plain path prefix like `src/tools` matches that whole subtree.
- `file_types`: extension filter, e.g. `["rs", "toml"]`
- `case_sensitive`, `whole_word`, `max_results`, `include_hidden`

**Rules**:
- ALWAYS use Grep for search tasks. NEVER invoke `grep` or `rg` as a Bash command. The Grep tool has been optimized for correct permissions and access.
- Supports full regex syntax (e.g., `log.*Error`, `function\s+\w+`). Patterns match within a single line.
- Dotfiles are searched by default (`.github/workflows/`, `.env.example` are reachable); `.git`, `.svn`, `.hg`, `.bzr`, `.jj` and `.sl` are always skipped, as is anything ignored by `.gitignore`, `.starignore`, `.ignore`/`.rgignore` or `~/.star/ignore`.
- Use `exclude_pattern` to leave out tests or generated code — nothing is filtered out on your behalf.
- After match at file:line, use `Read(offset=line-10, limit=60)` — don't read whole file
- Parallel search: batch multiple candidate patterns in one response
- For open-ended searches requiring multiple rounds, use the Agent tool instead
