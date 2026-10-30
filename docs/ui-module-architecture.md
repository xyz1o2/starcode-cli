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
| A | 截断函数 | `utils/text.rs` 之外还有 **7 份** `truncate_str`：`agent_task_render.rs:514`、`collapsed_group.rs:357`、`command_suggestions.rs:271`、`confirmation_dialog.rs:886`、`mcp_modal.rs:327`、`market_modal.rs:119`、`plugin_modal.rs:136` | 4 种实现：`collapsed_group` 与 `confirmation_dialog` 是**字节切片**（`&s[..n]`），多字节边界直接 panic；`command_suggestions` / 三个 modal 按字符数（CJK 实际占 2 列，长度算少一半）；`agent_task_render` 按显示宽度 | ✅ 统一到 `utils::text::truncate_display` |
| B | 折叠切换 | `tool_render.rs`（读）、`mouse.rs`（写）、`input.rs`（写）三处各写一遍 | `input.rs` 额外做 `virtual_list.mark_dirty` + `auto_follow = false`，`mouse.rs` 没有 —— 两条路径行为不一致 | ✅ `ChatState::toggle_tool_expansion` + `is_tool_expanded` |
| C | 次要文字样式 | 全 UI 200+ 处裸写 `Style::default().fg(Color::DarkGray)`，一半不跟随主题 | 新增 `theme::muted_style()` / `muted_dim_style()`（无主题上下文时的**唯一定义**）与 `Theme::subtle_style()` / `dim_style()`（有主题时用）；200+ 处改写；亮色预设的 `subtle` 从 175 灰压到 130 灰（白底上 1.9:1 → 3.6:1） | ✅ |
| D | 展开提示 | 工具块 `(Tab to expand)`、thinking 块 `(ctrl+o 展开思考)`、子代理块 `(ctrl+o to expand)` 三处各写一遍，缩进与括号都不一致 | 样式统一到 `components::common::expand_hint(text)`，**措辞与按键由调用方给**（Tab / Ctrl+O 是两套真实按键，不合并） | ✅ |
| D' | 折叠摘要行 | Read / Grep / Glob / LS / CodebaseSearch / ProjectMap / WebFetch 七个臂各写一份 | `push_collapsed_summary` + `push_expand_hint` | ✅ |
| E | 大文件 | `events/input.rs` 5617 行、`components/palette.rs` 2907 行 | `input.rs` → `input/{mod,provider_form,palette_action,ask_user_question,chat_search,bg_agent,tool_approval}.rs`；`palette.rs` → `palette/{mod,items,describe}.rs` | ✅ |
| F | 测试隔离 | `services/stream.rs::cache_merge_tests` 两个用例抢 `STAR_MODEL_CACHE_PATH`（进程级环境变量），并行时互相覆盖 —— 单跑第二个也失败 | 合并逻辑拆出 `merge_declared_and_discovered_models_with(..., cached)`，测试直接传 `load_from_path` 的结果，**完全不碰环境变量** | ✅ |

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

**C 的两个入口**（刻意分成两个）：

```rust
// 有主题上下文
theme.subtle_style()   // = fg(theme.subtle)
theme.dim_style()     // = fg(theme.subtle) + DIM

// 没有主题上下文（纯工具函数）
crate::ui::themes::theme::muted_style()      // = fg(Color::DarkGray)
crate::ui::themes::theme::muted_dim_style()  // = fg(Color::DarkGray) + DIM
```

> 为什么不把 200+ 处全换成 `theme.subtle_style()`？因为其中大部分函数**根本不接收
> `&Theme`**（`utils/render.rs`、`components/common/*`、`components/highlight/*`、
> 几个 modal）。为了统一颜色而给几十个纯函数加上主题参数，是比"留一个兜底入口"
> 大得多的改动。兜底入口保证**次要文字只有一个定义处**，将来要接主题时逐个替换
> 调用点即可，颜色本身不用再改。

**E 的拆分边界**：

```
src/ui/events/input/
  mod.rs               主分发（handle_key_event）+ 全局快捷键 + 粘贴 + Vim 动作
  provider_form.rs     provider 配置表单（字段切换 / 校验 / 保存 / 回滚）
  palette_action.rs    命令面板动作执行
  ask_user_question.rs 问答工具的交互输入
  chat_search.rs       会话内搜索
  bg_agent.rs          后台代理选择器 + 各类模态框按键
  tool_approval.rs     工具调用审批

src/ui/components/palette/
  mod.rs        调度（按 PaletteMode 挑条目、过滤）+ 渲染
  items.rs      各类面板的条目构造
  describe.rs   条目副标题文本
```

原先对外可见的 `pub(crate)` 函数（`handle_input_modal`、`execute_palette_action` 等）
在 `input/mod.rs` 里显式重新导出，**调用方的路径不变**。

### 3.2 仍待办

| # | 项 | 位置 | 说明 | 风险 |
|---|----|------|------|------|
| G | 把兜底色接上主题 | `muted_style()` 的 ~200 个调用点 | 纯函数没有 `&Theme`，要接上主题得先给它们加参数；属于接口改造，不是重构 | 中高，需要分批做 |
| H | 剩余大文件 | `services/stream.rs` 175KB、`state/store.rs` 95KB、`components/status_line/mod.rs` 89KB、`app/runtime.rs` 81KB | 这几个的内部边界比 input/palette 更难切（状态与副作用交织），建议等 input/palette 的拆分跑一段时间再动 | 中高 |
| I | 死代码 | `components/interrupted.rs`、`vim/operators.rs`、`vim/text_objects.rs` | 确认无调用后删除 | 低 |

---

## 四、本轮已实施

| 改动 | 文件 | 规模 |
|------|------|------|
| 新增选区模块 | `src/ui/selection.rs` | +430 |
| 列宽口径收口 | `src/ui/utils/text.rs` | +84 |
| 删掉 store 里的重复实现 | `src/ui/state/store.rs` | −186 |
| 删掉 chat_history 里的重复实现 | `src/ui/components/chat_history.rs` | −181 |
| 折叠摘要行 + 展开提示收口 | `src/ui/components/tool_render.rs`、`components/common/mod.rs` | ±210 |
| 截断函数统一（A） | 7 个组件 + `utils/text.rs` | −40 |
| 折叠切换收口（B） | `state/store.rs`、`events/mouse.rs`、`events/input/mod.rs`、`components/tool_render.rs` | −30 |
| 次要文字样式收口（C） | 34 个文件 + `themes/theme.rs`、`themes/presets.rs` | ±230 |
| `input.rs` 拆分（E） | `events/input/` 7 个文件（5617 行 → 最大 3315 行） | 纯搬移 |
| `palette.rs` 拆分（E） | `components/palette/` 3 个文件（2907 行 → 最大 1660 行） | 纯搬移 |
| 测试隔离（F） | `services/stream.rs` | ±49 |

新增测试 11 个：

- 高亮：选区起点落在 span 开头 / 落在 span 中间
- 取文本：CJK 可视列切分、跨条目选区
- 范围解析：字段缺失不 panic、反向拖选
- 截断：不切碎多字节字符、按显示列算、短文本原样返回
- 折叠：切换对渲染路径可见、切换让渲染缓存失效、非工具条目不可切换

---

## 五、验收口径

| 检查 | 命令 | 结果 |
|------|------|------|
| 编译 | `cargo check` | 新增 warning 0（仓库既有 2 条） |
| 格式 | `cargo fmt --check` | 改动文件无 diff（仓库既有 diff 在 `agent/`、`core/`、`utils/`） |
| 单测 | `cargo test --lib` | 1739 passed / 5 failed —— 失败的 5 条在 `model_catalog`、`team_apply`、`hooks`、`checkpoint_manager`，与本次改动无关（stash 对比确认） |
| UI 单测 | `cargo test --lib -- ui::` | 343 passed / 0 failed |
| 手工 | 拖选一段中文 → 应整段高亮并复制；Shift+拖选 → 交给终端原生选择；工具块 Tab 折叠 → 与鼠标单击行为一致 |

### 5.1 编辑器/终端的编码提醒

仓库里的 `.rs` 全部是 **UTF-8 无 BOM**。Windows 终端默认按 GBK 显示 UTF-8 字节，
中文会显示成乱码 —— 那是**显示层问题，不是文件坏了**。终端里 `chcp 65001` 可解决；
编辑器请确认编码是 UTF-8。

---

## 六、给后续改动的约定

1. **新的"第 N 列"逻辑一律走 `utils::text`**，不要再写 `UnicodeWidthStr::width` 或
   `chars().count()`。
2. **新的复制入口调 `selection::copy_selection`**，不要直接摸 `clipboard.rs`。
3. **新的折叠摘要行走 `tool_render::push_collapsed_summary`**，不要复制粘贴那四行。
4. **改动 `rendered_cache` 相关逻辑时**，记得 `store::get_selected_text` 依赖它兜底重渲染；
   缓存结构变了要同步 `selection::entry_rendered_lines`。
