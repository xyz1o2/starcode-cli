# Tool 渲染对标分析 — 剩余 4 项

> 参考: `docs/ui-tool-render.md` §III
> 当前: `src/ui/components/tool_render.rs`

## 已完成 (commit c2966d1)

1. ToolResult 整体缩进 (`  ⎿ ` 前缀, 续行缩进)
2. ANSI 颜色保留 (256色 / truecolor / dim/italic/reverse/crossed-out)
3. Span 级宽度计算 + CJK 感知
4. `build_tool_body_block` ANSI 感知渲染
5. 折叠预览颜色保留
6. Edit diff 折叠摘要 ("Added N lines, removed M lines")
7. 7 个单元测试

---

## 剩余项

### 3.1 ToolCall Header 格式

**当前**:
```
● bash <gray command text, 60 char truncation>
```

**目标**:
```
● ToolName(args)
```
- args 用终端宽度截断 (非固定 60 字符)
- 超长时: `● Bash(very long command that exceeds t...)`
- 颜色: 工具名用工具类型色, args 用灰色

**涉及**: `tool_render.rs` 中 ToolCall header 渲染逻辑

---

### 3.2 View/搜索工具单行摘要

**当前** (Read/Grep 等):
```
● Read(file.rs)
  ├ line 1: content...
  ├ line 2: content...
  ├ ... (8 行预览)
```

**目标**:
```
● Read(file.rs)
  Read 42 lines
```
或:
```
● Grep(pattern)
  Found 7 matches in 3 files
```

- 单行摘要替代 8 行预览
- Tab 键展开显示完整内容 (已有折叠机制可复用)
- 不同工具不同摘要格式:
  - Read: "Read N lines"
  - Grep: "Found N matches in M files"
  - Glob: "Found N files"
  - LS: "Listed N items"

**涉及**: `tool_render.rs`, 各工具的 `ToolResult.data` 解析

---

### 3.3 Bash 结果摘要

**当前**: 原样输出 stdout/stderr

**目标**:
```
# 空输出:
● Bash(command)
  Done

# 后台任务:
● Bash(long-running-command)
  Running in the background (task_id: abc123)
```

- 空 stdout + exit code 0 → "Done"
- 后台任务 → "Running in the background" + task_id
- 需要 `ToolResult.data` 中包含 `background_task_id` 字段

**涉及**: `tool_render.rs`, bash 工具的 result 格式

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
