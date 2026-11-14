<!--
name: 'Tool Description: MultiEdit'
description: Multiple edits in single operation
routing: edit-many
-->
Apply multiple edits across one or more files in a single operation.

**Use for**: coordinated changes across multiple files, batch renames, refactoring.
**NOT for**: single simple edit (use `Edit`); prefer `Write` when the main goal is creating a file.

**Rules**:
- Validates edits before applying them, but cross-file writes are not fully atomic; a write failure triggers best-effort rollback. Inspect the working tree if an operation fails.
- Each edit needs unique `old_string`
- Preserve exact indentation