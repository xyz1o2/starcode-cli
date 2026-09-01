<!--
name: 'Tool Description: EnterWorktree'
description: Create isolated Git worktree
-->
Create isolated worktree for experimental changes. Main directory unaffected.

**Use for**: high-risk changes, experimental refactors, trying approaches.
**NOT for**: small changes, non-Git repos.

**Params**: `path` (optional, uses temp dir if omitted)

**Rules**:
- Requires user confirmation
- Changes don't affect main working directory
- Use `exit_worktree` to clean up