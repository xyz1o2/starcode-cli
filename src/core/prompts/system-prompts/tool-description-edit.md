<!--
name: 'Tool Description: Edit'
description: Exact string replacement in files
-->
Performs exact string replacements in files.

**Use for**: modifying existing code, renaming symbols, fixing bugs.
**NOT for**: creating new files (use `Write`), bulk rewrites (use `multi_edit`).

**Rules**:
- MUST read file first — edit fails on unread files
- Preserve exact indentation (tabs/spaces)
- `old_string` must be unique, or set `replace_all: true`
- Smallest possible match to avoid unintended changes
- ALWAYS prefer editing existing files in the codebase. NEVER write new files unless explicitly required.
- Use `replace_all: true` for renaming a symbol across the whole file; use `expected_replacements: N` when you know the exact count and want the edit to fail if it differs.
- The edit will FAIL if `old_string` is not unique in the file. Either provide a larger string with more surrounding context to make it unique, or set `replace_all: true` to change every instance.
- Only use emojis if the user explicitly requests it. Avoid adding emojis to files unless asked.
