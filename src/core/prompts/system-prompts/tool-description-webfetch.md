<!--
name: 'Tool Description: WebFetch'
description: Fetch and read web page content
-->
Fetch URL content and return as markdown/text.

**Use for**: reading documentation, fetching API responses, web scraping.
**NOT for**: searching web (use `WebSearch`), local files (use `Read`).

**Params**: `url`, `format` (markdown/text/html)

**Rules**:
- Converts HTML to clean markdown
- Use after `WebSearch` to read full content
- Handle timeouts for slow sites