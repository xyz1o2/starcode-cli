<!--
name: 'Tool Description: WebSearch'
description: Search the web for information
-->
Search the web for current information, documentation, solutions.

**Use for**: latest docs, error solutions, API references, package info.
**NOT for**: local code search (use `grep`), project files (use `glob`).

**Params**: `query` (search terms), `num` (results, default 5, max 10)

**Rules**:
- Returns titles, URLs and snippets only — not page bodies. To read a page, call `WebFetch` on its URL with a `prompt`.
- One search per question. The same query returns the same list, so re-running it tells you nothing new; if the results miss, change the wording or accept that the answer is not indexed.
- Snippets are often enough. Fetch a page only when you need something the snippet does not give you.
- Use the current year in queries when asking for "latest" anything.
