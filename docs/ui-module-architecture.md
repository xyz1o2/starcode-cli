# UI 模块架构与重构方案（starcode TUI）

> 目标：把 `src/ui` 从"能跑但处处重复"的状态，收敛成"少数几个说得清的不变式"。
> 本文档记录现状、已落地的选区重构，以及后续按风险排序的抽取清单。
>
> 相关文档：`docs/ui-tool-render.md`（工具渲染对标）、`docs/parity-ui-framework.md`（UI 框架对标）

---

## 一、模块地图

### 1.1 分层（依赖方向自上而下，无编译期环）

```
app/          帧循环、页面装配（render_page 是真正的枢纽）
  ↓
events/       键盘 / 鼠标 / 模态输入分发          ← input.rs 236KB，是最大的单文件
  ↓
components/   纯渲染函数（chat_history / tool_render / message_render / task_panel …）
  ↓
state/        ChatState（共享状态图，store.rs 95KB）
  ↓
utils/        无状态工具（render / text / format / clipboard）—— 唯一可以放心单测的一层
  ↓
themes/       颜色与样式
```

`utils/` 不 import 任何 `ui` 内部模块，是唯一纯的一层；`state/` 依赖 `components/virtual_list`，
反向不依赖。

### 1.2 体量最大的文件

| 文件 | 体积 | 问题 |
|------|------|------|
| `events/input.rs` | 236KB | 键盘分发 + 模式 + Vim 动作 + 粘贴 + 命令解析全塞在一起 |
| `services/stream.rs` | 175KB | SSE 解析 + 工具分发 + 状态变更混在一起 |
| `components/palette.rs` | 113KB | 30+ 个 `get_<X>_palette_items()`，一个模块装下所有动作目录 |
| `state/store.rs` | 95KB | `ChatState` 本体，450+ 字段 |
| `components/status_line/mod.rs` | 89KB | spinner + token 告警 + git + 花费 + 上下文窗口 |
| `app/runtime.rs` | 81KB | 事件循环 |
| `utils/render.rs` | 65KB | 排版基建（宽度、折行、ANSI、diff） |
| `components/tool_render.rs` | 58KB | 工具调用/结果渲染 |

> 这些是**体量**问题，不等于都要拆。拆分优先级见第三节。

### 1.3 死代码（0 调用）

| 位置 | 说明 |
|------|------|
| `components/interrupted.rs` | 注释自述"零调用" |
| `vim/operators.rs` / `vim/text_objects.rs` | `ciw` / `d{motion}` 从未实现 |
| `components/chat_history.rs::render_chat_history` | 滚动条逻辑已迁到 `app/mod.rs`，函数本身没人调 |

---

## 二、选区与复制（本轮已重构）

### 2.1 原状：四个文件各写一遍，且有一个真 bug

| 位置 | 职责 |
|------|------|
| `state/store.rs` | `TextSelection`、`get_selected_text()`、`char_index_at_visual_col()`、`get_selection_range()`、`is_position_selected()` |
| `components/chat_history.rs` | `apply_selection_highlight()`、`split_by_width()` |
| `events/mouse.rs` | 拖选、鼠标松开即复制 |
| `events/input.rs` | Ctrl+C 复制 |

问题：

1. **高亮会静默丢失**（真 bug，用户可见）
   `split_by_width(text, start_width, end_width)` 里 `in_middle` 只在前缀分支里被置位，
   于是 `start_width == 0` 时选中段恒为空、整段文字掉进后缀。而
   `apply_selection_highlight` 的 `rel_start` 在 `current_col >= sel_start_col` 时
   恰好就是 0 —— 也就是**选区起点之后的每一个 span 都不显示高亮**。
   屏幕上只有第一个 span 的后半截变蓝，后面选中的一大片完全没反应。

2. **范围解析会 panic**
   `get_selection_range()` 对 `Option<usize>` 比大小后 `unwrap()`，四个字段缺一个
   （流式更新中途清掉一半、会话恢复）就直接 panic。

3. **列宽口径三套**
   `split_by_width` 按字符累加、`apply_selection_highlight` 用 `UnicodeWidthStr::width`、
   `char_index_at_visual_col` 用 `UnicodeWidthChar::width` —— 三处不是一个口径。

4. **复制入口两个**
   鼠标松开（`mouse.rs`）和 Ctrl+C（`input.rs`）各写一遍"取文本 → 写剪贴板 → 设提示"。

### 2.2 现在的结构

新增 `src/ui/selection.rs`，选区只有一份实现：

```
ui/selection.rs
  SelectionRange::resolve(selection) -> Option<SelectionRange>   范围解析（无 panic）
  SelectionRange::contains(entry, row, col) -> bool               点是否在选区内
  selected_text(range, lines_for) -> Option<String>               取文本（纯函数）
  entry_text(state, entry_idx) -> Option<String>                 整个条目的文本
  apply_highlight(lines, range, entry_idx)                        上高亮（纯函数）
  copy_selection(state) -> Result<(), String>                     ★ 唯一复制入口
  entry_rendered_lines(state, entry_idx)                          取渲染行（缓存缺失时兜底重渲染）

ui/utils/text.rs
  split_by_width / char_index_at_visual_col / line_plain_text     列宽口径唯一出处
```

数据流：

```
鼠标按下/拖动 ──► mouse.rs ──► state.text_selection（只有起止两个锚点）
                                        │
渲染帧  ──► chat_history.rs ──► selection::apply_highlight（纯函数，就地改 spans）
                                        │
松开 / Ctrl+C ──► selection::copy_selection ──► utils/clipboard（arboard → OSC 52）
```

### 2.3 不变式（改这个模块时必须守住）

| 不变式 | 含义 |
|--------|------|
| **单一列宽口径** | 任何"第 N 列"都用 `render::char_display_width`；与 ratatui 分配缓冲区格子的规则一致 |
| **高亮与复制同源** | 高亮和取文本都走 `selection.rs`，不允许再出现第二份换算 |
| **范围解析不 panic** | 字段缺失 → `None` → 当作"没有选区"，绝不 `unwrap()` |
| **复制只有一个入口** | 任何新的复制入口（右键、快捷键、"复制代码块"按钮）都调 `copy_selection` |
| **选区是渲染坐标** | (条目下标, 条目内**渲染行号**, 可视列)；不是文件行号 |

### 2.4 复用点（还没做，但接口已经留好）

- `entry_text(state, idx)` —— "复制这个代码块 / 这个工具结果"直接用它，
  与选区复制共用渲染与列宽口径，不会出现"屏幕上是一份、复制出来是另一份"。
- `selected_text(range, lines_for)` —— `lines_for` 是回调，所以任何能提供
  "条目 → 渲染行"的地方（搜索面板、日志窗口、模态框）都能复用同一套切分。

---

## 三、抽取清单

### 3.1 已完成

| 项 | 位置 | 问题 | 改法 | 状态 |
|----|------|------|------|------|
| A | 截断函数 | `utils/text.rs` 之外还有 **7 份** `truncate_str`：`agent_task_render.rs:514`、`collapsed_group.rs:357`、`command_suggestions.rs:271`、`confirmation_dialog.rs:886`、`mcp_modal.rs:327`、`market_modal.rs:119`、`plugin_modal.rs:136` | 4 种实现：`collapsed_group` 与 `confirmation_dialog` 是**字节切片**（`&s[..n]`），多字节边界直接 panic；`command_suggestions` / 三个 modal 按字符数（CJK 实际占 2 列，长度算少一半）；`agent_task_render` 按显示宽度 | ✅ 已统一到 `utils::text::truncate_display` |
| B | 折叠切换 | `tool_render.rs:121`（读）、`mouse.rs:135`（写）、`input.rs:2929`（写） | 三处各写一遍 `expanded_tool_call_ids` 的增删；`input.rs` 额外做 `virtual_list.mark_dirty` + `auto_follow = false`，`mouse.rs` 没有 —— 两条路径行为不一致 | ✅ 已收成 `ChatState::toggle_tool_expansion` + `is_tool_expanded` |
| D | 摘要行 | `tool_render.rs` 里 Read / Grep / Glob / LS / CodebaseSearch / ProjectMap / WebFetch 七个臂各写一份折叠摘要 | 四行 `lines.push` + 样式 + 提示文案全重复 | ✅ 已收成 `push_collapsed_summary` + `push_expand_hint` |

**A 的具体改法**：

```rust
// src/ui/utils/text.rs
/// 按**显示宽度**截断，超出部分用 `...` 收尾（不切碎宽字符）。
pub fn truncate_display(text: &str, max_width: usize) -> String {
    crate::ui::utils::render::truncate_to_display_width(text, max_width)
}
```

> 省略号沿用 `render::truncate_to_display_width` 的 `...`（占 3 列），不改成 `…` ——
> 它是全项目用得最广的实现，`...` 与它的 3 列预算配套。四个原本用 `…` 的组件
> （`command_suggestions` / 三个 modal）随之从 `…` 变成 `...`。

**B 的具体改法**：

```rust
// src/ui/state/store.rs
impl ChatState {
    /// 工具块当前是展开还是折叠（默认展开 + 翻转位）。
    pub fn is_tool_expanded(&self, entry: &ChatEntry) -> bool
    /// 折叠 / 展开一个工具块，返回是否真的切换了。
    pub fn toggle_tool_expansion(&mut self, entry_idx: usize) -> bool
}
```

调用点：

- `mouse.rs::try_toggle_on_click` → `state.toggle_tool_expansion(entry_idx)`
- `input.rs`（Tab）→ `state.toggle_tool_expansion(idx)`，随后保留
  `auto_follow = false`（展开会改变高度，视口锚点要重算）
- `tool_render.rs` 读折叠状态 → `state.is_tool_expanded(entry)`

### 3.2 待办

| # | 项 | 位置 | 问题 | 改法 | 风险 |
|---|----|------|------|------|------|
| C | 次要文字样式 | 全 UI 60+ 处 `Color::DarkGray + DIM` 与 `theme.subtle` 两套写法并存 | 同一语义两套颜色来源，主题切换时有一半不跟随 | 加 `theme.dim_style()`，逐模块替换 | **中**：会改变视觉基线，要先定基线 |
| D' | 展开提示文案 | `message_render.rs:273` 与 `agent_task_render.rs:88` 写的是 `(ctrl+o to expand)`，工具块写的是 `(Tab to expand)` | 不是同一个按键 | 保持两套 helper（`push_expand_hint` / `push_expand_hint_ctrl_o`），不强行合并 | 低 |
| E | 大文件拆分 | `events/input.rs` 236KB、`services/stream.rs` 175KB、`components/palette.rs` 113KB | 改一处要读半份文件 | `palette.rs` 按 `PaletteMode` 切文件最省事；`input.rs` 按"按键 / 命令 / Vim"切 | 中高，需要连续几次独立提交 |
| F | 测试隔离 | `services/stream.rs::cache_merge_tests` 两个用例共享状态，第二个单跑也失败 | 与本次改动无关（stash 对比确认），但会污染全量测试的信号 | 给两个用例各自独立的临时目录/配置 | 低 |

---

## 四、本轮已实施

| 改动 | 文件 | 净变化 |
|------|------|--------|
| 新增选区模块 | `src/ui/selection.rs` | +430 |
| 列宽口径收口 | `src/ui/utils/text.rs` | +84 |
| 删掉 store 里的重复实现 | `src/ui/state/store.rs` | −186 |
| 删掉 chat_history 里的重复实现 | `src/ui/components/chat_history.rs` | −181 |
| 折叠摘要行收口（A2） | `src/ui/components/tool_render.rs` | ±195 |
| 两个复制入口合并 | `src/ui/events/mouse.rs` / `src/ui/events/input.rs` | −20 |
| 截断函数统一（A） | 7 个组件 + `utils/text.rs` | −40 |
| 折叠切换收口（B） | `state/store.rs` / `mouse.rs` / `input.rs` / `tool_render.rs` | −30 |

新增测试 11 个：

- 高亮：选区起点落在 span 开头 / 落在 span 中间
- 取文本：CJK 可视列切分、跨条目选区
- 范围解析：字段缺失不 panic、反向拖选
- 截断：不切碎多字节字符、按显示列算、短文本原样返回
- 折叠：切换对渲染路径可见、切换让渲染缓存失效、非工具条目不可切换

---

## 五、验收口径

| 检查 | 命令 |
|------|------|
| 编译 | `cargo check`（新增 warning 数为 0） |
| 格式 | `cargo fmt --check`（改动文件无 diff） |
| 单测 | `cargo test --lib`（UI 相关全绿；`model_catalog` / `hooks` / `checkpoint_manager` 等模块的既有失败不在本轮范围） |
| 手工 | 拖选一段中文 → 应整段高亮并复制；Shift+拖选 → 交给终端原生选择 |

---

## 六、给后续改动的约定

1. **新的"第 N 列"逻辑一律走 `utils::text`**，不要再写 `UnicodeWidthStr::width` 或
   `chars().count()`。
2. **新的复制入口调 `selection::copy_selection`**，不要直接摸 `clipboard.rs`。
3. **新的折叠摘要行走 `tool_render::push_collapsed_summary`**，不要复制粘贴那四行。
4. **改动 `rendered_cache` 相关逻辑时**，记得 `store::get_selected_text` 依赖它兜底重渲染；
   缓存结构变了要同步 `selection::entry_rendered_lines`。
