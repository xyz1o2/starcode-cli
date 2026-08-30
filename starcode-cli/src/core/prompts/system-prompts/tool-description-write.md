<!--
name: 'Tool Description: Write'
description: Create or overwrite files
-->
Writes a file to the local filesystem.

**Use for**: creating new files, complete file rewrites.
**NOT for**: modifying existing code (use `replace`/`multi_edit`).

**Rules**:
- This tool will overwrite the existing file if there is one at the provided path.
- If this is an existing file, you MUST use the Read tool first to read the file's contents.
- Prefer the Edit tool for modifying existing files — it only sends the diff. Only use this tool to create new files or for complete rewrites.
- NEVER create documentation files (*.md) or README files unless explicitly requested by the User.
- Only use emojis if the user explicitly requests it. Avoid adding emojis to files unless asked.
- The file_path must be a distinct file path, not a directory path. If the path resolves to an existing directory, the tool will reject it.
