<!--
name: 'Tool Description: MultiEdit'
description: Multiple edits in single operation
-->
Apply multiple edits across one or more files in a single operation.

**Use for**: coordinated changes across multiple files, batch renames, refactoring.
**NOT for**: single simple edit (use `Edit`), creating files (use `Write`).

**Rules**:
- All edits applied atomically
- Each edit needs unique `old_string`
- Preserve exact indentation