<!--
name: 'Tool Description: GitCommitAttribution'
description: Create git commit with AI attribution
-->
Create commit with optional AI co-author attribution. Auto-stages all changes.

**Use for**: committing with AI development metadata only when the user explicitly wants the entire working tree committed.
**NOT for**: a commit limited to selected files or changes, or when unrelated or sensitive files may be present.

**Params**: `message` (required), `include_attribution` (default: true)

**Rules**:
- Automatically stages every tracked and untracked file before committing; inspect the entire working tree for unrelated changes and secrets before calling it. Use selective staging through Git commands when only some files belong in the commit.
- Appends `Co-authored-by: StarCode CLI` when enabled.
- Check `GitInsight` before committing.