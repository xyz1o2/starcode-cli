<!--
name: 'Tool Description: SuggestPR'
description: Generate PR title and description
-->
Analyze branch changes and generate PR metadata. Does NOT create PR.

**Use for**: auto-generating PR title, description, change summary.
**NOT for**: actually creating PR (use `gh` via bash), non-feature branches.

**Params**: `base_branch` (auto-detects main/master)

**Rules**:
- Analyzes commit log and diff
- Generates formatted PR suggestion
- Review before creating actual PR