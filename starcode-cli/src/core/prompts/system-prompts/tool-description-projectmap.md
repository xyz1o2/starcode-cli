<!--
name: 'Tool Description: ProjectMap'
description: Generate codebase structure overview
-->
Generate hierarchical map of project structure and dependencies.

**Use for**: understanding codebase architecture, module relationships, entry points.
**NOT for**: finding specific files (use `Glob`), reading file content (use `Read`).

**Trigger conditions (call when ANY of these apply)**:
1. User explicitly asks about project structure/architecture (e.g., "what's the project structure", "help me understand this codebase")
2. User is new to the project and needs an overview (e.g., "what does this project do")
3. Before making cross-module architectural changes, to understand module relationships
4. User asks about entry points, dependencies, module boundaries, or other architectural questions

**Do NOT trigger when**:
- User is looking for a specific file → use `glob`
- User wants to read file content → use `Read`
- User's question only involves changes within a single file → no project map needed
- User already has sufficient context, no additional overview needed

**Rules**:
- Shows module hierarchy and key relationships
- Useful for onboarding to new codebases
- Run before making architectural changes