<!--
name: 'Tool Description: LSP'
description: Interact with Language Server Protocol (LSP) for code navigation like go-to-definition, find-references, hover info.
-->
Interact with Language Server Protocol (LSP) for code navigation.

**Use for**: go-to-definition, find-references, hover information, workspace symbols.
**NOT for**: plain text search (use `grep`/`glob`).

**Rules**:
- Requires an active language server for the file's language
- Prefer LSP over grep when you need semantic understanding (definitions, references)
