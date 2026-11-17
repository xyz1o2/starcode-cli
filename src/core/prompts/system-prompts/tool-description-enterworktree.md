<!--
name: 'Tool Description: EnterWorktree'
description: Create isolated Git worktree
-->
Create a detached Git worktree from the configured project's HEAD. This creates a directory; it does not switch the session's working directory.

**Use for**: high-risk changes, experimental refactors, trying approaches.
**NOT for**: small changes, non-Git repos.

**Params**: `path` (optional, uses temp dir if omitted)

**Rules**:
- Requires user confirmation
- Pass the returned absolute path explicitly to subsequent tools; their default directory is unchanged
- Changes made inside the returned directory are separate from the main working tree
- Use `ExitWorktree` to clean up