# UI 框架架构对标分析 — 重构计划

> 参考: `study_or_copy_projects/claude-code-main/` — Ink/React 组件模型
> 当前: ratatui 0.30 + crossterm 0.28 + tokio 1.0

## 核心结论

**依赖版本无需升级** — ratatui 0.30 / crossterm 0.28 / tokio 1.0 均为当前版本。
**问题在架构层面**, 不在框架能力。

---

## P0 — 立即需要

### 1. 统一 Modal Stack

**当前问题**:
```
ChatState {
    show_global_search: bool,
    show_quick_open: bool,
    show_history_search: bool,
    show_theme_picker: bool,
    show_statistics: bool,
    show_export: bool,
    show_compact: bool,
    show_context_visualizer: bool,
    show_error_overlay: bool,
    // ... 10+ 个 show_* 布尔值
    modal_stack: Vec<ModalEntry>,  // 仅 4 个在 stack 中
}
```

- 多个 modal 可以同时 `show_*: true`, 但只有 4 个有正确的层级管理
- Esc 关闭逻辑分散在各处, 不一致
- 新增 modal 需要: 加字段 + 加 match arm + 加 Esc 处理 — 容易遗漏

**目标**:
```rust
// 所有 modal 统一注册到 stack
enum ModalKind {
    GlobalSearch(GlobalSearchState),
    QuickOpen(QuickOpenState),
    HistorySearch(HistorySearchState),
    ThemePicker(ThemePickerState),
    Statistics,
    Export,
    Compact,
    ContextVisualizer,
    ErrorOverlay(ErrorMessage),
    Confirmation(ConfirmationDialog),
    // ...
}

struct ModalStack {
    stack: Vec<(ModalKind, ModalMetadata)>,
}

impl ModalStack {
    fn push(&mut self, modal: ModalKind);
    fn pop(&mut self) -> Option<ModalKind>;
    fn top(&self) -> Option<&ModalKind>;
    fn is_empty(&self) -> bool;
}
```

**迁移步骤**:
1. 定义 `ModalKind` enum
2. 将 `show_*` 布尔值逐个迁移为 `ModalKind` variant
3. 统一 Esc 处理: `modal_stack.pop()`
4. 渲染: 按 stack 顺序从底到顶渲染

**涉及文件**: `src/ui/state/store.rs`, `src/ui/state/modal.rs`, `src/ui/app/mod.rs`, `src/ui/events/input.rs`

---

### 2. Component Trait

**当前问题**:
- 每个组件是独立的 `render_xxx()` 函数, 签名不统一
- 有的需要 `&ChatState`, 有的需要 `&mut ChatState`
- 事件处理: `handle_xxx_input()` 分散在 `input.rs` / `modal_input.rs`
- 无统一的生命周期: 创建 / 更新 / 销毁

**目标**:
```rust
trait Component {
    /// 组件的唯一标识
    fn id(&self) -> &'static str;

    /// 是否需要重新渲染 (dirty flag)
    fn is_dirty(&self) -> bool;

    /// 处理事件, 返回是否消费了事件
    fn handle_event(&mut self, event: &AppEvent, ctx: &mut ComponentContext) -> bool;

    /// 更新状态 (每帧调用)
    fn update(&mut self, ctx: &mut ComponentContext);

    /// 渲染
    fn render(&self, frame: &mut Frame, area: Rect, ctx: &ComponentContext);
}

struct ComponentContext<'a> {
    state: &'a mut ChatState,
    agent_tx: &'a Sender<AgentRequest>,
    // ...
}
```

**迁移策略**:
- 从最简单的组件开始 (StatusLine, TaskPanel)
- 逐步迁移, 不需要一次性全部改完
- 旧的 `render_xxx()` 函数可以包装为 `Component` 实现

**涉及文件**: 新建 `src/ui/component.rs` (trait 定义), 各组件文件

---

## P1 — 尽快需要

### 3. ChatState 分解

**当前**: 330+ 字段的 mega-struct

**目标拆分**:
```rust
struct ChatState {
    pub input: InputState,        // 输入相关
    pub modal: ModalState,        // modal stack
    pub streaming: StreamingState, // 流式输出
    pub task: TaskPanelState,     // 任务面板
    pub agent: AgentState,        // agent 状态
    pub chat: ChatHistoryState,   // 聊天历史
    pub ui: UIState,              // UI 配置
}
```

**迁移策略**:
- 逐步拆分, 每次移动一组相关字段
- 保持向后兼容: `ChatState` 保留 `pub` 字段, 内部代理到子状态
- 最终: 字段通过 `state.input.xxx` 访问

**涉及文件**: `src/ui/state/store.rs` (主要), 所有访问 ChatState 的文件

---

### 4. Action/Event 枚举

**当前**: `input.rs` 3,213 行 raw KeyEvent match

**目标**:
```rust
enum AppEvent {
    // 输入
    KeyInput(KeyEvent),
    MouseInput(MouseEvent),
    Paste(String),

    // Agent
    AgentMessage(StreamMessage),
    AgentError(String),

    // UI
    ModalOpen(ModalKind),
    ModalClose,
    FocusChange(FocusTarget),

    // 系统
    Resize(u16, u16),
    Tick,
}

enum Action {
    // 消息
    SendMessage(String),
    AbortAgent,

    // 导航
    ScrollUp(usize),
    ScrollDown(usize),
    PageUp,
    PageDown,

    // Modal
    OpenSearch,
    OpenQuickOpen,
    CloseModal,

    // 编辑
    InsertChar(char),
    DeleteChar,
    NewLine,

    // ...
}
```

**迁移策略**:
1. 定义 `AppEvent` 和 `Action` enum
2. `input.rs` 中的 match arm 逐步改为产生 `Action`
3. `Action` dispatch 集中处理

**涉及文件**: `src/ui/events/input.rs` (主要)

---

### 5. 渲染与状态分离

**当前**: `render_page()` 中有状态变异
```rust
// 当前 (有问题)
fn render_page(state: &mut ChatState, frame: &mut Frame) {
    // 渲染过程中修改了 state.scroll, state.total_rendered_lines 等
    state.total_rendered_lines = calculated_lines;
    state.scroll = adjusted_scroll;
}
```

**目标**: 渲染为纯函数
```rust
fn render_page(state: &ChatState, frame: &mut Frame) -> RenderResult {
    // 只读访问 state
    // 返回需要更新的状态变更
    RenderResult {
        total_lines: calculated_lines,
        scroll_delta: 0,
    }
}
// 调用方在渲染后应用 RenderResult
```

**涉及文件**: `src/ui/app/mod.rs`, `render_page()` 及其调用链

---

## P2 — 中期改进

### 6. 渲染脏标记

**当前**: 30fps 全量重绘

**目标**: per-component dirty flag
```rust
impl Component for MyComponent {
    fn is_dirty(&self) -> bool {
        self.dirty || self.state_changed
    }

    fn render(&self, ...) {
        if !self.is_dirty() { return; }
        // 实际渲染
    }
}
```

**收益**: 空闲时 CPU 占用显著降低

**涉及文件**: Component trait 实现, 渲染循环

---

### 7. 业务逻辑抽取

**当前**: `tool_render.rs` 混合了:
- JSON 解析 (ToolResult.data)
- 路径格式化 (home 缩写, 相对路径)
- ANSI 处理
- Span 构建 (颜色, 样式)
- 布局计算 (截断, 换行)

**目标分层**:
```
tool_render/
├── mod.rs          // render 入口
├── parse.rs        // JSON / data 解析
├── format.rs       // 路径、数字、时间格式化
├── ansi.rs         // ANSI 处理 (已有 highlight/ansi.rs)
└── layout.rs       // 截断、换行、宽度计算
```

**涉及文件**: `src/ui/components/tool_render.rs` (拆分)

---

## 实施顺序建议

```
Phase 1 (P0):
  └─ Modal Stack 统一 → Component trait

Phase 2 (P1):
  ├─ ChatState 分解 (可与 Phase 1 并行)
  ├─ Action/Event 枚举 (依赖 Component trait)
  └─ 渲染/状态分离

Phase 3 (P2):
  ├─ 脏标记渲染 (依赖 Component trait)
  └─ 业务逻辑抽取 (可独立进行)
```

---

## 与 CCB 的架构对比

| 维度 | CCB (Ink/React) | StarCode (ratatui) | 差距 |
|------|-----------------|---------------------|------|
| 组件模型 | React 组件 + hooks | 独立函数 | 需要 Component trait |
| 状态管理 | useState + useReducer + Context | ChatState mega-struct | 需要拆分 |
| 事件处理 | React 事件系统 | raw KeyEvent match | 需要 Action enum |
| 渲染 | React 虚拟 DOM (自动 diff) | 手动 ratatui 绘制 | 需要脏标记 |
| Modal | useRegisterOverlay hook | show_* 布尔值 | 需要 Modal Stack |
| 生命周期 | useEffect / useMemo | 无 | 需要 Component trait |
