# Explore UI 对标分析 — CCB vs Starcode

## 架构对比

| 维度 | CCB (Claude Code) | Starcode | 差距 |
|------|-------------------|----------|------|
| **共享组件** | `FuzzyPicker<T>` 泛型组件，三个对话框共用 | 三个独立组件，无共享抽象 | 🟡 待重构 |
| **方向** | `direction='up'` atuin风格，搜索框在底部 | 标准垂直布局，搜索框在顶部 | 🟡 风格差异 |
| **响应式布局** | 根据终端宽度切换预览位置（右/下） | ✅ 已实现（140/120/100 阈值） | ✅ |
| **键盘输入** | 完整 readline 编辑 + Ctrl+P/N 导航 | ✅ 已实现（Up/Down/Enter/Esc/Char/Backspace） | ✅ |
| **Tab/Shift+Tab** | Tab=mention, Shift+Tab=insert path | ✅ 已实现 | ✅ |
| **模糊匹配** | nucleo 近似匹配（QuickOpen）| fd 字面匹配 | 🟡 够用 |
| **预览** | 文件内容预览，AbortController 防堆积 | ✅ 已实现（file:line + 内容 + 预览区） | ✅ |
| **结果高亮** | inverse video 高亮 query 匹配 | ✅ 已实现（highlight_query_matches） | ✅ |
| **流式搜索** | ripgrep 流式输出，增量合并结果 | 一次性等待 ripgrep 完成 | 🟡 可优化 |
| **Debounce** | GlobalSearch: 100ms debounce | Generation counter 防过期 | ✅ |
| **Overlay 协调** | useRegisterOverlay，一次只能打开一个 | Modal Stack 已实现 | ✅ |
| **Keybinding** | Ctrl+Shift+F / Ctrl+Shift+P / Ctrl+R | ✅ 已实现 | ✅ |
| **matchLabel 防跳** | 空结果时传 ' ' 保留行高 | ✅ 已实现 | ✅ |
| **Empty Message** | "Searching…", "No matches", "Type to search…" | ✅ 已对标 CCB | ✅ |
| **Byline 提示** | "↑/↓ navigate  Enter open  Tab mention  Shift+Tab insert path  Esc cancel" | ✅ 已实现 | ✅ |

## CCB Explore UI 核心实现规格

### FuzzyPicker 通用壳

```
Props<T>:
  title: string
  placeholder: string (default: 'Type to search…')
  items: T[]
  getKey: (item: T) => string
  renderItem: (item: T, isFocused: boolean) => ReactNode
  renderPreview?: (item: T) => ReactNode
  previewPosition: 'bottom' | 'right' (default: 'bottom')
  visibleCount: number (default: 8)
  direction: 'down' | 'up' (default: 'down')
  onQueryChange: (query: string) => void
  onSelect: (item: T) => void      // Enter
  onTab?: PickerAction<T>          // Tab
  onShiftTab?: PickerAction<T>     // Shift+Tab
  onCancel: () => void             // Esc
  emptyMessage: string | ((query: string) => string)
  matchLabel: string               // 状态行
  selectAction: string             // default: 'select'
```

**布局结构 (direction='up'):**
```
Pane (paddingTop=1, paddingX=2)
  ───────────────────────────────── (Divider)
  Box (flexDirection=column, gap=1)
    Title (bold, color="permission")
    ListGroup (column-reverse, item[0] 在底部)
    SearchBox (borderStyle="round", 3行高)
    Byline (↑/↓ · Enter · Tab · Shift+Tab · Esc)
```

### GlobalSearchDialog

**常量:**
- VISIBLE_RESULTS = 12
- DEBOUNCE_MS = 100
- PREVIEW_CONTEXT_LINES = 4
- MAX_MATCHES_PER_FILE = 10
- MAX_TOTAL_MATCHES = 500

**响应式:**
- previewOnRight = columns >= 140
- visibleResults = min(12, max(4, rows - 14))

**Item 格式:** `path:line matchText`（path 灰色，match 高亮）

**选择动作:**
- Enter: 打开外部编辑器
- Tab: 插入 `@file#Lline ` (mention)
- Shift+Tab: 插入 `file:line ` (plain)

### QuickOpenDialog

**常量:**
- VISIBLE_RESULTS = 8
- PREVIEW_LINES = 20

**响应式:**
- previewOnRight = columns >= 120

**选择动作:**
- Enter: 打开外部编辑器
- Tab: 插入 `@path ` (mention)
- Shift+Tab: 插入 `path ` (plain)

### HistorySearchDialog

**常量:**
- PREVIEW_ROWS = 6
- AGE_WIDTH = 8

**响应式:**
- previewOnRight = columns >= 100

**过滤算法（两级）:**
1. 精确子串匹配（String.includes）→ 排前面
2. 子序列匹配（isSubsequence）→ 排后面

**Item 格式:** `{8字符 age} {首行内容}`

**选择动作:**
- Enter: 使用条目（填入输入框）

## 已实现的功能

### Phase 1: 键盘输入 + Keybinding ✅

- `modal_input.rs`: handle_global_search, handle_quick_open, handle_history_search
- `input.rs`: Ctrl+Shift+F, Ctrl+Shift+P, Ctrl+R 快捷键
- 支持: Up/Down/Ctrl+P/Ctrl+N 导航, Enter/Tab/BackTab 选择, Esc 关闭, 字符输入, Backspace

### Phase 2: 搜索增强 ✅

- `execute_search` 返回 `(Vec<SearchResult>, bool)` 含截断标志
- `merge_results` 辅助函数支持追加+去重
- ripgrep 参数对标 CCB: `-n --no-heading -i -m 10 -F -e query`
- Generation counter 防过期结果

### Phase 3: 预览功能 ✅

- GlobalSearch: 底部/右侧预览区显示 file:line + 匹配内容
- QuickOpen: 新增预览区，显示文件路径和预览提示
- HistorySearch: 新增预览区，圆角边框，自动换行显示完整 prompt

### Phase 4: 响应式布局 ✅

- GlobalSearch: columns >= 140 时预览在右侧
- QuickOpen: columns >= 120 时预览在右侧
- HistorySearch: columns >= 100 时预览在右侧

### Phase 5: UI 细节对标 ✅

- Tab/Shift+Tab 动作: mention 和 insert path 格式
- 结果高亮匹配: `highlight_query_matches` 函数，inverse video 高亮
- matchLabel 防跳: 空结果时传 ' ' 保留行高
- Empty Message 对标 CCB: "Searching…", "No matches", "Type to search…"
- 标题文案对标 CCB: " Search ", " Quick Open ", " Search prompts "
- Byline 快捷键提示: 完整的快捷键说明

### Phase 6: 滚动和截断 ✅ 已完成

15. **滚动指示器** ✅ — 列表边缘显示 ↑/↓ 箭头（对标 CCB ListItem）
    - GlobalSearch/QuickOpen/HistorySearch 均已实现
    - 窗口化渲染：只渲染可见区域，超出部分用 ↑/↓ 指示

16. **响应式可见行数** ✅ — 根据 list_area.height 动态计算可见行数
    - `visible_count = list_area.height.saturating_sub(2).max(2)`

17. **路径截断** ✅ — `truncate_path_middle` 函数保留两端
    - 例如: `/very/long/path/to/file.rs` → `/very/.../file.rs`
    - GlobalSearch: 40 字符宽度
    - QuickOpen: 50 字符宽度

### Phase 7: 客户端预过滤 + FuzzyPicker 抽象 ✅ 已完成

18. **客户端预过滤** ✅ — ripgrep/fd 等待期间先过滤现有结果，避免空白闪烁
    - GlobalSearch: `retain` 过滤现有 results
    - QuickOpen: `retain` 过滤现有 files

19. **FuzzyPicker 抽象** ✅ — `fuzzy_picker.rs` 共享模块
    - `compute_layout`: 响应式布局计算
    - `render_search_input`: 搜索输入框
    - `compute_window`: 可见窗口计算
    - `scroll_indicator`: 滚动指示器
    - `render_scrolling_list`: 带滚动指示器的列表
    - `render_empty_state`: 空状态消息
    - `format_match_label`: matchLabel 格式化
    - `render_byline`: 快捷键提示

### Phase 8: UI 细节精对标 ✅ 已完成

20. **Pane 分割线** ✅ — 顶部全宽 `─` 水平线 + paddingTop=1 + paddingX=2
21. **SearchBox 前缀** ✅ — `⌖` (U+2316) 前缀字符 + placeholder 首字符反色
22. **圆角边框** ✅ — SearchBox 使用 `BorderType::Rounded`
23. **Byline middot 分隔** ✅ — 用 ` · ` 分隔快捷键提示
24. **Compact 模式** ✅ — columns < 120 时缩短标签（navigate→nav, mention→ment, insert path→path）

**所有 24 项功能已全部实现。**

---

**文档版本:** 2026-09-04
**状态:** 全部 24 项功能已实现 ✅
