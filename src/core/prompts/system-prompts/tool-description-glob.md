<!--
name: 'Tool Description: Glob'
description: Find files by name pattern. Returns up to 100 paths, most recently modified first. Bare patterns like `*.rs` match at any depth; `*` does not cross `/`. Paths ignored by .gitignore/.starignore are skipped unless respect_git_ignore is false.
-->
Fast file pattern matching tool that works with any codebase size.

**Use for**: finding files by name/path patterns (`**/*.js`, `src/**/*.ts`).
**NOT for**: searching file contents (use `Grep`).

**Key params**:
- `pattern`: glob. A pattern with no `/` matches at any depth (`*.py` ≡ `**/*.py`, same as `rg -g`). `*` does not cross `/` — use `**` to span directories.
- `dir_path`: directory to search in (defaults to the working directory)
- `case_sensitive`: defaults to false
- `respect_git_ignore` / `respect_star_ignore`: both default to true

**Rules**:
- Results are sorted by modification time (newest first), then capped at 100. If the output says it was truncated, narrow the pattern or pass `dir_path` — do not page through by retrying the same pattern.
- Dotfiles and dot-directories are matched normally; `.git`, `.svn`, `.hg`, `.bzr`, `.jj` and `.sl` are always skipped.
- Files ignored by `.gitignore`, `.starignore`, `.ignore`/`.rgignore` or `~/.star/ignore` are skipped. Pass `respect_git_ignore: false` only when you specifically need build output or vendored code.
- When you are doing an open ended search that may require multiple rounds of globbing and grepping, use the Agent tool instead
- Batch parallel globs for multiple candidate patterns
