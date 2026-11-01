# Tool 渲染对标分析

> 参考: `docs/ui-tool-render.md` §III
> 当前: `src/ui/components/tool_render.rs`
> 快照: 2026-10-13

## 已完成

1. ToolResult 整体缩进 (`  ⎿ ` 前缀, 续行缩进)
2. ANSI 颜色保留 (256色 / truecolor / dim/italic/reverse/crossed-out)
3. Span 级宽度计算 + CJK 感知
4. `build_tool_body_block` ANSI 感知渲染
5. 折叠预览颜色保留
6. Edit diff 折叠摘要 ("Added N lines, removed M lines")
7. 7 个单元测试
8. ToolCall Header 格式 (`● ToolName(args)` + 终端宽度截断)
9. View/搜索工具单行摘要 ("Read N lines", "Found N matches", "Found N files")
10. Bash 结果摘要 ("Done" 空输出, "Running in the background" 后台任务)
11. Write 结果摘要 ("Wrote N lines to {path}")
12. **WebFetch / WebSearch 的调用行摘要**（URL / query）与结果块摘要（`Fetched N bytes` + 预览）
13. **Task / Agent 的调用行摘要**（description + `type:`）与结果块摘要（子代理交回的第一行）
14. **MCP 调用行标出 server 名**（`mcp__<server>__<tool>` 的中间段）
15. **Edit diff 保留 `@@` hunk 头**（`ui/utils/render.rs::build_diff_block`）

12–15 是 2026-10-13 补的。此前的「无剩余项」结论不实：这四类工具全部落进
`_ => summarize_object_inline(obj)` 的通用 JSON 倾倒，用户既看不出抓了什么网页、
也看不出子代理干了什么；diff 又把 hunk 头和文件头一起过滤掉，看不出改在第几行。

## 仍然缺

- `Task` 结果块在**展开态**仍走通用文本渲染（折叠态已给摘要）。
- MCP 工具没有按 server 的 schema 做结构化渲染（参数结构由外部 server 决定，
  通用倾倒是合理退路，标注 server 名已是能做到的上限）。

---

### 3.4 Write 结果摘要

**当前**: 原样输出或空

**目标**:
```
● Write(file.rs)
  Wrote 42 lines to src/main.rs
```

- 从 ToolResult 中提取: 文件路径 + 行数
- 行数: 计算写入内容的换行符数量

**涉及**: `tool_render.rs`, write 工具的 result 格式

---

## 实施建议

| 序号 | 任务 | 工作量 | 备注 |
|------|------|--------|------|
| 3.1 | ToolCall header 格式 | 小 | 改截断逻辑 |
| 3.2 | View/搜索摘要 | 中 | 需解析各工具 result |
| 3.3 | Bash 结果摘要 | 小 | 需 bash 工具配合 |
| 3.4 | Write 结果摘要 | 小 | 需 write 工具配合 |
