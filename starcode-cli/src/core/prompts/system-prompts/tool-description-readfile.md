<!--
name: 'Tool Description: ReadFile'
description: Read file from filesystem
-->
Reads a file from the local filesystem. You can access any file directly by using this tool.

**Use for**: viewing code, reading configs, inspecting files.
**NOT for**: `cat`/`head`/`tail` via bash — this tool is optimized for the task.

**Key params**:
- `filePath`: absolute path (required for single file)
- `file_paths`: list of paths (optional, for batch reading)
- `offset`: start line (0-indexed)
- `limit`: max lines (default 2000)
- `pages`: page range for PDF files (e.g., "1-5", "10-20")

**Rules**:
- The file_path parameter must be an absolute path, not a relative path
- By default, it reads up to 2000 lines starting from the beginning of the file
- Results are returned using cat -n format, with line numbers starting at 1
- This tool allows reading images (eg PNG, JPG, etc)
- This tool can read PDF files (.pdf). For large PDFs (more than 10 pages), provide the pages parameter.
- This tool can read Jupyter notebooks (.ipynb files)
- This tool can only read files, not directories. To read a directory, use an ls command via the Bash tool.
- After grep match, use offset to jump: `Read(path, offset=line-10, limit=60)`
- Batch parallel reads when multiple files needed (use `file_paths` parameter)
- Avoid tiny slices — read 60-100 lines for context
