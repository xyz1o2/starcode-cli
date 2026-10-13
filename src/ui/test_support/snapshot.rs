//! 快照式渲染断言 —— 把 ratatui 的 `Buffer` 序列化成可逐行 diff 的文本。
//!
//! ## 为什么不用 ratatui 官方的 `assert_snapshot!(terminal.backend())`
//!
//! 官方那个配方（<https://ratatui.rs/concepts/snapshots/>）只序列化字符，**颜色
//! 和样式完全不进基线**（上游 issue ratatui#1402）。这意味着一次"把选中项改成
//! 暗红"的重构会静默通过 —— 测试全绿，界面上却已经坏了。本模块把每个 cell 的
//! `fg` / `bg` / `modifier` 一并编码进基线，**改颜色即改基线**。
//!
//! ## 输出格式
//!
//! ```text
//! ─── 44x9 ───
//!   0│──────────────────────────── 3:6            │
//!   1│first line                                  │
//!   3│third▌                                      │
//!   8│────────────────────────────────────────────│
//!
//! ─── styles ───
//!   0│0-38 fg:red · 39-43 fg:darkgray│
//!   3│0-4 plain · 5 reversed · 6-43 plain│
//!   8│0-43 fg:red│
//! ```
//!
//! 每行**定宽补齐**、两侧都有 `│`，所以行宽恒定、diff 逐行对齐，也没有一行以
//! 空格结尾（否则 git 的"行尾空白"规整会制造假 diff）。尾部空格保留是有意的 ——
//! 在终端里空格也是 cell，选中高亮和背景色就靠它们撑。
//!
//! `styles` 段只列出**不是纯默认样式**的行，按 cell 区间给出样式段。位置信息保留，
//! 所以"红的那几格挪了一列"同样能被 diff 抓到。
//!
//! ## 更新基线
//!
//! 基线变更确属预期时：`UPDATE_SNAPSHOTS=1 cargo test --lib`（或
//! `INSTA_UPDATE=always`）。基线文件在 `tests/snapshots/ui/<name>.snap`。

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use ratatui::text::{Line, Text};
use ratatui::widgets::Paragraph;
use ratatui::{Frame, Terminal};
use similar::TextDiff;
use unicode_width::UnicodeWidthStr;

/// 基线文件目录（相对 crate 根）。
const SNAPSHOT_DIR: &str = "tests/snapshots/ui";

/// 把 `&Buffer` 序列化成快照文本。
pub fn serialize_buffer(buf: &Buffer) -> String {
    let area = buf.area;
    let mut out = String::new();
    out.push_str(&format!("─── {}x{} ───\n", area.width, area.height));

    for y in 0..area.height {
        let row = row_symbols(buf, area, y);
        out.push_str(&format!("{:>3}│{}│\n", y, row));
    }

    let styled: Vec<String> = (0..area.height)
        .filter_map(|y| style_line(buf, area, y))
        .collect();
    if !styled.is_empty() {
        out.push_str("\n─── styles ───\n");
        for line in styled {
            out.push_str(&line);
            out.push('\n');
        }
    }

    out
}

/// 一行的显示结果：文本 + 样式段。
struct RowWalk {
    text: String,
    /// `(起始列, 结束列含, 样式)`，`结束列`是**终端列**而不是字符序号
    runs: Vec<(u16, u16, String)>,
}

/// 走一遍一行，同时产出文本和样式段。
///
/// ## 为什么要按显示宽度走、跳过占位 cell
///
/// ratatui 把双宽字符（`安`）写进一格，再把**紧跟的那一格留成 `Cell::EMPTY`**，
/// 而 `Cell::symbol()` 对 `None` 会返回 `" "`。逐格读符号就会把 CJK 渲染成
/// 「安 装」，而且那格还是 `Reset` 样式，于是样式段里每个汉字后面都跟一段
/// `plain` —— 基线既错又吵。
///
/// 这里按 `unicode-width` 的显示宽度跳过占位 cell，并让它落进前一个字符的样式段
/// （终端是把一个字形跨两格用同一组属性画出来的，那格的样式本来就该跟前一格）。
fn walk_row(buf: &Buffer, area: Rect, y: u16) -> RowWalk {
    let mut text = String::with_capacity(area.width as usize);
    let mut runs: Vec<(u16, u16, String)> = Vec::new();
    let mut used = 0u16;
    let mut x = 0u16;

    while x < area.width {
        let cell = buf.cell((x, y));
        let symbol = cell.map(|c| c.symbol()).unwrap_or(" ");
        let style = cell
            .map(|c| format_cell_style(c.fg, c.bg, c.modifier))
            .unwrap_or_else(|| "plain".to_string());

        // width 0 = 零宽组合字符，仍占一格显示位置；EMPTY 占位格已被上一轮跳过
        let width = (UnicodeWidthStr::width(symbol) as u16).max(1);
        let span_end = x.saturating_add(width.saturating_sub(1));

        text.push_str(symbol);
        match runs.last_mut() {
            Some(last) if last.2 == style => last.1 = span_end.max(last.1),
            _ => runs.push((x, span_end, style)),
        }

        used += width;
        x += width;
    }

    RowWalk { text, runs }
}

/// 一行的字符，补齐到整宽。
///
/// **必须补齐而不是裁掉尾部空白**：一是行宽恒定，diff 逐行对齐；二是尾部空格
/// 在终端里也是 cell（选中高亮、背景色就靠它们撑），裁掉等于把样式信息丢了。
/// 收尾的 `│` 保证没有一行以空格结束，git 不会把它当"行尾空白"规整掉。
fn row_symbols(buf: &Buffer, area: Rect, y: u16) -> String {
    let walked = walk_row(buf, area, y);
    let mut s = walked.text;
    let used = UnicodeWidthStr::width(s.as_str()) as u16;
    for _ in used..area.width {
        s.push(' ');
    }
    s
}

/// 一行的样式段；整行都是默认样式时返回 `None`（不进基线）。
fn style_line(buf: &Buffer, area: Rect, y: u16) -> Option<String> {
    let walked = walk_row(buf, area, y);
    if walked.runs.len() == 1 && walked.runs[0].2 == "plain" {
        return None;
    }

    let body = walked
        .runs
        .iter()
        .map(|(from, to, style)| {
            if from == to {
                format!("{from} {style}")
            } else {
                format!("{from}-{to} {style}")
            }
        })
        .collect::<Vec<_>>()
        .join(" · ");
    Some(format!("{:>3}│{body}│", y))
}

/// 一个 cell 的样式，压成最紧凑的可读形式。
fn format_cell_style(fg: Color, bg: Color, modifier: Modifier) -> String {
    let mut parts: Vec<String> = Vec::new();
    if modifier.contains(Modifier::BOLD) {
        parts.push("bold".into());
    }
    if modifier.contains(Modifier::DIM) {
        parts.push("dim".into());
    }
    if modifier.contains(Modifier::ITALIC) {
        parts.push("italic".into());
    }
    if modifier.contains(Modifier::UNDERLINED) {
        parts.push("underline".into());
    }
    if modifier.contains(Modifier::REVERSED) {
        parts.push("reversed".into());
    }
    if modifier.contains(Modifier::CROSSED_OUT) {
        parts.push("strikethrough".into());
    }
    if fg != Color::Reset {
        parts.push(format!("fg:{}", format_color(fg)));
    }
    if bg != Color::Reset {
        parts.push(format!("bg:{}", format_color(bg)));
    }
    if parts.is_empty() {
        "plain".to_string()
    } else {
        parts.join(",")
    }
}

/// `Color` 的短表示。`Color` 自带的 `Display` 是 `"Reset"` / `"Rgb(1, 2, 3)"`
/// 这种形式，作为基线太吵。
fn format_color(color: Color) -> String {
    match color {
        Color::Reset => "reset".into(),
        Color::Black => "black".into(),
        Color::Red => "red".into(),
        Color::Green => "green".into(),
        Color::Yellow => "yellow".into(),
        Color::Blue => "blue".into(),
        Color::Magenta => "magenta".into(),
        Color::Cyan => "cyan".into(),
        Color::Gray => "gray".into(),
        Color::DarkGray => "darkgray".into(),
        Color::White => "white".into(),
        Color::LightRed => "lred".into(),
        Color::LightGreen => "lgreen".into(),
        Color::LightYellow => "lyellow".into(),
        Color::LightBlue => "lblue".into(),
        Color::LightMagenta => "lmagenta".into(),
        Color::LightCyan => "lcyan".into(),
        Color::Rgb(r, g, b) => format!("rgb({r},{g},{b})"),
        Color::Indexed(i) => format!("idx({i})"),
    }
}

/// 把 `f` 画进一块 `w x h` 的 `TestBackend`，返回序列化后的快照文本。
///
/// 组件分两类：直接接受 `&mut Frame` 的（`render_input`、`render_status_bar` …）
/// 和只产出 `Vec<Line>` 的（`render_tool_entry_blocks`、`welcome_header_lines` …）。
/// 前者用这个，后者用 [`capture_lines`]。
pub fn capture<F>(w: u16, h: u16, draw: F) -> String
where
    F: FnOnce(&mut Frame<'_>),
{
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("TestBackend 构造失败");
    terminal.draw(draw).expect("TestBackend 绘制失败");
    serialize_buffer(terminal.backend().buffer())
}

/// 把 `Vec<Line>` 当作普通文本画进一块 `w x h` 的缓冲区，返回快照文本。
///
/// 很多组件不碰 `Frame`，只产出行。对这类组件走这条路比造一个假 `Frame` 更贴近
/// 真实渲染 —— `Paragraph` 的换行、截断、样式都会照常生效。
pub fn capture_lines(w: u16, h: u16, lines: &[Line<'_>]) -> String {
    capture(w, h, |f| {
        let area = Rect::new(0, 0, w, h);
        f.render_widget(Paragraph::new(Text::from(lines.to_vec())), area);
    })
}

/// 断言某次渲染的输出与基线一致。
///
/// 基线不存在时**写入并通过**（并在输出里提示），基线存在但不同时**失败并给出
/// unified diff**。确属预期的变更用 `UPDATE_SNAPSHOTS=1` 覆盖。
pub fn assert_render_named(name: &str, actual: &str) {
    let path = snapshot_path(name);
    let updating = matches!(
        std::env::var("UPDATE_SNAPSHOTS").as_deref(),
        Ok("1") | Ok("true") | Ok("always")
    ) || matches!(
        std::env::var("INSTA_UPDATE").as_deref(),
        Ok("always") | Ok("force")
    );

    let existing = std::fs::read_to_string(&path)
        .ok()
        // 兜底归一化行尾：`.gitattributes` 已经把 `*.snap` 钉成 eol=lf，但本地
        // git 配置（core.autocrlf）或手工编辑仍可能留下 CRLF。那样每行都会多一个
        // `\r`，逐行比较全盘报红 —— 这里读进来就洗掉，不指望仓库配置永远正确。
        .map(|s| s.replace("\r\n", "\n"));

    match existing {
        None => {
            write_snapshot(&path, actual);
            panic!(
                "已新建渲染基线 {}\n\n\
                 下面是刚写入的内容 —— **先确认它确实是你要的渲染**，再重跑一次让断言转绿、\n\
                 然后把 .snap 一起提交。\n\n{}",
                path.display(),
                actual
            );
        }
        Some(expected) if expected == actual => {}
        Some(expected) => {
            if updating {
                write_snapshot(&path, actual);
                return;
            }
            let diff = TextDiff::from_lines(&expected, actual);
            let mut report = String::new();
            for change in diff.iter_all_changes() {
                let sign = match change.tag() {
                    similar::ChangeTag::Delete => "-",
                    similar::ChangeTag::Insert => "+",
                    similar::ChangeTag::Equal => " ",
                };
                report.push_str(sign);
                report.push_str(change.value());
                report.push('\n');
            }
            panic!(
                "渲染基线不匹配: {}\n{}\n\
                 如果这是预期改动，执行：UPDATE_SNAPSHOTS=1 cargo test --lib",
                path.display(),
                report
            );
        }
    }
}

fn snapshot_path(name: &str) -> std::path::PathBuf {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    root.join(SNAPSHOT_DIR).join(format!("{name}.snap"))
}

fn write_snapshot(path: &std::path::Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("创建基线目录失败");
    }
    std::fs::write(path, contents).expect("写入渲染基线失败");
}

/// 断言一次 `Frame` 渲染的输出与基线一致。
///
/// ```ignore
/// assert_render!("chat_input_default", 80, 24, |f| {
///     render_input(f, &state, Rect::new(0, 0, 80, 24));
/// });
/// ```
#[macro_export]
macro_rules! assert_render {
    ($name:expr, $w:expr, $h:expr, $draw:expr) => {{
        let actual = $crate::ui::test_support::capture($w, $h, $draw);
        $crate::ui::test_support::assert_render_named($name, &actual);
    }};
}

/// 断言一组 `Line` 渲染出的文本与基线一致。同 [`assert_render!`]，但组件只产出
/// 行、不接受 `Frame` 时用这个。
#[macro_export]
macro_rules! assert_lines_render {
    ($name:expr, $w:expr, $h:expr, $lines:expr) => {{
        let actual = $crate::ui::test_support::capture_lines($w, $h, $lines);
        $crate::ui::test_support::assert_render_named($name, &actual);
    }};
}
