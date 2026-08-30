<!--
name: 'Tool Description: ExitWorktree'
description: Remove isolated Git worktree
-->
Remove worktree. Discards changes by default, `keep_changes=true` to merge.

**Use for**: cleaning up after worktree work.
**NOT for**: worktree already removed, still working in it.

**Params**: `path` (required), `keep_changes` (default: false)

**Rules**:
- Default discards all uncommitted changes
- Requires user confirmation
- Verify work complete before exiting