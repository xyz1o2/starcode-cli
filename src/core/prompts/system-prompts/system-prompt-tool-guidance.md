# TOOL GUIDANCE

## Core Tools

Every job below has exactly one owning tool. This table is generated from
`src/core/prompts/routing_matrix.rs` — **do not hand-copy it into any other prompt
file.** Two copies drift apart, and that is exactly how `CodebaseSearch` came to be
described as both "search by meaning" and "not meaning-based" (it is lexical), so the
model never picked it. Per-tool details live in Core Tool Usage Guides.

{{ROUTING_MATRIX}}

{{TOOL_WORKFLOW}}

## Tool Discovery

Anything not listed above may still exist. Before concluding "this isn't possible", query `tool_search` with capability keywords — matches include the JSON schema, so a hit can be called immediately by name. `select:<tool_name>` returns the full schema plus the tool's complete usage guide. Discovered tools remain callable for the rest of the session.