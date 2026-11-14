<!--
name: 'Tool Description: ExitWorktree'
description: Remove isolated Git worktree
-->
Remove an isolated Git worktree. This operation may discard uncommitted changes.

**Use for**: cleaning up a worktree only after verifying its changes are safely saved elsewhere.
**NOT for**: preserving uncommitted work; the current keep-changes option does not reliably carry uncommitted changes to the parent branch.

**Params**: path (required), keep_changes (default: false).

**Rules**:
- Inspect the worktree before removal and obtain the user's confirmation for any data loss. A keep-changes request is not proof that files will survive removal.
- If preservation or merge fails, leave the worktree intact and report the failure; do not claim changes were kept.