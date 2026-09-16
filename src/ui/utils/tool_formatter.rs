//! Tool output formatting — concise, user-friendly display.
//!
//! 注意：本模块曾经提供 `format_tool_call_summary` / `format_tool_result_display` /
//! `format_status_message` / `format_tool_elapsed` 四个工具，但全部没有调用方
//! （真正的渲染走 `ui/utils/format.rs` 与 `ui/components/tool_render.rs`），
//! 已删除。调整工具结果显示时，请改 `format.rs` 与 `tool_render.rs`。

/// Format file path for display (relative to cwd, use ~ for home)
pub fn format_display_path(path: &str) -> String {
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let home = dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    // Try relative to cwd first
    if path.starts_with(&cwd) {
        let relative = &path[cwd.len()..];
        return relative.strip_prefix('/').unwrap_or(relative).to_string();
    }

    // Try relative to home
    if path.starts_with(&home) {
        return format!("~{}", &path[home.len()..]);
    }

    path.to_string()
}
