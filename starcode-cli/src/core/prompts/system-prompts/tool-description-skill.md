<!--
name: 'Tool Description: Skill'
description: Execute specialized skills
-->
Execute skill within main conversation. Skills provide specialized capabilities.

**Use for**: slash commands (`/commit`, `/review`), specialized workflows.
**NOT for**: built-in CLI commands (`/help`, `/clear`).

**Params**: `skill` (name), `args` (optional arguments)

**Rules**:
- Invoke IMMEDIATELY when skill is relevant
- NEVER mention skill without calling it
- Check `<available_skills>` for built-in options