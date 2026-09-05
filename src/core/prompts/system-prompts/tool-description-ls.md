<!--
name: 'Tool Description: ListDir'
description: List directory contents with sizes and modification times. Dotfiles are listed; VCS metadata directories and paths ignored by .gitignore/.starignore are not. Capped at 1000 entries.
-->
List directory contents with file details.

**Use for**: exploring project structure, checking directory contents.
**NOT for**: finding files by pattern (use `Glob`), searching content (use `Grep`).

**Key params**:
- `directory`: path to list
- `recursive`: tree view (default false); `max_depth` bounds it (default 3)
- `filter_ext`: extension filter, e.g. `["rs", "toml"]`
- `include_hidden`: defaults to true

**Rules**:
- Returns file sizes, modification times
- Shows subdirectories with `/` suffix
- Dotfiles and dot-directories are listed; `.git`, `.svn`, `.hg`, `.bzr`, `.jj` and `.sl` are always skipped, as is anything ignored by `.gitignore`, `.starignore`, `.ignore`/`.rgignore` or `~/.star/ignore`.
- Output is capped at 1000 entries. If it says it was truncated, list a narrower directory or lower `max_depth`.
- Use `Glob` for pattern-based file discovery
