<!--
name: 'Tool Description: SemanticSearch'
description: Natural language code search
-->
Search code by natural language meaning, not just keywords.

**Use for**: conceptual queries ("error handling", "auth flow"), finding by intent.
**NOT for**: exact string matches (use `grep`), file name search (use `glob`).

**Rules**:
- Returns ranked results by semantic relevance
- Best for exploring unfamiliar codebases
- Combine with `grep` for precise matches