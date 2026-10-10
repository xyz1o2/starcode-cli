# 修复设计 — TUI 模式分层 / 复制 / 闪屏

> 日期：2026-10-10
> 涉及：`src/ui/events/input.rs`、`src/ui/events/modal_input.rs`、`src/ui/state/modal.rs`、`src/ui/events/mouse.rs`、`src/ui/app/runtime.rs`、`src/ui/app/mod.rs`、`src/ui/components/status_line/mod.rs`
> 症状：① TUI 输出复制总是不行 ② 会闪屏 ③ 快捷键有时候会冲突
> 前置文档：[`parity-ui-framework.md`](parity-ui-framework.md)（已提出统一 Modal Stack，本文是其延续与落地）、[`ui-benchmark-ccb.md`](ui-benchmark-ccb.md)

本文是动工前的设计稿。每项给出**根因 → 改法 → 涉及文件 → 风险**。

---

## 核心结论

三个症状**不是三个独立 bug，而是同一个架构根因的三种表现**：

| 缺失的抽象 | 现状 | 直接后果 |
|---|---|---|
| **没有单一 mode 枚举** | `ChatState` 上散落 ~18 个独立 bool（`state/store.rs:347`），外加一个 `modal_stack` | 非法状态可达；按键优先级靠 25 层 `if` 的书写顺序隐式表达 |
| **没有按键消费协议** | 一部分层返回 `bool`，一部分裸 `return Ok(())`，`Ctrl+D` 干脆漏了 `return` | 同一个键被 2~3 层同时处理 |
| **渲染没有 dirty 检查** | `runtime.rs:825` 的绘制门条件恒为真 | 每秒 30 次全量重绘 |

外加两个不在预期内、但影响更大的问题：**渲染线程上跑 WSL 子进程**，以及**鼠标捕获使终端原生选区永久失效**。

---

## 状态：修复进度

| 阶段 | 内容 | 状态 |
|---|---|---|
| P0 | 止血：沙箱探测记忆化、去掉 clear()、Ctrl+C 回落 | ✅ 完成 |
| P1 | 修已验证的确定性 bug：隐形弹窗、modal 吞全局键、Ctrl+D 双处理 | ✅ 完成 |
| P2 | 引入 dirty 检查 + 动画门控 | ✅ 完成 |
| P3 | 模式分层收敛 | 🟡 部分完成（见下） |

### P3 已完成

- `has_overlay_active()` 补全（漏掉的 6 层会让 vim 把弹窗的 y/n/Esc 当命令吞掉）
- 后台代理选择器不再跨 modal 残留
- 异步 `ModelsList` 不再把用户正填的输入框顶掉
- 4 个无按键处理的只读弹窗补兜底（此前开着它们按 Enter 会**直接把消息发出去**）
- 快捷菜单互斥收口到 `open_provider_menu` / `open_session_menu`
- 删除 10 个只声明不写入的死 `show_*` 字段

### P3 未完成（建议后续单独 PR）

- `handle_key_event` 的 25 层阶梯 → `KeyContext` 优先级表（改动面最大，风险最高）
- `show_input_modal` 与 `Modal::InputModal` 双真相源合并
- 按键注册表 + help 文案由注册表生成
- vim 的 `operators.rs` / `text_objects.rs` 是死模块，`VimMode::Command` 是空壳

---

# P0 — 止血

## A. 沙箱探测在渲染线程上每帧 spawn WSL 子进程

### 根因

```rust
// src/ui/components/status_line/mod.rs:1242
if state.sandbox_enabled {
    let avail = crate::core::sandbox::SandboxManager::is_available();
```

`state.sandbox_enabled` 由**同一次调用**在 `state/store.rs:1114` 初始化。所以当沙箱**可用**时，这个分支每帧都进。而 Windows 上的实现是：

```rust
// src/core/sandbox/bubblewrap_windows.rs:201
fn is_available() -> bool {
    if which::which("wsl").is_err() { return false; }
    std::process::Command::new("wsl").args(["--", "which", "bwrap"]).output()...
}
```

**结果：每秒 30 次 WSL 子进程 spawn，卡在渲染线程上。** WSL 冷启动 300ms~2s，直接表现为卡顿与闪屏。沙箱*不可用*时反而没事（一次 `which` 失败就返回）。

### 改法

在 `ChatState` 上加 `sandbox_available: OnceCell<bool>`，探测结果进程内只算一次。`is_available()` 的语义是"本机有没有 bwrap"，本来就不随时间变化，缓存无副作用。

> 不选 TTL 缓存：探测结果在一次会话内不会变，TTL 只是为了应对用户中途装上 bwrap，收益极低。

### 风险

低。唯一行为差异是中途安装 bwrap 后需要重启进程才生效。

---

## B. `terminal.clear()` 全屏闪烁

### 根因

```rust
// src/ui/app/runtime.rs:678
let task_panel_changed = state.task_panel.is_visible != last_task_panel_visible;
if task_panel_changed { terminal.clear()... }
```

`is_visible` 是在**渲染过程中**被改的：`app/mod.rs:31` 的 `auto_show_if_needed()` 置 `true`，`app/mod.rs:34` 的 `check_auto_hide()` 置 `false`（含 5 秒定时器，`task_panel.rs:344-377`）。所以每加一条 TodoWrite 就触发一次全屏清屏，完成 5 秒后再来一次。

ratatui 的 `Terminal::clear()` 发 `ClearType::All` **并重置后缓冲**，导致下一次 draw 重绘 100% 的 cell —— 这就是肉眼看到的黑屏一闪。

**额外隐患**：`Terminal::clear()` 内部先调 `backend.get_cursor_position()`，在 CrosstermBackend 上会发 DSR `ESC[6n` **阻塞等回复**，而专门的键盘读取线程（`runtime.rs:496-528`）此刻正在同一个 stdin 上 `crossterm::event::read()`。回复可能被那个线程吃掉，于是 DSR 要等 crossterm 的超时（最长约 2 秒）。

### 改法

直接删掉这个 clear。`runtime.rs:676-677` 的注释本身已经写明"CJK 残影由 `render_page` 里的 `cell.reset()` 处理"——即清屏的原始目的已由渲染层覆盖。

### 风险

低。仅在极老终端上 CJK 残影可能回来，届时可用 `ClearType::Purge` 之外的手段（如整行重写）替代。

---

## C. `Ctrl+C` 复制失败却回落成"取消生成"

### 根因

```rust
// src/ui/events/input.rs:1715
if state.text_selection.has_selection() {
    if let Some(selected_text) = state.get_selected_text() {   // ← 可能返回 None
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            let _ = clipboard.set_text(&selected_text);        // ← error 被丢弃
        }
        state.text_selection.clear();
        state.current_status_line = Some("Copied".to_string()); // ← 无条件宣告成功
        return Ok(());
    }
}
// ↓↓↓ 选区提取为空时落到这里
if state.is_streaming || state.is_processing {
    let _ = agent_tx.send(AgentRequest::Abort).await;           // ← 把要复制的内容销毁了
```

两个独立缺陷：

1. **假成功**：`Clipboard::new()` 或 `set_text` 失败（WSL/SSH/headless、剪贴板被占用）时，错误被丢弃，却仍然显示 "Copied" 并清掉选区 —— 用户粘出来的是上次的旧内容。
2. **静默回落**：`get_selected_text()` 在没提取到非空行时返回 `None`（`state/store.rs:1465`），控制流继续往下走到取消/双击退出逻辑。

### 改法

- 剪贴板写入失败 → 保留选区、状态栏显示真实错误文案，不再宣告 "Copied"。
- **有选区时 `Ctrl+C` 一律不回落**到取消逻辑 —— 复制意图优先于中断意图。

### 风险

低。改变了"选区为空时 Ctrl+C 取消生成"的行为，但那种情况本来就会因为 `has_selection()` 为假而不进入本分支。

---

# P1 — 修已验证的确定性 bug

## D. Settings 弹窗是"隐形吞键窗口"

### 根因

```rust
// src/ui/state/modal.rs:524
pub fn open_status_modal(&mut self) {
    self.close_all_modals();
    self.modal_stack.push(Modal::StatusModal);   // ← 从不设 show_status_modal = true
}
```

`show_status_modal` 全仓库**只被赋 `false`**（`commands/mod.rs:88`、`commands/extended.rs:1538`、`modal_input.rs:1175`），从未赋 `true`。而渲染侧：

```rust
// src/ui/components/status_modal.rs:327
pub fn render_status_modal(...) {
    if !state.show_status_modal { return; }        // ← 什么都不画
```

按键侧却是无条件消费（`events/input.rs:3101-3131` 吞掉 `Esc/↑/↓/Enter`）。

从 `Ctrl+P → Status`（`components/palette.rs:372` → `events/input.rs:786`）可稳定复现：**看不见任何窗口，但按键全被吃掉。**

### 改法

`open_status_modal` 补 `self.show_status_modal = true;`，`pop_modal` 路径上确保复位。

> 长期应删除 `show_status_modal` 这个字段、统一由 `modal_stack` 表达（见 P3）。

### 风险

低。修复后弹窗可见，行为与 `open_*` 的其余分支一致。

---

## E. Modal 层吞掉全局快捷键

### 根因

`handle_key_event` 是 **25 层顺序 `if` 阶梯**，第 1 层是 `handle_modal_key`。而 `handle_palette` 的 `match key.code` **不检查修饰键**，且结尾无条件 `Ok(true)`：

```rust
// src/ui/events/modal_input.rs:142,168,173
Char(c) => { ... state.palette_filter.push(c); }
_ => {}
Ok(true)          // ← 任何键都算"已消费"
```

后果：

- `Ctrl+P` 变成往过滤框里打了一个 `'p'` —— `events/input.rs:1566-1579` 里那段"关闭 palette"的代码**不可达**。
- `Ctrl+C` 在 Palette/Mcp/Market/Plugins/GlobalSearch/QuickOpen/HistorySearch 打开时被第 1 层吃掉 → **流式输出无法中断**。
- `F1` 被 `_ => {}`（`modal_input.rs:171`）静默吞掉。

同样的 `Ok(true)` 无条件返回还出现在 `modal_input.rs:376, 542, 888, 967, 1075, 1145`。

### 改法

在 `handle_modal_key` 入口加**全局键白名单前置**：识别到 `Ctrl+C` / `F1` / `Ctrl+L` 时返回 `Ok(false)`，交还给全局层处理。这样 modal 内的 `Esc` 语义不受影响，全局键却始终可用。

### 风险

中低。改变了 modal 内的按键行为（这是**期望的**修正），但需实测 modal 内打字时 `Ctrl+C` 不再被吞。

---

## F. `Ctrl+D` 双重处理

### 根因

```rust
// src/ui/events/input.rs:1774
if key.modifiers.contains(CONTROL) && matches!(key.code, KeyCode::Char('d')) {
    if !state.task_panel.is_visible {
        if state.input.is_empty() {
            state.should_exit = true;
            return Ok(());      // ← 只有这一条路径 return
        }
    }
}
```

`task_panel.is_visible` 或 `input` 非空时**不 return**，控制流继续落到 `input.rs:2982`（切换权限调试面板）和 `input.rs:3146`（进入任务描述编辑）。这是全流程唯一一处真正的双重处理。

`Ctrl+D` 因此有 **3 个含义**：退出 / 权限面板 / 任务编辑。

### 改法

`Ctrl+D` 在本层一旦命中就 `return Ok(())`（非退出场景视为"未处理但已占用"），把任务面板的 `Ctrl+D` 语义换成不冲突的键，或让任务面板分支先于本层执行。

### 风险

低。

---

## G. 任务面板缺少 modal 守卫

### 根因

```rust
// src/ui/events/input.rs:3133
if state.task_panel.is_visible { ... }     // handle_overlay_input 的第一个分支，无任何 modal 守卫
```

紧邻的 bg-agent 选择器却有守卫（`events/input.rs:1796` 显式加了 `&& !state.show_input_modal`）。任务面板这个分支没有，所以打开 API Key 表单（`InputModal`，在第 19 层，更晚）时，`Ctrl+J/Ctrl+S/Ctrl+Space/Ctrl+N/Ctrl+Delete/Alt+←→/Ctrl+Enter/Alt+V` 全被任务面板截走。

### 改法

把任务面板分支加上与其他分支一致的 modal 守卫。

### 风险

低。

---

# P2 — dirty 检查与动画门控

## H. 绘制门条件恒为真

### 根因

```rust
// src/ui/app/runtime.rs:825
if needs_redraw || last_draw.elapsed() > framerate { ... draw ... }
// ...
// src/ui/app/runtime.rs:884
let elapsed_since_draw = last_draw.elapsed();
if elapsed_since_draw < framerate {
    tokio::time::sleep(framerate - elapsed_since_draw).await;   // ← 睡满剩余预算
}
```

循环尾部睡满帧预算，所以下一轮 `last_draw.elapsed() > framerate` **永远成立**。`needs_redraw` 形同虚设，空屏也在 30fps 重绘。

项目文档已记着这笔债：`parity-ui-framework.md:241`「30fps 全量重绘」。

### 改法

改成真正的 dirty 判断：维护 `FrameInputs` 哈希（流式消息计数、`animation_tick` 语义量、滚动位置、布局相关状态、选区、状态栏时间秒数），哈希未变且不在动画状态时跳过 `terminal.draw`。

## I. 每帧必变的动画源

### 根因

```rust
// src/ui/app/mod.rs:278
pub fn draw_ui(...) {
    state.animation_tick = state.animation_tick.wrapping_add(1);   // ← 在 draw 内部自增
```

于是帧与帧之间 buffer 必然不同，diff 永远非空。受影响的动画：spinner 字形（`status_line/mod.rs:585`）、shimmer 光带（`:643`）、颜色脉动（`:622`）、thinking 透明度正弦（`:675`）、token 数脉动（`:717`）、工具调用 ● 闪烁（`tool_render.rs:253`，600ms）、任务面板 spinner（`task_panel.rs:903`，200ms）。

### 改法

把 `animation_tick` 的自增移出渲染路径，改为由**事件循环按需推进**，且仅在 `is_animating(state)` 成立时推进；空闲时停止推进 → 帧内容稳定 → dirty 检查生效。

### H + I 风险

中。二者耦合，必须一起改：单独上 H 不生效（内容每帧都在变），单独上 I 会让动画变卡。改完必须实测 spinner、shimmer、流式滚动三种场景。

---

# P3 — 模式分层收敛

> 本阶段改动面大，建议在 P0~P2 验证通过、且有回归测试兜底之后再做。

## 现状盘点

**25 层 `if` 阶梯**（`events/input.rs:1489` `handle_key_event`）：每层用 `bool` 或裸 `return Ok(())` 表达"是否消费"，没有统一的优先级表。

**~18 个独立 mode 字段**（`state/store.rs:347`）：其中只有 `modal_stack` 是枚举。其中 10 个 `show_*` 字段（`store.rs:668-691`）**初始化后从未被写入**，是一层死状态，任何将来的 `if state.show_quick_open` 守卫都会恒为假。

**输入框有两个真相源**：`show_input_modal`（`store.rs:586`，渲染于 `components/input_modal.rs:10`、分发于 `events/input.rs:3228`、并在 `modal_input.rs:28` 用作模态层否决）与 `Modal::InputModal`（渲染于 `app/mod.rs:321`）。靠 `enter_input_modal`/`exit_input_modal`（`state/modal.rs:503-521`）手工同步，**异步 stream 消息会打破同步**（`services/stream.rs:1012-1025` 收到 `ModelsList` 时无条件 `open_palette(Model)`，压掉栈顶的 `InputModal` 但不清 flag）→ 面板画不出来、按键仍被隐形输入框吞掉。

**`has_overlay_active()` 不完整**（`state/modal.rs:530`）：漏了 `show_provider_menu`、`show_session_menu`、`bg_agent_selection`、`pending_model_confirmation`、`task_panel.is_visible`。它只被用在两处（`events/input.rs:1824` vim 门控、`events/input.rs:2343` 双 Esc），恰好是最需要完整性的地方。

**没有按键注册表**：`events/keymap.rs` 只映射 4 个键给建议浮层，真实绑定全是散落的 `if ... KeyModifiers::CONTROL`，已漂移出多处不一致（例如 help 弹窗称 Tab 是"补全"，`events/input.rs:2405` 实际是"折叠工具"）。

## 改法

1. **收敛 mode 到一个枚举** `UiMode`，消除 `show_*` 平行 flag；`modal_stack` 成为唯一的模态真相源，输入框不再有第二个来源。
2. **引入 `KeyContext` 优先级表**，把 25 层阶梯换成查表分发；每层统一返回 `Handled | Ignored`，杜绝"忘了 return"。
3. **`has_overlay_active()` 改为由枚举派生**，不再手工维护清单。 ✅ 部分完成（清单已补全，尚未改为派生）
4. **删除 10 个死 `show_*` 字段**。 ✅ 已完成
5. **建立按键注册表**，help 弹窗从注册表生成，消除文案漂移。（已手工修正 `Ctrl+C` / `Ctrl+D` / `Ctrl+Shift+D` 三条漂移，自动化留待后续）

## 风险

高。建议拆成多个 PR，每步单独可回滚。

> 注意：不要把 `task_panel.is_visible` 算进 `has_overlay_active()`。它是常驻面板而非
> 遮挡层，只吃 Ctrl/Alt 组合键与 ↑/↓/Enter，普通字符仍要落到输入框 —— 计进来的话，
> 开着任务面板时 vim 会被整个关掉，双 Esc 退出也会失灵。

---

# 未纳入本次修复

| 项 | 原因 |
|---|---|
| **鼠标捕获导致终端原生选区永久失效** | `EnableMouseCapture` 发 `?1003h`（any-event tracking），终端整个会话不再自己做选区。而代码注释声称 `Shift+Click` 可绕过（`events/mouse.rs:210-212`、`app/runtime.rs:1059`）——**这是错的**，没有终端会因修饰键豁免鼠标上报，导致 Shift 拖选时应用不处理、终端也不处理，两条路都断。输出又只存在于 alternate screen（`app/runtime.rs:1055`），滚轮回溯也不可能。彻底解法需要引入自己的输出回滚缓冲（inline viewport），属于独立特性，不适合混进本次修复。 |
| **OSC 52 兜底** | crossterm 0.29 自带 `crossterm::clipboard::CopyToClipboard`（OSC 52），仓库从未引用。SSH/WSL 下 `arboard` 写的是远端剪贴板，必然失败——这是那条环境下的正解。改动小，建议单独提交。 |
| **`/commit` 在 UI 线程上 `println!`** | `commands/git.rs:276-406` 直接写 stdout，会把字节注入活动屏幕造成错乱。属独立缺陷。 |
| **渲染循环内的阻塞 await** | `runtime.rs:765-810` 的 `/loop` + `/remote` 轮询各有 500ms 超时，每秒各阻塞一次；`logic.rs:400-414` 的 `!cmd` shell 执行会阻塞整个事件循环。应移出渲染任务。 |
| **crossterm 0.28 → 0.29** | 见 [`dependency-audit.md`](dependency-audit.md) §4.3。独立议题。 |

---

# 验证清单

改动后需手动验证（TUI 无法用单测覆盖）：

- [ ] Windows + 沙箱开启时，空闲 10s CPU 占用不再出现周期性尖峰
- [ ] TodoWrite 增删时不再全屏黑闪
- [ ] `Ctrl+P → Status` 能看到 Settings 弹窗
- [ ] Palette 打开时 `Ctrl+C` 能中断流式输出
- [ ] Palette 打开时 `Ctrl+P` 能关闭面板（而不是打字 'p'）
- [ ] API Key 表单打开时 `Ctrl+S` / `Alt+←→` 不被任务面板截走
- [ ] 有选区时 `Ctrl+C` 必定复制，绝不取消生成
- [ ] 剪贴板不可用时显示真实错误而非 "Copied"
- [ ] 空闲时终端写入降到约 1Hz（状态栏秒数）
- [ ] spinner / shimmer / 流式滚动动画仍流畅
- [ ] `Ctrl+D` 只退出；权限调试面板改用 `Ctrl+Shift+D`
- [ ] 上下文可视化窗口开着时按 Enter 不会把消息发出去

---

# 回归情况

`cargo test --lib`：**738 passed / 15 failed**。

失败项全部为改动前既有失败，集中在 `utils::checkpoint_manager`、
`core::auto_mode::dangerous_patterns`、`core::tools::*`、`core::utils::paths`、
`ui::utils::format` 等与本次改动无关的模块（Windows 路径分隔符与环境相关；
`checkpoint_manager` 一组在多次运行间数量还会漂移，属并行/顺序敏感）。
已用 `git stash` 对照基线逐项确认。

`ui::events::mouse` 的 8 个测试曾因中途一版改动（松开鼠标复制后清选区）失败，
已改为保留选区，现全部通过。