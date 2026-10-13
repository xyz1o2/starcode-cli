// 只有 motions 有实现并被 `events/input.rs` 使用。`operators.rs` 与
// `text_objects.rs` 曾经在这里，但全仓零调用方（`d{motion}` / `ciw` 之类都没接），
// 已删除 —— 留着会让人以为这些操作符可用。
pub mod motions;
pub mod state;

pub use state::{VimMode, VimState};
