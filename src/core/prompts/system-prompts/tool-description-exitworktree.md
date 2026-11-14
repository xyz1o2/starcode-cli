<!--
name: 'Tool Description: ExitWorktree'
description: Remove isolated Git worktree
-->
Remove a registered secondary Git worktree only if it is clean and its HEAD is contained in the main worktree's HEAD. The tool does not merge commits or discard local files.

**Use for**: cleaning up a worktree after its changes have been integrated into the main worktree.
**NOT for**: preserving or merging uncommitted work; save or integrate changes explicitly before removal.

**Params**: path (required), keep_changes (optional compatibility flag; neither value permits discarding changes).

**Rules**:
- Inspect the worktree first. Uncommitted, untracked or ignored files and unintegrated commits cause removal to fail without deletion.
- Do not claim the tool merged changes or silently retry with a force removal.