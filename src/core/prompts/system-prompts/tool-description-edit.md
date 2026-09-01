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
- `old_string` must be unique or use `replace_all`
- Smallest possible match to avoid unintended changes
- ALWAYS prefer editing existing files in the codebase. NEVER write new files unless explicitly required.
- Use `replace_all` for replacing and renaming strings across the file.
- The edit will FAIL if `old_string` is not unique in the file. Either provide a larger string with more surrounding context to make it unique or use `replace_all` to change every instance.
- Only use emojis if the user explicitly requests it. Avoid adding emojis to files unless asked.
