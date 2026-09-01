<!--
name: 'Tool Description: Glob'
description: Find files by name pattern
-->
Fast file pattern matching tool that works with any codebase size.

**Use for**: finding files by name/path patterns (`**/*.js`, `src/**/*.ts`).
**NOT for**: searching file contents (use `Grep`).

**Rules**:
- Supports glob patterns like `**/*.js` or `src/**/*.ts`
- Returns matching file paths sorted by modification time
- Use this tool when you need to find files by name patterns
- When you are doing an open ended search that may require multiple rounds of globbing and grepping, use the Agent tool instead
- Batch parallel globs for multiple candidate patterns
