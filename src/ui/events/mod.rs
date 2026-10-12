pub mod clipboard_paste;
/// UI Event Handling
///
/// # 模块
/// - `input.rs`: 按键分发的**主体**。各处理函数的实现都在这里。
/// - `keymap.rs`: 全局按键**注册表** —— 绑定的唯一定义处，帮助文案由它生成。
/// - `key_context.rs`: 「当前处于哪个模式」的唯一真相源（[`key_context::KeyContext::of`]）。
/// - `modal_input.rs`: 各模态对话框的按键处理。
/// - `mouse.rs`: 鼠标事件 —— 滚轮、点击、拖选
/// - `clipboard_paste.rs`: 剪贴板粘贴 —— 图片、文件路径、文本粘贴块哨兵占位
///
/// # 事件流
/// ```text
/// crossterm::Event → [Key Reader Thread] → mpsc → [UI Loop] → handle_key_event()
/// ```
///
/// # 模式分层约定
///
/// **判断模式只写一处**：[`key_context::KeyContext::of`]。各处理层用
/// `KeyContext` 查询，不要自己再拼一份 `if state.show_xxx || ...` —— 此前正是
/// 十几份互不一致的清单造成了「弹窗开着按键却穿透」和「按键被看不见的层吞掉」。
///
/// 派生的两个谓词覆盖了原先散落的全部用法：
///
/// | 谓词 | 含义 | 取代了 |
/// |---|---|---|
/// | [`key_context::KeyContext::is_exclusive`] | 有没有遮挡层占住键盘 | `ChatState::has_overlay_active()`、双 Esc 的内联清单 |
/// | [`key_context::KeyContext::is_dialog`] | 有没有独占屏幕的对话框 | `ChatState::has_dialog_active()`（死代码）、`stream.rs` 的 `user_is_elsewhere` |
///
/// # 按键约定
///
/// 全部全局绑定见 [`keymap::GLOBAL_BINDINGS`]，**只在那里定义一次**。
/// 帮助弹窗 (`components/help_popup.rs`) 从该表生成，不要再手抄文案。
///
/// 分发入口是 [`input::dispatch_global_binding`]：拿到键先用
/// [`keymap::binding_for`] 查表，再按 `GlobalBinding` 分派。**不要**在
/// `handle_key_event` 里手写 `if modifiers.contains(CONTROL) && matches!(...)` ——
/// 那样注册表与实际行为就是两份清单，且没有任何测试能发现漂移。
///
/// 在模态对话框打开时仍然生效的键，由 [`keymap::is_global_key`] 从同一张表推导 ——
/// 因为各模态的 `match key.code` 不看修饰键，`Ctrl+C` 会被当成字符打进过滤框。
pub mod input;
pub mod key_context;
pub mod keymap;
pub mod modal_input;
pub mod mouse;
