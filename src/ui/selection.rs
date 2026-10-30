//! 选区：一份数据、一个模型、一处复制入口。
//!
//! # 以前散在哪儿
//!
//! - `state/store.rs` —— `TextSelection`、`get_selected_text()`、`char_index_at_visual_col()`
//! - `components/chat_history.rs` —— `apply_selection_highlight()`、`split_by_width()`
//! - `events/mouse.rs` —— 拖选、鼠标松开即复制
//! - `events/input.rs` —— Ctrl+C 复制
//!
//! 同一套"起止 → 范围 → 文本"的换算在三个文件里各写了一遍，度量口径还不完全
//! 一致（有的按字符累加、有的按整个 span 宽度切），于是中文和框线字符一出现，
//! 高亮范围和复制出来的文本就对不上；范围解析还带着 `Option` 比较 + `unwrap()`，
//! 四个字段缺一个就 panic。
//!
//! # 现在
//!
//! 换算只在这里，列宽口径只在 [`crate::ui::utils::text`]。UI 的其它部分要复制
//! 东西，直接复用 [`selected_text`] / [`entry_text`] / [`copy_selection`]：
//!
//! ```ignore
//! // 复制鼠标选中的那一段
//! crate::ui::selection::copy_selection(state)?;
//!
//! // 复制某个条目（代码块、工具结果）的全文
//! let text = crate::ui::selection::entry_text(state, entry_idx);
//! ```

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;

use crate::ui::state::store::TextSelection;
use crate::ui::state::ChatState;
use crate::ui::utils::text::{char_index_at_visual_col, line_plain_text, split_by_width};

/// 选区的一端：**条目下标 + 条目内渲染行号 + 可视列**。
///
/// 三个坐标都必须是"渲染坐标系"里的：条目下标是 `chat_history` 的下标，行号是
/// 该条目**渲染后**的行号（不是文件行号），列是可视列（CJK 占 2 列）。
pub type Anchor = (usize, usize, usize);

/// 解析后的选区：起点一定 ≤ 终点（按 条目 → 行 → 列 的字典序）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionRange {
    pub start: Anchor,
    pub end: Anchor,
}

impl SelectionRange {
    /// 从状态里的选区解析出范围。
    ///
    /// 以前这里是 `Option<usize>` 比大小再 `unwrap()` —— 四个字段只要缺一个
    /// （流式更新中途清掉一半、会话恢复）就直接 panic。现在缺哪个都不给范围，
    /// 调用方按"没有选区"处理。
    pub fn resolve(selection: &TextSelection) -> Option<Self> {
        let (start, end) = (selection.start?, selection.end?);
        let (start_entry, end_entry) = (selection.start_entry_idx?, selection.end_entry_idx?);

        let forward = (start_entry, start.0, start.1) <= (end_entry, end.0, end.1);
        let (a, b) = if forward {
            ((start_entry, start.0, start.1), (end_entry, end.0, end.1))
        } else {
            ((end_entry, end.0, end.1), (start_entry, start.0, start.1))
        };

        Some(SelectionRange { start: a, end: b })
    }

    /// 这个位置是否落在选区内。
    pub fn contains(&self, entry_idx: usize, row: usize, col: usize) -> bool {
        let (start, end) = (self.start, self.end);
        if entry_idx < start.0 || entry_idx > end.0 {
            return false;
        }
        if entry_idx == start.0 && entry_idx == end.0 {
            if row < start.1 || row > end.1 {
                return false;
            }
            if row == start.1 && col < start.2 {
                return false;
            }
            if row == end.1 && col > end.2 {
                return false;
            }
            return true;
        }
        if entry_idx == start.0 {
            return row > start.1 || (row == start.1 && col >= start.2);
        }
        if entry_idx == end.0 {
            return row < end.1 || (row == end.1 && col <= end.2);
        }
        true
    }
}

/// 取某个条目已渲染的行。
///
/// 优先用 `rendered_cache`（渲染期写好的）；缓存缺失时按需重渲染一次 —— 流式
/// 更新刚把缓存清掉时，复制不能静默失败。
pub fn entry_rendered_lines(state: &ChatState, entry_idx: usize) -> Option<Vec<Line<'static>>> {
    if let Some((_, lines)) = state.rendered_cache.get(&entry_idx) {
        return Some(lines.clone());
    }
    if entry_idx >= state.chat_history.len() {
        return None;
    }
    let width = state.last_chat_area.map(|a| a.width).unwrap_or(80);
    Some(crate::ui::components::chat_history::render_entry_lines(
        state, entry_idx, width,
    ))
}

/// 取出选区里的纯文本。
///
/// `lines_for` 按条目下标给出该条目的渲染行；返回 `None` 表示这一段拿不到
/// （条目已被移除等），此时跳过该条目而不是整段失败。
pub fn selected_text<F>(range: &SelectionRange, mut lines_for: F) -> Option<String>
where
    F: FnMut(usize) -> Option<Vec<Line<'static>>>,
{
    let mut out = String::new();
    let mut any = false;

    for entry_idx in range.start.0..=range.end.0 {
        let Some(lines) = lines_for(entry_idx) else {
            continue;
        };
        let row_start = if entry_idx == range.start.0 {
            range.start.1
        } else {
            0
        };
        let row_end = if entry_idx == range.end.0 {
            range.end.1.min(lines.len().saturating_sub(1))
        } else {
            lines.len().saturating_sub(1)
        };

        for (i, line) in lines.iter().enumerate().take(row_end + 1).skip(row_start) {
            let text = line_plain_text(line);
            let col_start = if entry_idx == range.start.0 && i == range.start.1 {
                char_index_at_visual_col(&text, range.start.2)
            } else {
                0
            };
            // 终点列是闭区间（含末字符），换算时 +1 才包含最后一个字符。
            let col_end = if entry_idx == range.end.0 && i == range.end.1 {
                char_index_at_visual_col(&text, range.end.2.saturating_add(1))
            } else {
                text.chars().count()
            };

            let selected: String = text
                .chars()
                .skip(col_start)
                .take(col_end.saturating_sub(col_start))
                .collect();
            if !selected.is_empty() {
                if any {
                    out.push('\n');
                }
                out.push_str(&selected);
                any = true;
            }
        }
    }

    any.then_some(out)
}

/// 某个条目的全文（渲染后的纯文本）。
///
/// "复制这个代码块 / 这个工具结果"直接用它 —— 和选区复制共用同一套渲染与
/// 列宽口径，不会出现"屏幕上是一份、复制出来是另一份"。
pub fn entry_text(state: &ChatState, entry_idx: usize) -> Option<String> {
    let lines = entry_rendered_lines(state, entry_idx)?;
    let text: Vec<String> = lines
        .iter()
        .map(line_plain_text)
        .filter(|line| !line.trim().is_empty())
        .collect();
    if text.is_empty() {
        None
    } else {
        Some(text.join("\n"))
    }
}

/// 给落在选区里的字符上高亮。
///
/// `lines` 是**某一个条目**的渲染行；`entry_idx` 是这个条目在 `chat_history`
/// 里的下标。
pub fn apply_highlight(lines: &mut [Line<'_>], range: &SelectionRange, entry_idx: usize) {
    let (start_entry, end_entry) = (range.start.0, range.end.0);
    if entry_idx < start_entry || entry_idx > end_entry {
        return;
    }

    for (i, line) in lines.iter_mut().enumerate() {
        let row = i;

        // 1. 这一行被选中的列区间
        let (sel_start_col, sel_end_col) = if entry_idx == start_entry && entry_idx == end_entry {
            if row < range.start.1 || row > range.end.1 {
                continue;
            }
            let s = if row == range.start.1 {
                range.start.2
            } else {
                0
            };
            let e = if row == range.end.1 {
                range.end.2
            } else {
                usize::MAX
            };
            (s, e)
        } else if entry_idx == start_entry {
            if row < range.start.1 {
                continue;
            }
            let s = if row == range.start.1 {
                range.start.2
            } else {
                0
            };
            (s, usize::MAX)
        } else if entry_idx == end_entry {
            if row > range.end.1 {
                continue;
            }
            let e = if row == range.end.1 {
                range.end.2
            } else {
                usize::MAX
            };
            (0, e)
        } else {
            (0, usize::MAX)
        };

        // 2. 只在有交集的行上重建 spans
        let mut new_spans: Vec<ratatui::text::Span<'_>> = Vec::with_capacity(line.spans.len() * 2);
        let mut current_col = 0usize;

        for span in &line.spans {
            let span_text = span.content.as_ref();
            let span_len = crate::ui::utils::render::display_width(span_text);
            let span_end_col = current_col + span_len;

            if span_end_col <= sel_start_col || current_col > sel_end_col {
                new_spans.push(span.clone());
            } else {
                // 相对切点：本 span 内从第几列开始被选中。
                // 当前列已经在选区左边时 rel_start = 0（整段都在选区内）。
                let rel_start = sel_start_col.saturating_sub(current_col).min(span_len);
                // sel_end_col 是闭区间；换算成开区间时要防溢出。
                let sel_end_exclusive = sel_end_col.saturating_add(1);
                let rel_end = if span_end_col > sel_end_exclusive {
                    sel_end_exclusive.saturating_sub(current_col)
                } else {
                    span_len
                };

                let (prefix, selected, suffix) = split_by_width(span_text, rel_start, rel_end);

                if !prefix.is_empty() {
                    new_spans.push(ratatui::text::Span::styled(prefix, span.style));
                }
                if !selected.is_empty() {
                    new_spans.push(ratatui::text::Span::styled(
                        selected,
                        span.style
                            .bg(Color::Blue)
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ));
                }
                if !suffix.is_empty() {
                    new_spans.push(ratatui::text::Span::styled(suffix, span.style));
                }
            }
            current_col += span_len;
        }
        line.spans = new_spans;
    }
}

/// **唯一的复制入口**：鼠标松开、Ctrl+C，以及将来的"复制代码块"都走这里。
///
/// 只负责"把选区里的文字写进剪贴板"；要不要清选区、怎么提示，由调用方决定
/// （鼠标松开要保留高亮让用户看见复制了哪一段，Ctrl+C 则清掉）。
pub fn copy_selection(state: &mut ChatState) -> Result<(), String> {
    let Some(range) = SelectionRange::resolve(&state.text_selection) else {
        return Err("选区为空".to_string());
    };
    let text = selected_text(&range, |entry_idx| entry_rendered_lines(state, entry_idx))
        .ok_or_else(|| "选区为空".to_string())?;
    crate::ui::utils::clipboard::copy_to_clipboard(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> Line<'static> {
        Line::from(ratatui::text::Span::raw(text.to_string()))
    }

    /// 范围解析不能 panic：四个字段缺一个都不给范围。
    ///
    /// 以前这里是 `Option` 比大小 + `unwrap()`，缺一个就是 panic。
    #[test]
    fn resolving_an_incomplete_selection_yields_nothing() {
        let mut sel = TextSelection::new();
        sel.start = Some((0, 0));
        sel.end = Some((0, 3));
        // start_entry_idx / end_entry_idx 仍是 None
        assert_eq!(SelectionRange::resolve(&sel), None);
    }

    /// 反向拖选（从下往上）也要给出正确的起止。
    #[test]
    fn a_backwards_drag_resolves_to_the_same_range() {
        let mut sel = TextSelection::new();
        sel.start_selection(2, 5, 10);
        sel.update_selection(0, 1, 3);
        let range = SelectionRange::resolve(&sel).expect("应当解析出范围");
        assert_eq!(range.start, (0, 1, 3));
        assert_eq!(range.end, (2, 5, 10));
    }

    /// 选区文本要按可视列切，CJK 占 2 列不能按字符数切。
    #[test]
    fn selected_text_cuts_by_visual_column() {
        let mut sel = TextSelection::new();
        sel.start_selection(0, 0, 0);
        sel.update_selection(0, 0, 3); // "中文" = 4 列，取到第 4 列
        let range = SelectionRange::resolve(&sel).expect("范围");

        let text = selected_text(&range, |_| Some(vec![line("中文abc")])).expect("文本");
        assert_eq!(text, "中文");
    }

    /// 多条目选区：首条从起始列切，末条到结束列切，中间整条。
    #[test]
    fn selected_text_spans_multiple_entries() {
        let mut sel = TextSelection::new();
        sel.start_selection(0, 0, 2);
        sel.update_selection(2, 0, 3);
        let range = SelectionRange::resolve(&sel).expect("范围");

        let text = selected_text(&range, |idx| {
            Some(vec![line(match idx {
                0 => "aaaa",
                1 => "bbbb",
                _ => "cccc",
            })])
        })
        .expect("文本");
        // 终点列是闭区间：end_col = 3 表示第 4 列也在选中范围内。
        assert_eq!(text, "aa\nbbbb\ncccc");
    }

    /// 起点就在本段开头时，选中段不能为空。
    ///
    /// 回归：`split_by_width` 以前一律先进"前缀"分支，而 `in_middle` 只在前缀
    /// 分支里被置位 —— `start_width == 0` 时选中段恒为空，整段文字掉进后缀，
    /// 于是**选区起点落在某个 span 开头时屏幕上一点高亮都没有**（拖选看着像
    /// 没生效）。
    #[test]
    fn a_selection_starting_at_the_beginning_still_highlights() {
        let (prefix, middle, suffix) = crate::ui::utils::text::split_by_width("abcdef", 0, 3);
        assert_eq!(prefix, "");
        assert_eq!(middle, "abc");
        assert_eq!(suffix, "def");
    }

    /// 选区起点落在某个 span 中间时，该 span 的前半段不该被高亮，
    /// 后面的整段 span 也不该丢高亮。
    ///
    /// 回归：`rel_start` 算成了 `current_col - sel_start_col`（方向反了），
    /// 于是起点在 span 中间时切点变成 0，整段都被判成"不在选区内"。
    #[test]
    fn a_selection_starting_inside_a_span_keeps_the_rest_highlighted() {
        let mut sel = TextSelection::new();
        sel.start_selection(0, 0, 2); // 从第 3 列开始
        sel.update_selection(0, 0, 4); // 到第 5 列
        let range = SelectionRange::resolve(&sel).expect("范围");

        let mut lines = vec![line("abcdef")];
        apply_highlight(&mut lines, &range, 0);

        // 切成 "ab"（未选） + "cde"（选中） + "f"（未选）
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "abcdef");
        assert_eq!(lines[0].spans.len(), 3, "实际：{:?}", lines[0].spans);
        let selected = &lines[0].spans[1];
        assert_eq!(selected.content.as_ref(), "cde");
        assert!(selected.style.bg.is_some_and(|c| c == Color::Blue));
    }

    /// 高亮只作用在选区内的字符上。
    #[test]
    fn highlight_only_touches_the_selected_part() {
        let mut sel = TextSelection::new();
        sel.start_selection(0, 0, 0);
        sel.update_selection(0, 0, 2);
        let range = SelectionRange::resolve(&sel).expect("范围");

        let mut lines = vec![line("abcdef")];
        apply_highlight(&mut lines, &range, 0);

        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "abcdef");
        // 前三个字符被拆成高亮段，后三个保持原样。
        assert_eq!(lines[0].spans.len(), 2);
        assert!(lines[0].spans[0].style.bg.is_some_and(|c| c == Color::Blue));
    }
}
