<!--
name: 'Tool Description: GitCommitAttribution'
description: Create git commit with AI attribution
-->
Create commit with optional AI co-author attribution. Auto-stages all changes.

**Use for**: committing with AI development metadata.
**NOT for**: precise staging control (auto `git add -A`), no-attribution commits.

**Params**: `message` (required), `include_attribution` (default: true)

**Rules**:
- Runs `git add -A` automatically
- Appends `Co-authored-by: StarCode CLI` when enabled
- Check `git_insight` before committing