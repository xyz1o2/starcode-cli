# Tool 渲染对标分析 — 全部完成

> 参考: `docs/ui-tool-render.md` §III
> 当前: `src/ui/components/tool_render.rs`

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

---

## 无剩余项

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
