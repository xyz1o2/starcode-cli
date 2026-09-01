<!--
name: 'Tool Description: Ls'
description: List directory contents
-->
List directory contents with file details.

**Use for**: exploring project structure, checking directory contents.
**NOT for**: finding files by pattern (use `glob`), searching content (use `grep`).

**Rules**:
- Returns file sizes, modification times
- Shows subdirectories with `/` suffix
- Use `glob` for pattern-based file discovery