# 工具调用渲染对标文档（starcode TUI ↔ claude-code-main）

> 目标：让 Bash / Edit / Write / view / search 等工具的调用头部与结果输出，
> 与 Claude Code CLI 的显示效果一致：对齐、格式、折叠摘要文案。
>
> 参考实现：`study_or_copy_projects/claude-code-main/`（下称"参考项目"）
> 本项目实现：`src/ui/components/tool_render.rs` + `src/ui/utils/render.rs`

---

## 一、参考项目中的关键代码位置

| 效果 | 参考项目文件 | 关键点 |
|------|-------------|--------|
| 结果块统一前缀 `  ⎿ ` | `src/components/MessageResponse.tsx` | 固定渲染 `{'  '}⎿ &nbsp;`（5 列），**所有行**整体缩进，不是只缩进首行 |
| 折叠/展开前缀宽度注释 | `src/utils/terminal.ts` | `PADDING_TO_PREVENT_OVERFLOW = 10`，注释明确 "MessageResponse prefix (\"  ⎿ \" = 5 chars)" |
| 折叠显示行数 | `src/utils/terminal.ts` | `MAX_LINES_TO_SHOW = 3`，剩余行显示 `… +N lines (ctrl+o to expand)`；**只剩 1 行时直接显示**而不是折叠提示 |
| 工具头部 `● Name(args)` | `src/components/messages/AssistantToolUseMessage.tsx` | `<Text bold>{toolName}</Text><Text>({args})</Text>`，args 在括号内，超宽 truncate-end 省略 |
| Bash 输出保色 | `src/components/shell/OutputLine.tsx` | `<Ansi>{content}</Ansi>` 保留 ANSI 颜色；只剥下划线序列 `stripUnderlineAnsi`（"people complained about losing all formatting"） |
| 逐行 JSON 美化 | `src/components/shell/OutputLine.tsx` | `tryJsonFormatContent`：内容 ≤ 10000 字符时逐行 `tryFormatJson`（覆盖 ndjson/日志行） |
| Bash 无输出 | `packages/builtin-tools/.../BashTool/BashToolResultMessage.tsx` | `returnCodeInterpretation || (noOutputExpected ? 'Done' : '(No output)')`，dim 色 |
| Bash 后台任务 | 同上 | `Running in the background <KeyboardShortcutHint shortcut="↓" action="manage" parens />` |
| Read 结果单行 | `packages/builtin-tools/.../FileReadTool/UI.tsx` | `renderToolResultMessage` 的 text 分支永远一行：`Read <bold>N</bold> lines`（`height={1}` 固定），**内容永不内联展示**；图片 → `Read image (size)`；未变更 → `Unchanged since last read` |
| Grep 结果单行 | `packages/builtin-tools/.../GrepTool/UI.tsx` | `SearchResultSummary`（`height={1}`）：`Found <bold>N</bold> matches across <bold>M</bold> files` + `ctrl+o to expand`；**只有 verbose 才显示内容** |
| Write 结果单行 | `packages/builtin-tools/.../FileWriteTool/UI.tsx` | `Wrote <bold>N</bold> lines to <bold>{path}</bold>` |
| Edit 结果摘要 | `src/components/FileEditToolUpdatedMessage.tsx` | 折叠：`Added <bold>N</bold> lines, removed <bold>M</bold> lines`；展开：摘要行 + `StructuredDiffList` |

### 结论（已和用户确认的设计口径）

- **看代码类工具（view/Read、search/Grep、glob/ls）默认就是一行摘要**，不展示内容：
  - 头部 `● view(path)` 已说明看了哪个文件；
  - 结果行 `⎿ Read N lines` / `⎿ Found N matches` 即可；
  - 展开后（我们的 Tab，对应参考项目 verbose/ctrl+o）才显示完整内容。
- Bash 输出保留 ANSI 颜色（git/ls/测试运行器的彩色不再褪色）。
- 工具头部参数放进括号里，整行按终端宽度截断（不再 60 字符处硬切）。

---

## 二、本项目当前状态

### 已完成（commit c2966d1）

1. **ToolResult 整体缩进**：首行 `  ⎿  `（`theme.subtle` 色），续行 5 空格，整个输出块左缘对齐
   （`tool_render.rs` 的 ToolResult 分支）。
2. **ANSI 颜色保留**：`render.rs` 的 `parse_ansi_text` 支持 256 色（`38;5;N`）、truecolor（`38;2;R;G;B`）、
   dim/italic/reverse/crossed-out 及对应 reset（`apply_sgr_params`）。
3. **带色行的宽度处理**：新增 `truncate_spans_to_width` / `wrap_spans_to_width` /
   `split_span_at_width`（span 级、CJK 感知、宽字符不拆分不丢弃）。
4. **`build_tool_body_block`**：含 `\x1b` 的行走 `parse_ansi_text` 保色渲染；无色行维持原语法高亮/列表/
   key-value 逻辑；内容 ≤ 10000 字符时逐行 JSON 美化（`try_format_json_line`）。
5. **折叠预览保色**：`render_tool_result_text` 折叠态对带色行保色截断；错误输出带色行保色、无色行红色。
6. **Edit 折叠摘要**：diff 折叠态显示 `Added N lines, removed M lines`（去掉内嵌 ⎿，由统一前缀提供）。
7. 单元测试 7 个（`render.rs::tests`）：SGR 256/truecolor、保色、按宽换行、span 截断、宽字符不拆、
   逐行 JSON、CJK 截断。

### 遗留（下一步实施，见第三节）

- ToolCall 头部仍是 `● bash <灰色命令>`，60 字符截断 → 用户反馈"一大串然后省略号，效果不好"。
- view/search 结果折叠态仍显示最多 8 行正文预览（`TOOL_RESULT_PREVIEW_LINES = 8`），应改为一行摘要。
- Bash 无输出时没有 `Done`；后台任务没有 `Running in the background`。
- Write（create_file）结果没有 `Wrote N lines to path` 摘要。

---

## 三、待实施方案

### 3.1 ToolCall 头部：`● ToolName(args)`

`tool_render.rs` ToolCall 分支，把

```rust
Span::styled(tool_name, BOLD+primary), Span::raw(" "), Span::styled(short_summary, Gray)
```

改为（对标 AssistantToolUseMessage）：

```rust
Span::styled(tool_name, BOLD+primary),
Span::raw("("),
Span::styled(args_str, 默认前景色),   // 不再灰色
Span::raw(")")
```

- 截断：整行（前缀 5 + 工具名 + 括号 + args）按 `area_width` 用
  `truncate_to_display_width` 截断（省略号），即参考项目的 `wrap="truncate-end"`。
- `build_tool_argument_display` 中 Bash 的 `directory: ...` extra 不再拼进头部（参考项目无此干扰）。

### 3.2 看代码类工具：一行摘要（默认折叠态）

在 `render_rich_tool_content` 的非 diff 分支之前，按工具名分派（`expanded=false` 时）：

| 工具（tc.function.name） | 摘要行（dim 色，数字加粗） | 依据 |
|---|---|---|
| `view_file` / `Read` | `Read N lines`（N = output 行数） | FileReadTool/UI.tsx text 分支 |
| `Grep` / `search_file_content` / `grep_search` | `Found N matches`（N = 非空行数；行多为 `path:ln:text`） | SearchResultSummary |
| `find_by_name` / `list_directory` / `ListDir` | `Found N files`（N = 非空行数） | Glob/文件列表 |
| 其他工具 | 维持现 8 行预览逻辑不变 | OutputLine 的 3 行预览精神 |

- 摘要行后追加 dim 提示 ` (Tab to expand)`（对标 `ctrl+o to expand`），仅当有内容可展开。
- `expanded=true`（Tab 展开后）仍显示完整内容（等价参考项目 verbose）。
- 错误结果不受影响，仍红色显示。

### 3.3 Bash 结果特例

在 `render_tool_result_text` 之前（或其内部开头）：

- `success && output 为空（trim 后）` → 一行 dim `Done`（对标 `noOutputExpected ? 'Done' : '(No output)'`；
  我们无 returnCodeInterpretation，直接 `Done`）。
- 后台任务：`tr.data.background_task_id` 存在时 → `Running in the background (↓ to manage)`。
  ⚠️ 当前 `ToolResult.data` 尚无该字段（types/mod.rs:20-28），需先在 bash 工具执行器里写入，UI 侧先做
  兼容读取，取不到就跳过。

### 3.4 Write 结果摘要

`create_file` / `Write` 且无 diff 时：

- 行数 N 取 args 的 `content` / `file_text` 字段行数（头部已知 path）；
- 显示 `Wrote N lines to {shorten(path)}`（数字与路径加粗，对标 FileWriteTool/UI.tsx:59）；
- args 解析失败时回退现有输出预览。

### 3.5 验证

- `cargo check` + `cargo test --lib ui::`；
- 为 3.2/3.3/3.4 的摘要文案补单元测试（纯函数化 `tool_result_summary_line(tc, tr) -> Option<Line>` 便于测试）；
- 手工验证：`git log --color`、`ls --color`、ndjson 输出、view 大文件、空输出 bash、
  write/edit 文件后的折叠与 Tab 展开。

---

## 四、已知不动的部分

- `chat_input::tests::test_border_color_default` 失败为存量问题（期望 `DarkGray` 实际 `Rgb(100,100,100)`），
  与本次渲染改动无关（改动仅涉及 `render.rs` / `tool_render.rs`，见 `git show --stat c2966d1`）。
- 参考项目的 `Ratchet`/`NoSelect`（防收缩、选择区域）为 ink 框架特性，ratatui 无直接对应，暂不对标。
