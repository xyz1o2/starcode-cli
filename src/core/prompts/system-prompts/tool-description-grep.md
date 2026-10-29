<!--
name: 'Tool Description: Grep'
description: Content search on ripgrep. Regex or literal, glob include/exclude filters. Respects .gitignore/.starignore; dotfiles searched, VCS dirs skipped.
routing: exact-content
-->
A powerful search tool built on ripgrep.

**Use for**: finding code patterns, locating functions, searching text.
**NOT for**: `grep`/`rg` via bash — this tool handles permissions and access.

**Key params**:
- `pattern` (alias `query`): the search string. Literal by default; set `regex: true` to treat it as a regular expression.
- `path` (alias `dir_path`): directory to search
- `output_mode`: `content` (matching lines, default) | `files_with_matches` (unique paths only) | `count` (matches per file)
- `search_type`: `text` (file contents, default) | `files` (file names) | `both`. A query starting with `@` means "find files by name".
- `include_pattern` / `exclude_pattern`: file globs, e.g. `*.rs`, `src/**/*.ts`. A bare pattern matches at any depth; a plain path prefix like `src/tools` matches that whole subtree.
- `file_types`: extension filter, e.g. `["rs", "toml"]`
- `case_sensitive`, `whole_word`, `max_results`, `include_hidden`

**Rules**:
- Output is bounded on purpose — it costs context you will pay for later:
  - at most 100 matches, at most 10 lines per file (rest shown as `+N more`), long lines truncated, paths relative to the project root.
  - Start with `output_mode: "files_with_matches"` (or `"count"`) when you only need to know *where* something lives; switch to `content` when you actually need the lines.
  - If a search is truncated or flooded, narrow it (`path`, `include_pattern`, a tighter pattern) instead of raising `max_results`.
- ALWAYS use Grep for **exact** content search — you know the literal string, symbol or regex.
  NEVER invoke `grep` or `rg` as a Bash command; this tool handles permissions and access.
- When you only know the rough domain words (not the literal token) and want the top-ranked
  regions across many files rather than every match, that is `CodebaseSearch` — it is the
  better first pass in exactly that case. The difference is ranking, not correctness: either
  tool will do.
- Supports full regex syntax (e.g., `log.*Error`, `function\s+\w+`). Patterns match within a single line.
- Dotfiles are searched by default (`.github/workflows/`, `.env.example` are reachable); `.git`, `.svn`, `.hg`, `.bzr`, `.jj` and `.sl` are always skipped, as is anything ignored by `.gitignore`, `.starignore`, `.ignore`/`.rgignore` or `~/.star/ignore`.
- Use `exclude_pattern` to leave out tests or generated code — nothing is filtered out on your behalf.
- After match at file:line, use `Read(offset=line-10, limit=60)` — don't read whole file
- Parallel search: batch multiple candidate patterns in one response
- For open-ended searches requiring multiple rounds, use the Agent tool instead