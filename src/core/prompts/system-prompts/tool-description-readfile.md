<!--
name: 'Tool Description: ReadFile'
description: Read file from filesystem
-->
Reads a file from the local filesystem. You can access any file directly by using this tool.

**Use for**: viewing code, reading configs, inspecting files.
**NOT for**: `cat`/`head`/`tail` via bash — this tool is optimized for the task.

**Key params** (`Read`):
- `file_path`: absolute path (required for a single file)
- `file_paths`: list of paths — batch reading, replaces `file_path`
- `offset`: 0-based start line (requires `limit`)
- `limit`: max lines to read

**Key params** (`read_many_files` — same tool family, batch-oriented):
- `items`: list of paths to read
- `max_size_per_file` / `skip_binary` / `truncate_lines`: optional size and binary control

**Rules**:
- Paths must be absolute, not relative
- By default, it reads up to 2000 lines starting at the beginning of the file
- Results are returned using cat -n format, with line numbers starting at 1
- Handles images (PNG, JPG, GIF, WEBP, SVG, BMP) and PDFs as well as text
- Jupyter notebooks (`.ipynb`) have a dedicated tool: `notebook_read`
- This tool reads files, not directories — use `ListDir` for a directory listing
- To jump to a grep match: line numbers are 1-based but `offset` is 0-based, so pass `offset = line - 11` with `limit: 60` to start ~10 lines before the match
- Avoid tiny slices — read 60-100 lines for context
