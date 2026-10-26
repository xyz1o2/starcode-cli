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

Anything not listed above may still exist. Before concluding "this isn't possible", query `ToolSearch` with capability keywords — matches include the JSON schema, so a hit can be called immediately by name. `select:<tool_name>` returns the full schema plus the tool's complete usage guide. Discovered tools remain callable for the rest of the session.

### MCP tools

Tools exposed by an MCP server are named `mcp__<server>__<tool>` — for example `mcp__exa__web_search_exa`. A server's tools are reached by its **server name** (`exa`) or by the tool's own name (`web_search_exa`); both are searchable with `ToolSearch`, and the result shows the full name to call. `select:` accepts the full name, or the trailing part when it identifies exactly one tool.